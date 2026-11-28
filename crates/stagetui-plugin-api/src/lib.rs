//! Write a new pane type for stagetui — video, a web page, a 3D view, anything
//! that draws pictures — as a small program of its own.
//!
//! A plugin is an executable named `stagetui-plugin-<name>` (for example
//! `stagetui-plugin-video`). stagetui finds it next to its own Python or on the
//! `PATH`, starts it when a pane of that type opens, and talks to it over
//! stdin/stdout. The plugin draws RGBA frames; stagetui shows them with the
//! best graphics each terminal has, and sends keys and mouse clicks back.
//! If the plugin crashes, only that pane notices.
//!
//! ```no_run
//! use stagetui_plugin_api::{Describe, Frame, PaneSource, Ready, Start, serve};
//! use std::time::Duration;
//!
//! struct Gradient { w: u32, h: u32 }
//!
//! impl PaneSource for Gradient {
//!     fn describe(&self) -> Describe {
//!         Describe::new("gradient", "Gradient", "sparkles")
//!     }
//!     fn start(&mut self, s: &Start) -> Result<Ready, String> {
//!         (self.w, self.h) = (s.width, s.height);
//!         Ok(Ready::default())
//!     }
//!     fn resize(&mut self, width: u32, height: u32) {
//!         (self.w, self.h) = (width, height);
//!     }
//!     fn frame(&mut self, now: Duration) -> Option<Frame> {
//!         let shift = (now.as_secs_f32() * 60.0) as u32;
//!         Some(Frame::from_fn(self.w, self.h, |x, _| [((x + shift) % 256) as u8, 80, 200, 255]))
//!     }
//! }
//!
//! fn main() {
//!     std::process::exit(serve(Gradient { w: 1, h: 1 }));
//! }
//! ```
//!
//! Wire format (both ways): `[u32 little-endian length][u8 kind][payload]`.
//! Kind 1 is a JSON message ([`ToPlugin`] / [`FromPlugin`]); kind 2 (plugin →
//! app only) is a frame: `[u32 width][u32 height]` then `width × height × 4`
//! bytes of RGBA, premultiplied alpha (opaque pictures need no care).

use std::io::{self, BufReader, BufWriter, Read, Write};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Bumped when the messages change.
pub const PROTOCOL: u32 = 1;
pub const KIND_JSON: u8 = 1;
pub const KIND_FRAME: u8 = 2;
/// Largest message accepted (an 8K RGBA frame is ~130 MB; stay well below).
pub const MAX_MESSAGE: usize = 128 * 1024 * 1024;

/// What a plugin is, printed as JSON by `stagetui-plugin-<name> --describe`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Describe {
    /// The pane type, e.g. `"video"`.
    pub name: String,
    /// Shown in pane menus, e.g. `"Video"`.
    pub title: String,
    /// One of stagetui's icon names.
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub version: String,
    #[serde(default = "protocol")]
    pub protocol: u32,
    /// Shows pictures, so it is best in a terminal with picture support.
    #[serde(default = "yes")]
    pub pictures: bool,
}

fn protocol() -> u32 {
    PROTOCOL
}

fn yes() -> bool {
    true
}

impl Describe {
    pub fn new(name: &str, title: &str, icon: &str) -> Self {
        Self {
            name: name.into(),
            title: title.into(),
            icon: Some(icon.into()),
            version: String::new(),
            protocol: PROTOCOL,
            pictures: true,
        }
    }
}

/// From stagetui to the plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ToPlugin {
    /// The pane opened. Sizes are the pane's, in pixels and in cells.
    Start(Start),
    Resize {
        width: u32,
        height: u32,
        cols: u16,
        rows: u16,
    },
    /// A key, as stagetui names it: `"a"`, `"space"`, `"ctrl+s"`, `"left"`…
    Key {
        key: String,
        /// The text it types, if any (`"a"`, `"م"`).
        #[serde(default)]
        text: Option<String>,
    },
    /// The mouse, in pixels from the pane's top-left corner.
    Mouse(Mouse),
    Paste {
        text: String,
    },
    Focus {
        focused: bool,
    },
    /// The pane went out of sight (another pane type or tab is showing) or came
    /// back. While hidden, `serve` asks for no frames. Older plugins ignore it.
    Shown {
        shown: bool,
    },
    /// The app calls a method (`pane.content.call("play")`); answer with `Reply`.
    Call {
        id: u64,
        method: String,
        #[serde(default)]
        args: Value,
    },
    /// The pane is closing: clean up and exit.
    Stop,
}

/// The pane opened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Start {
    pub width: u32,
    pub height: u32,
    pub cols: u16,
    pub rows: u16,
    /// Options from the app, e.g. `{"path": "movie.mp4"}`.
    #[serde(default)]
    pub args: Value,
    /// Theme colors as `#rrggbb`.
    #[serde(default)]
    pub theme: Theme,
    #[serde(default = "protocol")]
    pub protocol: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub bg: String,
    pub fg: String,
    pub accent: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mouse {
    /// `down`, `up`, `drag`, `move`, `scroll_up`, `scroll_down`.
    pub kind: String,
    /// `left`, `right` or `middle` for down / up / drag.
    #[serde(default)]
    pub button: Option<String>,
    pub x: u32,
    pub y: u32,
}

/// From the plugin to stagetui.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum FromPlugin {
    /// Started; what the plugin wants.
    Ready(Ready),
    /// A short line shown at the bottom of the pane, e.g. `"▶ 01:23 / 04:56"`.
    Status { text: String },
    /// Something the app can react to (`on_<name>` in Python).
    Event {
        name: String,
        #[serde(default)]
        value: Value,
    },
    /// The answer to a `Call`.
    Reply {
        id: u64,
        #[serde(default)]
        value: Value,
        #[serde(default)]
        error: Option<String>,
    },
    /// Something went wrong — shown in the pane in three parts.
    Error {
        what: String,
        why: String,
        fix: String,
    },
}

/// What a plugin wants, sent once after `Start`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ready {
    /// Keys it handles while its pane has focus (others go to the app).
    #[serde(default)]
    pub keys: Vec<String>,
    /// It wants every key typed (a web page with a text box).
    #[serde(default)]
    pub text: bool,
    /// Most frames per second it will send.
    #[serde(default = "fps")]
    pub fps: u32,
}

fn fps() -> u32 {
    30
}

/// `#rrggbb` → bytes.
fn rgb(hex: &str) -> Option<[u8; 3]> {
    let s = hex.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

impl Default for Ready {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            text: false,
            fps: fps(),
        }
    }
}

// ----- keys the app can rename ------------------------------------------------

