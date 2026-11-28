//! The web add-on for stagetui: `StagePane("web", url="https://example.com")`.
//!
//! Your own Chrome, Chromium, Edge or Brave runs hidden (headless) at the
//! pane's size. Chrome's screencast sends a picture whenever the page changes;
//! clicks, the wheel and typing go back to the page. Nothing is downloaded.
//!
//! Keys while the pane has focus go to the page — as real key presses, in any
//! language — except: Ctrl+L type an address (shown at the bottom; Enter goes
//! there, Esc never minds), Alt+← / Alt+→ back and forward, F5 reload. The app
//! can give those other keys or turn them off: `keys={"reload": None}`.
//!
//! While its pane is out of sight the page rests: it is frozen (no scripts,
//! timers, pictures or sound — Web Audio and cross-site frames included), and
//! going to another address waits until it's shown again.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chromiumoxide::cdp::browser_protocol::browser::{
    Bounds, GetWindowForTargetParams, SetWindowBoundsParams, WindowState,
};
use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType,
    InsertTextParams, MouseButton,
};
use chromiumoxide::cdp::browser_protocol::page::{
    AddScriptToEvaluateOnNewDocumentParams, EventFrameNavigated, EventLoadEventFired,
    EventScreencastFrame, NavigateParams, ReloadParams, ScreencastFrameAckParams,
    SetWebLifecycleStateParams, SetWebLifecycleStateState, StartScreencastFormat,
    StartScreencastParams, StopScreencastParams,
};
use chromiumoxide::cdp::browser_protocol::target::SessionId;
use chromiumoxide::conn::Connection;
use chromiumoxide::types::{CallId, CdpJsonEventMessage, Message};
use chromiumoxide::{Browser, BrowserConfig, Command, Page};
use futures::StreamExt;
use serde_json::{Value, json};
use stagetui_plugin_api::{
    Describe, Frame, KeyAction, Keymap, Mouse, PaneSource, Ready, Start, serve,
};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

fn main() {
    std::process::exit(serve(Web::default()));
}

/// The pane's own keys; the app can give them other keys:
/// `StagePane("web", keys={"address": "ctrl+o", "reload": None})`. Every other
/// key (and one renamed away) goes to the page.
const ACTIONS: &[KeyAction] = &[
    KeyAction {
        name: "address",
        keys: &["ctrl+l"],
        about: "type an address at the bottom; Enter goes there",
    },
    KeyAction {
        name: "back",
        keys: &["alt+left"],
        about: "back to the page before",
    },
    KeyAction {
        name: "forward",
        keys: &["alt+right"],
        about: "forward again",
    },
    KeyAction {
        name: "reload",
        keys: &["f5", "ctrl+r"],
        about: "load the page again",
    },
];

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ----- finding a browser -----------------------------------------------------

fn on_path(program: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{program}.exe")
    } else {
        program.to_string()
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
}

/// Chrome, Chromium, Edge or Brave: `STAGETUI_CHROME`, then the usual places.
fn find_browser() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("STAGETUI_CHROME").filter(|v| !v.is_empty()) {
        return Some(p.into());
    }
    let mut places: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        for (app, exe) in [
            ("Google Chrome", "Google Chrome"),
            ("Chromium", "Chromium"),
            ("Microsoft Edge", "Microsoft Edge"),
            ("Brave Browser", "Brave Browser"),
        ] {
            places.push(format!("/Applications/{app}.app/Contents/MacOS/{exe}").into());
        }
    }
    if cfg!(windows) {
        for base in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(b) = std::env::var_os(base) {
                let b = PathBuf::from(b);
                places.push(b.join(r"Google\Chrome\Application\chrome.exe"));
                places.push(b.join(r"Microsoft\Edge\Application\msedge.exe"));
                places.push(b.join(r"Chromium\Application\chrome.exe"));
                places.push(b.join(r"BraveSoftware\Brave-Browser\Application\brave.exe"));
            }
        }
    }
    if let Some(found) = places.into_iter().find(|p| p.is_file()) {
        return Some(found);
    }
    [
        "chromium",
        "chromium-browser",
        "google-chrome",
        "google-chrome-stable",
        "microsoft-edge",
        "microsoft-edge-stable",
        "brave",
        "brave-browser",
        "chrome",
        "msedge",
    ]
    .iter()
    .find_map(|p| on_path(p))
}

fn no_browser_help() -> String {
    if cfg!(target_os = "macos") {
        "no Chrome, Chromium, Edge or Brave found — install Google Chrome (or set STAGETUI_CHROME)"
    } else if cfg!(windows) {
        "no Edge or Chrome found — Edge comes with Windows; or set STAGETUI_CHROME"
    } else {
        "no Chrome or Chromium found — install Chromium (e.g. sudo pacman -S chromium, sudo apt install chromium) or set STAGETUI_CHROME"
    }
    .into()
}

// ----- small helpers ---------------------------------------------------------

/// Remove browser profiles left by add-ons that were stopped hard. Each folder
/// is named after its process; on Linux we can tell whether that one still runs.
fn tidy_old_profiles() {
    if !cfg!(target_os = "linux") {
        return;
    }
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(pid) = name
            .strip_prefix("stagetui-web-")
            .and_then(|rest| rest.split('-').next())
            .and_then(|p| p.parse::<u32>().ok())
        else {
            continue;
        };
        if pid != std::process::id() && !Path::new(&format!("/proc/{pid}")).exists() {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Standard base64 (screencast pictures arrive this way).
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    }
    let clean: Vec<u8> = text
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
        .collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= value(*c)? << (18 - 6 * i);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..chunk.len().saturating_sub(1)]);
    }
    Some(out)
}

