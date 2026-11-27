//! A pane type served by a plugin program — video, a web page, a 3D view…
//!
//! The program starts when the widget is first drawn, at the widget's size in
//! pixels. It sends frames (pictures); this widget shows the latest one with
//! the terminal's best graphics. Keys the plugin asked for, the mouse, and
//! pastes go to it. The process is tracked by the reaper and stops with the
//! widget. If it crashes, the pane says so and Enter starts it again.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, BufWriter};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use serde_json::{Map, Value, json};
use stagetui_plugin_api::{self as api, FromPlugin, Ready, ToPlugin};

use super::{Cx, PaintCx, Reaction, Widget, object};
use crate::frame::Frame;
use crate::gfx::{Fit, Image};
use crate::input::{Code, Key, MouseKind};
use crate::reaper::TrackedChild;
use crate::{Error, Result, text};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A message in the pane instead of a picture: what, why, fix.
#[derive(Debug, Clone)]
struct Problem {
    what: String,
    why: String,
    fix: String,
    /// The plugin isn't installed (Enter asks to install it).
    missing: bool,
}

#[derive(Default)]
struct Inner {
    child: Mutex<Option<TrackedChild>>,
    stdin: Mutex<Option<BufWriter<ChildStdin>>>,
    frame: Mutex<Option<Image>>,
    ready: Mutex<Option<Ready>>,
    status: Mutex<Option<String>>,
    problem: Mutex<Option<Problem>>,
    events: Mutex<Vec<(String, Value)>>,
    replies: Mutex<HashMap<u64, (Value, Option<String>)>>,
    replied: Condvar,
    calls: AtomicU64,
    /// Last size sent: pixels and cells.
    size: Mutex<(u32, u32, u16, u16)>,
    cell_px: Mutex<(u32, u32)>,
    generation: AtomicU64,
    painted: AtomicU64,
    /// Frames received (for `describe`: how smoothly it plays).
    frames: AtomicU64,
    started: AtomicBool,
    stopping: AtomicBool,
    stderr: Mutex<VecDeque<String>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(w) = self
            .stdin
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let _ = api::write_json(w, &ToPlugin::Stop);
        }
        // The tracked child is killed when dropped.
    }
}

impl Inner {
    fn send(&self, msg: &ToPlugin) -> bool {
        match lock(&self.stdin).as_mut() {
            Some(w) => api::write_json(w, msg).is_ok(),
            None => false,
        }
    }

    fn changed(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        crate::app::wake_all();
    }

    fn fail(&self, problem: Problem) {
        *lock(&self.problem) = Some(problem);
        self.changed();
    }
}

#[derive(Clone)]
pub struct PluginView {
    /// The plugin (pane type) name, e.g. `"video"`.
    pub name: String,
    /// Options for it, e.g. `{"path": "movie.mp4"}`.
    pub args: Value,
    pub height: Option<u16>,
    inner: Arc<Inner>,
}

impl std::fmt::Debug for PluginView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PluginView({})", self.name)
    }
}

impl PluginView {
    pub fn new(name: &str, args: Value) -> Self {
        Self {
            name: name.to_string(),
            args,
            height: None,
            inner: Arc::new(Inner::default()),
        }
    }

    /// Keep the running program when the widget is rebuilt (updates).
    pub fn carry(&mut self, old: &PluginView) {
        if old.name == self.name {
            self.inner = Arc::clone(&old.inner);
        }
    }

    fn title(&self) -> String {
        crate::plugins::find(&self.name)
            .map(|p| p.describe.title)
            .or_else(|| crate::plugins::known(&self.name).map(|k| k.title.to_string()))
            .unwrap_or_else(|| self.name.clone())
    }

    /// Start the program at this size (once).
    fn start(&self, size: (u32, u32, u16, u16), theme: &crate::style::Theme) {
        if self.inner.started.swap(true, Ordering::SeqCst) {
            return;
        }
        *lock(&self.inner.size) = size;
        let Some(found) = crate::plugins::find(&self.name) else {
            let known = crate::plugins::known(&self.name);
            let (title, package) = match known {
                Some(k) => (k.title.to_string(), k.package.to_string()),
                None => (self.name.clone(), format!("stagetui-plugin-{}", self.name)),
            };
            let problem = match known {
                Some(k) if !k.available => Problem {
                    what: format!("{title} is coming soon"),
                    why: format!("the {package} add-on isn't out yet"),
                    fix: "once it is, you can install it right here".into(),
                    missing: true,
                },
                Some(_) => Problem {
                    what: format!("{title} isn't installed yet"),
                    why: format!("this pane needs the {package} add-on"),
                    fix: "press Enter (or click here) to install it".into(),
                    missing: true,
                },
                None => Problem {
                    what: format!("No add-on called \"{}\"", self.name),
                    why: format!("there is no {package} program on this computer"),
                    fix: "install it, or put it in STAGETUI_PLUGIN_PATH, then press Enter".into(),
                    missing: false,
                },
            };
            self.inner.fail(problem);
            return;
        };
        if let Err(e) = self.spawn(&found.path, size, theme) {
            self.inner.fail(Problem {
                what: format!("{} could not start", found.describe.title),
                why: e.to_string(),
                fix: "press Enter to try again".into(),
                missing: false,
            });
        }
    }