/// Something a key does in a plugin, with the keys it has unless the app says
/// otherwise: `KeyAction { name: "wires", keys: &["w"], about: "show the edges" }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyAction {
    /// What the app writes in `keys={...}`, e.g. `"wires"` (snake_case).
    pub name: &'static str,
    /// Its keys, as stagetui names them: `"w"`, `"space"`, `"shift+left"`, `"ctrl+s"`, `"+"`.
    pub keys: &'static [&'static str],
    /// What it does, in a few words (for help and docs).
    pub about: &'static str,
}

/// A plugin's keys after the app's changes. The app writes
/// `StagePane("3d", keys={"wires": "x", "grid": None, "spin": ["s", "space"]})`:
/// an action name, then one key, a list of keys, or `None` to turn it off.
///
/// ```
/// use stagetui_plugin_api::{KeyAction, Keymap};
/// use serde_json::json;
///
/// const ACTIONS: &[KeyAction] = &[
///     KeyAction { name: "spin", keys: &["space"], about: "spin or stop" },
///     KeyAction { name: "wires", keys: &["w"], about: "show the edges" },
/// ];
/// let keys = Keymap::new(ACTIONS, &json!({"keys": {"wires": "x"}})).unwrap();
/// assert_eq!(keys.action("x"), Some("wires"));
/// assert_eq!(keys.action("w"), None);
/// assert_eq!(keys.all_keys(), ["space", "x"]); // for `Ready.keys`
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
    /// Each action (in the plugin's order) and its keys, written like `key_name`.
    actions: Vec<(&'static str, Vec<String>)>,
}

impl Keymap {
    /// The plugin's `actions`, changed by the app's `args["keys"]` (if any).
    /// The error says what is wrong in words the app's author can act on: a
    /// name that isn't an action (the real ones are listed), a key stagetui
    /// doesn't know, or one key given to two actions.
    pub fn new(actions: &[KeyAction], args: &Value) -> Result<Keymap, String> {
        let mut map: Vec<(&'static str, Vec<String>)> = actions
            .iter()
            .map(|a| {
                let keys = a
                    .keys
                    .iter()
                    .map(|k| key_name(k).unwrap_or_else(|| k.to_lowercase()))
                    .collect();
                (a.name, keys)
            })
            .collect();
        let names = || {
            actions
                .iter()
                .map(|a| a.name)
                .collect::<Vec<_>>()
                .join(", ")
        };
        match args.get("keys") {
            None | Some(Value::Null) => {}
            Some(Value::Object(given)) => {
                for (name, value) in given {
                    let Some(slot) = map.iter_mut().find(|(n, _)| n == name) else {
                        return Err(format!(
                            "keys= names \"{name}\", but there is no action by that name; \
                             the actions are: {}",
                            names()
                        ));
                    };
                    slot.1 = given_keys(name, value)?;
                }
            }
            Some(other) => {
                return Err(format!(
                    "keys= should name actions and their keys, like keys={{\"{}\": \"x\"}}, \
                     not {other}; the actions are: {}",
                    actions.first().map_or("name", |a| a.name),
                    names()
                ));
            }
        }
        for slot in &mut map {
            let mut seen = Vec::new();
            slot.1.retain(|k| {
                let new = !seen.contains(k);
                seen.push(k.clone());
                new
            });
        }
        for (i, (first, keys)) in map.iter().enumerate() {
            for key in keys {
                if let Some((second, _)) = map[i + 1..].iter().find(|(_, ks)| ks.contains(key)) {
                    return Err(format!(
                        "\"{key}\" is given to both \"{first}\" and \"{second}\"; give one of \
                         them another key (keys={{\"{second}\": \"…\"}}) or turn it off \
                         (keys={{\"{second}\": None}})"
                    ));
                }
            }
        }
        Ok(Keymap { actions: map })
    }

    /// The action a key does (`None`: none, or it was turned off). `key` is a
    /// key as stagetui sends it (`"space"`, `"shift+left"`, `"shift+w"`).
    pub fn action(&self, key: &str) -> Option<&'static str> {
        let key = key_name(key).unwrap_or_else(|| key.to_lowercase());
        self.actions
            .iter()
            .find(|(_, keys)| keys.contains(&key))
            .map(|(name, _)| *name)
    }

    /// The keys an action has now (empty: turned off, or no such action).
    pub fn keys(&self, action: &str) -> &[String] {
        self.actions
            .iter()
            .find(|(name, _)| *name == action)
            .map_or(&[], |(_, keys)| keys.as_slice())
    }

    /// The action's first key, short for a status line (`"x"`, `"space"`, arrows as
    /// `← → ↑ ↓`, `"shift+←"`); `None` when the action is off.
    pub fn label(&self, action: &str) -> Option<String> {
        self.keys(action).first().map(|k| short(k))
    }

    /// The action's first key written for a sentence, the way stagetui shows
    /// keys (`"Enter"`, `"Ctrl+O"`, `"Shift+←"`, `"o"`): "press Enter to …".
    /// `None` when the action is off.
    pub fn spoken(&self, action: &str) -> Option<String> {
        self.keys(action).first().map(|k| spoken(k))
    }

    /// Every key of every action, for `Ready.keys` (so stagetui sends them on).
    pub fn all_keys(&self) -> Vec<String> {
        self.actions
            .iter()
            .flat_map(|(_, keys)| keys.iter().cloned())
            .collect()
    }
}

/// The keys the app gave one action: `"x"`, `["s", "space"]`, or `None` / `[]` (off).
fn given_keys(action: &str, value: &Value) -> Result<Vec<String>, String> {
    let one = |v: &Value| -> Result<String, String> {
        let text = v.as_str().ok_or_else(|| {
            format!(
                "keys={{\"{action}\": {value}}}: give a key like \"x\", \"space\" or \"ctrl+s\", \
                 a list of keys, or None to turn it off"
            )
        })?;
        key_name(text).ok_or_else(|| {
            format!(
                "\"{text}\" (for \"{action}\") isn't a key stagetui knows; keys are written \
                 like \"x\", \"space\", \"ctrl+s\", \"shift+left\", \"f5\" or \"+\""
            )
        })
    };
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => items.iter().map(one).collect(),
        v => Ok(vec![one(v)?]),
    }
}

/// Key names stagetui understands, and the name it sends for each.
const KEY_NAMES: [(&str, &str); 30] = [
    ("enter", "enter"),
    ("return", "enter"),
    ("escape", "esc"),
    ("esc", "esc"),
    ("tab", "tab"),
    ("backspace", "backspace"),
    ("delete", "delete"),
    ("del", "delete"),
    ("insert", "insert"),
    ("up", "up"),
    ("down", "down"),
    ("left", "left"),
    ("right", "right"),
    ("home", "home"),
    ("end", "end"),
    ("pageup", "pageup"),
    ("page_up", "pageup"),
    ("pagedown", "pagedown"),
    ("page_down", "pagedown"),
    ("space", "space"),
    ("backslash", "\\"),
    ("slash", "/"),
    ("comma", ","),
    ("period", "."),
    ("minus", "-"),
    ("plus", "+"),
    ("equals", "="),
    ("semicolon", ";"),
    ("left_bracket", "["),
    ("right_bracket", "]"),
];