/// A file path → a `file://` URL (Windows' `\\?\C:\…` form becomes `file:///C:/…`).
fn file_url(path: &str) -> String {
    let plain = path
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .unwrap_or_else(|| path.strip_prefix(r"\\?\").unwrap_or(path).to_string());
    let slashed = plain.replace('\\', "/");
    if let Some(share) = slashed.strip_prefix("//") {
        format!("file://{share}")
    } else if slashed.starts_with('/') {
        format!("file://{slashed}")
    } else {
        format!("file:///{slashed}")
    }
}

/// What someone typed in the address line → a URL.
fn to_url(text: &str) -> String {
    let t = text.trim();
    if t.contains("://") || t.starts_with("about:") || t.starts_with("data:") {
        return t.to_string();
    }
    let path = Path::new(t);
    if t.starts_with('/') || t.starts_with("./") || t.starts_with('~') || path.exists() {
        let full = if let Some(rest) = t.strip_prefix("~/") {
            std::env::var_os("HOME")
                .map_or_else(|| PathBuf::from(t), |h| PathBuf::from(h).join(rest))
        } else {
            std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        };
        return file_url(&full.display().to_string());
    }
    format!("https://{t}")
}

/// Keys with names: stagetui's name, the page's `key` (and `code`), and the
/// Windows virtual key code browsers want.
const NAMED: &[(&str, &str, i64)] = &[
    ("enter", "Enter", 13),
    ("tab", "Tab", 9),
    ("esc", "Escape", 27),
    ("backspace", "Backspace", 8),
    ("delete", "Delete", 46),
    ("insert", "Insert", 45),
    ("left", "ArrowLeft", 37),
    ("right", "ArrowRight", 39),
    ("up", "ArrowUp", 38),
    ("down", "ArrowDown", 40),
    ("home", "Home", 36),
    ("end", "End", 35),
    ("pageup", "PageUp", 33),
    ("pagedown", "PageDown", 34),
];

/// Where a character sits on a US keyboard: its `code` and virtual key code
/// (`None`: not on it — Arabic, Japanese… are typed as text alone).
fn us_key(c: char) -> Option<(String, i64)> {
    let upper = c.to_ascii_uppercase();
    if upper.is_ascii_uppercase() {
        return Some((format!("Key{upper}"), i64::from(upper as u8)));
    }
    if let Some(d) = c.to_digit(10) {
        return Some((format!("Digit{d}"), 48 + i64::from(d)));
    }
    let (code, vk) = match c {
        '-' | '_' => ("Minus", 189),
        '=' | '+' => ("Equal", 187),
        '[' | '{' => ("BracketLeft", 219),
        ']' | '}' => ("BracketRight", 221),
        '\\' | '|' => ("Backslash", 220),
        ';' | ':' => ("Semicolon", 186),
        '\'' | '"' => ("Quote", 222),
        ',' | '<' => ("Comma", 188),
        '.' | '>' => ("Period", 190),
        '/' | '?' => ("Slash", 191),
        '`' | '~' => ("Backquote", 192),
        '!' => ("Digit1", 49),
        '@' => ("Digit2", 50),
        '#' => ("Digit3", 51),
        '$' => ("Digit4", 52),
        '%' => ("Digit5", 53),
        '^' => ("Digit6", 54),
        '&' => ("Digit7", 55),
        '*' => ("Digit8", 56),
        '(' => ("Digit9", 57),
        ')' => ("Digit0", 48),
        _ => return None,
    };
    Some((code.into(), vk))
}

/// A key as stagetui names it (`"a"`, `"shift+tab"`, `"ctrl+l"`, `"م"`) as a real
/// key press for the page: its `keydown` listeners hear it, Tab moves between
/// fields, Esc reaches it. What the key types — in any language — goes along as
/// `text`. `None`: not a key a page knows.
fn press(name: &str, text: Option<&str>) -> Option<DispatchKeyEventParams> {
    let mut base = name;
    let mut modifiers = 0i64;
    while let Some((held, bit)) = [("alt+", 1), ("ctrl+", 2), ("super+", 4), ("shift+", 8)]
        .into_iter()
        .find(|(held, _)| base.len() > held.len() && base.starts_with(held))
    {
        modifiers |= bit;
        base = &base[held.len()..];
    }
    let plain = modifiers & 7 == 0; // no Ctrl, Alt or Super: it types
    let (key, code, vk, typed) = if let Some((_, key, vk)) = NAMED.iter().find(|(n, ..)| *n == base)
    {
        let typed = (base == "enter" && plain).then(|| "\r".to_string());
        (key.to_string(), Some(key.to_string()), *vk, typed)
    } else if base == "space" {
        let typed = plain.then(|| " ".to_string());
        (" ".into(), Some("Space".into()), 32, typed)
    } else if let Some(n) = base.strip_prefix('f').and_then(|n| n.parse::<i64>().ok()) {
        if !(1..=24).contains(&n) {
            return None;
        }
        (format!("F{n}"), Some(format!("F{n}")), 111 + n, None)
    } else {
        let mut chars = base.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            return None;
        };
        let shown = if modifiers & 8 != 0 {
            c.to_uppercase().collect()
        } else {
            c.to_string()
        };
        let typed = text.map(String::from).filter(|_| plain);
        let (code, vk) = us_key(c).map_or((None, 0), |(code, vk)| (Some(code), vk));
        (typed.clone().unwrap_or(shown), code, vk, typed)
    };
    // With text it's a key that types (keydown, keypress, input); without, a raw one.
    let mut p = DispatchKeyEventParams::new(if typed.is_some() {
        DispatchKeyEventType::KeyDown
    } else {
        DispatchKeyEventType::RawKeyDown
    });
    p.modifiers = Some(modifiers);
    p.key = Some(key);
    p.code = code;
    p.windows_virtual_key_code = Some(vk);
    p.native_virtual_key_code = Some(vk);
    p.unmodified_text.clone_from(&typed);
    p.text = typed;
    Some(p)
}

// ----- talking to the browser ------------------------------------------------

/// Going somewhere. While the pane is hidden these wait until it's shown.
enum Go {
    To(String),
    Back,
    Forward,
    Reload,
}

enum Cmd {
    Go(Go),
    Resize(u32, u32),
    Mouse(DispatchMouseEventType, f64, f64, Option<MouseButton>, f64),
    Key(DispatchKeyEventParams),
    Text(String),
    Eval(String, std_mpsc::Sender<Result<Value, String>>),
    /// The pane went out of sight (`false`) or came back.
    Shown(bool),
    Stop,
}

/// The screencast follows the pane: its size, and off while it's out of sight.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cast {
    size: (u32, u32),
    hidden: bool,
}

impl Cast {
    /// The pane changed size: `true` if the screencast must restart at it.
    fn resize(&mut self, size: (u32, u32)) -> bool {
        self.size = size;
        !self.hidden
    }

    /// The pane hid or came back: `Some(true)` start the screencast (at the
    /// latest size), `Some(false)` stop it, `None` nothing changed.
    fn shown(&mut self, shown: bool) -> Option<bool> {
        if self.hidden != shown {
            return None;
        }
        self.hidden = !shown;
        Some(shown)
    }
}

#[derive(Default)]
struct Shared {
    frame: Option<Frame>,
    url: String,
    title: String,
    loading: bool,
    events: Vec<(String, Value)>,
    trouble: Option<String>,
    /// Asked to rest or wake, the page didn't answer: it may still be playing.
    stuck: bool,
}

/// How long one command to the page may take. A busy or resting page mustn't
/// hold up pictures, input and events; what times out is let go.
const QUICK: Duration = Duration::from_millis(500);

/// Send a command; `true` if the page answered in time.
async fn quick<T: Command>(page: &Page, cmd: T) -> bool {
    matches!(
        tokio::time::timeout(QUICK, page.execute(cmd)).await,
        Ok(Ok(_))
    )
}

async fn screencast(page: &Page, w: u32, h: u32) {
    let p = StartScreencastParams {
        format: Some(StartScreencastFormat::Jpeg),
        quality: Some(80),
        max_width: Some(i64::from(w)),
        max_height: Some(i64::from(h)),
        every_nth_frame: Some(1),
    };
    quick(page, p).await;
}

/// The page's size, in CSS pixels, for a pane of `w × h` pixels.
async fn size_page(page: &Page, (w, h): (u32, u32), zoom: f64) {
    let css = ((f64::from(w) / zoom) as i64, (f64::from(h) / zoom) as i64);
    let p = SetDeviceMetricsOverrideParams::new(css.0.max(1), css.1.max(1), zoom, false);
    quick(page, p).await;
}

