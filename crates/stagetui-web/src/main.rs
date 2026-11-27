//! The web add-on for stagetui: `StagePane("web", url="https://example.com")`.
//!
//! Your own Chrome, Chromium, Edge or Brave runs hidden (headless) at the
//! pane's size. Chrome's screencast sends a picture whenever the page changes;
//! clicks, the wheel and typing go back to the page. Nothing is downloaded.
//!
//! Keys while the pane has focus go to the page, except: Ctrl+L type an
//! address (shown at the bottom; Enter goes there), Alt+← / Alt+→ back and
//! forward, F5 reload.

use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType,
    InsertTextParams, MouseButton,
};
use chromiumoxide::cdp::browser_protocol::page::{
    EventFrameNavigated, EventLoadEventFired, EventScreencastFrame, ScreencastFrameAckParams,
    StartScreencastFormat, StartScreencastParams, StopScreencastParams,
};
use chromiumoxide::{Browser, BrowserConfig, Page};
use futures::StreamExt;
use serde_json::{Value, json};
use stagetui_plugin_api::{Describe, Frame, Mouse, PaneSource, Ready, Start, serve};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

fn main() {
    std::process::exit(serve(Web::default()));
}

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

fn key_code(name: &str) -> Option<(&'static str, &'static str, i64, Option<&'static str>)> {
    // (key, code, Windows virtual key code, text it types)
    Some(match name {
        "enter" => ("Enter", "Enter", 13, Some("\r")),
        "backspace" => ("Backspace", "Backspace", 8, None),
        "delete" => ("Delete", "Delete", 46, None),
        "left" => ("ArrowLeft", "ArrowLeft", 37, None),
        "right" => ("ArrowRight", "ArrowRight", 39, None),
        "up" => ("ArrowUp", "ArrowUp", 38, None),
        "down" => ("ArrowDown", "ArrowDown", 40, None),
        "home" => ("Home", "Home", 36, None),
        "end" => ("End", "End", 35, None),
        "pageup" => ("PageUp", "PageUp", 33, None),
        "pagedown" => ("PageDown", "PageDown", 34, None),
        "space" => (" ", "Space", 32, Some(" ")),
        "esc" => ("Escape", "Escape", 27, None),
        _ => return None,
    })
}

// ----- talking to the browser ------------------------------------------------

enum Cmd {
    Navigate(String),
    Back,
    Forward,
    Reload,
    Resize(u32, u32),
    Mouse(DispatchMouseEventType, f64, f64, Option<MouseButton>, f64),
    Key(DispatchKeyEventParams),
    Text(String),
    Eval(String, std_mpsc::Sender<Result<Value, String>>),
    Stop,
}

#[derive(Default)]
struct Shared {
    frame: Option<Frame>,
    url: String,
    title: String,
    loading: bool,
    events: Vec<(String, Value)>,
    trouble: Option<String>,
}