/// A key written the way stagetui names the keys it sends a plugin
/// (`"Ctrl+s"` → `"ctrl+s"`, `"shift+ctrl+left"` → `"ctrl+shift+left"`,
/// `"plus"` → `"+"`, `"Escape"` → `"esc"`); `None` if it isn't a key. As in an
/// app's `BINDINGS`, a capital letter means Shift: `"W"` is `"shift+w"`.
pub fn key_name(text: &str) -> Option<String> {
    let lower = text.trim().to_lowercase();
    if lower.is_empty() {
        return None;
    }
    // A lone "+" is the plus key; otherwise "+" joins the parts.
    let parts: Vec<&str> = if lower == "+" {
        vec!["+"]
    } else if let Some(head) = lower.strip_suffix("++") {
        head.split('+').chain(std::iter::once("+")).collect()
    } else {
        lower.split('+').collect()
    };
    let (last, held) = parts.split_last()?;
    let (mut ctrl, mut alt, mut sup, mut shift) = (false, false, false, false);
    for m in held {
        match *m {
            "ctrl" | "control" => ctrl = true,
            "alt" | "option" | "meta" => alt = true,
            "super" | "cmd" | "win" => sup = true,
            "shift" => shift = true,
            _ => return None,
        }
    }
    let base = if let Some((_, name)) = KEY_NAMES.iter().find(|(n, _)| n == last) {
        (*name).to_string()
    } else if let Some(n) = last.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
        if !(1..=24).contains(&n) {
            return None;
        }
        format!("f{n}")
    } else {
        let mut chars = last.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => {
                if text.trim().chars().last().is_some_and(char::is_uppercase) {
                    // "W" is Shift+W, like stagetui reads it everywhere.
                    shift = true;
                } else if !c.is_alphabetic() && c != ' ' {
                    // Symbols say shift already ("?", "+"): stagetui never adds it.
                    shift = false;
                }
                c.to_string()
            }
            _ => return None,
        }
    };
    let mut name = String::new();
    for (on, m) in [
        (ctrl, "ctrl+"),
        (alt, "alt+"),
        (sup, "super+"),
        (shift, "shift+"),
    ] {
        if on {
            name.push_str(m);
        }
    }
    name.push_str(&base);
    Some(name)
}

/// `"shift+left"` → `"shift+←"`; other keys stay as they are.
fn short(key: &str) -> String {
    let (held, base) = if key == "+" {
        ("", "+")
    } else if let Some(head) = key.strip_suffix("++") {
        (&key[..head.len() + 1], "+")
    } else {
        key.rsplit_once('+')
            .map_or(("", key), |(h, b)| (&key[..h.len() + 1], b))
    };
    let base = match base {
        "left" => "←",
        "right" => "→",
        "up" => "↑",
        "down" => "↓",
        other => other,
    };
    format!("{held}{base}")
}

/// `"ctrl+o"` → `"Ctrl+O"`, `"enter"` → `"Enter"`, `"shift+left"` → `"Shift+←"`.
fn spoken(key: &str) -> String {
    let (held, base) = if key == "+" {
        ("", "+")
    } else if let Some(head) = key.strip_suffix("++") {
        (head, "+")
    } else {
        key.rsplit_once('+').unwrap_or(("", key))
    };
    let mut out = String::new();
    for m in held.split('+').filter(|m| !m.is_empty()) {
        let mut c = m.chars();
        out.extend(c.next().map(|f| f.to_ascii_uppercase()));
        out.push_str(c.as_str());
        out.push('+');
    }
    let named = [
        ("space", "Space"),
        ("enter", "Enter"),
        ("esc", "Esc"),
        ("tab", "Tab"),
        ("backspace", "Backspace"),
        ("delete", "Del"),
        ("insert", "Ins"),
        ("up", "↑"),
        ("down", "↓"),
        ("left", "←"),
        ("right", "→"),
        ("home", "Home"),
        ("end", "End"),
        ("pageup", "PgUp"),
        ("pagedown", "PgDn"),
    ];
    if let Some((_, name)) = named.iter().find(|(n, _)| *n == base) {
        out.push_str(name);
    } else if base.len() > 1 && base.starts_with('f') {
        out.push('F');
        out.push_str(&base[1..]);
    } else if held.contains("ctrl") || held.contains("alt") {
        out.push_str(&base.to_uppercase());
    } else {
        out.push_str(base);
    }
    out
}

// ----- typing a file's path in the pane ----------------------------------------

/// What a key did to a [`PathLine`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typed {
    /// The line changed (or the key wasn't for it): nothing to do yet.
    Editing,
    /// Enter: open this. `~` is already the home folder, and quotes around a
    /// dragged-in path are taken off. If it can't be opened, call
    /// [`PathLine::fail`]: the line stays open and says why.
    Open(String),
    /// Esc: the line closed.
    Closed,
}

/// A line at the bottom of the pane for typing a file's path, like a web
/// browser's address bar: type (any language) or paste, Backspace, Ctrl+U clears,
/// Tab completes a file name, `~` is the home folder, Enter opens, Esc closes.
///
/// While it is open the plugin needs every key: return [`PathLine::ready`] from
/// [`PaneSource::ready`], and your usual `Ready` again once it closes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathLine {
    open: bool,
    text: String,
    problem: Option<String>,
}

impl PathLine {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Open the line with `text` already typed (`""`, a folder, the last path).
    pub fn open(&mut self, text: &str) {
        self.open = true;
        self.text = text.to_string();
        self.problem = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.problem = None;
    }

    /// What is typed so far.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Opening what was typed failed: say why on the line, and keep it open.
    pub fn fail(&mut self, why: impl Into<String>) {
        self.problem = Some(why.into());
    }

    /// A key while the line is open (as `PaneSource::key` gets it).
    pub fn key(&mut self, key: &str, text: Option<&str>) -> Typed {
        if !self.open {
            return Typed::Closed;
        }
        match key {
            "enter" => {
                let path = expand_home(&unquote(self.text.trim()));
                if path.is_empty() {
                    self.problem = Some("type a file's path first".into());
                    return Typed::Editing;
                }
                return Typed::Open(path);
            }
            "esc" => {
                self.close();
                return Typed::Closed;
            }
            "backspace" => {
                self.text.pop();
            }
            "ctrl+u" => self.text.clear(),
            "tab" => self.text = complete(&self.text),
            _ => match text {
                // A capital arrives as "shift+h" (older stagetui sent the text "h").
                Some(t) if key.starts_with("shift+") => self.text.push_str(&t.to_uppercase()),
                Some(t) => self.text.push_str(t),
                None => return Typed::Editing,
            },
        }
        self.problem = None;
        Typed::Editing
    }