/// In every frame, before the page's own scripts:
/// - remember each `<video>` / `<audio>` that is played, even one never put on
///   the page (`new Audio()`), so `HUSH_MEDIA` can find it;
/// - a link or form that would open a new window opens in this one instead
///   (`_self` at the top, `_top` in a frame of the same site), so a form keeps
///   what it sends. A new window the page still opens (`window.open`, a frame of
///   another site) is shown by the watcher.
const IN_EVERY_FRAME: &str = r"void (() => {
  if (window.__stagetuiMedia || !window.HTMLMediaElement) return;
  const media = new Set(), known = new WeakSet();
  Object.defineProperty(window, '__stagetuiMedia', { value: media });
  const P = HTMLMediaElement.prototype, play = P.play;
  P.play = function (...args) {
    if (!known.has(this)) { known.add(this); media.add(new WeakRef(this)); }
    return play.apply(this, args);
  };
  // Where a new window goes instead (null: leave it to the browser).
  const instead = () => {
    if (window === top) return '_self';
    try {
      for (let w = window; w !== top; w = w.parent) {
        const el = w.frameElement;
        if (!el) return null;
        const sandbox = el.getAttribute('sandbox');
        if (sandbox !== null && !/allow-top-navigation/.test(sandbox)) return null;
      }
      return '_top';
    } catch (e) { return null; }
  };
  const opensNew = (target) => {
    const t = (target || '').trim(), k = t.toLowerCase();
    if (!t || k === '_self' || k === '_parent' || k === '_top') return false;
    if (k === '_blank') return true;
    try {
      for (let w = window; ; w = w.parent) {
        for (const f of w.document.querySelectorAll('iframe, frame')) if (f.name === t) return false;
        if (w === w.parent) return true;
      }
    } catch (e) { return false; }
  };
  const base = (doc) => { const b = doc.querySelector('base[target]'); return b ? b.getAttribute('target') : ''; };
  const retarget = (el, attr) => {
    const to = instead();
    if (to && opensNew(el.getAttribute(attr) || base(el.ownerDocument))) el.setAttribute(attr, to);
  };
  const form = (f, by) => {
    if (by && by.hasAttribute('formtarget')) retarget(by, 'formtarget'); else retarget(f, 'target');
  };
  addEventListener('submit', (e) => form(e.target, e.submitter), true);
  const F = HTMLFormElement.prototype, submit = F.submit;
  F.submit = function (...args) { form(this, null); return submit.apply(this, args); };
  addEventListener('click', (e) => {
    // Ctrl/Shift/middle-click ask for a new window on purpose: the watcher shows it.
    if (e.button !== 0 || e.ctrlKey || e.metaKey || e.shiftKey) return;
    const a = e.target && e.target.closest && e.target.closest('a[href], area[href]');
    if (a && !a.hasAttribute('download')) retarget(a, 'target');
  }, true);
})()";

/// Going out of sight: quiet every playing `<video>` / `<audio>` (on the page or
/// only in script) and speech, in this frame and the same-origin frames below it,
/// remembering them so they play on later. Until shown, `play()` and
/// `AudioContext.resume()` wait, a new AudioContext starts suspended, and media
/// that starts by itself is paused. Run again, it only sweeps.
const HUSH_MEDIA: &str = r"void (() => {
  let h = window.__stagetuiHidden;
  if (!h) {
    h = { media: new Set(), audio: new Set(), speech: new Set(), later: [], wins: new Set(), undo: [] };
    h.hush = (m) => { if (!m.paused) { m.pause(); h.media.add(m); } };
    h.onPlay = (e) => h.hush(e.target);
  }
  const swap = (obj, name, make) => {
    const real = obj && obj[name];
    if (typeof real !== 'function') return;
    h.undo.push(() => { obj[name] = real; });
    obj[name] = make(real);
  };
  const wait = (real) => function (...args) {
    return new Promise((ok, no) => h.later.push(() => real.apply(this, args).then(ok, no)));
  };
  const visit = (win) => {
    let doc = null;
    try { doc = win.document; } catch (e) {}
    if (doc) {
      if (!h.wins.has(win)) {
        h.wins.add(win);
        win.__stagetuiHidden = h;
        doc.addEventListener('play', h.onPlay, true);
        swap(win.HTMLMediaElement && win.HTMLMediaElement.prototype, 'play', wait);
        for (const name of ['AudioContext', 'webkitAudioContext']) {
          if (typeof win[name] !== 'function') continue;
          swap(win[name].prototype, 'resume', wait);
          swap(win, name, (Real) => {
            const Quiet = function (...args) { const c = new Real(...args); c.suspend(); h.audio.add(c); return c; };
            Quiet.prototype = Real.prototype;
            return Quiet;
          });
        }
      }
      doc.querySelectorAll('video, audio').forEach((m) => h.hush(m));
      const seen = win.__stagetuiMedia;
      for (const r of seen || []) { const m = r.deref(); if (m) h.hush(m); else seen.delete(r); }
      const s = win.speechSynthesis;
      if (s && s.speaking && !s.paused) { s.pause(); h.speech.add(s); }
    }
    let n = 0;
    try { n = win.frames.length; } catch (e) {}
    for (let i = 0; i < n; i++) { try { visit(win.frames[i]); } catch (e) {} }
  };
  visit(window);
  for (const c of h.audio) if (c.state === 'running') c.suspend();
})()";

/// Back in sight: everything as it was, and play on what `HUSH_MEDIA` quieted —
/// plus what was asked to play meanwhile.
const WAKE_MEDIA: &str = r"void (() => {
  const h = window.__stagetuiHidden;
  if (!h) return;
  for (const win of h.wins) {
    try { delete win.__stagetuiHidden; win.document.removeEventListener('play', h.onPlay, true); } catch (e) {}
  }
  for (const undo of h.undo.reverse()) { try { undo(); } catch (e) {} }
  for (const s of h.speech) s.resume();
  for (const c of h.audio) if (c.state === 'suspended') c.resume().catch(() => {});
  for (const m of h.media) if (m.paused) m.play()?.catch(() => {});
  for (const go of h.later) { try { go(); } catch (e) {} }
})()";

/// Shown when the page didn't answer when it was told to rest (or wake).
const NOT_RESPONDING: &str =
    "⚠ This page isn't responding — it may keep playing; close the pane to stop it";

/// How long a resting or waking page may take to answer before we say it isn't
/// responding.
const PATIENT: Duration = Duration::from_secs(2);

/// `true` if `job` finished within `time`.
async fn answered<F: std::future::Future>(time: Duration, job: F) -> bool {
    tokio::time::timeout(time, job).await.is_ok()
}

/// Run page script on a best-effort basis: if it fails there is nothing to do
/// about it, and a busy page isn't waited for long. `true` if the page answered.
async fn try_script(page: &Page, js: &str) -> bool {
    answered(QUICK, page.evaluate(js)).await
}