    fn spawn(
        &self,
        path: &std::path::Path,
        (width, height, cols, rows): (u32, u32, u16, u16),
        theme: &crate::style::Theme,
    ) -> Result<()> {
        let mut cmd = Command::new(path);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = crate::reaper::global().spawn(cmd)?;
        let proc = child
            .child_mut()
            .ok_or_else(|| Error::friendly("No plugin process", "it ended at once", "try again"))?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (proc.stdin.take(), proc.stdout.take(), proc.stderr.take())
        else {
            return Err(Error::friendly(
                "The plugin has no pipes",
                "it could not be connected to",
                "try again",
            ));
        };
        *lock(&self.inner.stdin) = Some(BufWriter::new(stdin));
        *lock(&self.inner.child) = Some(child);
        *lock(&self.inner.problem) = None;
        let start = ToPlugin::Start(api::Start {
            width,
            height,
            cols,
            rows,
            args: self.args.clone(),
            theme: api::Theme {
                bg: theme.bg.hex(),
                fg: theme.fg.hex(),
                accent: theme.accent.hex(),
            },
            protocol: api::PROTOCOL,
        });
        self.inner.send(&start);
        let title = self.title();
        let weak = Arc::downgrade(&self.inner);
        std::thread::Builder::new()
            .name("stagetui-plugin-err".into())
            .spawn(move || {
                for line in BufReader::new(stderr)
                    .lines()
                    .map_while(std::io::Result::ok)
                {
                    let Some(inner) = weak.upgrade() else { break };
                    let mut tail = lock(&inner.stderr);
                    tail.push_back(line);
                    while tail.len() > 20 {
                        tail.pop_front();
                    }
                }
            })?;
        let weak = Arc::downgrade(&self.inner);
        let name = self.name.clone();
        std::thread::Builder::new()
            .name("stagetui-plugin".into())
            .spawn(move || {
                let mut out = BufReader::new(stdout);
                while let Ok((kind, payload)) = api::read_message(&mut out) {
                    let Some(inner) = weak.upgrade() else { return };
                    match kind {
                        api::KIND_FRAME => {
                            if let Some(f) = api::parse_frame(&payload)
                                && let Some(size) = tiny_skia::IntSize::from_wh(f.width, f.height)
                                && let Some(px) = tiny_skia::Pixmap::from_vec(f.rgba, size)
                            {
                                *lock(&inner.frame) =
                                    Some(Image::from_pixmap(format!("plugin-{name}"), px));
                                inner.frames.fetch_add(1, Ordering::SeqCst);
                                inner.changed();
                            }
                        }
                        api::KIND_JSON => match serde_json::from_slice::<FromPlugin>(&payload) {
                            Ok(FromPlugin::Ready(r)) => {
                                *lock(&inner.ready) = Some(r);
                                inner.changed();
                            }
                            Ok(FromPlugin::Status { text }) => {
                                *lock(&inner.status) = Some(text);
                                inner.changed();
                            }
                            Ok(FromPlugin::Event { name, value }) => {
                                lock(&inner.events).push((name, value));
                                inner.changed();
                            }
                            Ok(FromPlugin::Reply { id, value, error }) => {
                                lock(&inner.replies).insert(id, (value, error));
                                inner.replied.notify_all();
                            }
                            Ok(FromPlugin::Error { what, why, fix }) => inner.fail(Problem {
                                what,
                                why,
                                fix,
                                missing: false,
                            }),
                            Err(_) => {}
                        },
                        _ => {}
                    }
                }
                // The program ended.
                let Some(inner) = weak.upgrade() else { return };
                lock(&inner.stdin).take();
                if !inner.stopping.load(Ordering::SeqCst) {
                    let tail: Vec<String> = lock(&inner.stderr).iter().cloned().collect();
                    let why = tail
                        .iter()
                        .rev()
                        .find(|l| !l.trim().is_empty())
                        .cloned()
                        .unwrap_or_else(|| "it ended unexpectedly".into());
                    inner.fail(Problem {
                        what: format!("{title} stopped"),
                        why,
                        fix: "press Enter to start it again".into(),
                        missing: false,
                    });
                }
                inner.replied.notify_all();
            })?;
        Ok(())
    }

