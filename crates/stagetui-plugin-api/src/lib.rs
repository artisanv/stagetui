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

impl Default for Ready {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            text: false,
            fps: fps(),
        }
    }
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

    /// A method the app calls by name.
    fn call(&mut self, method: &str, _args: &Value) -> Result<Value, String> {
        Err(format!("this plugin has no method \"{method}\""))
    }

    /// Status line text, checked after every message and frame (`None`: unchanged).
    fn status(&mut self) -> Option<String> {
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
pub fn serve<S: PaneSource>(mut source: S) -> i32 {
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
    let mut out = BufWriter::new(io::stdout().lock());
    let begun = Instant::now();
    let mut every = Duration::from_millis(33);
    let mut started = false;
    let mut next = Instant::now();
    loop {
        let wait = if started {
            next.saturating_duration_since(Instant::now())
        } else {
            Duration::from_secs(3600)
        };
        let msg = match rx.recv_timeout(wait) {
            Ok(m) => Some(m),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => Some(ToPlugin::Stop),
        };
        let sent = match msg {
            Some(ToPlugin::Start(s)) => match source.start(&s) {
                Ok(ready) => {
                    every = Duration::from_secs_f64(1.0 / f64::from(ready.fps.clamp(1, 120)));
                    started = true;
                    next = Instant::now();
                    write_json(&mut out, &FromPlugin::Ready(ready))
                }
                Err(why) => write_json(
                    &mut out,
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
                write_json(&mut out, &reply)
            }
            Some(ToPlugin::Stop) => {
                source.stop();
                return 0;
            }
            None => {
                next = Instant::now() + every;
                match source.frame(begun.elapsed()) {
                    Some(f) => write_frame(&mut out, &f),
                    None => Ok(()),
                }
            }
        };
        let mut talk = || -> io::Result<()> {
            if let Some(text) = source.status() {
                write_json(&mut out, &FromPlugin::Status { text })?;
            }
            for (name, value) in source.events() {
                write_json(&mut out, &FromPlugin::Event { name, value })?;
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
}