/// Asked of the watcher: run this script in every cross-site frame; answer
/// whether they all did in time.
struct Everywhere(&'static str, oneshot::Sender<bool>);

/// Run `js` in every cross-site frame (through the watcher). `true` if each
/// answered in time — or there is no watcher to ask.
async fn in_other_sites(asks: &UnboundedSender<Everywhere>, js: &'static str) -> bool {
    let (tx, rx) = oneshot::channel();
    if asks.send(Everywhere(js, tx)).is_err() {
        return true;
    }
    matches!(
        tokio::time::timeout(PATIENT + QUICK, rx).await,
        Ok(Ok(true))
    )
}

/// An address worth showing (a new window starts out blank, then goes somewhere).
fn real_url(url: &str) -> bool {
    !url.is_empty() && url != "about:blank"
}

/// Our own line to the browser, beside chromiumoxide's. It does two things
/// chromiumoxide can't:
/// - it reaches into cross-site frames, which run in processes of their own: it
///   puts `IN_EVERY_FRAME` in each, and runs scripts there when asked;
/// - it watches for new windows the page opens (a link with `target=_blank`,
///   `window.open`) — they can't be seen, so it closes each and sends its
///   address to `found`, to be shown in the pane instead.
async fn watch(
    ws: String,
    ours: String,
    mut asks: tokio::sync::mpsc::UnboundedReceiver<Everywhere>,
    found: UnboundedSender<String>,
) {
    let Ok(mut conn) = Connection::<CdpJsonEventMessage>::connect(&ws).await else {
        return;
    };
    let send = |conn: &mut Connection<CdpJsonEventMessage>,
                method: &'static str,
                session: Option<&str>,
                params: Value| {
        conn.submit_command(
            method.into(),
            session.map(|s| SessionId::from(s.to_string())),
            params,
        )
        .ok()
    };
    // New frames wait for us (as they do for chromiumoxide) until `IN_EVERY_FRAME` is in.
    let auto = json!({"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true, "filter": [{"type": "iframe"}]});
    let seen = json!({"source": IN_EVERY_FRAME, "runImmediately": true});
    // What is there already isn't a new window; then hear of each one that comes.
    // Chrome's first tab is closed: we started this browser for the pane alone.
    let listing = send(&mut conn, "Target.getTargets", None, json!({}));
    let mut known: HashSet<String> = HashSet::new();
    let attach = send(
        &mut conn,
        "Target.attachToTarget",
        None,
        json!({"targetId": ours, "flatten": true}),
    );
    let mut frames: HashSet<String> = HashSet::new();
    // New windows still blank, and since when (one that stays blank is closed).
    let mut blank: HashMap<String, Instant> = HashMap::new();
    // Scripts running in the frames: the answers still to come, the reply, the deadline.
    let mut runs: Vec<(HashSet<CallId>, oneshot::Sender<bool>, Instant)> = Vec::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let shown = |conn: &mut Connection<CdpJsonEventMessage>, id: &str, url: &str| {
        send(conn, "Target.closeTarget", None, json!({"targetId": id}));
        let _ = found.send(url.to_string());
    };
    loop {
        tokio::select! {
            msg = conn.next() => match msg {
                None => return,
                Some(Ok(Message::Response(r))) => {
                    if Some(r.id) == listing {
                        let all = r.result.as_ref().and_then(|v| v["targetInfos"].as_array().cloned());
                        for t in all.unwrap_or_default() {
                            let Some(id) = t["targetId"].as_str() else { continue };
                            known.insert(id.to_string());
                            // (Gone already: Chrome just says so, and that's fine.)
                            if t["type"] == "page" && id != ours {
                                send(&mut conn, "Target.closeTarget", None, json!({"targetId": id}));
                            }
                        }
                        send(&mut conn, "Target.setDiscoverTargets", None, json!({"discover": true}));
                    }
                    if Some(r.id) == attach {
                        // Our page: from now on, its cross-site frames come to us.
                        if let Some(session) = r.result.as_ref().and_then(|v| v["sessionId"].as_str()) {
                            send(&mut conn, "Target.setAutoAttach", Some(session), auto.clone());
                        }
                    }
                    for (left, ..) in &mut runs {
                        left.remove(&r.id);
                    }
                }
                Some(Ok(Message::Event(e))) => {
                    let p = &e.params;
                    let info = &p["targetInfo"];
                    let id = info["targetId"].as_str().unwrap_or_default();
                    let url = info["url"].as_str().unwrap_or_default();
                    match e.method.as_ref() {
                        "Target.attachedToTarget" => {
                            let Some(session) = p["sessionId"].as_str() else { continue };
                            if info["type"] == "iframe" {
                                frames.insert(session.to_string());
                                send(&mut conn, "Target.setAutoAttach", Some(session), auto.clone());
                                send(&mut conn, "Page.enable", Some(session), json!({}));
                                send(&mut conn, "Page.addScriptToEvaluateOnNewDocument", Some(session), seen.clone());
                                if p["waitingForDebugger"] != true {
                                    send(&mut conn, "Runtime.evaluate", Some(session), json!({"expression": IN_EVERY_FRAME}));
                                }
                            }
                            send(&mut conn, "Runtime.runIfWaitingForDebugger", Some(session), json!({}));
                        }
                        "Target.detachedFromTarget" => {
                            if let Some(session) = p["sessionId"].as_str() {
                                frames.remove(session);
                            }
                        }
                        "Target.targetCreated" | "Target.targetInfoChanged"
                            if info["type"] == "page" && id != ours && !known.contains(id) =>
                        {
                            if real_url(url) {
                                blank.remove(id);
                                shown(&mut conn, id, url);
                            } else if e.method == "Target.targetCreated" {
                                blank.insert(id.to_string(), Instant::now());
                            }
                        }
                        "Target.targetDestroyed" => {
                            blank.remove(p["targetId"].as_str().unwrap_or_default());
                        }
                        _ => {}
                    }
                }
                Some(Err(_)) => {}
            },
            ask = asks.recv() => {
                let Some(Everywhere(js, reply)) = ask else { return };
                let left: HashSet<CallId> = frames
                    .iter()
                    .filter_map(|s| send(&mut conn, "Runtime.evaluate", Some(s), json!({"expression": js})))
                    .collect();
                runs.push((left, reply, Instant::now() + PATIENT));
            }
            _ = tick.tick() => {
                // A window that stays blank has nothing to show: close it.
                let old: Vec<String> = blank
                    .iter()
                    .filter(|(_, since)| since.elapsed() > Duration::from_secs(3))
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in old {
                    blank.remove(&id);
                    send(&mut conn, "Target.closeTarget", None, json!({"targetId": id}));
                }
            }
        }
        // Answer the scripts that are done, or out of time.
        let mut i = 0;
        while i < runs.len() {
            let (left, _, by) = &runs[i];
            if left.is_empty() || Instant::now() > *by {
                let (left, reply, _) = runs.swap_remove(i);
                let _ = reply.send(left.is_empty());
            } else {
                i += 1;
            }
        }
    }
}

/// Out of sight: quiet the media everywhere (so it can play on later), stop the
/// pictures, then freeze the page. Frozen, nothing in it runs — scripts, timers,
/// media and Web Audio stop, in every frame. `false`: the page didn't answer
/// (busy for good?), so it may still be playing.
async fn rest(page: &Page, asks: &UnboundedSender<Everywhere>) -> bool {
    let here = answered(PATIENT, page.evaluate(HUSH_MEDIA)).await;
    let there = in_other_sites(asks, HUSH_MEDIA).await;
    quick(page, StopScreencastParams::default()).await;
    quick(
        page,
        SetWebLifecycleStateParams::new(SetWebLifecycleStateState::Frozen),
    )
    .await;
    here && there
}

/// Back in sight: unfreeze the page and let it be seen again. (Freezing marks it
/// hidden, and unfreezing doesn't undo that; minimizing and restoring the
/// window does — without it no pictures come.) `false`: the page didn't answer.
async fn wake(page: &Page, asks: &UnboundedSender<Everywhere>) -> bool {
    quick(
        page,
        SetWebLifecycleStateParams::new(SetWebLifecycleStateState::Active),
    )
    .await;
    if let Ok(Ok(w)) =
        tokio::time::timeout(QUICK, page.execute(GetWindowForTargetParams::default())).await
    {
        for state in [WindowState::Minimized, WindowState::Normal] {
            let bounds = Bounds {
                window_state: Some(state),
                ..Default::default()
            };
            quick(page, SetWindowBoundsParams::new(w.result.window_id, bounds)).await;
        }
    }
    let here = answered(PATIENT, page.evaluate(WAKE_MEDIA)).await;
    let there = in_other_sites(asks, WAKE_MEDIA).await;
    here && there
}

fn picture(data: &str) -> Option<Frame> {
    let bytes = base64_decode(data)?;
    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
        .ok()?
        .to_rgba8();
    Some(Frame {
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
    })
}

/// The page finished loading: read the title and address together, so they always match.
async fn finish_load(page: &Page, shared: &Arc<Mutex<Shared>>) {
    let title = tokio::time::timeout(QUICK, page.get_title())
        .await
        .ok()
        .and_then(Result::ok)
        .flatten()
        .unwrap_or_default();
    let url = tokio::time::timeout(QUICK, page.url())
        .await
        .ok()
        .and_then(Result::ok)
        .flatten();
    let mut s = lock(shared);
    s.loading = false;
    if let Some(url) = url {
        s.url = url;
    }
    if title != s.title {
        s.events.push(("title".into(), json!(title)));
        s.title = title;
    }
    let url = s.url.clone();
    s.events.push(("loaded".into(), json!(url)));
}

/// Go somewhere. Only start it: the page events tell when it has loaded.
/// (Waiting here would stop pictures, input and events until the page is done.)
async fn go(page: &Page, shared: &Arc<Mutex<Shared>>, to: Go) {
    match to {
        Go::To(url) => {
            let trouble = match page.execute(NavigateParams::new(url.as_str())).await {
                Ok(r) => r
                    .result
                    .error_text
                    .clone()
                    .map(|e| format!("couldn't open {url}: {e}")),
                Err(e) => Some(format!("couldn't open {url}: {e}")),
            };
            lock(shared).trouble = trouble;
        }
        Go::Back => {
            try_script(page, "history.back()").await;
        }
        Go::Forward => {
            try_script(page, "history.forward()").await;
        }
        Go::Reload => {
            quick(page, ReloadParams::default()).await;
        }
    }
}

/// Everything after launch: pictures, page events, and commands from the pane.
async fn drive(
    mut browser: Browser,
    page: Page,
    shared: Arc<Mutex<Shared>>,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Cmd>,
    zoom: f64,
    mut cast: Cast,
) {
    let (Ok(mut frames), Ok(mut navs), Ok(mut loads)) = (
        page.event_listener::<EventScreencastFrame>().await,
        page.event_listener::<EventFrameNavigated>().await,
        page.event_listener::<EventLoadEventFired>().await,
    ) else {
        lock(&shared).trouble = Some("the browser stopped answering".into());
        return;
    };
    // Before any page loads: every frame remembers its media (cross-site frames
    // get it from the watcher).
    quick(
        &page,
        AddScriptToEvaluateOnNewDocumentParams::new(IN_EVERY_FRAME),
    )
    .await;
    let (asks, asked) = unbounded_channel();
    let (found_tx, mut found) = unbounded_channel();
    let ours = page.target_id().inner().clone();
    tokio::spawn(watch(
        browser.websocket_address().clone(),
        ours,
        asked,
        found_tx,
    ));
    // While the pane is hidden: where to go once it's shown, and whether its
    // size changed meanwhile.
    let mut later: Vec<Go> = Vec::new();
    let mut resized = false;
    // `evaluate` runs on its own (it may wait for the page); it says when it's done.
    let (done_tx, mut done_rx) = unbounded_channel::<()>();
    if cast.hidden {
        lock(&shared).stuck = !rest(&page, &asks).await;
    } else {
        screencast(&page, cast.size.0, cast.size.1).await;
    }
    let mut check = tokio::time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            Some(f) = frames.next() => {
                // (One may still arrive just after the screencast stopped.)
                if !cast.hidden && let Some(pic) = picture(f.data.as_ref()) {
                    lock(&shared).frame = Some(pic);
                }
                quick(&page, ScreencastFrameAckParams::new(f.session_id)).await;
            }
            Some(n) = navs.next() => {
                if n.frame.parent_id.is_none() {
                    {
                        let mut s = lock(&shared);
                        s.url = n.frame.url.clone();
                        s.loading = true;
                        s.events.push(("navigated".into(), json!(n.frame.url)));
                    }
                    // One already on its way when the pane hid: the new page rests too.
                    if cast.hidden {
                        lock(&shared).stuck = !rest(&page, &asks).await;
                    }
                }
            }
            Some(_) = loads.next() => finish_load(&page, &shared).await,
            _ = check.tick() => {
                // Page events come on separate streams, so a "navigated" can be handled
                // after its "load": while we think the page is loading, ask it.
                // (A resting page doesn't load; it's asked once it's shown.)
                if !cast.hidden && lock(&shared).loading {
                    let ready = tokio::time::timeout(QUICK, page.evaluate("document.readyState")).await;
                    if ready.ok().and_then(Result::ok).and_then(|r| r.into_value::<String>().ok()).as_deref() == Some("complete") {
                        finish_load(&page, &shared).await;
                    }
                }
            }
            Some(url) = found.recv() => {
                // A new window the page opened: shown here instead.
                if cast.hidden {
                    later.push(Go::To(url));
                } else {
                    go(&page, &shared, Go::To(url)).await;
                }
            }
            Some(()) = done_rx.recv() => {
                // Script the app ran in a hidden page may have started sound: quiet it.
                if cast.hidden {
                    try_script(&page, HUSH_MEDIA).await;
                    in_other_sites(&asks, HUSH_MEDIA).await;
                }
            }
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    Cmd::Stop => break,
                    Cmd::Go(to) if cast.hidden => later.push(to),
                    Cmd::Go(to) => go(&page, &shared, to).await,
                    Cmd::Resize(w, h) => {
                        let restart = cast.resize((w, h));
                        if cast.hidden {
                            resized = true;
                        } else {
                            size_page(&page, (w, h), zoom).await;
                        }
                        if restart {
                            quick(&page, StopScreencastParams::default()).await;
                            screencast(&page, w, h).await;
                        }
                    }
                    Cmd::Shown(shown) => match cast.shown(shown) {
                        Some(false) => lock(&shared).stuck = !rest(&page, &asks).await,
                        Some(true) => {
                            lock(&shared).stuck = !wake(&page, &asks).await;
                            if std::mem::take(&mut resized) {
                                size_page(&page, cast.size, zoom).await;
                            }
                            screencast(&page, cast.size.0, cast.size.1).await;
                            for to in std::mem::take(&mut later) {
                                go(&page, &shared, to).await;
                            }
                        }
                        None => {}
                    },
                    // Nothing to hand to a resting page (the app sends none while hidden).
                    Cmd::Mouse(..) | Cmd::Key(_) | Cmd::Text(_) if cast.hidden => {}
                    Cmd::Mouse(kind, x, y, button, wheel) => {
                        let mut p = DispatchMouseEventParams::new(kind.clone(), x, y);
                        if let Some(b) = button {
                            p.buttons = Some(match b { MouseButton::Left => 1, MouseButton::Right => 2, MouseButton::Middle => 4, _ => 0 });
                            p.button = Some(b);
                            if matches!(kind, DispatchMouseEventType::MousePressed | DispatchMouseEventType::MouseReleased) {
                                p.click_count = Some(1);
                            }
                        }
                        if matches!(kind, DispatchMouseEventType::MouseWheel) {
                            p.delta_x = Some(0.0);
                            p.delta_y = Some(wheel);
                        }
                        let _ = page.execute(p).await;
                    }
                    Cmd::Key(k) => {
                        let mut up = k.clone();
                        up.r#type = DispatchKeyEventType::KeyUp;
                        up.text = None;
                        up.unmodified_text = None;
                        let _ = page.execute(k).await;
                        let _ = page.execute(up).await;
                    }
                    Cmd::Text(t) => { let _ = page.execute(InsertTextParams::new(t)).await; }
                    Cmd::Eval(js, reply) => {
                        // A resting page runs plain script, but what it waits for
                        // (a timer, a download) only finishes once it's shown.
                        let (page, done, hidden) = (page.clone(), done_tx.clone(), cast.hidden);
                        tokio::spawn(async move {
                            let r = match tokio::time::timeout(Duration::from_secs(9), page.evaluate(js.as_str())).await {
                                Ok(Ok(v)) => Ok(v.value().cloned().unwrap_or(Value::Null)),
                                Ok(Err(e)) => Err(e.to_string()),
                                Err(_) if hidden => Err("the page didn't answer: it rests while its pane is hidden, and what it waits for finishes once it's shown".to_string()),
                                Err(_) => Err("the page didn't answer in 9 seconds".to_string()),
                            };
                            let _ = reply.send(r);
                            let _ = done.send(());
                        });
                    }
                }
            }
            else => break,
        }
    }
    let _ = browser.close().await;
    let _ = browser.wait().await;
}