async fn screencast(page: &Page, w: u32, h: u32) -> chromiumoxide::Result<()> {
    let p = StartScreencastParams {
        format: Some(StartScreencastFormat::Jpeg),
        quality: Some(80),
        max_width: Some(i64::from(w)),
        max_height: Some(i64::from(h)),
        every_nth_frame: Some(1),
    };
    page.execute(p).await?;
    Ok(())
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

/// Everything after launch: pictures, page events, and commands from the pane.
async fn drive(
    mut browser: Browser,
    page: Page,
    shared: Arc<Mutex<Shared>>,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Cmd>,
    zoom: f64,
    size: (u32, u32),
) {
    let (Ok(mut frames), Ok(mut navs), Ok(mut loads)) = (
        page.event_listener::<EventScreencastFrame>().await,
        page.event_listener::<EventFrameNavigated>().await,
        page.event_listener::<EventLoadEventFired>().await,
    ) else {
        lock(&shared).trouble = Some("the browser stopped answering".into());
        return;
    };
    let _ = screencast(&page, size.0, size.1).await;
    loop {
        tokio::select! {
            Some(f) = frames.next() => {
                if let Some(pic) = picture(f.data.as_ref()) {
                    lock(&shared).frame = Some(pic);
                }
                let _ = page.execute(ScreencastFrameAckParams::new(f.session_id)).await;
            }
            Some(n) = navs.next() => {
                if n.frame.parent_id.is_none() {
                    let mut s = lock(&shared);
                    s.url = n.frame.url.clone();
                    s.loading = true;
                    s.events.push(("navigated".into(), json!(n.frame.url)));
                }
            }
            Some(_) = loads.next() => {
                // Read both now, so the title and the address always match.
                let title = page.get_title().await.ok().flatten().unwrap_or_default();
                let url = page.url().await.ok().flatten();
                let mut s = lock(&shared);
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
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    Cmd::Stop => break,
                    Cmd::Navigate(url) => {
                        if let Err(e) = page.goto(url.as_str()).await {
                            lock(&shared).trouble = Some(format!("couldn't open {url}: {e}"));
                        } else {
                            lock(&shared).trouble = None;
                        }
                    }
                    Cmd::Back => { let _ = page.evaluate("history.back()").await; }
                    Cmd::Forward => { let _ = page.evaluate("history.forward()").await; }
                    Cmd::Reload => { let _ = page.reload().await; }
                    Cmd::Resize(w, h) => {
                        let css = ((f64::from(w) / zoom) as i64, (f64::from(h) / zoom) as i64);
                        let _ = page.execute(SetDeviceMetricsOverrideParams::new(css.0.max(1), css.1.max(1), zoom, false)).await;
                        let _ = page.execute(StopScreencastParams::default()).await;
                        let _ = screencast(&page, w, h).await;
                    }
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
                        let _ = page.execute(k).await;
                        let _ = page.execute(up).await;
                    }
                    Cmd::Text(t) => { let _ = page.execute(InsertTextParams::new(t)).await; }
                    Cmd::Eval(js, reply) => {
                        let r = match page.evaluate(js.as_str()).await {
                            Ok(v) => Ok(v.value().cloned().unwrap_or(Value::Null)),
                            Err(e) => Err(e.to_string()),
                        };
                        let _ = reply.send(r);
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
    status: String,
    status_due: Instant,
    resize_at: Option<Instant>,
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
            status: String::new(),
            status_due: Instant::now(),
            resize_at: None,
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
            return format!("Go to: {typed}█  (Enter go · Ctrl+L cancel)");
        }
        let s = lock(&self.shared);
        if let Some(t) = &s.trouble {
            return format!("⚠ {t}");
        }
        let title = if s.title.is_empty() {
            "…"
        } else {
            s.title.as_str()
        };
        let dots = if s.loading { " · loading" } else { "" };
        format!(
            "◍ {title} — {}{dots} · Ctrl+L address · Alt+← → · F5",
            s.url
        )
    }

    fn address_key(&mut self, key: &str, text: Option<&str>) {
        let Some(typed) = &mut self.address else {
            return;
        };
        match key {
            "enter" => {
                let url = to_url(typed);
                self.address = None;
                if !url.ends_with("://") {
                    self.send(Cmd::Navigate(url));
                }
            }
            "backspace" => {
                typed.pop();
            }
            "ctrl+l" | "ctrl+g" => self.address = None,
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
        let size = self.size;
        self.task = Some(rt.spawn(drive(launched.0, launched.1, shared, rx, zoom, size)));
        self.rt = Some(rt);
        self.tx = Some(tx);
        self.profile = Some(profile);
        if first != "about:blank" {
            self.send(Cmd::Navigate(first));
        } else {
            lock(&self.shared).url = first;
        }
        Ok(Ready {
            keys: Vec::new(),
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
        match key {
            "ctrl+l" => {
                self.address = Some(String::new());
                self.status_due = Instant::now();
            }
            "alt+left" => self.send(Cmd::Back),
            "alt+right" => self.send(Cmd::Forward),
            "f5" | "ctrl+r" => self.send(Cmd::Reload),
            _ => {
                // Plain typing (any language) goes in as text; the rest as keys.
                let mods: Vec<&str> = key.split('+').collect();
                let (base, held) = mods.split_last().map_or((key, &[][..]), |(b, h)| (*b, h));
                if held.is_empty()
                    && let Some(t) = text
                    && key_code(base).is_none()
                {
                    self.send(Cmd::Text(t.to_string()));
                    return true;
                }
                let modifiers = held.iter().fold(0i64, |m, h| {
                    m | match *h {
                        "alt" => 1,
                        "ctrl" => 2,
                        "super" => 4,
                        "shift" => 8,
                        _ => 0,
                    }
                });
                let mut p = DispatchKeyEventParams::new(DispatchKeyEventType::KeyDown);
                p.modifiers = Some(modifiers);
                if let Some((k, code, vk, typed)) = key_code(base) {
                    p.key = Some(k.into());
                    p.code = Some(code.into());
                    p.windows_virtual_key_code = Some(vk);
                    p.text = typed.filter(|_| modifiers == 0).map(String::from);
                } else if base.chars().count() == 1 {
                    let c = base.chars().next().unwrap_or(' ');
                    p.key = Some(c.to_string());
                    if c.is_ascii_alphabetic() {
                        p.code = Some(format!("Key{}", c.to_ascii_uppercase()));
                        p.windows_virtual_key_code = Some(i64::from(c.to_ascii_uppercase() as u8));
                    }
                } else {
                    return false;
                }
                self.send(Cmd::Key(p));
            }
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
                .send(Cmd::Navigate(to_url(&text(args).ok_or(
                    "navigate needs a URL, e.g. call(\"navigate\", \"example.com\")",
                )?))),
            "back" => self.send(Cmd::Back),
            "forward" => self.send(Cmd::Forward),
            "reload" => self.send(Cmd::Reload),
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
        assert_eq!(key_code("enter").map(|k| k.2), Some(13));
        assert!(key_code("q").is_none());
    }
}