    /// Stop the program and start it again.
    pub fn restart(&self) {
        self.inner.stopping.store(true, Ordering::SeqCst);
        self.inner.send(&ToPlugin::Stop);
        lock(&self.inner.stdin).take();
        lock(&self.inner.child).take();
        *lock(&self.inner.frame) = None;
        *lock(&self.inner.problem) = None;
        *lock(&self.inner.ready) = None;
        *lock(&self.inner.status) = None;
        crate::plugins::forget();
        self.inner.stopping.store(false, Ordering::SeqCst);
        self.inner.started.store(false, Ordering::SeqCst);
        self.inner.changed();
    }

    fn problem(&self) -> Option<Problem> {
        lock(&self.inner.problem).clone()
    }

    /// Ask the plugin something and wait (briefly) for the answer.
    fn ask(&self, method: &str, args: &Value) -> Result<Value> {
        let id = self.inner.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let call = ToPlugin::Call {
            id,
            method: method.into(),
            args: args.clone(),
        };
        if !self.inner.send(&call) {
            return Err(Error::friendly(
                format!("{} isn't running", self.title()),
                "the pane hasn't started it yet, or it stopped",
                "show the pane first, or press Enter in it to start it again",
            ));
        }
        let mut replies = lock(&self.inner.replies);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some((value, error)) = replies.remove(&id) {
                return match error {
                    None => Ok(value),
                    Some(e) => Err(Error::friendly(
                        format!("{} couldn't do \"{method}\"", self.title()),
                        e,
                        "check the method name and its arguments",
                    )),
                };
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() || lock(&self.inner.stdin).is_none() {
                return Err(Error::friendly(
                    format!("{} didn't answer", self.title()),
                    format!("\"{method}\" got no reply within 5 seconds"),
                    "try again, or restart the pane (Enter)",
                ));
            }
            replies = self
                .inner
                .replied
                .wait_timeout(replies, left)
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }

    fn picture_area(&self, area: Rect) -> Rect {
        let status = lock(&self.inner.status).is_some();
        if status && area.height > 2 {
            Rect::new(area.x, area.y, area.width, area.height - 1)
        } else {
            area
        }
    }
}

fn key_text(k: &Key) -> Option<String> {
    if k.mods.ctrl || k.mods.alt {
        return None;
    }
    match k.code {
        Code::Char(c) => Some(c.to_string()),
        _ => None,
    }
}

impl Widget for PluginView {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        (max_width, self.height.unwrap_or(12))
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let t = p.theme;
        *lock(&self.inner.cell_px) = p.cell_px;
        let pic = self.picture_area(area);
        let (cw, ch) = (p.cell_px.0.max(1), p.cell_px.1.max(1));
        let size = (
            u32::from(pic.width) * cw,
            u32::from(pic.height) * ch,
            pic.width,
            pic.height,
        );
        if !self.inner.started.load(Ordering::SeqCst) {
            self.start(size, t);
        } else if *lock(&self.inner.size) != size && size.0 > 0 && size.1 > 0 {
            *lock(&self.inner.size) = size;
            self.inner.send(&ToPlugin::Resize {
                width: size.0,
                height: size.1,
                cols: size.2,
                rows: size.3,
            });
        }
        self.inner.painted.store(
            self.inner.generation.load(Ordering::SeqCst),
            Ordering::SeqCst,
        );
        let style = |fg: crate::style::Rgb| Style::new().fg(fg.color()).bg(t.bg.color());
        if let Some(problem) = self.problem() {
            let lines = [
                (problem.what.as_str(), t.fg),
                (problem.why.as_str(), t.muted),
                (problem.fix.as_str(), t.accent),
            ];
            let mut y = area.y + area.height.saturating_sub(4) / 2;
            for (line, color) in lines {
                for row in text::layout(
                    line,
                    usize::from(area.width.saturating_sub(4)),
                    text::Direction::Ltr,
                )
                .into_iter()
                .take(2)
                {
                    if y >= area.bottom() {
                        break;
                    }
                    frame.put_line(
                        area.x + 2,
                        y,
                        area.width.saturating_sub(4),
                        &row,
                        color,
                        t.bg,
                        false,
                    );
                    y += 1;
                }
            }
            return;
        }
        match lock(&self.inner.frame).clone() {
            Some(image) => frame.put_image(pic, image, Fit::Contain, t.bg),
            None => {
                let msg = format!("Starting {}…", self.title());
                let y = area.y + area.height / 2;
                let x = area.x + area.width.saturating_sub(text::width(&msg) as u16) / 2;
                frame.put_str(x, y, &msg, area.width, style(t.muted));
            }
        }
        if let Some(status) = lock(&self.inner.status).clone()
            && pic.height < area.height
        {
            let y = area.bottom() - 1;
            frame.put_str(
                area.x + 1,
                y,
                &status,
                area.width.saturating_sub(2),
                style(t.muted),
            );
        }
    }