// ----- the add-on ------------------------------------------------------------

struct Web {
    rt: Option<tokio::runtime::Runtime>,
    /// The task that drives the browser (it closes it when told to stop).
    task: Option<tokio::task::JoinHandle<()>>,
    tx: Option<UnboundedSender<Cmd>>,
    shared: Arc<Mutex<Shared>>,
    profile: Option<PathBuf>,
    zoom: f64,
    size: (u32, u32),
    /// Typing an address (Ctrl+L): what's typed so far.
    address: Option<String>,
    keys: Keymap,
    status: String,
    status_due: Instant,
    resize_at: Option<Instant>,
    /// Out of sight: no pictures are made.
    hidden: bool,
}

impl Default for Web {
    fn default() -> Self {
        Self {
            rt: None,
            task: None,
            tx: None,
            shared: Arc::default(),
            profile: None,
            zoom: 1.0,
            size: (800, 600),
            address: None,
            keys: Keymap::new(ACTIONS, &Value::Null).unwrap_or_default(),
            status: String::new(),
            status_due: Instant::now(),
            resize_at: None,
            hidden: false,
        }
    }
}

impl Web {
    fn send(&self, cmd: Cmd) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(cmd);
        }
    }

    fn css(&self, px: u32) -> f64 {
        f64::from(px) / self.zoom
    }

    fn state(&self) -> Value {
        let s = lock(&self.shared);
        json!({ "url": s.url, "title": s.title, "loading": s.loading, "zoom": self.zoom })
    }

    fn status_line(&self) -> String {
        if let Some(typed) = &self.address {
            return format!("Go to: {typed}█  (Enter go · Esc cancel)");
        }
        let s = lock(&self.shared);
        if s.stuck {
            return NOT_RESPONDING.into();
        }
        if let Some(t) = &s.trouble {
            return format!("⚠ {t}");
        }
        let title = if s.title.is_empty() {
            "…"
        } else {
            s.title.as_str()
        };
        let dots = if s.loading { " · loading" } else { "" };
        let mut line = format!("◍ {title} — {}{dots}", s.url);
        // The keys as the app named them (one turned off isn't shown).
        for action in ["address", "back", "forward", "reload"] {
            if let Some(key) = self.keys.label(action) {
                line.push_str(&format!(" · {key} {action}"));
            }
        }
        line
    }

    fn address_key(&mut self, key: &str, text: Option<&str>) {
        // Esc, Ctrl+G or the address key again: never mind.
        let cancel = matches!(key, "esc" | "ctrl+g") || self.keys.action(key) == Some("address");
        let Some(typed) = &mut self.address else {
            return;
        };
        match key {
            _ if cancel => self.address = None,
            "enter" => {
                let url = to_url(typed);
                self.address = None;
                if !url.ends_with("://") {
                    self.send(Cmd::Go(Go::To(url)));
                }
            }
            "backspace" => {
                typed.pop();
            }
            _ => {
                if let Some(t) = text {
                    typed.push_str(t);
                }
            }
        }
        self.status_due = Instant::now();
    }
}