    /// Pasted (or dragged-in) text goes on the line (its first line only).
    pub fn paste(&mut self, text: &str) {
        if self.open {
            self.text
                .push_str(text.lines().find(|l| !l.trim().is_empty()).unwrap_or(""));
            self.problem = None;
        }
    }

    /// The status line while it is open: `"Open a model: ~/robot.glb█ · …"`.
    pub fn status(&self, what: &str) -> String {
        let mut line = format!("{what}: {}█", self.text);
        if let Some(p) = &self.problem {
            line.push_str(&format!(" · ⚠ {p}"));
        }
        line.push_str(" · Enter open · Tab complete · Esc cancel");
        line
    }

    /// What the plugin wants while the line is open: every key (Tab and Esc too).
    pub fn ready(&self, fps: u32) -> Ready {
        Ready {
            keys: vec!["esc".into(), "tab".into()],
            text: true,
            fps,
        }
    }
}

fn home() -> Option<std::path::PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|h| !h.is_empty())
        .map(std::path::PathBuf::from)
}

/// `~/models/a.glb` → `/home/you/models/a.glb`; anything else stays as it is.
pub fn expand_home(path: &str) -> String {
    let rest = path.strip_prefix('~');
    match (rest, home()) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            format!("{}{rest}", home.display())
        }
        _ => path.to_string(),
    }
}

/// `'/a b/c.glb'` → `/a b/c.glb`; `/a\ b/c.glb` → `/a b/c.glb` (how terminals paste
/// a dragged-in file).
fn unquote(text: &str) -> String {
    for q in ['\'', '"'] {
        if text.len() >= 2 && text.starts_with(q) && text.ends_with(q) {
            return text[1..text.len() - 1].to_string();
        }
    }
    if cfg!(windows) {
        text.to_string()
    } else {
        text.replace("\\ ", " ")
    }
}

/// Tab: finish the file name being typed — all of it when one file fits, as
/// much as they share when several do. Folders get a `/`.
fn complete(typed: &str) -> String {
    if typed == "~" {
        return format!("~{}", std::path::MAIN_SEPARATOR);
    }
    let full = expand_home(typed);
    let cut = full
        .rfind(['/', std::path::MAIN_SEPARATOR])
        .map_or(0, |i| i + 1);
    let (dir, start) = full.split_at(cut);
    if !typed.ends_with(start) {
        return typed.to_string();
    }
    let Ok(entries) = std::fs::read_dir(if dir.is_empty() { "." } else { dir }) else {
        return typed.to_string();
    };
    let mut found: Vec<(String, bool)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let hidden = name.starts_with('.') && !start.starts_with('.');
            (name.starts_with(start) && !hidden).then(|| (name, e.path().is_dir()))
        })
        .collect();
    found.sort();
    let Some((first, is_dir)) = found.first() else {
        return typed.to_string();
    };
    let mut shared = first.clone();
    for (other, _) in &found[1..] {
        let same = shared
            .char_indices()
            .zip(other.chars())
            .find(|((_, a), b)| a != b)
            .map_or(shared.len().min(other.len()), |((i, _), _)| i);
        shared.truncate(same);
    }
    if found.len() == 1 && *is_dir {
        shared.push(std::path::MAIN_SEPARATOR);
    }
    // Keep what was typed before the name (e.g. "~/"), finish the name.
    format!("{}{shared}", &typed[..typed.len() - start.len()])
}

/// One picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// `width × height × 4` bytes, RGBA, premultiplied alpha.
    pub rgba: Vec<u8>,
}

impl Frame {
    /// A quiet picture for a pane with nothing to show yet: the theme's
    /// background with a dashed square and a "+" in the middle.
    pub fn placeholder(width: u32, height: u32, theme: &Theme) -> Self {
        let bg = rgb(&theme.bg).unwrap_or([26, 27, 38]);
        let fg = rgb(&theme.fg).unwrap_or([192, 202, 245]);
        let ink: [u8; 3] =
            std::array::from_fn(|i| ((u16::from(fg[i]) * 2 + u16::from(bg[i]) * 3) / 5) as u8);
        let (w, h) = (width.max(1), height.max(1));
        let half = (w.min(h) as f32 * 0.2).max(4.0);
        let t = (half / 20.0).max(1.0);
        let dash = half / 4.0;
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        Self::from_fn(w, h, |x, y| {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let (ax, ay) = (dx.abs(), dy.abs());
            let dashed = |along: f32| ((along + half) / dash).floor() as i32 % 2 == 0;
            let side = (ax - half).abs() <= t && ay <= half + t && dashed(dy);
            let top = (ay - half).abs() <= t && ax <= half + t && dashed(dx);
            let plus = (ax <= t && ay <= half * 0.4) || (ay <= t && ax <= half * 0.4);
            let c = if side || top || plus { ink } else { bg };
            [c[0], c[1], c[2], 255]
        })
    }

    /// A frame where `color(x, y)` gives each pixel's RGBA.
    pub fn from_fn(width: u32, height: u32, color: impl Fn(u32, u32) -> [u8; 4]) -> Self {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&color(x, y));
            }
        }
        Self {
            width,
            height,
            rgba,
        }
    }
}

// ----- framing ---------------------------------------------------------------

pub fn write_message(w: &mut impl Write, kind: u8, payload: &[u8]) -> io::Result<()> {
    let len = u32::try_from(payload.len() + 1).map_err(|_| io::Error::other("too large"))?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&[kind])?;
    w.write_all(payload)?;
    w.flush()
}

pub fn read_message(r: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad message size",
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    let kind = buf.remove(0);
    Ok((kind, buf))
}

pub fn write_json<T: Serialize>(w: &mut impl Write, msg: &T) -> io::Result<()> {
    write_message(
        w,
        KIND_JSON,
        &serde_json::to_vec(msg).map_err(io::Error::other)?,
    )
}

pub fn write_frame(w: &mut impl Write, frame: &Frame) -> io::Result<()> {
    let expected = frame.width as usize * frame.height as usize * 4;
    if frame.rgba.len() != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "frame has {} bytes, {}×{} needs {expected}",
                frame.rgba.len(),
                frame.width,
                frame.height
            ),
        ));
    }
    let mut payload = Vec::with_capacity(8 + expected);
    payload.extend_from_slice(&frame.width.to_le_bytes());
    payload.extend_from_slice(&frame.height.to_le_bytes());
    payload.extend_from_slice(&frame.rgba);
    write_message(w, KIND_FRAME, &payload)
}

/// Read a frame's payload (`[u32 w][u32 h][rgba…]`).
pub fn parse_frame(payload: &[u8]) -> Option<Frame> {
    let w = u32::from_le_bytes(payload.get(0..4)?.try_into().ok()?);
    let h = u32::from_le_bytes(payload.get(4..8)?.try_into().ok()?);
    let rgba = payload.get(8..)?;
    (rgba.len() == w as usize * h as usize * 4 && w > 0 && h > 0).then(|| Frame {
        width: w,
        height: h,
        rgba: rgba.to_vec(),
    })
}