    fn describe(&self) -> Map<String, Value> {
        let problem = self.problem();
        object(json!({
            "plugin": self.name,
            "args": self.args,
            "installed": crate::plugins::find(&self.name).is_some(),
            "running": lock(&self.inner.stdin).is_some(),
            "status": *lock(&self.inner.status),
            "frames": self.inner.frames.load(Ordering::SeqCst),
            "error": problem.as_ref().map(|p| format!("{} — {}", p.what, p.why)),
        }))
    }

    fn focusable(&self) -> bool {
        true
    }

    fn captures_keys(&self) -> bool {
        lock(&self.inner.ready).as_ref().is_some_and(|r| r.text)
    }

    fn key(&mut self, k: &Key, _area: Rect) -> Reaction {
        if let Some(problem) = self.problem() {
            if k.code == Code::Enter {
                if problem.missing {
                    return Reaction::Emit("install_plugin", json!(self.name));
                }
                self.restart();
                return Reaction::Redraw;
            }
            return Reaction::Ignored;
        }
        let label = k.name();
        let wanted = lock(&self.inner.ready).as_ref().is_some_and(|r| {
            (r.text && !matches!(k.code, Code::Tab | Code::Esc))
                || r.keys.iter().any(|w| w.eq_ignore_ascii_case(&label))
        });
        if !wanted {
            return Reaction::Ignored;
        }
        self.inner.send(&ToPlugin::Key {
            key: label,
            text: key_text(k),
        });
        Reaction::Redraw
    }

    fn click(&mut self, _x: u16, _y: u16, _area: Rect) -> Reaction {
        // The press itself already went to the plugin (`mouse`).
        match self.problem() {
            Some(p) if p.missing => Reaction::Emit("install_plugin", json!(self.name)),
            Some(_) => {
                self.restart();
                Reaction::Redraw
            }
            None => Reaction::Redraw,
        }
    }

    fn mouse(&mut self, kind: MouseKind, x: u16, y: u16, _area: Rect) -> Reaction {
        if self.problem().is_some() || lock(&self.inner.ready).is_none() {
            return Reaction::Ignored;
        }
        let (cw, ch) = *lock(&self.inner.cell_px);
        let (name, button) = match kind {
            MouseKind::Down(b) => ("down", Some(b)),
            MouseKind::Up(b) => ("up", Some(b)),
            MouseKind::Drag(b) => ("drag", Some(b)),
            MouseKind::Move => ("move", None),
            MouseKind::ScrollUp => ("scroll_up", None),
            MouseKind::ScrollDown => ("scroll_down", None),
        };
        let button = button.map(|b| {
            match b {
                crate::input::Button::Left => "left",
                crate::input::Button::Right => "right",
                crate::input::Button::Middle => "middle",
            }
            .to_string()
        });
        self.inner.send(&ToPlugin::Mouse(api::Mouse {
            kind: name.into(),
            button,
            x: u32::from(x) * cw + cw / 2,
            y: u32::from(y) * ch + ch / 2,
        }));
        match kind {
            // Presses also focus the pane (the normal click handling).
            MouseKind::Down(_) | MouseKind::Move => Reaction::Ignored,
            _ => Reaction::Redraw,
        }
    }

    fn paste(&mut self, text: &str) -> Reaction {
        if self.inner.send(&ToPlugin::Paste { text: text.into() }) {
            Reaction::Redraw
        } else {
            Reaction::Ignored
        }
    }

    fn poll_dirty(&self) -> bool {
        self.inner.generation.load(Ordering::SeqCst) != self.inner.painted.load(Ordering::SeqCst)
    }

    fn take_events(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut *lock(&self.inner.events))
    }

    fn call(&mut self, method: &str, args: &Value) -> Result<Value> {
        match method {
            "restart" => {
                self.restart();
                Ok(Value::Null)
            }
            _ => self.ask(method, args),
        }
    }
}