impl Drop for Web {
    fn drop(&mut self) {
        self.stop();
    }
}

impl PaneSource for Web {
    fn describe(&self) -> Describe {
        let mut d = Describe::new("web", "Web", "globe");
        d.version = env!("CARGO_PKG_VERSION").into();
        d
    }

    fn start(&mut self, start: &Start) -> Result<Ready, String> {
        let args = &start.args;
        self.keys = Keymap::new(ACTIONS, args)?;
        let first = ["url", "path", "src"]
            .iter()
            .find_map(|k| args.get(*k).and_then(Value::as_str))
            .map_or_else(|| "about:blank".to_string(), to_url);
        self.zoom = args
            .get("zoom")
            .and_then(Value::as_f64)
            .unwrap_or(1.0)
            .clamp(0.25, 4.0);
        self.size = (start.width.max(16), start.height.max(16));
        let chrome = find_browser().ok_or_else(no_browser_help)?;
        tidy_old_profiles();
        let profile = std::env::temp_dir().join(format!("stagetui-web-{}", std::process::id()));
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| format!("couldn't start: {e}"))?;
        let (w, h) = (self.css(self.size.0), self.css(self.size.1));
        let mut config = BrowserConfig::builder()
            .chrome_executable(&chrome)
            .new_headless_mode()
            .window_size(w as u32, h as u32)
            .viewport(None)
            .user_data_dir(&profile)
            .launch_timeout(Duration::from_secs(30))
            .args([
                "--no-first-run",
                "--no-default-browser-check",
                "--hide-scrollbars",
            ]);
        if std::env::var("STAGETUI_WEB_NO_SANDBOX").is_ok_and(|v| v == "1") {
            config = config.no_sandbox();
        }
        let config = config.build()?;
        let zoom = self.zoom;
        let launched = rt.block_on(async move {
            let (browser, mut handler) = Browser::launch(config)
                .await
                .map_err(|e| format!("{} didn't start: {e}", chrome.display()))?;
            tokio::spawn(async move { while handler.next().await.is_some() {} });
            let page = browser
                .new_page("about:blank")
                .await
                .map_err(|e| format!("no page: {e}"))?;
            page.execute(SetDeviceMetricsOverrideParams::new(
                w as i64, h as i64, zoom, false,
            ))
            .await
            .map_err(|e| format!("couldn't size the page: {e}"))?;
            Ok::<_, String>((browser, page))
        })?;
        let (tx, rx) = unbounded_channel();
        let shared = Arc::clone(&self.shared);
        let cast = Cast {
            size: self.size,
            hidden: self.hidden,
        };
        self.task = Some(rt.spawn(drive(launched.0, launched.1, shared, rx, zoom, cast)));
        self.rt = Some(rt);
        self.tx = Some(tx);
        self.profile = Some(profile);
        if first != "about:blank" {
            self.send(Cmd::Go(Go::To(first)));
        } else {
            lock(&self.shared).url = first;
        }
        Ok(Ready {
            keys: self.keys.all_keys(),
            // Every key: the page gets what the pane doesn't use itself.
            text: true,
            fps: 30,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(16), height.max(16));
        self.resize_at = Some(Instant::now() + Duration::from_millis(200));
    }