// ----- writing a plugin ------------------------------------------------------

/// A pane type. Implement the parts you need; `frame` is the heart of it.
pub trait PaneSource {
    /// Name, title and icon (for `--describe`).
    fn describe(&self) -> Describe;

    /// The pane opened. Return what you need, or a message why you can't start.
    fn start(&mut self, start: &Start) -> Result<Ready, String>;

    /// The pane changed size (pixels).
    fn resize(&mut self, _width: u32, _height: u32) {}

    /// Called up to `fps` times a second: return a frame when the picture changed.
    fn frame(&mut self, now: Duration) -> Option<Frame>;

    /// A key from `Ready.keys` (or any key with `Ready.text`). Return `true` if used.
    fn key(&mut self, _key: &str, _text: Option<&str>) -> bool {
        false
    }

    fn mouse(&mut self, _mouse: &Mouse) {}

    fn paste(&mut self, _text: &str) {}

    fn focus(&mut self, _focused: bool) {}

    /// The pane went out of sight, or came back. `frame` isn't called while it's
    /// hidden; pause what makes sound or costs work (a video, a browser).
    /// `frame`'s `now` keeps counting while hidden, so the first one back can
    /// jump: limit any step you take from the time between frames.
    fn shown(&mut self, _shown: bool) {}

    /// A method the app calls by name.
    fn call(&mut self, method: &str, _args: &Value) -> Result<Value, String> {
        Err(format!("this plugin has no method \"{method}\""))
    }

    /// Status line text, checked after every message and frame (`None`: unchanged).
    fn status(&mut self) -> Option<String> {
        None
    }

    /// What the plugin wants changed while it runs, checked after every message
    /// and frame (`None`: unchanged) — e.g. every key while a [`PathLine`] is
    /// open, then its usual keys again.
    fn ready(&mut self) -> Option<Ready> {
        None
    }

    /// Events for the app, checked after every message and frame.
    fn events(&mut self) -> Vec<(String, Value)> {
        Vec::new()
    }

    /// The pane is closing.
    fn stop(&mut self) {}
}

/// Run a plugin: answer `--describe`, then talk to stagetui over stdin/stdout
/// until the pane closes. Returns the exit code for `std::process::exit`.
pub fn serve<S: PaneSource>(source: S) -> i32 {
    if std::env::args().nth(1).as_deref() == Some("--describe") {
        let json = serde_json::to_string(&source.describe()).unwrap_or_default();
        println!("{json}");
        return 0;
    }
    let (tx, rx) = mpsc::channel::<ToPlugin>();
    std::thread::spawn(move || {
        let mut input = BufReader::new(io::stdin().lock());
        while let Ok((kind, payload)) = read_message(&mut input) {
            if kind != KIND_JSON {
                continue;
            }
            match serde_json::from_slice::<ToPlugin>(&payload) {
                Ok(msg) => {
                    if tx.send(msg).is_err() {
                        break;
                    }
                }
                Err(_) => continue,
            }
        }
        let _ = tx.send(ToPlugin::Stop);
    });
    run(source, rx, &mut BufWriter::new(io::stdout().lock()))
}