    fn frame(&mut self, _now: Duration) -> Option<Frame> {
        if self.resize_at.is_some_and(|t| Instant::now() >= t) {
            self.resize_at = None;
            self.send(Cmd::Resize(self.size.0, self.size.1));
        }
        lock(&self.shared).frame.take()
    }

    fn key(&mut self, key: &str, text: Option<&str>) -> bool {
        if self.address.is_some() {
            self.address_key(key, text);
            return true;
        }
        match self.keys.action(key) {
            Some("address") => {
                self.address = Some(String::new());
                self.status_due = Instant::now();
            }
            Some("back") => self.send(Cmd::Go(Go::Back)),
            Some("forward") => self.send(Cmd::Go(Go::Forward)),
            Some("reload") => self.send(Cmd::Go(Go::Reload)),
            // The rest — and a key renamed away — is the page's.
            _ => match text {
                // More than a key press types (a word from an input method).
                Some(t) if t.encode_utf16().count() > 4 => self.send(Cmd::Text(t.to_string())),
                _ => match press(key, text) {
                    Some(p) => self.send(Cmd::Key(p)),
                    None => return false,
                },
            },
        }
        true
    }

    fn mouse(&mut self, m: &Mouse) {
        let (x, y) = (self.css(m.x), self.css(m.y));
        let button = match m.button.as_deref() {
            Some("left") => Some(MouseButton::Left),
            Some("right") => Some(MouseButton::Right),
            Some("middle") => Some(MouseButton::Middle),
            _ => None,
        };
        let cmd = match m.kind.as_str() {
            "down" => Cmd::Mouse(DispatchMouseEventType::MousePressed, x, y, button, 0.0),
            "up" => Cmd::Mouse(DispatchMouseEventType::MouseReleased, x, y, button, 0.0),
            "drag" | "move" => Cmd::Mouse(DispatchMouseEventType::MouseMoved, x, y, button, 0.0),
            "scroll_up" => Cmd::Mouse(DispatchMouseEventType::MouseWheel, x, y, None, -120.0),
            "scroll_down" => Cmd::Mouse(DispatchMouseEventType::MouseWheel, x, y, None, 120.0),
            _ => return,
        };
        self.send(cmd);
    }

    /// Out of sight, the browser stops sending pictures; back, it starts again
    /// (at the size the pane has now).
    fn shown(&mut self, shown: bool) {
        self.hidden = !shown;
        if shown && self.resize_at.take().is_some() {
            // Resized while hidden (`frame` isn't called then): size it first.
            self.send(Cmd::Resize(self.size.0, self.size.1));
        }
        self.send(Cmd::Shown(shown));
    }

    fn paste(&mut self, text: &str) {
        if let Some(typed) = &mut self.address {
            typed.push_str(text.lines().next().unwrap_or_default());
            self.status_due = Instant::now();
        } else {
            self.send(Cmd::Text(text.to_string()));
        }
    }

    fn call(&mut self, method: &str, args: &Value) -> Result<Value, String> {
        let text = |v: &Value| {
            v.as_str()
                .map(String::from)
                .or_else(|| v.get("url").and_then(Value::as_str).map(String::from))
        };
        match method {
            "navigate" | "go" => self
                .send(Cmd::Go(Go::To(to_url(&text(args).ok_or(
                    "navigate needs a URL, e.g. call(\"navigate\", \"example.com\")",
                )?)))),
            "back" => self.send(Cmd::Go(Go::Back)),
            "forward" => self.send(Cmd::Go(Go::Forward)),
            "reload" => self.send(Cmd::Go(Go::Reload)),
            "evaluate" | "js" => {
                let js = args
                    .as_str()
                    .or_else(|| args.get("js").and_then(Value::as_str))
                    .ok_or(
                        "evaluate needs JavaScript, e.g. call(\"evaluate\", \"document.title\")",
                    )?;
                let (reply, rx) = std_mpsc::channel();
                self.send(Cmd::Eval(js.to_string(), reply));
                return rx
                    .recv_timeout(Duration::from_secs(10))
                    .map_err(|_| "the page didn't answer in 10 seconds".to_string())?;
            }
            "state" => {}
            other => return Err(format!("the web add-on has no method \"{other}\"")),
        }
        Ok(self.state())
    }

    fn status(&mut self) -> Option<String> {
        if Instant::now() < self.status_due {
            return None;
        }
        self.status_due = Instant::now() + Duration::from_millis(250);
        let line = self.status_line();
        (line != self.status).then(|| {
            self.status = line.clone();
            line
        })
    }

    fn events(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut lock(&self.shared).events)
    }

    fn stop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Cmd::Stop);
        }
        if let Some(rt) = self.rt.take() {
            // Let the browser close properly, then stop everything else.
            if let Some(task) = self.task.take() {
                let _ = rt.block_on(tokio::time::timeout(Duration::from_secs(3), task));
            }
            rt.shutdown_timeout(Duration::from_secs(1));
        }
        if let Some(dir) = self.profile.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_base64() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(
            base64_decode("2YXYsdit2KjYpw==").unwrap(),
            "مرحبا".as_bytes()
        );
        assert_eq!(base64_decode("5pel5pys6Kqe").unwrap(), "日本語".as_bytes());
        assert!(base64_decode("@@@@").is_none());
    }

    #[test]
    fn addresses_become_urls() {
        assert_eq!(to_url("example.com"), "https://example.com");
        assert_eq!(to_url("http://a.b/c"), "http://a.b/c");
        assert_eq!(to_url("about:blank"), "about:blank");
        assert!(to_url("/tmp").starts_with("file:///"));
        assert_eq!(
            file_url(r"\\?\C:\Users\me\مرحبا 日本語.html"),
            "file:///C:/Users/me/مرحبا 日本語.html"
        );
        assert_eq!(file_url(r"C:\a\b.html"), "file:///C:/a/b.html");
        assert_eq!(
            file_url(r"\\?\UNC\server\share\a.html"),
            "file://server/share/a.html"
        );
        assert_eq!(file_url("/home/me/a.html"), "file:///home/me/a.html");
    }

    /// `(type is keyDown, key, code, virtual key, modifiers, text)` of a key press.
    fn pressed(
        name: &str,
        text: Option<&str>,
    ) -> (bool, String, Option<String>, i64, i64, Option<String>) {
        let p = press(name, text).unwrap_or_else(|| panic!("{name} is a key"));
        (
            p.r#type == DispatchKeyEventType::KeyDown,
            p.key.unwrap_or_default(),
            p.code,
            p.windows_virtual_key_code.unwrap_or_default(),
            p.modifiers.unwrap_or_default(),
            p.text,
        )
    }

    #[test]
    fn keys_go_to_the_page_as_real_key_presses() {
        let s = |v: &str| Some(v.to_string());
        // Letters type, with their key and place on the keyboard.
        assert_eq!(
            pressed("a", Some("a")),
            (true, "a".into(), s("KeyA"), 65, 0, s("a"))
        );
        assert_eq!(
            pressed("shift+a", Some("A")),
            (true, "A".into(), s("KeyA"), 65, 8, s("A"))
        );
        assert_eq!(
            pressed("7", Some("7")),
            (true, "7".into(), s("Digit7"), 55, 0, s("7"))
        );
        assert_eq!(
            pressed("+", Some("+")),
            (true, "+".into(), s("Equal"), 187, 0, s("+"))
        );
        // Keys that don't type: raw key presses, modifiers kept.
        assert_eq!(
            pressed("tab", None),
            (false, "Tab".into(), s("Tab"), 9, 0, None)
        );
        assert_eq!(
            pressed("shift+tab", None),
            (false, "Tab".into(), s("Tab"), 9, 8, None)
        );
        assert_eq!(
            pressed("esc", None),
            (false, "Escape".into(), s("Escape"), 27, 0, None)
        );
        assert_eq!(
            pressed("ctrl+a", None),
            (false, "a".into(), s("KeyA"), 65, 2, None)
        );
        assert_eq!(
            pressed("ctrl++", None),
            (false, "+".into(), s("Equal"), 187, 2, None)
        );
        assert_eq!(pressed("ctrl+shift+left", None).4, 10);
        assert_eq!(
            pressed("f5", None),
            (false, "F5".into(), s("F5"), 116, 0, None)
        );
        assert_eq!(pressed("enter", None).5, s("\r"));
        assert_eq!(pressed("ctrl+enter", None).5, None);
        assert_eq!(
            pressed("space", Some(" ")),
            (true, " ".into(), s("Space"), 32, 0, s(" "))
        );
        // Any language types as text: Arabic with harakat, Japanese, Devanagari.
        for c in ["م", "َ", "ْ", "ً", "日", "न", "्"] {
            assert_eq!(
                pressed(c, Some(c)),
                (true, c.to_string(), None, 0, 0, s(c)),
                "{c}"
            );
        }
        assert!(press("nonsense", None).is_none() && press("f99", None).is_none());
    }

    fn sent(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<Cmd> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    #[test]
    fn keys_can_be_renamed_or_turned_off() {
        let mut w = Web::default();
        let (tx, mut rx) = unbounded_channel();
        w.tx = Some(tx);
        // As it comes.
        assert!(w.key("ctrl+l", None) && w.address.is_some());
        assert!(w.key("esc", None) && w.address.is_none(), "Esc never minds");
        w.key("ctrl+l", None);
        w.key("ctrl+l", None);
        assert!(w.address.is_none(), "the address key again: never mind");
        w.key("alt+left", None);
        w.key("alt+right", None);
        w.key("f5", None);
        w.key("ctrl+r", None);
        assert!(matches!(
            sent(&mut rx)[..],
            [
                Cmd::Go(Go::Back),
                Cmd::Go(Go::Forward),
                Cmd::Go(Go::Reload),
                Cmd::Go(Go::Reload)
            ]
        ));
        let line = w.status_line();
        assert!(
            line.ends_with("ctrl+l address · alt+← back · alt+→ forward · f5 reload"),
            "{line}"
        );

        // Renamed and turned off: the old keys are the page's now.
        w.keys = Keymap::new(
            ACTIONS,
            &json!({"keys": {"address": "ctrl+o", "reload": null}}),
        )
        .unwrap();
        assert!(w.key("ctrl+l", None) && w.address.is_none());
        assert!(w.key("f5", None));
        match &sent(&mut rx)[..] {
            [Cmd::Key(l), Cmd::Key(f5)] => {
                assert_eq!((l.key.as_deref(), l.modifiers), (Some("l"), Some(2)));
                assert_eq!(f5.key.as_deref(), Some("F5"));
            }
            _ => panic!("ctrl+l and f5 go to the page"),
        }
        assert!(w.key("ctrl+o", None) && w.address.is_some());
        assert!(w.status_line().contains("Esc cancel"));
        w.key("ctrl+o", None);
        assert!(w.address.is_none());
        let line = w.status_line();
        assert!(
            line.contains("ctrl+o address") && !line.contains("reload"),
            "{line}"
        );
        assert!(
            Keymap::new(ACTIONS, &json!({"keys": {"adress": "x"}}))
                .unwrap_err()
                .contains("address")
        );
    }

    #[test]
    fn page_scripts_run_as_expressions() {
        // chromiumoxide takes a string starting like `(…) =>` or `function` for a
        // function to call; these must run as they are.
        for js in [IN_EVERY_FRAME, HUSH_MEDIA, WAKE_MEDIA] {
            assert!(js.starts_with("void (() => {") && js.ends_with("})()"));
        }
    }

    #[test]
    fn a_page_that_doesnt_answer_says_so() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        // Told to rest, a page stuck in its own script never answers.
        let never = std::future::pending::<()>();
        assert!(!rt.block_on(answered(Duration::from_millis(50), never)));
        assert!(rt.block_on(answered(Duration::from_millis(50), async {})));
        let w = Web::default();
        lock(&w.shared).stuck = true;
        assert_eq!(w.status_line(), NOT_RESPONDING);
        assert!(NOT_RESPONDING.contains("close the pane"));
        lock(&w.shared).stuck = false;
        assert!(w.status_line().starts_with("◍"));
        // New windows: one still blank waits for its address.
        assert!(!real_url("") && !real_url("about:blank") && real_url("https://a.b/"));
    }

    #[test]
    fn screencast_stops_while_hidden() {
        let mut cast = Cast {
            size: (800, 600),
            hidden: false,
        };
        assert!(cast.resize((640, 480)), "shown: restart at the new size");
        assert_eq!(cast.shown(true), None, "already shown");
        assert_eq!(cast.shown(false), Some(false), "hide: stop");
        assert_eq!(cast.shown(false), None);
        assert!(!cast.resize((1024, 768)), "hidden: no restart");
        assert_eq!(cast.shown(true), Some(true), "back: start");
        assert_eq!(cast.size, (1024, 768), "at the size it has now");
    }
}