/// The heart of `serve`: messages from `rx`, answers and frames to `out`.
fn run<S: PaneSource>(mut source: S, rx: mpsc::Receiver<ToPlugin>, out: &mut impl Write) -> i32 {
    let begun = Instant::now();
    let mut every = Duration::from_millis(33);
    let mut started = false;
    let mut shown = true;
    let mut next = Instant::now();
    loop {
        let msg = if started && shown {
            match rx.recv_timeout(next.saturating_duration_since(Instant::now())) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => Some(ToPlugin::Stop),
            }
        } else {
            // Nothing to draw (not started, or out of sight): just wait for news.
            Some(rx.recv().unwrap_or(ToPlugin::Stop))
        };
        let sent = match msg {
            Some(ToPlugin::Start(s)) => match source.start(&s) {
                Ok(ready) => {
                    every = Duration::from_secs_f64(1.0 / f64::from(ready.fps.clamp(1, 120)));
                    started = true;
                    next = Instant::now();
                    write_json(out, &FromPlugin::Ready(ready))
                }
                Err(why) => write_json(
                    out,
                    &FromPlugin::Error {
                        what: format!("{} could not start", source.describe().title),
                        why,
                        fix: "check the pane's options, then press Enter to try again".into(),
                    },
                ),
            },
            Some(ToPlugin::Resize { width, height, .. }) => {
                source.resize(width, height);
                next = Instant::now();
                Ok(())
            }
            Some(ToPlugin::Key { key, text }) => {
                source.key(&key, text.as_deref());
                next = Instant::now();
                Ok(())
            }
            Some(ToPlugin::Mouse(m)) => {
                source.mouse(&m);
                next = Instant::now();
                Ok(())
            }
            Some(ToPlugin::Paste { text }) => {
                source.paste(&text);
                Ok(())
            }
            Some(ToPlugin::Focus { focused }) => {
                source.focus(focused);
                Ok(())
            }
            Some(ToPlugin::Shown { shown: now_shown }) => {
                if now_shown != shown {
                    shown = now_shown;
                    source.shown(shown);
                    next = Instant::now();
                }
                Ok(())
            }
            Some(ToPlugin::Call { id, method, args }) => {
                let reply = match source.call(&method, &args) {
                    Ok(value) => FromPlugin::Reply {
                        id,
                        value,
                        error: None,
                    },
                    Err(e) => FromPlugin::Reply {
                        id,
                        value: Value::Null,
                        error: Some(e),
                    },
                };
                write_json(out, &reply)
            }
            Some(ToPlugin::Stop) => {
                source.stop();
                return 0;
            }
            None => {
                next = Instant::now() + every;
                match source.frame(begun.elapsed()) {
                    Some(f) => write_frame(out, &f),
                    None => Ok(()),
                }
            }
        };
        let mut talk = || -> io::Result<()> {
            if started && let Some(ready) = source.ready() {
                every = Duration::from_secs_f64(1.0 / f64::from(ready.fps.clamp(1, 120)));
                write_json(out, &FromPlugin::Ready(ready))?;
            }
            if let Some(text) = source.status() {
                write_json(out, &FromPlugin::Status { text })?;
            }
            for (name, value) in source.events() {
                write_json(out, &FromPlugin::Event { name, value })?;
            }
            Ok(())
        };
        if sent.and_then(|()| talk()).is_err() {
            // stagetui went away: nothing left to draw for.
            source.stop();
            return 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_and_frames_round_trip() {
        let mut buf = Vec::new();
        let start = ToPlugin::Start(Start {
            width: 640,
            height: 360,
            cols: 80,
            rows: 18,
            args: serde_json::json!({"path": "مَرْحَبًا Hello 日本語 नमस्ते.mp4"}),
            theme: Theme::default(),
            protocol: PROTOCOL,
        });
        write_json(&mut buf, &start).unwrap();
        let frame = Frame::from_fn(3, 2, |x, y| [x as u8, y as u8, 7, 255]);
        write_frame(&mut buf, &frame).unwrap();
        let mut r = io::Cursor::new(buf);
        let (kind, json) = read_message(&mut r).unwrap();
        assert_eq!(kind, KIND_JSON);
        assert_eq!(serde_json::from_slice::<ToPlugin>(&json).unwrap(), start);
        let (kind, payload) = read_message(&mut r).unwrap();
        assert_eq!((kind, parse_frame(&payload).unwrap()), (KIND_FRAME, frame));
        assert!(
            parse_frame(&[1, 0, 0, 0, 1, 0, 0, 0, 9]).is_none(),
            "wrong size"
        );
        let bad = Frame {
            width: 2,
            height: 2,
            rgba: vec![0; 3],
        };
        assert!(write_frame(&mut Vec::new(), &bad).is_err());
    }

    #[test]
    fn json_is_plain_and_forgiving() {
        let ready: FromPlugin = serde_json::from_str(r#"{"t":"ready"}"#).unwrap();
        assert_eq!(ready, FromPlugin::Ready(Ready::default()));
        let key: ToPlugin = serde_json::from_str(r#"{"t":"key","key":"space"}"#).unwrap();
        assert_eq!(
            key,
            ToPlugin::Key {
                key: "space".into(),
                text: None
            }
        );
        let d: Describe = serde_json::from_str(r#"{"name":"web","title":"Web"}"#).unwrap();
        assert!(d.pictures && d.protocol == PROTOCOL);
    }

    #[test]
    fn shown_round_trips() {
        let hide = ToPlugin::Shown { shown: false };
        let show = ToPlugin::Shown { shown: true };
        let mut buf = Vec::new();
        write_json(&mut buf, &hide).unwrap();
        write_json(&mut buf, &show).unwrap();
        let exact = br#"{"t":"shown","shown":false}"#;
        // [u32 length = kind + payload][kind][payload]
        assert_eq!(buf[..4], ((exact.len() + 1) as u32).to_le_bytes());
        let mut r = io::Cursor::new(buf);
        let (kind, json) = read_message(&mut r).unwrap();
        assert_eq!((kind, json.as_slice()), (KIND_JSON, &exact[..]));
        assert_eq!(serde_json::from_slice::<ToPlugin>(&json).unwrap(), hide);
        let (kind, json) = read_message(&mut r).unwrap();
        assert_eq!(kind, KIND_JSON);
        assert_eq!(serde_json::from_slice::<ToPlugin>(&json).unwrap(), show);
        assert!(read_message(&mut r).is_err(), "nothing more");
        // A message this version doesn't know fails to read, and `serve` skips it:
        // that is how plugins built before `shown` ignore it.
        assert!(serde_json::from_str::<ToPlugin>(r#"{"t":"someday","x":1}"#).is_err());
    }

    const ACTIONS: &[KeyAction] = &[
        KeyAction {
            name: "spin",
            keys: &["space"],
            about: "spin or stop",
        },
        KeyAction {
            name: "zoom_in",
            keys: &["+", "="],
            about: "closer",
        },
        KeyAction {
            name: "back",
            keys: &["shift+left"],
            about: "back 30 s",
        },
        KeyAction {
            name: "wires",
            keys: &["w"],
            about: "show the edges",
        },
        KeyAction {
            name: "grid",
            keys: &["g"],
            about: "the floor grid",
        },
    ];

    fn keymap(keys: Value) -> Result<Keymap, String> {
        Keymap::new(
            ACTIONS,
            &serde_json::json!({ "path": "a.glb", "keys": keys }),
        )
    }

    #[test]
    fn keymap_defaults_rename_and_turn_off() {
        let plain = Keymap::new(ACTIONS, &Value::Null).unwrap();
        assert_eq!(
            plain,
            keymap(Value::Null).unwrap(),
            "no keys= means the defaults"
        );
        assert_eq!(plain.action("space"), Some("spin"));
        assert_eq!(plain.action("="), Some("zoom_in"));
        assert_eq!(plain.keys("zoom_in"), ["+", "="]);
        assert_eq!(
            plain.all_keys(),
            ["space", "+", "=", "shift+left", "w", "g"]
        );

        let k = keymap(serde_json::json!({"wires": "x", "grid": null, "spin": ["s", "space"]}))
            .unwrap();
        assert_eq!(k.action("x"), Some("wires"));
        assert_eq!(k.action("w"), None, "the old key does nothing now");
        assert_eq!(k.action("g"), None);
        assert!(
            k.keys("grid").is_empty() && k.label("grid").is_none(),
            "off"
        );
        assert_eq!(k.keys("spin"), ["s", "space"]);
        assert_eq!(k.label("spin").as_deref(), Some("s"), "the first key");
        assert_eq!(k.all_keys(), ["s", "space", "+", "=", "shift+left", "x"]);
        assert!(
            keymap(serde_json::json!({"zoom_in": []}))
                .unwrap()
                .keys("zoom_in")
                .is_empty()
        );
        assert!(k.keys("explode").is_empty() && k.label("explode").is_none());
    }

    #[test]
    fn keymap_names_keys_like_stagetui() {
        // Modifier order and other names for the same key don't matter; as in
        // BINDINGS, a capital letter means Shift.
        let k = keymap(serde_json::json!({
            "wires": "Ctrl+w", "grid": "shift+ctrl+g", "zoom_in": "plus", "spin": "Escape",
            "back": ["SHIFT+LEFT", "J"]
        }))
        .unwrap();
        assert_eq!(k.keys("wires"), ["ctrl+w"]);
        assert_eq!(k.action("ctrl+w"), Some("wires"));
        assert_eq!(k.keys("grid"), ["ctrl+shift+g"]);
        assert_eq!(k.keys("back"), ["shift+left", "shift+j"]);
        assert_eq!(k.action("shift+j"), Some("back"));
        assert_eq!(k.action("j"), None, "plain j is another key");
        assert_eq!(k.keys("zoom_in"), ["+"]);
        assert_eq!(k.keys("spin"), ["esc"]);
        assert_eq!(k.label("back").as_deref(), Some("shift+←"));
        for (written, named) in [
            ("+", "+"),
            ("ctrl++", "ctrl++"),
            ("shift+?", "?"),
            ("shift+space", "shift+space"),
            ("F5", "f5"),
            ("page_down", "pagedown"),
            ("option+left", "alt+left"),
            ("cmd+shift+alt+ctrl+k", "ctrl+alt+super+shift+k"),
            ("W", "shift+w"),
            ("Ctrl+S", "ctrl+shift+s"),
            ("shift+w", "shift+w"),
            ("ENTER", "enter"),
            ("Ж", "shift+ж"),
            // Keys on other keyboards: Arabic, Japanese, Devanagari.
            ("ب", "ب"),
            ("ctrl+あ", "ctrl+あ"),
            ("क", "क"),
        ] {
            assert_eq!(key_name(written).as_deref(), Some(named), "{written}");
            // What stagetui sends reads back the same.
            assert_eq!(key_name(named).as_deref(), Some(named), "{named}");
        }
        for bad in ["", "  ", "ctrl+", "hyper+x", "banana", "f25", "f0"] {
            assert_eq!(key_name(bad), None, "{bad:?}");
        }
        assert_eq!(short("left"), "←");
        assert_eq!(short("ctrl++"), "ctrl++");
        assert_eq!(short("alt+right"), "alt+→");
        assert_eq!(short("+"), "+");
        for (key, said) in [
            ("enter", "Enter"),
            ("o", "o"),
            ("ctrl+o", "Ctrl+O"),
            ("shift+left", "Shift+←"),
            ("ctrl++", "Ctrl++"),
            ("f5", "F5"),
            ("space", "Space"),
            ("f", "f"),
        ] {
            assert_eq!(spoken(key), said, "{key}");
        }
        assert_eq!(k.spoken("back").as_deref(), Some("Shift+←"));
    }

    #[test]
    fn keymap_mistakes_are_explained() {
        // A name that isn't an action: the real ones are listed.
        let e = keymap(serde_json::json!({"wire": "x"})).unwrap_err();
        assert!(
            e.contains("\"wire\"") && e.contains("spin, zoom_in, back, wires, grid"),
            "{e}"
        );
        let e = keymap(serde_json::json!({"مَرْحَبًا Hello 日本語 नमस्ते": "x"})).unwrap_err();
        assert!(
            e.contains("مَرْحَبًا Hello 日本語 नमस्ते") && e.contains("wires"),
            "{e}"
        );
        // One key, two actions — also when one of them keeps its default.
        let e = keymap(serde_json::json!({"wires": "x", "grid": ["h", "x"]})).unwrap_err();
        assert!(
            e.contains("\"x\"") && e.contains("\"wires\"") && e.contains("\"grid\""),
            "{e}"
        );
        let e = keymap(serde_json::json!({"wires": "g"})).unwrap_err();
        assert!(e.contains("\"wires\" and \"grid\""), "{e}");
        assert!(e.contains("keys={\"grid\": None}"), "it says how: {e}");
        // Moving the other one away too is fine.
        let k = keymap(serde_json::json!({"wires": "g", "grid": "h"})).unwrap();
        assert_eq!(
            (k.action("g"), k.action("h")),
            (Some("wires"), Some("grid"))
        );
        // The same key twice for one action is just one key; a capital is another.
        assert_eq!(
            keymap(serde_json::json!({"wires": ["x", "x", "X", "shift+x"]}))
                .unwrap()
                .keys("wires"),
            ["x", "shift+x"]
        );
        // Not a key, not a list of keys, not a dict.
        let e = keymap(serde_json::json!({"wires": "ctrl+banana"})).unwrap_err();
        assert!(
            e.contains("\"ctrl+banana\"") && e.contains("\"wires\""),
            "{e}"
        );
        let e = keymap(serde_json::json!({"wires": 5})).unwrap_err();
        assert!(e.contains("None to turn it off"), "{e}");
        let e = keymap(serde_json::json!({"wires": ["x", true]})).unwrap_err();
        assert!(e.contains("a list of keys"), "{e}");
        let e = keymap(serde_json::json!("x")).unwrap_err();
        assert!(
            e.contains("keys={\"spin\": \"x\"}") && e.contains("wires"),
            "{e}"
        );
    }

    #[test]
    fn a_path_line_types_completes_and_opens() {
        let dir = std::env::temp_dir().join(format!("stagetui-path-{}", std::process::id()));
        let name = "مَرْحَبًا Hello 日本語 नमस्ते";
        std::fs::create_dir_all(dir.join("models")).unwrap();
        std::fs::write(dir.join(format!("{name}.glb")), b"x").unwrap();
        std::fs::write(dir.join("robot-one.obj"), b"x").unwrap();
        std::fs::write(dir.join("robot-two.obj"), b"x").unwrap();
        std::fs::write(dir.join(".hidden"), b"x").unwrap();
        let base = format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR);

        let mut line = PathLine::default();
        assert!(!line.is_open());
        assert_eq!(
            line.key("a", Some("a")),
            Typed::Closed,
            "closed: not for it"
        );
        line.open(&base);
        assert!(line.ready(30).text && line.ready(30).keys.contains(&"esc".to_string()));
        // Tab: one file fits — all of it; several — what they share; a folder gets a "/".
        line.key("m", Some("م"));
        line.key("tab", None);
        assert_eq!(line.text(), format!("{base}{name}.glb"));
        line.open(&format!("{base}rob"));
        line.key("tab", None);
        assert_eq!(line.text(), format!("{base}robot-"));
        line.open(&format!("{base}mo"));
        line.key("tab", None);
        assert_eq!(
            line.text(),
            format!("{base}models{}", std::path::MAIN_SEPARATOR)
        );
        line.open(&format!("{base}zzz"));
        line.key("tab", None);
        assert_eq!(line.text(), format!("{base}zzz"), "nothing fits: as it was");
        line.open(&base);
        line.key("tab", None);
        assert_eq!(line.text(), base, "hidden files only when asked for");

        // Typing (capitals too), Backspace, Ctrl+U.
        line.open("");
        for (key, text) in [("shift+h", "h"), ("i", "i"), ("space", " "), ("ب", "ب")] {
            assert_eq!(line.key(key, Some(text)), Typed::Editing);
        }
        assert_eq!(line.text(), "Hi ب");
        line.key("backspace", None);
        line.key("ctrl+r", None);
        assert_eq!(line.text(), "Hi ");
        line.key("ctrl+u", None);
        assert_eq!(line.text(), "");
        // Enter with nothing typed asks for a path; the line stays.
        assert_eq!(line.key("enter", None), Typed::Editing);
        assert!(line.is_open() && line.status("Open a model").contains("type a file's path"));
        // Enter: the path, with ~ and quotes sorted out.
        line.paste("'~/a b.glb'\n");
        let home = home().unwrap().display().to_string();
        assert_eq!(
            line.key("enter", None),
            Typed::Open(format!("{home}/a b.glb"))
        );
        // It failed: the reason shows, the line stays open, typing clears it.
        line.fail("\"a b.glb\" was not found");
        let status = line.status("Open a model");
        assert!(status.starts_with("Open a model: '~/a b.glb'█ · ⚠ \"a b.glb\" was not found"));
        assert!(
            status.ends_with("Enter open · Tab complete · Esc cancel"),
            "{status}"
        );
        line.key("x", Some("x"));
        assert!(!line.status("Open").contains('⚠'));
        assert_eq!(line.key("esc", None), Typed::Closed);
        assert!(!line.is_open());
        assert_eq!(expand_home("~"), home);
        assert_eq!(expand_home("~x/a"), "~x/a");
        assert_eq!(expand_home("https://a.b/c.mp4"), "https://a.b/c.mp4");
        if !cfg!(windows) {
            assert_eq!(unquote("/a\\ b.glb"), "/a b.glb");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_placeholder_is_quiet() {
        let theme = Theme {
            bg: "#000000".into(),
            fg: "#ffffff".into(),
            accent: String::new(),
        };
        let f = Frame::placeholder(200, 100, &theme);
        assert_eq!(f.rgba.len(), 200 * 100 * 4);
        let px = |x: usize, y: usize| f.rgba[(y * 200 + x) * 4];
        assert_eq!(px(0, 0), 0, "the background in the corners");
        assert_eq!(px(100, 50), 102, "a muted plus in the middle");
    }

    /// Wants every key after "enter", its usual keys again after "esc".
    #[derive(Default)]
    struct Typist {
        line: PathLine,
        sent: Option<Ready>,
    }

    impl Typist {
        fn wanted(&self) -> Ready {
            if self.line.is_open() {
                self.line.ready(20)
            } else {
                Ready {
                    keys: vec!["enter".into()],
                    ..Ready::default()
                }
            }
        }
    }

    impl PaneSource for Typist {
        fn describe(&self) -> Describe {
            Describe::new("typist", "Typist", "cube")
        }
        fn start(&mut self, _start: &Start) -> Result<Ready, String> {
            self.sent = Some(self.wanted());
            Ok(self.wanted())
        }
        fn frame(&mut self, _now: Duration) -> Option<Frame> {
            None
        }
        fn key(&mut self, key: &str, text: Option<&str>) -> bool {
            if self.line.is_open() {
                self.line.key(key, text);
            } else {
                self.line.open("");
            }
            true
        }
        fn ready(&mut self) -> Option<Ready> {
            let want = self.wanted();
            (self.sent.as_ref() != Some(&want)).then(|| {
                self.sent = Some(want.clone());
                want
            })
        }
    }

    #[test]
    fn what_a_plugin_wants_can_change_while_it_runs() {
        let (tx, rx) = mpsc::channel();
        tx.send(ToPlugin::Start(Start {
            width: 8,
            height: 8,
            cols: 1,
            rows: 1,
            args: Value::Null,
            theme: Theme::default(),
            protocol: PROTOCOL,
        }))
        .unwrap();
        for key in ["enter", "a", "esc"] {
            tx.send(ToPlugin::Key {
                key: key.into(),
                text: None,
            })
            .unwrap();
        }
        tx.send(ToPlugin::Stop).unwrap();
        let mut out = Vec::new();
        assert_eq!(run(Typist::default(), rx, &mut out), 0);
        let mut r = io::Cursor::new(out);
        let mut readies = Vec::new();
        while let Ok((_, json)) = read_message(&mut r) {
            if let Ok(FromPlugin::Ready(ready)) = serde_json::from_slice(&json) {
                readies.push((ready.text, ready.keys));
            }
        }
        // Started; the line opened (every key); it closed (the usual keys) — once each.
        assert_eq!(
            readies,
            [
                (false, vec!["enter".to_string()]),
                (true, vec!["esc".to_string(), "tab".to_string()]),
                (false, vec!["enter".to_string()]),
            ]
        );
    }

    /// Counts frames asked for, and remembers `shown` calls.
    struct Counter {
        frames: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        shown: std::sync::Arc<std::sync::Mutex<Vec<bool>>>,
    }

    impl PaneSource for Counter {
        fn describe(&self) -> Describe {
            Describe::new("count", "Count", "cube")
        }
        fn start(&mut self, _start: &Start) -> Result<Ready, String> {
            Ok(Ready {
                fps: 120,
                ..Ready::default()
            })
        }
        fn frame(&mut self, _now: Duration) -> Option<Frame> {
            self.frames
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            None
        }
        fn shown(&mut self, shown: bool) {
            self.shown.lock().unwrap().push(shown);
        }
    }

    type Run = std::thread::JoinHandle<(i32, Vec<u8>)>;

    fn counted(
        rx: mpsc::Receiver<ToPlugin>,
    ) -> (
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
        std::sync::Arc<std::sync::Mutex<Vec<bool>>>,
        Run,
    ) {
        let frames = std::sync::Arc::default();
        let shown = std::sync::Arc::default();
        let source = Counter {
            frames: std::sync::Arc::clone(&frames),
            shown: std::sync::Arc::clone(&shown),
        };
        let runner = std::thread::spawn(move || {
            let mut out = Vec::new();
            (run(source, rx, &mut out), out)
        });
        (frames, shown, runner)
    }

    #[test]
    fn no_frames_before_start_or_while_hidden() {
        use std::sync::atomic::Ordering::SeqCst;
        let settle = || std::thread::sleep(Duration::from_millis(120));
        // Not started: no frames, and the app going away ends it.
        let (tx, rx) = mpsc::channel();
        let (frames, _, runner) = counted(rx);
        settle();
        assert_eq!(frames.load(SeqCst), 0, "not started");
        drop(tx);
        assert_eq!(runner.join().unwrap(), (0, Vec::new()));

        // Hidden right after starting (queued before it runs, so no frame can
        // slip in between): input still arrives, but nothing is drawn.
        let (tx, rx) = mpsc::channel();
        tx.send(ToPlugin::Start(Start {
            width: 64,
            height: 32,
            cols: 8,
            rows: 2,
            args: Value::Null,
            theme: Theme::default(),
            protocol: PROTOCOL,
        }))
        .unwrap();
        tx.send(ToPlugin::Shown { shown: false }).unwrap();
        tx.send(ToPlugin::Shown { shown: false }).unwrap();
        tx.send(ToPlugin::Key {
            key: "space".into(),
            text: None,
        })
        .unwrap();
        let (frames, shown, runner) = counted(rx);
        settle();
        assert_eq!(frames.load(SeqCst), 0, "hidden: no frames");
        tx.send(ToPlugin::Shown { shown: true }).unwrap();
        settle();
        assert!(frames.load(SeqCst) > 0, "back in sight: frames again");
        tx.send(ToPlugin::Stop).unwrap();
        let (code, out) = runner.join().unwrap();
        assert_eq!(code, 0);
        assert_eq!(*shown.lock().unwrap(), [false, true], "only changes");
        let (kind, json) = read_message(&mut io::Cursor::new(out)).unwrap();
        assert_eq!(kind, KIND_JSON);
        assert!(matches!(
            serde_json::from_slice(&json).unwrap(),
            FromPlugin::Ready(Ready { fps: 120, .. })
        ));
    }
}
