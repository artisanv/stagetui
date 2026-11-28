//! A Session owns app state: the widget tree, focus, scrolling, animations and
//! the events waiting for the app. Views render it; input changes it.
//!
//! Everything here is deterministic given the inputs and the clock, so headless
//! runs (virtual clock) and real terminals behave the same.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use serde::Serialize;
use serde_json::{Value, json};

use crate::frame::Frame;
use crate::gfx::Image;
use crate::input::{Button, Code, Input, Key, Mouse, MouseKind};
use crate::keymap::Keymap;
use crate::layout::{self, Laid, MARGIN};
use crate::motion::{self, Effect};
use crate::style::{self, Rgb};
use crate::text::{self, Direction};
use crate::tree::{AppSpec, IRect, Kind, NodeSpec, Tree};
use crate::widgets::{Card, Cx, PaintCx, Reaction, Widget, draw_box, fill, scrollbar};
use crate::workspace::Layout;

/// An install running in a pane, and what to do when it is done.
#[derive(Debug, Clone)]
struct Install {
    pane: usize,
    node: usize,
    after: After,
}

#[derive(Debug, Clone)]
enum After {
    /// Switch the pane to this add-on.
    Plugin(String),
    /// A picture terminal: switch the pane back to this type (it then opens there).
    Pictures(String),
}

/// What an open dialog asks.
#[derive(Debug, Clone, PartialEq)]
enum Ask {
    /// Detach, quit the app, or cancel (q in an attached window).
    Leave,
    /// Install an add-on for this pane?
    Install { pane: usize, plugin: String },
    /// Install a terminal that shows real pictures?
    Pictures { pane: usize },
    /// Quit? (the app set CONFIRM_QUIT)
    Quit,
}

#[derive(Debug, Clone)]
struct Dialog {
    ask: Ask,
    title: String,
    lines: Vec<String>,
    buttons: Vec<String>,
    hint: String,
    selected: usize,
}
/// In a popped-out pane's empty spot, and in its window's header.
const PLACEHOLDER_BUTTON: &str = "↙ Bring it back";
const POPOUT_BUTTON: &str = "↙ Put back";

/// Where the "put back" button sits in a pop-out window's header.
fn popout_button(cols: u16) -> Rect {
    let w = (text::width(POPOUT_BUTTON) as u16 + 2).min(cols);
    Rect::new(cols - w, 0, w, 1)
}
use crate::{Error, Result};

/// Something the app should know about (`on_click`, `on_change`, actions…).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppEvent {
    /// The caller's handle for the widget (Python), if it gave one.
    pub key: Option<String>,
    /// Widget id, or `"app"` for app-level events.
    pub id: String,
    /// `click`, `change`, `submit`, `select`, `frame`, `close`, `action`, `quit`…
    pub name: String,
    pub value: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Node(usize),
    Divider(usize),
    Tab(usize, usize),
    Backdrop(usize),
    Toast(usize),
    /// A pane header's title (opens the pane menu).
    PaneTitle(usize),
    /// A pane button: 0 split right, 1 split down, 2 pop out / back, 3 close,
    /// 4 bring a popped-out pane back.
    PaneAction(usize, u8),
}

/// What one pop-out window shows: its own layout and click areas.
#[derive(Debug, Default)]
struct WindowState {
    laid: Laid,
    hits: Vec<(Rect, Hit)>,
    size: (u16, u16),
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    divider: crate::layout::Divider,
}

#[derive(Debug, Clone)]
struct Toast {
    text: String,
    icon: Option<&'static str>,
    accent: Rgb,
    born: Duration,
    ttl: Duration,
}

#[derive(Debug, Clone, Copy, Default)]
struct FrameClock {
    next: Duration,
    pending: bool,
    count: u64,
}

/// App state, independent of any terminal.
#[derive(Debug)]
pub struct Session {
    tree: Tree,
    cx: Cx,
    margin: u16,
    now: Duration,
    focus: Option<usize>,
    hover: Option<usize>,
    hover_divider: Option<usize>,
    drag: Option<Drag>,
    root_scroll: u16,
    laid: Laid,
    hits: Vec<(Rect, Hit)>,
    toasts: Vec<(Rect, Toast)>,
    outbox: Vec<AppEvent>,
    dirty: bool,
    size: (u16, u16),
    quit: bool,
    /// A window asked to detach (prefix + d); the runtime decides what that means.
    detach: bool,
    /// The current input comes from a window that can detach (an attached terminal).
    detachable: bool,
    /// The session's name, for "Leave <name>?".
    session_name: Option<String>,
    /// An open question ("Leave?", "Install?", "Real pictures?").
    dialog: Option<Dialog>,
    /// Popped-out panes' windows.
    windows: HashMap<usize, WindowState>,
    /// The popped-out pane whose window the current input comes from.
    view_pane: Option<usize>,
    /// Panes (ids) that need a new window, and panes brought back.
    popouts: Vec<String>,
    popins: Vec<String>,
    /// A pane that pops out into a specific terminal (for real pictures).
    popout_terminal: HashMap<String, String>,
    /// Add-ons and terminals being installed, shown in a pane.
    installing: Vec<Install>,
    /// The window in use shows real pictures (not blocks).
    pictures: bool,
    /// A terminal here that does (`"ghostty"`, `"kitty"`, `"foot"`…).
    picture_terminal: Option<String>,
    /// Panes already offered real pictures.
    pictures_offered: std::collections::HashSet<usize>,
    frames: HashMap<usize, FrameClock>,
    modal_return: Vec<(usize, Option<usize>)>,
    autofocused: bool,
    /// The prefix key (default Ctrl+Space) was pressed: the next key is for the app.
    prefix_armed: bool,
    /// Built-in keys by name (defaults, the app's KEYS, the person's keys.json).
    keys: Keymap,
    key_file: Option<std::path::PathBuf>,
    key_problems: Vec<String>,
    /// Text waiting to be copied to the system clipboard (OSC 52).
    clipboard: Option<String>,
    /// The open pane menu.
    menu: Option<crate::panes::Menu>,
    hover_pane_button: Option<(usize, u8)>,
}

const TICK: Duration = Duration::from_millis(16);

impl Session {
    /// Build from a JSON app description (the Python → Rust boundary).
    pub fn from_json(json: &str) -> Result<Self> {
        let spec: AppSpec = serde_json::from_str(json).map_err(|e| {
            Error::friendly(
                "The app description has a mistake",
                e.to_string(),
                "check widget names and property spellings (see docs/reference.md)",
            )
        })?;
        Self::from_spec(&spec)
    }

    /// Build from a typed app description (Rust users).
    pub fn from_spec(spec: &AppSpec) -> Result<Self> {
        let tree = Tree::build(spec)?;
        let mut keys = Keymap::default();
        if let Some(p) = &spec.prefix {
            keys.apply(&json!({ "prefix": p }), "PREFIX")?;
        }
        if let Some(k) = &spec.keys {
            keys.apply(k, "KEYS")?;
        }
        keys.check()?;
        if let Some(b) = tree.bindings.iter().find(|b| keys.is("prefix", &b.key)) {
            return Err(Error::friendly(
                format!("{} is the prefix key", b.key.label()),
                format!("the shortcut for \"{}\" could never be pressed", b.action),
                "give it another key in BINDINGS, or move the prefix with KEYS = {\"prefix\": …}",
            ));
        }
        // A person's own keys: a problem there is explained, never fatal.
        let taken: Vec<Key> = tree.bindings.iter().map(|b| b.key).collect();
        let key_file = crate::keymap::user_file();
        let problems = key_file
            .as_ref()
            .map(|path| crate::keymap::apply_user_file(&mut keys, path, &taken))
            .unwrap_or_default();
        let mut s = Self {
            tree,
            cx: Cx::default(),
            margin: MARGIN,
            now: Duration::ZERO,
            focus: None,
            hover: None,
            hover_divider: None,
            drag: None,
            root_scroll: 0,
            laid: Laid::default(),
            hits: Vec::new(),
            toasts: Vec::new(),
            outbox: Vec::new(),
            dirty: true,
            size: (0, 0),
            quit: false,
            detach: false,
            detachable: false,
            session_name: None,
            dialog: None,
            windows: HashMap::new(),
            view_pane: None,
            popouts: Vec::new(),
            popout_terminal: HashMap::new(),
            installing: Vec::new(),
            pictures: true,
            picture_terminal: None,
            pictures_offered: std::collections::HashSet::new(),
            popins: Vec::new(),
            frames: HashMap::new(),
            modal_return: Vec::new(),
            autofocused: false,
            prefix_armed: false,
            keys,
            key_file,
            key_problems: problems.clone(),
            clipboard: None,
            menu: None,
            hover_pane_button: None,
        };
        if !problems.is_empty() {
            let n = problems.len();
            let what = if n == 1 {
                "1 of your keys was"
            } else {
                "some of your keys were"
            };
            let text = format!("{what} skipped — run `stagetui keys` to see why");
            let _ = s.notify(&text, Some("warning"), None, Duration::from_secs(8));
        }
        Ok(s)
    }

    /// Stop every add-on and shell now (a headless run is over; a snapshot of
    /// it may live on, but nothing should keep running behind it).
    pub fn release_processes(&mut self) {
        for n in &mut self.tree.nodes {
            match &mut n.kind {
                Kind::Plugin(p) => p.release(),
                Kind::Terminal(t) => t.release(),
                _ => {}
            }
        }
    }

    /// The built-in keys in effect (`stagetui keys`, agents, docs).
    pub fn keymap(&self) -> &Keymap {
        &self.keys
    }

    /// Every key this app answers to, as JSON: built-in keys, the app's own
    /// shortcuts, the person's keys file and any problem in it.
    pub fn keys_json(&self) -> Value {
        json!({
            "keys": self.keys.to_json(),
            "shortcuts": self.tree.bindings.iter().map(|b| json!({
                "key": b.key.name(),
                "label": b.key.label(),
                "action": b.action,
                "description": b.description,
            })).collect::<Vec<_>>(),
            "confirm_quit": self.tree.confirm_quit,
            "file": self.key_file.as_ref().map(|p| p.display().to_string()),
            "problems": self.key_problems,
        })
    }

    /// The same, as a table people read (`stagetui keys app.py`).
    pub fn keys_text(&self) -> String {
        let shortcuts: Vec<(String, String)> = self
            .tree
            .bindings
            .iter()
            .map(|b| {
                let what = b.description.clone().unwrap_or_else(|| b.action.clone());
                (b.key.label(), what)
            })
            .collect();
        crate::keymap::text(
            &self.keys,
            &shortcuts,
            self.key_file.as_deref(),
            &self.key_problems,
        )
    }

    /// Text the app asked to copy, if any (the terminal view sends it as OSC 52).
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard.take()
    }

    /// Run a widget-specific method, e.g. `chat.add` or `log.append`.
    pub fn call(&mut self, target: &str, method: &str, args: &Value) -> Result<Value> {
        let i = self.target(target)?;
        let n = &mut self.tree.nodes[i];
        let name = n.kind.name();
        let w = n.kind.widget_mut().ok_or_else(|| {
            Error::friendly(
                format!("A {name} has no method \"{method}\""),
                "containers only hold other widgets",
                "call the method on the widget inside it",
            )
        })?;
        let out = w.call(method, args)?;
        self.dirty = true;
        Ok(out)
    }

    /// The widget tree.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// Tell the session how big a terminal cell is in pixels (affects image sizes).
    pub fn set_cell_px(&mut self, cell_px: (u32, u32)) {
        self.cx.cell_px = (cell_px.0.max(1), cell_px.1.max(1));
        self.dirty = true;
    }

    pub fn cell_px(&self) -> (u32, u32) {
        self.cx.cell_px
    }

    /// Cells kept free left+right of content (default 4; 0 for content-sized renders).
    pub fn set_margin(&mut self, margin: u16) {
        self.margin = margin;
    }

    // ----- time -----------------------------------------------------------

    /// The size of the last frame `(cols, rows)`.
    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    /// The animation clock.
    pub fn now(&self) -> Duration {
        self.now
    }

    /// Move the clock to `now` (real time in terminals, virtual time headless).
    pub fn set_time(&mut self, now: Duration) {
        if now > self.now {
            self.now = now;
        }
        self.tick();
    }

    /// Advance the clock by `dt`.
    pub fn advance(&mut self, dt: Duration) {
        self.set_time(self.now + dt);
    }

    fn tick(&mut self) {
        let before = self.toasts.len();
        let now = self.now;
        self.toasts.retain(|(_, t)| now < t.born + t.ttl);
        if self.toasts.len() != before {
            self.dirty = true;
        }
        // Live canvases ask for a new picture.
        let canvases: Vec<usize> = self
            .tree
            .preorder()
            .into_iter()
            .filter(|i| {
                let n = &self.tree.nodes[*i];
                matches!(n.kind, Kind::Image(_)) && n.events.contains("frame") && n.bounds.width > 0
            })
            .collect();
        for i in canvases {
            let fps = self.tree.nodes[i].spec.props.fps.unwrap_or(30).clamp(1, 60);
            let clock = self.frames.entry(i).or_default();
            if !clock.pending && now >= clock.next {
                clock.pending = true;
                clock.next = now + Duration::from_secs_f64(1.0 / f64::from(fps));
                clock.count += 1;
                let count = clock.count;
                self.emit(
                    i,
                    "frame",
                    json!({ "t": now.as_secs_f64(), "frame": count }),
                );
            }
        }
    }

    /// How long until the next frame is needed; `None` when idle.
    pub fn next_wake(&self) -> Option<Duration> {
        if self.dirty {
            return Some(Duration::ZERO);
        }
        let mut wake: Option<Duration> = None;
        let mut want = |d: Duration| wake = Some(wake.map_or(d, |w| w.min(d)));
        for i in self.tree.preorder() {
            let n = &self.tree.nodes[i];
            if n.bounds.width == 0 || n.bounds.height == 0 {
                continue;
            }
            if let Some(born) = n.born
                && motion::active(
                    n.animate,
                    self.now.saturating_sub(born),
                    self.tree.reduce_motion,
                )
            {
                want(if n.animate.loops() { TICK * 3 } else { TICK });
            }
            if n.widget().is_some_and(Widget::animating) {
                want(Duration::from_millis(40));
            }
            if let Some(c) = self.frames.get(&i)
                && !c.pending
            {
                want(c.next.saturating_sub(self.now));
            }
        }
        for (_, t) in &self.toasts {
            let age = self.now.saturating_sub(t.born);
            want(if age < motion::ENTER {
                TICK
            } else {
                (t.born + t.ttl).saturating_sub(self.now)
            });
        }
        wake
    }

    /// Whether something changed since the last frame.
    pub fn is_dirty(&self) -> bool {
        self.dirty || self.next_wake().is_some_and(|w| w.is_zero()) || self.outside_changes()
    }

    /// A widget changed by something outside the app (shell output).
    fn outside_changes(&self) -> bool {
        self.tree
            .nodes
            .iter()
            .any(|n| n.alive && n.widget().is_some_and(Widget::poll_dirty))
    }

    /// Mark the screen as needing a redraw.
    pub fn touch(&mut self) {
        self.dirty = true;
    }

    /// The user asked to quit (q, Esc, Ctrl+C or a `quit` action).
    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    /// Whether the input handled next comes from a window that can detach
    /// (an attached terminal). Then q / Ctrl+C ask before quitting.
    pub fn set_detachable(&mut self, yes: bool) {
        self.detachable = yes;
    }

    /// The session's name, shown in the "Leave?" prompt.
    pub fn set_session_name(&mut self, name: Option<String>) {
        self.session_name = name;
    }

    /// Whether the "Leave?" prompt is open.
    pub fn leave_prompt_open(&self) -> bool {
        self.dialog.as_ref().is_some_and(|d| d.ask == Ask::Leave)
    }

    /// The open question, if any (`"leave"`, `"install"`, `"pictures"`, `"quit"`).
    pub fn dialog_open(&self) -> Option<&'static str> {
        self.dialog.as_ref().map(|d| match d.ask {
            Ask::Leave => "leave",
            Ask::Install { .. } => "install",
            Ask::Pictures { .. } => "pictures",
            Ask::Quit => "quit",
        })
    }

    fn ask(&mut self, ask: Ask, title: String, lines: Vec<String>, buttons: &[&str], hint: &str) {
        self.dialog = Some(Dialog {
            ask,
            title,
            lines,
            buttons: buttons.iter().map(|b| (*b).to_string()).collect(),
            hint: hint.into(),
            selected: 0,
        });
        self.menu = None;
        self.dirty = true;
    }

    /// Someone pressed q / Ctrl+C / Esc to leave. In an attached window, ask
    /// first: detach (the app keeps running) or quit the app.
    fn user_quit(&mut self) {
        // In a pop-out window, leaving puts the pane back.
        if let Some(p) = self.view_pane {
            self.pop_in_idx(p);
            return;
        }
        if self.detachable && !self.quit {
            let title = match &self.session_name {
                Some(n) => format!("Leave \"{n}\"?"),
                None => "Leave this app?".to_string(),
            };
            self.ask(
                Ask::Leave,
                title,
                vec![
                    "Detach: it keeps running, come back later.".into(),
                    "Quit: the app stops for every window.".into(),
                ],
                &["Detach", "Quit app", "Cancel"],
                "d detach · q quit · Esc cancel · ← → choose",
            );
        } else if self.tree.confirm_quit && !self.quit {
            self.ask(
                Ask::Quit,
                "Quit this app?".to_string(),
                vec!["It stops, and its windows close.".into()],
                &["Quit", "Cancel"],
                "Enter or y quit · Esc or n cancel",
            );
        } else {
            self.request_quit();
        }
    }

    fn dialog_key(&mut self, k: Key) {
        let Some(d) = &mut self.dialog else { return };
        let n = d.buttons.len().max(1);
        let sel = d.selected;
        let last = n - 1;
        let shortcut = |c: char| match (&d.ask, c) {
            (Ask::Leave, 'd') => Some(0),
            (Ask::Leave, 'q') => Some(1),
            (Ask::Leave, 'c' | 'n') => Some(2),
            (Ask::Quit, 'q') => Some(0),
            (_, 'i' | 'y') => Some(0),
            (_, 'n') => Some(last),
            _ => None,
        };
        let choice = match k.code {
            // Ctrl+C again: quit (a second Ctrl+C is a clear "yes, really").
            Code::Char('c') if k.mods.ctrl => Some(match d.ask {
                Ask::Leave => 1,
                Ask::Quit => 0,
                _ => last,
            }),
            Code::Enter | Code::Char(' ') => Some(sel),
            Code::Esc => Some(last),
            Code::Char(c) => shortcut(c),
            Code::Right | Code::Down => {
                d.selected = (sel + 1) % n;
                None
            }
            Code::Tab => {
                d.selected = if k.mods.shift {
                    (sel + n - 1) % n
                } else {
                    (sel + 1) % n
                };
                None
            }
            Code::Left | Code::Up => {
                d.selected = (sel + n - 1) % n;
                None
            }
            _ => None,
        };
        self.dirty = true;
        if let Some(c) = choice {
            self.dialog_choose(c);
        }
    }

    fn dialog_choose(&mut self, choice: usize) {
        let Some(d) = self.dialog.take() else { return };
        self.dirty = true;
        match (d.ask, choice) {
            (Ask::Leave, 0) => self.detach = true,
            (Ask::Leave, 1) | (Ask::Quit, 0) => self.request_quit(),
            (Ask::Install { pane, plugin }, 0) => self.install_plugin(pane, &plugin),
            (Ask::Pictures { pane }, 0) => self.install_picture_terminal(pane),
            _ => {}
        }
    }

    /// The dialog's box and its buttons, centered on `screen`.
    fn dialog_rects(&self, screen: Rect) -> Option<(Rect, Vec<Rect>)> {
        let d = self.dialog.as_ref()?;
        let widest = d
            .lines
            .iter()
            .chain([&d.hint, &d.title])
            .map(|l| text::width(l) as u16)
            .max()
            .unwrap_or(0);
        let w = (widest + 8).clamp(50, 72).min(screen.width);
        let lines = self.dialog_lines(w).len() as u16;
        let h = (lines + 7).min(screen.height);
        let rect = Rect::new(
            screen.x + (screen.width - w) / 2,
            screen.y + (screen.height - h) / 2,
            w,
            h,
        );
        let widths: Vec<u16> = d
            .buttons
            .iter()
            .map(|l| text::width(l) as u16 + 4)
            .collect();
        let total: u16 = widths.iter().sum::<u16>() + 2 * (widths.len() as u16).saturating_sub(1);
        let mut x = rect.x + rect.width.saturating_sub(total) / 2;
        let y = (rect.y + 3 + lines).min(rect.bottom().saturating_sub(3));
        let mut buttons = Vec::new();
        for w in widths {
            buttons.push(Rect::new(x, y, w, 1).intersection(rect));
            x += w + 2;
        }
        Some((rect, buttons))
    }

    /// A dialog's lines wrapped to its width (never cut off).
    fn dialog_lines(&self, width: u16) -> Vec<text::Line> {
        let Some(d) = &self.dialog else {
            return Vec::new();
        };
        let inner = usize::from(width.saturating_sub(6)).max(1);
        d.lines
            .iter()
            .flat_map(|l| text::layout(l, inner, self.tree.dir))
            .collect()
    }

    fn paint_dialog(&self, frame: &mut Frame) {
        let Some(d) = &self.dialog else { return };
        let Some((rect, buttons)) = self.dialog_rects(frame.area()) else {
            return;
        };
        let t = &self.tree.theme;
        frame.dim_except(Rect::default(), t.bg, 0.55);
        frame.occlude(rect);
        fill(frame, rect, t.fg, t.surface);
        draw_box(
            frame,
            rect,
            true,
            t.accent,
            t.bg,
            Some(&d.title),
            t.accent,
            false,
        );
        let inner = rect.width.saturating_sub(6);
        let muted = Style::new().fg(t.muted.color()).bg(t.surface.color());
        for (k, line) in self.dialog_lines(rect.width).iter().enumerate() {
            let y = rect.y + 2 + k as u16;
            if y + 4 >= rect.bottom() {
                break;
            }
            frame.put_line(rect.x + 3, y, inner, line, t.fg, t.surface, false);
        }
        for (k, (b, label)) in buttons.iter().zip(&d.buttons).enumerate() {
            let (fg, bg) = if k == d.selected {
                (t.bg, t.accent)
            } else {
                (t.fg, t.surface.mix(t.fg, 0.12))
            };
            fill(frame, *b, fg, bg);
            let style = Style::new().fg(fg.color()).bg(bg.color());
            frame.put_str(b.x + 2, b.y, label, b.width.saturating_sub(2), style);
        }
        frame.put_str(
            rect.x + 3,
            rect.bottom().saturating_sub(2),
            &d.hint,
            inner,
            muted,
        );
    }

    /// Whether a window asked to detach since the last call (prefix + d).
    pub fn take_detach(&mut self) -> bool {
        std::mem::take(&mut self.detach)
    }

    /// Ask the app to quit.
    pub fn request_quit(&mut self) {
        if !self.quit {
            self.quit = true;
            self.outbox.push(AppEvent {
                key: None,
                id: "app".into(),
                name: "quit".into(),
                value: Value::Null,
            });
        }
    }

    /// Events since the last call.
    pub fn take_events(&mut self) -> Vec<AppEvent> {
        std::mem::take(&mut self.outbox)
    }

    fn emit(&mut self, i: usize, name: &str, value: Value) {
        let n = &self.tree.nodes[i];
        self.outbox.push(AppEvent {
            key: n.key.clone(),
            id: n.id.clone(),
            name: name.into(),
            value,
        });
    }

    fn emit_app(&mut self, name: &str, value: Value) {
        self.outbox.push(AppEvent {
            key: None,
            id: "app".into(),
            name: name.into(),
            value,
        });
    }

    // ----- drawing --------------------------------------------------------

    /// Lay out and paint a frame of `cols × rows`.
    pub fn frame(&mut self, cols: u16, rows: u16) -> Frame {
        let (cols, rows) = (cols.max(1), rows.max(1));
        if self.size != (cols, rows) {
            self.size = (cols, rows);
        }
        self.laid = layout::layout(
            &mut self.tree,
            cols,
            rows,
            self.margin,
            &self.cx,
            &mut self.root_scroll,
        );
        self.prepass(cols, rows);
        self.sync_shown();
        self.check_installs();
        self.collect_widget_events();
        self.offer_pictures();
        if !self.autofocused {
            self.autofocused = true;
            if self.focus.is_none() {
                // A text box (prompt, input) first; a terminal only if there is none.
                let list = self.focusables();
                let wants = |i: &usize, terminal: bool| {
                    self.tree.nodes[*i]
                        .widget()
                        .is_some_and(|w| w.takes_text() && w.captures_keys() == terminal)
                };
                self.focus = list
                    .iter()
                    .find(|i| wants(i, false))
                    .or_else(|| list.iter().find(|i| wants(i, true)))
                    .copied();
            }
        }
        if let Some(f) = self.focus
            && !self.focusables().contains(&f)
            && !(self.tree.popped_nodes().contains(&f) && self.can_focus(f))
        {
            self.focus = None;
        }
        let frame = self.paint(cols, rows);
        self.dirty = false;
        self.tick();
        frame
    }

    /// Tell add-on panes whether they can be seen: a hidden video pauses, a
    /// hidden 3D model stops drawing.
    fn sync_shown(&self) {
        for (i, n) in self.tree.nodes.iter().enumerate() {
            if let Kind::Plugin(p) = &n.kind
                && n.alive
            {
                p.set_shown(self.tree.shown(i));
            }
        }
    }

    /// Bounds, birth times and hit regions from the fresh layout.
    fn prepass(&mut self, cols: u16, rows: u16) {
        let screen = Rect::new(0, 0, cols, rows);
        self.hits.clear();
        let popped = self.tree.popped_nodes();
        for (k, n) in self.tree.nodes.iter_mut().enumerate() {
            if !popped.contains(&k) {
                n.bounds = Rect::default();
            }
        }
        let roots = self.tree.roots.clone();
        for r in roots {
            let clip = if matches!(self.tree.nodes[r].kind, Kind::Bar(_)) {
                screen
            } else {
                self.laid.content
            };
            self.walk_bounds(r, clip);
        }
        for (k, d) in self.laid.dividers.iter().enumerate() {
            let r = d.rect.intersection(self.laid.content);
            if !r.is_empty() {
                self.hits.push((r, Hit::Divider(k)));
            }
        }
        for m in self.laid.modals.clone() {
            self.hits.push((screen, Hit::Backdrop(m)));
            let rect = self.tree.nodes[m].rect.clip(screen);
            self.walk_bounds(m, rect);
        }
        let now = self.now;
        for (k, n) in self.tree.nodes.iter_mut().enumerate() {
            if popped.contains(&k) {
                continue;
            }
            if n.alive && n.bounds.width > 0 && n.bounds.height > 0 {
                n.born.get_or_insert(now);
            } else if n.rect == IRect::default() {
                n.born = None;
            }
        }
    }

    fn walk_bounds(&mut self, i: usize, clip: Rect) {
        let n = &self.tree.nodes[i];
        if !n.alive || !n.visible || (n.rect.w == 0 && n.rect.h == 0) {
            return;
        }
        let bounds = n.rect.clip(clip);
        self.tree.nodes[i].bounds = bounds;
        if bounds.is_empty() {
            return;
        }
        self.hits.push((bounds, Hit::Node(i)));
        if self.tree.is_typed_pane(i) {
            let (title, buttons) = self.pane_header(i);
            for (r, h) in std::iter::once((title, Hit::PaneTitle(i))).chain(
                buttons
                    .into_iter()
                    .enumerate()
                    .map(|(k, r)| (r, Hit::PaneAction(i, k as u8))),
            ) {
                let r = r.intersection(bounds);
                if !r.is_empty() {
                    self.hits.push((r, h));
                }
            }
        }
        if self.is_popped(i) {
            // The content is in another window; here is just the way back.
            if let Some(r) = self.placeholder_button(i) {
                let r = r.intersection(bounds);
                if !r.is_empty() {
                    self.hits.push((r, Hit::PaneAction(i, 4)));
                }
            }
            return;
        }
        if let Kind::Tabs(_) = &self.tree.nodes[i].kind {
            for (k, r) in self.tab_titles(i) {
                let r = r.intersection(bounds);
                if !r.is_empty() {
                    self.hits.push((r, Hit::Tab(i, k)));
                }
            }
        }
        let kids = self.tree.nodes[i].children.clone();
        for c in kids {
            if !matches!(self.tree.nodes[c].kind, Kind::Modal(_)) {
                self.walk_bounds(c, bounds);
            }
        }
    }

    /// Tab title rectangles `(child position, rect)` for a tabs node.
    fn tab_titles(&self, i: usize) -> Vec<(usize, Rect)> {
        let n = &self.tree.nodes[i];
        let r = n.rect;
        if r.y < 0 || r.x < 0 {
            return Vec::new();
        }
        let mut x = r.x as u16 + 1;
        let mut out = Vec::new();
        let visible: Vec<usize> = n
            .children
            .iter()
            .copied()
            .filter(|c| self.tree.nodes[*c].alive && self.tree.nodes[*c].visible)
            .collect();
        for (k, c) in visible.iter().enumerate() {
            let w = text::width(&self.tab_label(*c)) as u16 + 4;
            out.push((k, Rect::new(x, r.y as u16, w, 1)));
            x += w + 1;
        }
        if n.rtl {
            let right = r.right() as u16;
            out = out
                .into_iter()
                .map(|(k, t)| {
                    (
                        k,
                        Rect::new(right - (t.x - r.x as u16) - t.width, t.y, t.width, 1),
                    )
                })
                .collect();
        }
        out
    }

    /// Title chip and the three buttons (split right, split down, close) of a pane header.
    fn pane_header(&self, i: usize) -> (Rect, Vec<Rect>) {
        let r = self.tree.nodes[i].rect;
        if r.x < 0 || r.y < 0 {
            return (Rect::default(), Vec::new());
        }
        let (x, y, w) = (r.x as u16, r.y as u16, r.w);
        let title_w = (text::width(&self.pane_label(i)) as u16 + 2).min(w);
        let buttons = if w >= title_w + 13 {
            (0..4)
                .map(|k| Rect::new(x + w - 12 + 3 * k, y, 3, 1))
                .collect()
        } else {
            Vec::new()
        };
        (Rect::new(x, y, title_w, 1), buttons)
    }

    fn pane_label(&self, i: usize) -> String {
        let Kind::Pane(pb) = &self.tree.nodes[i].kind else {
            return String::new();
        };
        let t = self.tree.pane_types.iter().find(|t| t.name == pb.ptype);
        let icon = t
            .and_then(|t| t.icon.as_deref())
            .and_then(|i| crate::widgets::icon(i).ok())
            .unwrap_or("◆");
        let title = t
            .and_then(|t| t.title.clone())
            .unwrap_or_else(|| pb.ptype.clone());
        format!("{icon} {title} ▾")
    }

    /// The pane holding the focus (or the first pane).
    fn current_pane(&self) -> Option<usize> {
        let mut cur = self.focus;
        while let Some(c) = cur {
            if self.tree.is_typed_pane(c) || self.tree.is_pane(c) {
                return Some(c);
            }
            cur = self.tree.nodes[c].parent;
        }
        self.tree
            .preorder()
            .into_iter()
            .find(|i| self.tree.is_typed_pane(*i) && self.tree.nodes[*i].bounds.width > 0)
    }

    fn paint_pane_header(&self, i: usize, target: &mut Frame, area: Rect, off: (i32, i32)) {
        let n = &self.tree.nodes[i];
        let t = &n.theme;
        let active = self.focus.is_some_and(|f| self.within(f, i));
        let menu_open = self.menu.as_ref().is_some_and(|m| m.pane == i);
        let bg = if active {
            t.accent.mix(t.surface, 0.78)
        } else {
            t.surface.mix(t.border, 0.35)
        };
        let row = Rect::new(area.x, area.y, area.width, 1);
        fill(target, row, t.fg, bg);
        let (title, buttons) = self.pane_header(i);
        let shift = |r: Rect| -> Rect {
            Rect::new(
                (i32::from(r.x) + off.0).max(0) as u16,
                (i32::from(r.y) + off.1).max(0) as u16,
                r.width,
                r.height,
            )
        };
        let tr = shift(title).intersection(row);
        let (tfg, tbg) = if menu_open {
            (t.bg, t.accent)
        } else if active {
            (t.accent, bg)
        } else {
            (t.fg.mix(t.muted, 0.3), bg)
        };
        fill(target, tr, tfg, tbg);
        let line = text::layout(
            &self.pane_label(i),
            usize::from(tr.width.saturating_sub(2)).max(1),
            Direction::Ltr,
        )
        .swap_remove(0);
        target.put_line(
            tr.x + 1,
            tr.y,
            tr.width.saturating_sub(2),
            &line,
            tfg,
            tbg,
            active,
        );
        for (k, b) in buttons.into_iter().enumerate() {
            let b = shift(b).intersection(row);
            let glyph = ["⊞", "⊟", if self.is_popped(i) { "↙" } else { "↗" }, "×"][k];
            let hot = self.hover_pane_button == Some((i, k as u8));
            let fg = if hot {
                t.accent
            } else {
                t.muted.mix(t.fg, 0.25)
            };
            target.put_str(
                b.x + 1,
                b.y,
                glyph,
                1,
                Style::new().fg(fg.color()).bg(bg.color()),
            );
        }
    }

    /// Where the open pane menu is drawn.
    fn menu_rect(&self) -> Option<Rect> {
        let m = self.menu.as_ref()?;
        let pane = self.tree.nodes.get(m.pane)?;
        let (cols, rows) = self.size;
        let screen = Rect::new(0, 0, cols, rows);
        let w = (m
            .items
            .iter()
            .map(|i| text::width(&i.label))
            .max()
            .unwrap_or(10) as u16
            + 6)
        .min(cols);
        let h = (menu_rows(m) as u16 + 2).min(rows);
        // Kept on screen: shifted left at the right edge, up at the bottom.
        let x = (pane.rect.x.max(0) as u16).min(cols.saturating_sub(w));
        let below = (pane.rect.y + 1).max(0) as u16;
        let y = if below + h <= rows {
            below
        } else {
            rows.saturating_sub(h)
        };
        Some(Rect::new(x, y, w, h).intersection(screen))
    }

    /// The first content row shown: the menu scrolls so the highlight is seen.
    fn menu_scroll(&self, rect: Rect) -> usize {
        let Some(m) = self.menu.as_ref() else {
            return 0;
        };
        let visible = usize::from(rect.height.saturating_sub(2)).max(1);
        let row = menu_row_of(m, m.hover);
        (row + 1).saturating_sub(visible)
    }

    /// Menu row → item index (the separator row has none).
    fn menu_item_at(&self, row: u16) -> Option<usize> {
        let m = self.menu.as_ref()?;
        let rect = self.menu_rect()?;
        let r = usize::from(row.checked_sub(1)?);
        if r >= usize::from(rect.height.saturating_sub(2)) {
            return None;
        }
        menu_item_of_row(m, r + self.menu_scroll(rect))
    }

    fn paint_menu(&self, frame: &mut Frame) {
        let (Some(m), Some(rect)) = (&self.menu, self.menu_rect()) else {
            return;
        };
        let t = &self.tree.nodes[m.pane].theme;
        frame.occlude(rect);
        let outside = crate::widgets::behind(frame, rect.x, rect.y, t.bg);
        fill(frame, rect, t.fg, t.surface);
        draw_box(
            frame,
            rect,
            true,
            t.accent,
            outside,
            Some("Pane"),
            t.muted,
            false,
        );
        let scroll = self.menu_scroll(rect);
        let visible = usize::from(rect.height.saturating_sub(2));
        let total = menu_rows(m);
        for r in scroll..(scroll + visible).min(total) {
            let y = rect.y + 1 + (r - scroll) as u16;
            let Some(k) = menu_item_of_row(m, r) else {
                let rule = Style::new().fg(t.border.color()).bg(t.surface.color());
                for x in rect.x + 1..rect.right().saturating_sub(1) {
                    frame.cells[(x, y)].set_symbol("─").set_style(rule);
                }
                continue;
            };
            let item = &m.items[k];
            let on = k == m.hover;
            let bg = if on {
                t.accent.mix(t.surface, 0.55)
            } else {
                t.surface
            };
            fill(
                frame,
                Rect::new(rect.x + 1, y, rect.width.saturating_sub(2), 1),
                t.fg,
                bg,
            );
            let mark = if item.current { "✓" } else { " " };
            let label = format!("{mark} {}", item.label);
            let line = text::layout(
                &label,
                usize::from(rect.width.saturating_sub(4)),
                Direction::Ltr,
            )
            .swap_remove(0);
            let fg = if item.current { t.accent } else { t.fg };
            frame.put_line(
                rect.x + 2,
                y,
                rect.width.saturating_sub(4),
                &line,
                fg,
                bg,
                item.current,
            );
        }
        // More above or below: say so on the border.
        let mark = Style::new().fg(t.accent.color()).bg(t.surface.color());
        let x = rect.right().saturating_sub(3);
        if scroll > 0 && rect.width > 4 {
            frame.cells[(x, rect.y)].set_symbol("▲").set_style(mark);
        }
        if scroll + visible < total && rect.width > 4 && rect.height > 1 {
            frame.cells[(x, rect.bottom() - 1)]
                .set_symbol("▼")
                .set_style(mark);
        }
    }

    fn open_menu(&mut self, pane: usize) {
        let current = match &self.tree.nodes[pane].kind {
            Kind::Pane(pb) => pb.ptype.clone(),
            _ => String::new(),
        };
        self.menu = Some(crate::panes::Menu::new(
            pane,
            &self.tree.pane_types,
            &current,
            self.is_popped(pane),
        ));
        self.dirty = true;
    }

    fn menu_key(&mut self, k: Key) {
        let Some(m) = &mut self.menu else { return };
        let n = m.items.len();
        match k.code {
            Code::Up => m.hover = (m.hover + n - 1) % n,
            Code::Down | Code::Tab => m.hover = (m.hover + 1) % n,
            Code::Home | Code::PageUp => m.hover = 0,
            Code::End | Code::PageDown => m.hover = n - 1,
            Code::Enter | Code::Char(' ') => {
                let (pane, key) = (m.pane, m.items[m.hover].key.clone());
                self.menu = None;
                self.menu_choose(pane, &key);
            }
            _ => self.menu = None,
        }
        self.dirty = true;
    }

    fn menu_choose(&mut self, pane: usize, key: &str) {
        match key {
            "@split_right" => self.split_pane(pane, true),
            "@split_down" => self.split_pane(pane, false),
            "@close" => self.close_pane(pane),
            "@pop" => self.toggle_pop(pane),
            name => self.switch_pane(pane, name),
        }
    }

    /// Show another type in a pane: restore its stashed content, build a
    /// built-in one, or ask the app to build a custom one.
    pub fn switch_pane(&mut self, pane: usize, name: &str) {
        let Kind::Pane(pb) = &self.tree.nodes[pane].kind else {
            return;
        };
        if pb.ptype == name {
            return;
        }
        let plugin = self
            .tree
            .pane_types
            .iter()
            .find(|t| t.name == name && t.plugin)
            .cloned();
        if plugin
            .as_ref()
            .is_some_and(|t| t.missing && crate::plugins::find(name).is_none())
        {
            // An add-on that isn't installed: offer to install it.
            self.ask_install(pane, name);
            return;
        }
        let stashed = self.tree.nodes[pane]
            .children
            .iter()
            .zip(&pb.types)
            .any(|(c, t)| t == name && self.tree.nodes[*c].alive);
        let custom = self
            .tree
            .pane_types
            .iter()
            .any(|t| t.name == name && t.custom);
        if stashed {
            if let Kind::Pane(pb) = &mut self.tree.nodes[pane].kind {
                pb.ptype = name.to_string();
            }
            self.focus_into(pane);
            self.emit(
                pane,
                "pane",
                json!({ "type": name, "built": true, "restored": true }),
            );
        } else if let Some(spec) = crate::panes::builtin_content(name)
            .filter(|_| !custom)
            .or_else(|| plugin.map(|_| crate::panes::plugin_content(name, Value::Null)))
        {
            let _ = self.pane_add(pane, name, &spec);
            self.emit(
                pane,
                "pane",
                json!({ "type": name, "built": true, "restored": false }),
            );
        } else {
            self.emit(
                pane,
                "pane",
                json!({ "type": name, "built": false, "restored": false }),
            );
        }
        self.dirty = true;
    }

    // ----- pop-out windows ------------------------------------------------

    /// Whether pane `i` is popped out into its own window.
    fn is_popped(&self, i: usize) -> bool {
        matches!(&self.tree.nodes[i].kind, Kind::Pane(pb) if pb.detached)
    }

    /// What a popped-out pane shows.
    fn popped_content(&self, pane: usize) -> Option<usize> {
        match &self.tree.nodes[pane].kind {
            Kind::Pane(pb) if pb.detached && self.tree.nodes[pane].alive => {
                crate::layout::pane_child(&self.tree, pane, pb)
            }
            _ => None,
        }
    }

    /// The pane `target` is, or is inside.
    fn typed_pane(&self, target: &str) -> Result<usize> {
        let start = self.target(target)?;
        let mut cur = Some(start);
        while let Some(i) = cur {
            if self.tree.is_typed_pane(i) {
                return Ok(i);
            }
            cur = self.tree.nodes[i].parent;
        }
        Err(Error::friendly(
            format!("\"{}\" is not in a pane", self.tree.nodes[start].id),
            "only StagePane widgets (and what is inside them) open in their own window",
            "put the widget in a StagePane(...), then call detach()",
        ))
    }

    fn set_popped(&mut self, pane: usize, yes: bool) -> bool {
        let Kind::Pane(pb) = &mut self.tree.nodes[pane].kind else {
            return false;
        };
        if pb.detached == yes {
            return false;
        }
        pb.detached = yes;
        self.dirty = true;
        true
    }

    /// Pop a pane out into its own terminal window. The runtime opens the
    /// window ([`Session::take_popouts`]); meanwhile its spot says where it went.
    pub fn pop_out(&mut self, target: &str) -> Result<()> {
        let p = self.typed_pane(target)?;
        self.pop_out_idx(p);
        Ok(())
    }

    fn pop_out_idx(&mut self, pane: usize) {
        if self.set_popped(pane, true) {
            self.popouts.push(self.tree.nodes[pane].id.clone());
            if self.focus.is_some_and(|f| self.within(f, pane)) {
                self.focus = Some(pane);
            }
            self.emit(pane, "window", json!("out"));
        }
    }

    /// Bring a popped-out pane back; its window closes.
    pub fn pop_in(&mut self, target: &str) -> Result<()> {
        let p = self.typed_pane(target)?;
        self.pop_in_idx(p);
        Ok(())
    }

    fn pop_in_idx(&mut self, pane: usize) {
        if self.set_popped(pane, false) {
            self.popins.push(self.tree.nodes[pane].id.clone());
            self.windows.remove(&pane);
            if self.focus.is_none_or(|f| f == pane || self.within(f, pane)) {
                self.focus = None;
                self.focus_into(pane);
            }
            self.emit(pane, "window", json!("in"));
        }
    }

    fn toggle_pop(&mut self, pane: usize) {
        if self.is_popped(pane) {
            self.pop_in_idx(pane);
        } else {
            self.pop_out_idx(pane);
        }
    }

    /// Panes (ids) waiting for a new terminal window.
    pub fn take_popouts(&mut self) -> Vec<String> {
        std::mem::take(&mut self.popouts)
    }

    /// Panes (ids) brought back since the last call: close their windows.
    pub fn take_popins(&mut self) -> Vec<String> {
        std::mem::take(&mut self.popins)
    }

    /// A window for pane `pane` connected (it may have been asked for, or
    /// someone ran `stagetui attach app:pane`).
    pub fn pane_window_opened(&mut self, pane: usize) {
        if self.set_popped(pane, true) {
            if self.focus.is_some_and(|f| self.within(f, pane)) {
                self.focus = Some(pane);
            }
            self.emit(pane, "window", json!("out"));
        }
    }

    /// The window of pane `pane` was closed: the pane snaps back.
    pub fn pane_window_closed(&mut self, pane: usize) {
        self.pop_in_idx(pane);
        self.popins.retain(|id| *id != self.tree.nodes[pane].id);
    }

    fn swap_window(&mut self, pane: usize) {
        let w = self.windows.entry(pane).or_default();
        std::mem::swap(&mut self.laid, &mut w.laid);
        std::mem::swap(&mut self.hits, &mut w.hits);
        std::mem::swap(&mut self.size, &mut w.size);
    }

    /// Paint a popped-out pane for its own window of `cols × rows`
    /// (`None` once it is back in the main window).
    pub fn pane_frame(&mut self, pane: usize, cols: u16, rows: u16) -> Option<Frame> {
        let content = self.popped_content(pane)?;
        let (cols, rows) = (cols.max(1), rows.max(2));
        self.swap_window(pane);
        self.size = (cols, rows);
        self.laid = layout::layout_popout(&mut self.tree, content, cols, rows, &self.cx);
        self.hits.clear();
        let area = self.laid.content;
        let mut stack = vec![content];
        let mut nodes = Vec::new();
        while let Some(i) = stack.pop() {
            self.tree.nodes[i].bounds = Rect::default();
            nodes.push(i);
            stack.extend(self.tree.nodes[i].children.iter().copied());
        }
        self.walk_bounds(content, area);
        for (k, d) in self.laid.dividers.iter().enumerate() {
            let r = d.rect.intersection(area);
            if !r.is_empty() {
                self.hits.push((r, Hit::Divider(k)));
            }
        }
        self.hits
            .push((popout_button(cols), Hit::PaneAction(pane, 4)));
        let now = self.now;
        for i in nodes {
            let n = &mut self.tree.nodes[i];
            if n.alive && n.bounds.width > 0 && n.bounds.height > 0 {
                n.born.get_or_insert(now);
            }
        }
        let theme = self.tree.nodes[pane].theme;
        let mut frame = Frame::new(Rect::new(0, 0, cols, rows), theme.bg, theme.fg);
        self.paint_node(content, &mut frame, (0, 0), area, &[]);
        if let Some((i, popup)) = self.popup()
            && self.within(i, content)
        {
            frame.occlude(popup);
            if let Some(w) = self.tree.nodes[i].widget() {
                w.paint_popup(popup, &mut frame, &self.pcx(i, 0.0, &[]));
            }
        }
        self.paint_popout_header(pane, &mut frame, cols);
        self.swap_window(pane);
        Some(frame)
    }

    /// Input from a popped-out pane's window (`Some(pane)`) or the main one (`None`).
    pub fn handle_in(&mut self, window: Option<usize>, input: &Input) -> bool {
        // Typing or clicking in a window works there (Esc may mean "leave").
        let pressed = matches!(input, Input::Key(k) if k.code != Code::Esc)
            || matches!(input, Input::Paste(_))
            || matches!(input, Input::Mouse(m) if matches!(m.kind, MouseKind::Down(_)));
        let Some((pane, content)) = window.and_then(|p| self.popped_content(p).map(|c| (p, c)))
        else {
            // The main window: keys there don't go to a pane shown elsewhere.
            if pressed
                && let Some(f) = self.focus
                && let Some((p, _)) = self
                    .tree
                    .popped()
                    .into_iter()
                    .find(|(_, c)| self.within(f, *c))
            {
                self.focus = Some(p);
            }
            return self.handle(input);
        };
        self.swap_window(pane);
        self.view_pane = Some(pane);
        if pressed && self.focus.is_none_or(|f| !self.within(f, content)) {
            self.focus = self.focusables().first().copied();
        }
        let changed = self.handle(input);
        self.view_pane = None;
        // The pane may have been put back meanwhile (its state is gone then).
        if self.windows.contains_key(&pane) || self.is_popped(pane) {
            self.swap_window(pane);
        }
        changed
    }

    /// The message in a popped-out pane's empty spot, wrapped to fit.
    fn placeholder_lines(&self, i: usize) -> Vec<text::Line> {
        let r = self.tree.nodes[i].rect;
        if r.h < 6 || r.w < 8 {
            return Vec::new();
        }
        let label = self.pane_label(i);
        let room = usize::from(r.h - 5).min(4);
        let width = usize::from(r.w - 4);
        let message = format!("{} is open in its own window", label.trim_end_matches(" ▾"));
        let mut lines = text::layout(&message, width, Direction::Ltr);
        if lines.len() > room {
            // Too narrow for all of it: a shorter message, never a cut one.
            lines = text::layout("In its own window", width, Direction::Ltr);
        }
        lines.truncate(room);
        lines
    }

    /// The "bring it back" button in a popped-out pane's empty spot.
    fn placeholder_button(&self, i: usize) -> Option<Rect> {
        let r = self.tree.nodes[i].rect;
        if r.x < 0 || r.y < 0 || r.h < 3 || r.w < 6 {
            return None;
        }
        let lines = self.placeholder_lines(i).len() as u16;
        let y = r.y as u16 + if lines > 0 { 3 + lines } else { 2 };
        let w = (text::width(PLACEHOLDER_BUTTON) as u16 + 2).min(r.w - 4);
        Some(Rect::new(r.x as u16 + 2, y, w, 1))
    }

    fn paint_placeholder(&self, i: usize, target: &mut Frame, off: (i32, i32)) {
        let n = &self.tree.nodes[i];
        let t = &n.theme;
        let r = n.rect;
        for (k, line) in self.placeholder_lines(i).iter().enumerate() {
            let x = (r.x + off.0 + 2).max(0) as u16;
            let y = (r.y + off.1 + 2 + k as i32).max(0) as u16;
            let bg = crate::widgets::behind(target, x, y, t.bg);
            target.put_line(x, y, r.w.saturating_sub(4), line, t.muted, bg, false);
        }
        if let Some(b) = self.placeholder_button(i) {
            let b = Rect::new(
                (i32::from(b.x) + off.0).max(0) as u16,
                (i32::from(b.y) + off.1).max(0) as u16,
                b.width,
                1,
            )
            .intersection(target.area());
            let hot = self.hover_pane_button == Some((i, 4));
            let (fg, bg) = if hot {
                (t.bg, t.accent)
            } else {
                (t.accent, t.surface)
            };
            fill(target, b, fg, bg);
            target.put_str(
                b.x + 1,
                b.y,
                PLACEHOLDER_BUTTON,
                b.width.saturating_sub(2),
                Style::new().fg(fg.color()).bg(bg.color()),
            );
        }
    }

    fn paint_popout_header(&self, pane: usize, frame: &mut Frame, cols: u16) {
        let t = &self.tree.nodes[pane].theme;
        let row = Rect::new(0, 0, cols, 1);
        let bg = t.accent.mix(t.surface, 0.78);
        fill(frame, row, t.fg, bg);
        let label = self.pane_label(pane);
        let mut title = format!(" ↗ {}", label.trim_end_matches(" ▾"));
        if let Some(name) = &self.session_name {
            title.push_str(&format!("  ·  {name}"));
        }
        let style = Style::new().fg(t.accent.color()).bg(bg.color());
        frame.put_str(0, 0, &title, cols, style);
        let b = popout_button(cols);
        let hot = self.hover_pane_button == Some((pane, 4));
        let (fg, bbg) = if hot {
            (t.bg, t.accent)
        } else {
            (t.fg, t.surface)
        };
        fill(frame, b, fg, bbg);
        frame.put_str(
            b.x + 1,
            0,
            POPOUT_BUTTON,
            b.width.saturating_sub(1),
            Style::new().fg(fg.color()).bg(bbg.color()),
        );
    }

    // ----- add-ons and real pictures ---------------------------------------

    /// Whether the window in use shows real pictures (set by the runtime).
    pub fn set_pictures(&mut self, yes: bool) {
        self.pictures = yes;
    }

    /// A terminal here that shows real pictures (set by the runtime).
    pub fn set_picture_terminal(&mut self, name: Option<String>) {
        self.picture_terminal = name;
    }

    /// The terminal a pane asked to pop out into, if a specific one.
    pub fn popout_terminal(&mut self, pane_id: &str) -> Option<String> {
        self.popout_terminal.remove(pane_id)
    }

    /// Offer to install an add-on for `pane`.
    fn ask_install(&mut self, pane: usize, plugin: &str) {
        let Some(k) = crate::plugins::known(plugin) else {
            let _ = self.notify(
                &format!("No add-on called \"{plugin}\" is known"),
                Some("info"),
                None,
                Duration::from_secs(4),
            );
            return;
        };
        if !k.available {
            let _ = self.notify(
                &format!("The {} add-on is coming soon", k.title),
                Some("info"),
                None,
                Duration::from_secs(5),
            );
            return;
        }
        self.ask(
            Ask::Install {
                pane,
                plugin: plugin.to_string(),
            },
            format!("Install {}?", k.title),
            vec![
                format!("{} needs the {} add-on: {}.", k.title, k.package, k.about),
                "It goes into this app's Python. No admin password needed.".into(),
            ],
            &["Install", "Not now"],
            "Enter install · Esc not now · ← → choose",
        );
    }

    /// Install an add-on: its output shows in the pane, then the pane switches to it.
    fn install_plugin(&mut self, pane: usize, plugin: &str) {
        // Only published add-ons install by name (an unpublished name could be anyone's).
        let Some(k) = crate::plugins::known(plugin).filter(|k| k.available) else {
            return;
        };
        match crate::plugins::installer(k.package) {
            Some(cmd) => self.run_install(
                pane,
                cmd,
                After::Plugin(plugin.to_string()),
                &format!("Installing {}", k.title),
            ),
            None => {
                let _ = self.notify(
                    &format!("Can't install {} from here: no pip or uv. In a terminal run: pip install {}", k.title, k.package),
                    Some("warning"),
                    None,
                    Duration::from_secs(10),
                );
            }
        }
    }

    /// Install a terminal with real pictures, then open `pane` in it.
    fn install_picture_terminal(&mut self, pane: usize) {
        match crate::launch::picture_install() {
            Some((name, cmd)) => {
                let back = match &self.tree.nodes[pane].kind {
                    Kind::Pane(pb) => pb.ptype.clone(),
                    _ => return,
                };
                self.run_install(
                    pane,
                    cmd,
                    After::Pictures(back),
                    &format!("Installing {name}"),
                );
            }
            None => {
                let _ = self.notify(
                    "Install Ghostty, Kitty, WezTerm or foot for real pictures (see `stagetui doctor`)",
                    Some("info"),
                    None,
                    Duration::from_secs(10),
                );
            }
        }
    }

    /// Run an install command in a terminal inside `pane` (a password can be typed there).
    fn run_install(&mut self, pane: usize, cmd: Vec<String>, after: After, label: &str) {
        let spec = NodeSpec {
            kind: "terminal".into(),
            content: json!(cmd),
            props: crate::tree::Props {
                title: Some(label.to_string()),
                ..crate::tree::Props::default()
            },
            ..NodeSpec::default()
        };
        if let Some(old) = self.installing.iter().position(|j| j.pane == pane) {
            self.installing.remove(old);
        }
        if let Ok(node) = self.pane_add(pane, "installing", &spec) {
            self.focus = Some(node);
            self.installing.push(Install { pane, node, after });
        }
    }

    /// Finished installs: switch the pane to what was installed.
    fn check_installs(&mut self) {
        let mut k = 0;
        while k < self.installing.len() {
            let job = &self.installing[k];
            let n = &self.tree.nodes[job.node];
            let exit = n
                .widget()
                .and_then(|w| w.describe().get("exit_code").and_then(Value::as_u64));
            if n.alive && exit.is_none() {
                k += 1;
                continue;
            }
            let job = self.installing.remove(k);
            match (exit, job.after) {
                (Some(0), After::Plugin(name)) => {
                    crate::plugins::forget();
                    let found = crate::plugins::find(&name);
                    for t in &mut self.tree.pane_types {
                        if t.name == name {
                            t.missing = found.is_none();
                        }
                    }
                    let title = found.map_or_else(|| name.clone(), |p| p.describe.title);
                    self.tree.remove(job.node);
                    self.switch_pane(job.pane, &name);
                    let _ = self.notify(
                        &format!("{title} is installed"),
                        Some("check"),
                        None,
                        Duration::from_secs(4),
                    );
                }
                (Some(0), After::Pictures(back)) => {
                    self.picture_terminal = crate::launch::picture_terminal();
                    self.pictures_offered.remove(&job.pane);
                    self.tree.remove(job.node);
                    self.switch_pane(job.pane, &back);
                }
                (Some(_), _) => {
                    let _ = self.notify(
                        "The install didn't work: its output is in the pane",
                        Some("warning"),
                        None,
                        Duration::from_secs(8),
                    );
                }
                (None, _) => {}
            }
            self.dirty = true;
        }
    }

    /// Things widgets did on their own (plugin events) go to the app.
    fn collect_widget_events(&mut self) {
        for i in 0..self.tree.nodes.len() {
            if !self.tree.nodes[i].alive {
                continue;
            }
            let events = self.tree.nodes[i]
                .kind
                .widget_mut()
                .map(|w| w.take_events())
                .unwrap_or_default();
            for (name, value) in events {
                self.emit(i, &name, value);
            }
        }
    }

    /// A picture add-on on screen in a terminal that draws blocks: open it in
    /// a terminal with real pictures, or offer to install one.
    fn offer_pictures(&mut self) {
        if self.pictures || self.dialog.is_some() {
            return;
        }
        let popped = self.tree.popped_nodes();
        let showing = (0..self.tree.nodes.len()).find_map(|i| {
            let n = &self.tree.nodes[i];
            let Kind::Plugin(v) = &n.kind else {
                return None;
            };
            if !n.alive || n.bounds.is_empty() || popped.contains(&i) {
                return None;
            }
            // Only an installed add-on that draws pictures (not a "not installed" note).
            let wants = crate::plugins::find(&v.name).is_some_and(|p| p.describe.pictures);
            let pane = self.typed_pane(&format!("#{}", n.id)).ok()?;
            (wants && !self.pictures_offered.contains(&pane)).then_some(pane)
        });
        let Some(pane) = showing else { return };
        self.pictures_offered.insert(pane);
        if let Some(term) = self.picture_terminal.clone() {
            let id = self.tree.nodes[pane].id.clone();
            self.popout_terminal.insert(id, term);
            self.pop_out_idx(pane);
        } else if let Some((name, _)) = crate::launch::picture_install() {
            self.ask(
                Ask::Pictures { pane },
                "Real pictures?".into(),
                vec![
                    "This terminal draws pictures with blocks.".into(),
                    format!("Install {name}? This pane will open there, sharp."),
                ],
                &[&format!("Install {name}"), "Not now"],
                "Enter install · Esc not now · ← → choose",
            );
        }
    }

    // ----- workspaces -----------------------------------------------------

    /// The workspace tabs (`StageWorkspaces`), if the app has them.
    fn workspace_tabs(&self) -> Option<usize> {
        self.tree
            .preorder()
            .into_iter()
            .find(|i| matches!(&self.tree.nodes[*i].kind, Kind::Tabs(t) if t.workspaces))
    }

    /// Where a workspace lives: `target`, the open workspace tab, or the
    /// app's main split / pane.
    fn workspace_root(&self, target: Option<&str>) -> Result<usize> {
        if let Some(t) = target {
            return self.target(t);
        }
        if let Some(tabs) = self.workspace_tabs()
            && let Kind::Tabs(t) = &self.tree.nodes[tabs].kind
        {
            let pages: Vec<usize> = self.tree.nodes[tabs]
                .children
                .iter()
                .copied()
                .filter(|c| self.tree.nodes[*c].alive && self.tree.nodes[*c].visible)
                .collect();
            if let Some(p) = pages.get(t.active.min(pages.len().saturating_sub(1))) {
                return Ok(*p);
            }
        }
        self.tree
            .preorder()
            .into_iter()
            .find(|i| matches!(self.tree.nodes[*i].kind, Kind::Split(_) | Kind::Pane(_)))
            .ok_or_else(|| {
                Error::friendly(
                    "This app has no panes to save",
                    "a workspace is a layout of StageSplit and StagePane widgets",
                    "put StagePane(...) widgets in a StageSplit",
                )
            })
    }

    /// The current layout of splits and panes (the open workspace, or `target`).
    pub fn workspace(&self, target: Option<&str>) -> Result<Layout> {
        Ok(self.tree.layout(self.workspace_root(target)?))
    }

    /// Arrange the splits and panes like `layout`. Panes that still exist (same
    /// id) are moved, not rebuilt: shells and chats keep running.
    pub fn load_workspace(&mut self, target: Option<&str>, layout: &Layout) -> Result<()> {
        let root = self.workspace_root(target)?;
        let applied = self.tree.apply_layout(root, layout)?;
        let mut build = applied.fresh.clone();
        for (pane, kind, args) in &applied.switch {
            let plain = args.is_null() || args.as_object().is_some_and(|o| o.is_empty());
            let stashed = match &self.tree.nodes[*pane].kind {
                Kind::Pane(pb) => self.tree.nodes[*pane]
                    .children
                    .iter()
                    .zip(&pb.types)
                    .any(|(c, t)| t == kind && self.tree.nodes[*c].alive),
                _ => false,
            };
            if plain || stashed {
                self.switch_pane(*pane, kind);
            } else {
                // With its options: built like a new pane of that type.
                build.push((*pane, kind.clone(), args.clone()));
            }
        }
        for (pane, kind, args) in &build {
            let custom = self
                .tree
                .pane_types
                .iter()
                .any(|t| t.name == *kind && t.custom);
            let plain = args.is_null() || args.as_object().is_some_and(|o| o.is_empty());
            match crate::panes::builtin_content(kind).filter(|_| !custom && plain) {
                Some(spec) => {
                    self.pane_add(*pane, kind, &spec)?;
                }
                // Options or a custom type: the app builds it (like the pane menu).
                None => self.emit(
                    *pane,
                    "pane",
                    json!({ "type": kind, "built": false, "restored": false, "args": args }),
                ),
            }
        }
        if self.focus.is_none_or(|f| !self.tree.nodes[f].alive) {
            self.focus = None;
            if let Some(first) = applied.first {
                self.focus_into(first);
            }
        }
        self.dirty = true;
        Ok(())
    }

    /// The name the open workspace is saved under: its tab's title, or `default`.
    pub fn workspace_name(&self) -> String {
        self.workspace_tabs()
            .and_then(|_| self.workspace_root(None).ok())
            .and_then(|r| self.tree.nodes[r].spec.props.title.clone())
            .unwrap_or_else(|| "default".into())
    }

    /// Save the current layout as `name` in a workspaces file.
    pub fn save_workspace(&self, target: Option<&str>, name: &str, path: &Path) -> Result<()> {
        crate::workspace::save(path, name, &self.workspace(target)?)
    }

    /// Load the layout saved as `name` from a workspaces file.
    pub fn open_workspace(&mut self, target: Option<&str>, name: &str, path: &Path) -> Result<()> {
        let layout = crate::workspace::load(path, name)?;
        self.load_workspace(target, &layout)
    }

    /// Show workspace tab `k` (0-based), if there are workspace tabs.
    fn select_workspace(&mut self, k: usize) {
        if let Some(tabs) = self.workspace_tabs() {
            let count = self.tree.nodes[tabs]
                .children
                .iter()
                .filter(|c| self.tree.nodes[**c].alive && self.tree.nodes[**c].visible)
                .count();
            if k < count {
                self.select_tab(tabs, k);
            }
        }
    }

    /// Put `spec` into `pane` as the content for type `name` and show it.
    fn pane_add(&mut self, pane: usize, name: &str, spec: &NodeSpec) -> Result<usize> {
        let c = self.tree.insert(spec, Some(pane))?;
        self.tree.nodes[pane].children.push(c);
        if let Kind::Pane(pb) = &mut self.tree.nodes[pane].kind {
            pb.types.push(name.to_string());
            pb.ptype = name.to_string();
        }
        self.focus_into(pane);
        self.dirty = true;
        Ok(c)
    }

    /// Set what a pane shows (from the app): `replace` drops a stashed copy of the same type.
    pub fn pane_set(
        &mut self,
        target: &str,
        name: &str,
        spec: &NodeSpec,
        replace: bool,
    ) -> Result<String> {
        let pane = self.target(target)?;
        let Kind::Pane(pb) = &self.tree.nodes[pane].kind else {
            return Err(Error::friendly(
                format!("\"{}\" is not a pane", self.tree.nodes[pane].id),
                "only StagePane widgets have a type you can switch",
                "use StagePane(\"chat\") in your layout, then pane.set_type(...)",
            ));
        };
        let old: Vec<usize> = self.tree.nodes[pane]
            .children
            .iter()
            .zip(&pb.types)
            .filter(|(c, t)| *t == name && self.tree.nodes[**c].alive)
            .map(|(c, _)| *c)
            .collect();
        if !replace && !old.is_empty() {
            self.switch_pane(pane, name);
            return Ok(self.tree.nodes[old[0]].id.clone());
        }
        for c in old {
            self.tree.remove(c);
        }
        let c = self.pane_add(pane, name, spec)?;
        Ok(self.tree.nodes[c].id.clone())
    }

    /// Remember the options the app gave a pane type (saved with the layout).
    pub fn pane_args(&mut self, target: &str, name: &str, args: Value) -> Result<()> {
        let pane = self.target(target)?;
        if let Kind::Pane(pb) = &mut self.tree.nodes[pane].kind {
            if args.is_null() || args.as_object().is_some_and(|o| o.is_empty()) {
                pb.args.remove(name);
            } else {
                pb.args.insert(name.to_string(), args);
            }
        }
        Ok(())
    }

    /// Move the focus into a pane's content (or onto the pane itself) — only if
    /// the focus was in that pane or nowhere, so code switching a pane doesn't
    /// pull the user away from what they are typing.
    fn focus_into(&mut self, pane: usize) {
        if self.focus.is_some_and(|f| !self.within(f, pane)) {
            return;
        }
        let Kind::Pane(pb) = &self.tree.nodes[pane].kind else {
            return;
        };
        let target = crate::layout::pane_child(&self.tree, pane, pb)
            .and_then(|c| {
                self.tree.preorder().into_iter().find(|d| {
                    self.within(*d, c)
                        && self.tree.nodes[*d].widget().is_some_and(Widget::focusable)
                })
            })
            .unwrap_or(pane);
        self.focus = Some(target);
    }

    fn split_pane(&mut self, pane: usize, horizontal: bool) {
        match self
            .tree
            .split(pane, &crate::panes::empty_pane(), horizontal)
        {
            Ok(new) => {
                self.focus = Some(new);
                self.emit(
                    new,
                    "mount",
                    json!({ "type": "pane", "split_from": self.tree.nodes[pane].id }),
                );
            }
            Err(_) => return,
        }
        self.dirty = true;
    }

    fn close_pane(&mut self, pane: usize) {
        let in_split = self.tree.nodes[pane]
            .parent
            .is_some_and(|p| matches!(self.tree.nodes[p].kind, Kind::Split(_)));
        if !in_split {
            if self.tree.pane_count() <= 1 {
                // Closing the last pane closes the app (an attached window asks
                // first; a pop-out window puts the pane back).
                self.user_quit();
            } else {
                let _ = self.notify(
                    "This pane can't close: it isn't in a split",
                    Some("info"),
                    None,
                    Duration::from_secs(2),
                );
            }
            return;
        }
        let id = self.tree.nodes[pane].id.clone();
        self.emit(pane, "close", Value::Null);
        let _ = self.remove(&format!("#{id}"));
    }

    fn tab_label(&self, c: usize) -> String {
        let n = &self.tree.nodes[c];
        let title = n.spec.props.title.clone().unwrap_or_else(|| n.id.clone());
        match n
            .spec
            .props
            .icon
            .as_deref()
            .and_then(|i| crate::widgets::icon(i).ok())
        {
            Some(icon) => format!("{icon} {title}"),
            None => title,
        }
    }

    fn effect(&self, i: usize) -> Effect {
        let n = &self.tree.nodes[i];
        match n.born {
            Some(born) => motion::effect(
                n.animate,
                self.now.saturating_sub(born),
                self.now,
                self.tree.reduce_motion,
            ),
            None => motion::effect(n.animate, Duration::ZERO, self.now, self.tree.reduce_motion),
        }
    }

    pub(crate) fn hints(&self) -> Vec<(String, String)> {
        if self.dialog.is_some() {
            return [("Enter", "Choose"), ("← →", "Move"), ("Esc", "Cancel")]
                .iter()
                .map(|(k, d)| ((*k).to_string(), (*d).to_string()))
                .collect();
        }
        let label = |name: &str| self.keys.label(name);
        if self.prefix_armed {
            let mut out: Vec<(String, String)> = [
                ("focus_next", "Next pane"),
                ("split_right", "Split →"),
                ("split_down", "Split ↓"),
                ("close_pane", "Close pane"),
                ("pane_menu", "Pane type"),
                ("pop_out", "Pop out"),
                ("detach", "Detach"),
            ]
            .iter()
            .filter_map(|(name, what)| Some((label(name)?, (*what).to_string())))
            .collect();
            if self.workspace_tabs().is_some() {
                out.push(("1-9".into(), "Workspace".into()));
            }
            if let Some(q) = self.quit_label(true) {
                out.push((q, "Quit".into()));
            }
            return out;
        }
        // A shell keeps every key: say how to get out.
        let in_shell = self.focus.is_some_and(|f| {
            self.tree.nodes[f]
                .widget()
                .is_some_and(Widget::captures_keys)
        });
        let prefix = label("prefix").unwrap_or_default();
        if in_shell {
            let shell = self
                .focus
                .is_some_and(|f| matches!(self.tree.nodes[f].kind, Kind::Terminal(_)));
            let leave = if shell {
                "Leave the shell"
            } else {
                "Leave this pane"
            };
            let mut out = Vec::new();
            if let Some(next) = label("focus_next") {
                out.push((format!("{prefix} {next}"), leave.into()));
            }
            if let Some(q) = self.quit_label(true) {
                out.push((format!("{prefix} {q}"), "Quit".into()));
            }
            return out;
        }
        let mut out: Vec<(String, String)> = self
            .tree
            .bindings
            .iter()
            .filter_map(|b| Some((b.key.label(), b.description.clone()?)))
            .collect();
        if !self.focusables().is_empty()
            && let Some(next) = label("focus_next")
        {
            out.push((next, "Next".into()));
        }
        if let Some(q) = self.quit_label(false) {
            out.push((q, "Quit".into()));
        }
        out
    }

    /// The quit key to show: the first one that really quits right now — not
    /// one an app shortcut takes, not Esc after the prefix, and not a letter
    /// (or Esc) while a text box has the focus (it would type, or step back).
    fn quit_label(&self, after_prefix: bool) -> Option<String> {
        let typing = !after_prefix
            && self
                .focus
                .and_then(|f| self.tree.nodes[f].widget())
                .is_some_and(Widget::takes_text);
        self.keys
            .keys("quit")
            .iter()
            .find(|k| {
                !(after_prefix && k.is(Code::Esc))
                    && !(typing && (k.text().is_some() || k.is(Code::Esc)))
                    && !self.tree.bindings.iter().any(|b| b.key == **k)
            })
            .map(Key::label)
    }

    fn paint(&self, cols: u16, rows: u16) -> Frame {
        let theme = self.tree.theme;
        let screen = Rect::new(0, 0, cols, rows);
        let mut frame = Frame::new(screen, theme.bg, theme.fg);
        let hints = self.hints();
        for &r in &self.tree.roots {
            let n = &self.tree.nodes[r];
            if !n.alive || matches!(n.kind, Kind::Modal(_)) {
                continue;
            }
            let clip = if matches!(n.kind, Kind::Bar(_)) {
                screen
            } else {
                self.laid.content
            };
            self.paint_node(r, &mut frame, (0, 0), clip, &hints);
        }
        let c = self.laid.content;
        if self.laid.content_rows > c.height && c.width > 0 {
            let x = if self.tree.dir == Direction::Rtl {
                c.x
            } else {
                c.right() - 1
            };
            scrollbar(
                &mut frame,
                x,
                c.y,
                c.height,
                usize::from(self.root_scroll),
                usize::from(self.laid.content_rows),
                theme.border,
                theme.accent,
            );
        }
        // An open dropdown floats above the page.
        if let Some((i, popup)) = self.popup() {
            frame.occlude(popup);
            let n = &self.tree.nodes[i];
            if let Some(w) = n.widget() {
                w.paint_popup(popup, &mut frame, &self.pcx(i, 0.0, &hints));
            }
        }
        for &m in &self.laid.modals {
            self.paint_modal(m, &mut frame, &hints);
        }
        self.paint_toasts(&mut frame);
        // The open pane menu is what the person is using: above messages.
        self.paint_menu(&mut frame);
        self.paint_dialog(&mut frame);
        frame
    }

    fn pcx<'a>(&'a self, i: usize, glow: f32, hints: &'a [(String, String)]) -> PaintCx<'a> {
        let n = &self.tree.nodes[i];
        PaintCx {
            theme: &n.theme,
            cell_px: self.cx.cell_px,
            focused: self.focus == Some(i),
            hovered: self.hover == Some(i),
            now: self.now,
            glow,
            rtl: n.rtl,
            pane: self.tree.is_pane(i),
            hints,
        }
    }

    fn paint_node(
        &self,
        i: usize,
        target: &mut Frame,
        off: (i32, i32),
        clip: Rect,
        hints: &[(String, String)],
    ) {
        let n = &self.tree.nodes[i];
        if !n.alive || !n.visible || (n.rect.w == 0 && n.rect.h == 0) {
            return;
        }
        let r = IRect {
            x: n.rect.x + off.0,
            y: n.rect.y + off.1,
            ..n.rect
        };
        let eff = self.effect(i);
        let vis = r.clip(clip);
        if vis.is_empty() && eff.dx == 0 && eff.dy == 0 {
            return;
        }
        let whole = r.x >= 0 && r.y >= 0 && vis == Rect::new(r.x as u16, r.y as u16, r.w, r.h);
        if eff.is_identity() && whole {
            self.paint_self(i, target, vis, (off.0, off.1), eff.glow, hints);
            self.paint_children(i, target, off, vis, hints);
        } else {
            let local = Rect::new(0, 0, n.rect.w, n.rect.h);
            let mut sub = Frame::transparent(local, n.theme.bg);
            let sub_off = (-n.rect.x, -n.rect.y);
            self.paint_self(i, &mut sub, local, sub_off, eff.glow, hints);
            self.paint_children(i, &mut sub, sub_off, local, hints);
            let toward = crate::widgets::behind(target, vis.x, vis.y, n.theme.bg);
            target.blit(
                &sub,
                r.x + eff.dx,
                r.y + eff.dy,
                clip,
                eff.alpha,
                toward,
                eff.settled,
            );
        }
    }

    fn paint_self(
        &self,
        i: usize,
        target: &mut Frame,
        area: Rect,
        off: (i32, i32),
        glow: f32,
        hints: &[(String, String)],
    ) {
        let n = &self.tree.nodes[i];
        match &n.kind {
            Kind::Pane(_) => {
                self.paint_pane_header(i, target, area, off);
                if self.is_popped(i) {
                    self.paint_placeholder(i, target, off);
                }
            }
            Kind::Tabs(t) => {
                let theme = &n.theme;
                let titles = self.tab_titles(i);
                let y = area.y;
                let visible: Vec<usize> = n
                    .children
                    .iter()
                    .copied()
                    .filter(|c| self.tree.nodes[*c].alive && self.tree.nodes[*c].visible)
                    .collect();
                let active = t.active.min(visible.len().saturating_sub(1));
                let bg = if t.workspaces {
                    fill(
                        target,
                        Rect::new(area.x, y, area.width, 1),
                        theme.fg,
                        theme.surface,
                    );
                    theme.surface
                } else {
                    crate::widgets::behind(target, area.x, area.y, theme.bg)
                };
                if area.height > 1 && !t.workspaces {
                    let rule = Style::new().fg(theme.border.color()).bg(bg.color());
                    for x in area.left()..area.right() {
                        target.cells[(x, y + 1)].set_symbol("─").set_style(rule);
                    }
                }
                for (k, rect) in titles {
                    let x = (i32::from(rect.x) + off.0).max(0) as u16;
                    let on = k == active;
                    let label = self.tab_label(visible[k]);
                    let (fg, tbg) = if on {
                        (theme.bg, theme.accent)
                    } else if self.hover == Some(visible[k]) {
                        (theme.fg, theme.surface)
                    } else {
                        (theme.muted.mix(theme.fg, 0.4), bg)
                    };
                    let tab = Rect::new(x, y, rect.width, 1).intersection(area);
                    fill(target, tab, fg, tbg);
                    let line = text::layout(&label, usize::from(rect.width), Direction::Ltr)
                        .swap_remove(0);
                    let lw = (line.width as u16).min(tab.width);
                    target.put_line(
                        x + (tab.width.saturating_sub(lw)) / 2,
                        y,
                        tab.width,
                        &line,
                        fg,
                        tbg,
                        on,
                    );
                    if on && area.height > 1 && !t.workspaces {
                        let under = Style::new().fg(theme.accent.color()).bg(bg.color());
                        for ux in x..(x + rect.width).min(area.right()) {
                            target.cells[(ux, y + 1)].set_symbol("━").set_style(under);
                        }
                    }
                }
            }
            _ => {
                if let Some(w) = n.widget() {
                    w.paint(area, target, &self.pcx(i, glow, hints));
                }
            }
        }
    }

    fn paint_children(
        &self,
        i: usize,
        target: &mut Frame,
        off: (i32, i32),
        clip: Rect,
        hints: &[(String, String)],
    ) {
        let n = &self.tree.nodes[i];
        if self.is_popped(i) {
            return; // shown in its own window
        }
        for &c in &n.children {
            if !matches!(self.tree.nodes[c].kind, Kind::Modal(_)) {
                self.paint_node(c, target, off, clip, hints);
            }
        }
        let theme = &n.theme;
        // Split dividers.
        for (k, d) in self.laid.dividers.iter().enumerate() {
            if d.split != i {
                continue;
            }
            let hot = self.hover_divider == Some(k) || self.drag.is_some_and(|g| g.divider == *d);
            let color = if hot { theme.accent } else { theme.border };
            let r = IRect {
                x: i32::from(d.rect.x) + off.0,
                y: i32::from(d.rect.y) + off.1,
                w: d.rect.width,
                h: d.rect.height,
            }
            .clip(clip);
            let sym = if d.horizontal { "│" } else { "─" };
            let bg = crate::widgets::behind(target, r.x, r.y, theme.bg);
            let style = Style::new().fg(color.color()).bg(bg.color());
            for y in r.top()..r.bottom() {
                for x in r.left()..r.right() {
                    target.cells[(x, y)].set_symbol(sym).set_style(style);
                }
            }
            // A grip in the middle shows it can be dragged.
            if r.width > 0 && r.height > 0 {
                let (gx, gy) = if d.horizontal {
                    (r.x, r.y + r.height / 2)
                } else {
                    (r.x + r.width / 2, r.y)
                };
                target.cells[(gx, gy)]
                    .set_symbol(if d.horizontal { "┃" } else { "━" })
                    .set_style(style.fg(if hot { theme.accent } else { theme.muted }.color()));
            }
        }
        // Scrolling stacks get a scrollbar.
        if matches!(n.kind, Kind::Stage(_)) && n.content_h > n.rect.h {
            let r = IRect {
                x: n.rect.x + off.0,
                y: n.rect.y + off.1,
                ..n.rect
            };
            if r.x >= 0 && r.y >= 0 {
                let x = if n.rtl {
                    r.x as u16
                } else {
                    (r.right() - 1) as u16
                };
                let vis = r.clip(clip);
                if vis.contains((x, vis.y).into()) {
                    scrollbar(
                        target,
                        x,
                        r.y as u16,
                        n.rect.h,
                        usize::from(n.scroll),
                        usize::from(n.content_h),
                        theme.border,
                        theme.accent,
                    );
                }
            }
        }
    }

    fn paint_modal(&self, m: usize, frame: &mut Frame, hints: &[(String, String)]) {
        let n = &self.tree.nodes[m];
        let Kind::Modal(mb) = &n.kind else { return };
        let screen = frame.area();
        let rect = n.rect.clip(screen);
        if rect.is_empty() {
            return;
        }
        let theme = &n.theme;
        let eff = self.effect(m);
        let shadow =
            Rect::new(rect.x + 1, rect.y + 1, rect.width, rect.height).intersection(screen);
        frame.dim_except(Rect::default(), theme.bg, 0.55);
        frame.occlude(rect.union(shadow));
        let shade = theme.bg.mix(Rgb(0, 0, 0), 0.5);
        for y in shadow.top()..shadow.bottom() {
            for x in shadow.left()..shadow.right() {
                if !rect.contains((x, y).into()) {
                    frame.cells[(x, y)]
                        .set_symbol(" ")
                        .set_style(Style::new().bg(shade.color()));
                }
            }
        }
        let local = Rect::new(0, 0, rect.width, rect.height);
        let mut sub = Frame::transparent(local, theme.bg);
        fill(&mut sub, local, theme.fg, theme.surface);
        let heading = match (mb.icon, mb.title.as_deref()) {
            (Some(i), Some(t)) => Some(format!("{i} {t}")),
            (None, Some(t)) => Some(t.to_string()),
            (Some(i), None) => Some(i.to_string()),
            _ => None,
        };
        draw_box(
            &mut sub,
            local,
            true,
            theme.accent.mix(theme.fg, eff.glow * 0.6),
            theme.bg,
            heading.as_deref(),
            theme.accent,
            n.rtl,
        );
        let off = (-n.rect.x, -n.rect.y);
        let inner = Rect::new(
            1,
            1,
            local.width.saturating_sub(2),
            local.height.saturating_sub(2),
        );
        for &c in &n.children {
            self.paint_node(c, &mut sub, off, inner, hints);
        }
        if n.content_h > n.rect.h {
            let x = if n.rtl { 0 } else { local.width - 1 };
            scrollbar(
                &mut sub,
                x,
                1,
                local.height.saturating_sub(2),
                usize::from(n.scroll),
                usize::from(n.content_h),
                theme.border,
                theme.accent,
            );
        }
        frame.blit(
            &sub,
            i32::from(rect.x) + eff.dx,
            i32::from(rect.y) + eff.dy,
            screen,
            eff.alpha,
            theme.bg,
            eff.settled,
        );
    }

    fn toast_rects(&self) -> Vec<Rect> {
        let (cols, rows) = self.size;
        let top = self.laid.content.y + 1;
        let mut y = top;
        let rtl = self.tree.dir == Direction::Rtl;
        self.toasts
            .iter()
            .map(|(_, t)| {
                let w = (text::width(&t.text) as u16 + 8)
                    .min(48)
                    .min(cols.saturating_sub(2))
                    .max(10);
                let h = 3u16;
                let x = if rtl { 1 } else { cols.saturating_sub(w + 2) };
                let r = Rect::new(x, y, w, h).intersection(Rect::new(0, 0, cols, rows));
                y += h;
                r
            })
            .collect()
    }

    fn paint_toasts(&self, frame: &mut Frame) {
        let theme = self.tree.theme;
        for ((_, t), rect) in self.toasts.iter().zip(self.toast_rects()) {
            if rect.is_empty() {
                continue;
            }
            let mut card = Card::new(t.text.clone());
            card.icon = t.icon;
            card.padding = (1, 0);
            card.dir = "auto".into();
            let mut th = theme;
            th.accent = t.accent;
            let local = Rect::new(0, 0, rect.width, rect.height);
            let mut sub = Frame::transparent(local, theme.bg);
            let content = match t.icon {
                Some(i) => format!("{i}  {}", t.text),
                None => t.text.clone(),
            };
            card.content = content;
            card.icon = None;
            let hints = [];
            card.paint(
                local,
                &mut sub,
                &PaintCx {
                    theme: &th,
                    cell_px: self.cx.cell_px,
                    focused: true,
                    hovered: false,
                    now: self.now,
                    glow: 0.0,
                    rtl: self.tree.dir == Direction::Rtl,
                    pane: false,
                    hints: &hints,
                },
            );
            let e = motion::effect(
                motion::Anim::SlideLeft,
                self.now.saturating_sub(t.born),
                self.now,
                self.tree.reduce_motion,
            );
            let dx = if self.tree.dir == Direction::Rtl {
                -e.dx
            } else {
                e.dx
            };
            frame.occlude(rect);
            frame.blit(
                &sub,
                i32::from(rect.x) + dx,
                i32::from(rect.y),
                frame.area(),
                e.alpha,
                theme.bg,
                e.settled,
            );
        }
    }

    /// The open pop-up, if any: `(widget, rect)`.
    fn popup(&self) -> Option<(usize, Rect)> {
        let f = self.focus?;
        let n = &self.tree.nodes[f];
        let area = n.bounds;
        let screen = Rect::new(0, 0, self.size.0, self.size.1);
        n.widget()?.popup(area, screen).map(|r| (f, r))
    }

    /// The natural size of the content (cells), capped at `max_cols`. Used to crop
    /// single-card renders (`stagetui render "text"`, `ts.show("text")`).
    pub fn natural_size(&self, max_cols: u16) -> (u16, u16) {
        let cx = self.cx;
        let sizes: Vec<_> = self
            .tree
            .roots
            .iter()
            .filter_map(|r| self.tree.nodes[*r].widget())
            .map(|w| w.measure(max_cols, &cx))
            .collect();
        let w = sizes.iter().map(|s| s.0).max().unwrap_or(1);
        let h: u16 = sizes.iter().map(|s| s.1).sum::<u16>() + sizes.len().saturating_sub(1) as u16;
        (w.max(1), h.max(1))
    }

    /// The agent-facing JSON tree (bounds reflect the last frame).
    pub fn tree_json(&self, cols: u16, rows: u16) -> Value {
        let mut v = self.tree.to_json(cols, rows, self.focus);
        if let Value::Object(map) = &mut v {
            if !self.toasts.is_empty() {
                map.insert(
                    "toasts".into(),
                    json!(
                        self.toasts
                            .iter()
                            .map(|(_, t)| t.text.clone())
                            .collect::<Vec<_>>()
                    ),
                );
            }
            if self.laid.content_rows > self.laid.content.height {
                map.insert(
                    "scroll".into(),
                    json!({ "offset": self.root_scroll, "content_rows": self.laid.content_rows }),
                );
            }
        }
        v
    }

    // ----- focus ----------------------------------------------------------

    /// The open modal on top, straight from the tree (so it is right even
    /// before the next frame is laid out).
    fn top_modal(&self) -> Option<usize> {
        self.tree.preorder().into_iter().rev().find(|i| {
            let n = &self.tree.nodes[*i];
            matches!(n.kind, Kind::Modal(_)) && n.visible && self.shown(*i)
        })
    }

    /// Visible, and so are all its ancestors.
    fn shown(&self, mut i: usize) -> bool {
        loop {
            if !self.tree.nodes[i].visible {
                return false;
            }
            match self.tree.nodes[i].parent {
                Some(p) => i = p,
                None => return true,
            }
        }
    }

    /// Whether node `i` can hold the keyboard focus: interactive widgets, and
    /// panes whose content has nothing focusable (so they can still be split or closed).
    fn can_focus(&self, i: usize) -> bool {
        let n = &self.tree.nodes[i];
        if n.widget().is_some_and(Widget::focusable) {
            return true;
        }
        let Kind::Pane(pb) = &n.kind else {
            return false;
        };
        let Some(c) = crate::layout::pane_child(&self.tree, i, pb) else {
            return true;
        };
        !self.tree.preorder().into_iter().any(|d| {
            self.within(d, c) && self.tree.nodes[d].widget().is_some_and(Widget::focusable)
        })
    }

    /// Widgets that can take focus, in order (only inside the top modal while one is open).
    fn focusables(&self) -> Vec<usize> {
        let scope = self.top_modal();
        // Each window cycles through its own widgets.
        let popped = self.tree.popped_nodes();
        let window = self.view_pane.and_then(|p| self.popped_content(p));
        self.tree
            .preorder()
            .into_iter()
            .filter(|i| {
                let n = &self.tree.nodes[*i];
                n.visible
                    && n.bounds.width > 0
                    && self.can_focus(*i)
                    && match window {
                        Some(c) => self.within(*i, c),
                        None => {
                            !popped.contains(i)
                                && scope.is_none_or(|m| self.within(*i, m))
                                && (scope.is_some() || !self.in_modal(*i))
                        }
                    }
            })
            .collect()
    }

    fn within(&self, mut i: usize, ancestor: usize) -> bool {
        loop {
            if i == ancestor {
                return true;
            }
            match self.tree.nodes[i].parent {
                Some(p) => i = p,
                None => return false,
            }
        }
    }

    fn in_modal(&self, mut i: usize) -> bool {
        loop {
            if matches!(self.tree.nodes[i].kind, Kind::Modal(_)) {
                return true;
            }
            match self.tree.nodes[i].parent {
                Some(p) => i = p,
                None => return false,
            }
        }
    }

    fn set_focus(&mut self, i: Option<usize>) {
        if self.focus != i {
            if let Some(old) = self.focus
                && let Some(w) = self.tree.nodes[old].kind.widget_mut()
            {
                w.close_popup();
            }
            self.focus = i;
            self.dirty = true;
        }
        if let Some(i) = i {
            self.reveal(i);
        }
    }

    fn move_focus(&mut self, forward: bool) {
        let list = self.focusables();
        if list.is_empty() {
            return;
        }
        let pos = self.focus.and_then(|f| list.iter().position(|x| *x == f));
        let next = match (pos, forward) {
            (None, true) => 0,
            (None, false) => list.len() - 1,
            (Some(p), true) => (p + 1) % list.len(),
            (Some(p), false) => (p + list.len() - 1) % list.len(),
        };
        self.set_focus(Some(list[next]));
    }

    /// Scroll so that node `i` is visible.
    fn reveal(&mut self, i: usize) {
        let target = self.tree.nodes[i].rect;
        let mut cur = self.tree.nodes[i].parent;
        while let Some(p) = cur {
            let n = self.tree.nodes[p].clone_geometry();
            if n.scrolls {
                let (top, bottom) = (n.rect.y, n.rect.bottom());
                let mut s = i32::from(n.scroll);
                if target.y < top {
                    s -= top - target.y;
                } else if target.bottom() > bottom {
                    s += (target.bottom() - bottom).min(target.y - top);
                }
                let max = i32::from(n.content_h - n.rect.h);
                self.tree.nodes[p].scroll = s.clamp(0, max) as u16;
                self.dirty = true;
            }
            cur = n.parent;
        }
        if self.in_modal(i) {
            return;
        }
        let c = self.laid.content;
        let (top, bottom) = (i32::from(c.y), i32::from(c.bottom()));
        let mut s = i32::from(self.root_scroll);
        if target.y < top {
            s -= top - target.y;
        } else if target.bottom() > bottom {
            s += (target.bottom() - bottom).min(target.y - top).max(0);
        }
        let max = i32::from(self.laid.content_rows.saturating_sub(c.height));
        let s = s.clamp(0, max.max(0)) as u16;
        if s != self.root_scroll {
            self.root_scroll = s;
            self.dirty = true;
        }
    }

    // ----- input ----------------------------------------------------------

    /// Apply one input. Returns whether anything changed.
    pub fn handle(&mut self, input: &Input) -> bool {
        let before = self.dirty;
        self.dirty = false;
        match input {
            Input::Key(k) => self.key(*k),
            Input::Mouse(m) => self.mouse(*m),
            Input::Paste(s) => {
                if let Some(f) = self.focus {
                    let r = self.tree.nodes[f].kind.widget_mut().map(|w| w.paste(s));
                    if let Some(r) = r {
                        self.react(f, r);
                    }
                }
            }
            Input::Resize(..) | Input::FocusGained => self.dirty = true,
            Input::FocusLost => {}
        }
        let changed = self.dirty;
        self.dirty |= before;
        changed
    }

    fn react(&mut self, i: usize, r: Reaction) {
        match r {
            Reaction::Ignored => {}
            Reaction::Redraw => self.dirty = true,
            Reaction::Emit("install_plugin", value) => {
                self.dirty = true;
                if let (Ok(pane), Some(plugin)) = (
                    self.typed_pane(&format!("#{}", self.tree.nodes[i].id)),
                    value.as_str(),
                ) {
                    self.ask_install(pane, plugin);
                }
            }
            Reaction::Emit(name, value) => {
                self.dirty = true;
                self.emit(i, name, value);
            }
            Reaction::Copy(text) => {
                self.dirty = true;
                self.emit(i, "copy", Value::String(text.clone()));
                self.clipboard = Some(text);
                self.toasts.push((
                    Rect::default(),
                    Toast {
                        text: "Copied".into(),
                        icon: Some("✓"),
                        accent: self.tree.theme.accent,
                        born: self.now,
                        ttl: Duration::from_millis(1200),
                    },
                ));
            }
        }
    }

    fn key(&mut self, k: Key) {
        if self.dialog.is_some() {
            self.dialog_key(k);
            return;
        }
        if self.menu.is_some() {
            self.menu_key(k);
            return;
        }
        // The prefix key, then one key for the app — reaches it even from a shell pane.
        if self.prefix_armed {
            self.prefix_armed = false;
            self.dirty = true;
            self.prefixed(k);
            return;
        }
        if self.keys.is("prefix", &k) {
            self.prefix_armed = true;
            self.dirty = true;
            return;
        }
        if let Some(f) = self.focus
            && self.tree.nodes[f]
                .widget()
                .is_some_and(Widget::captures_keys)
        {
            let area = self.tree.nodes[f].bounds;
            let r = self.tree.nodes[f]
                .kind
                .widget_mut()
                .map(|w| w.key(&k, area));
            if let Some(r) = r {
                self.react(f, r);
            }
            return;
        }
        // A quit key with Ctrl/Alt (Ctrl+C) works even in a text box.
        if (k.mods.ctrl || k.mods.alt || k.mods.sup) && self.keys.is("quit", &k) {
            self.user_quit();
            return;
        }
        // 1. The focused widget (or its open dropdown).
        if let Some(f) = self.focus {
            let area = self.tree.nodes[f].bounds;
            let r = self.tree.nodes[f]
                .kind
                .widget_mut()
                .map(|w| w.key(&k, area));
            if let Some(r) = r
                && r != Reaction::Ignored
            {
                self.react(f, r);
                return;
            }
        }
        // 2. The app's shortcuts.
        if let Some(b) = self.tree.bindings.iter().find(|b| b.key == k).cloned() {
            self.action(&b.action);
            return;
        }
        // 3. Built-in keys (named in the keymap, so an app can move them).
        if self.keys.is("focus_next", &k) {
            return self.move_focus(true);
        }
        if self.keys.is("focus_previous", &k) {
            return self.move_focus(false);
        }
        if self.keys.is("next_tab", &k) || self.keys.is("previous_tab", &k) {
            return self.switch_tab(self.keys.is("next_tab", &k));
        }
        // Esc steps back first: a dialog, then the focused widget.
        if k.is(Code::Esc) {
            if self.close_top_modal() {
                return;
            }
            if self.focus.is_some() {
                return self.set_focus(None);
            }
        }
        if self.keys.is("quit", &k) {
            return self.user_quit();
        }
        let rtl = self.tree.dir == Direction::Rtl;
        let plain = k.mods == Default::default();
        match k.code {
            Code::Up | Code::Down | Code::Left | Code::Right if plain && self.focus.is_some() => {
                let forward = match k.code {
                    Code::Down => true,
                    Code::Up => false,
                    Code::Right => !rtl,
                    _ => rtl,
                };
                self.move_focus(forward);
            }
            Code::Up | Code::Down | Code::PageUp | Code::PageDown | Code::Home | Code::End
                if plain =>
            {
                let page = i32::from(self.laid.content.height.saturating_sub(1).max(1));
                let delta = match k.code {
                    Code::Up => -1,
                    Code::Down => 1,
                    Code::PageUp => -page,
                    Code::PageDown => page,
                    Code::Home => -i32::from(u16::MAX),
                    _ => i32::from(u16::MAX),
                };
                self.scroll_root(delta);
            }
            _ => {}
        }
    }

    /// The key after the prefix: pane and app commands.
    fn prefixed(&mut self, k: Key) {
        if let Some(b) = self.tree.bindings.iter().find(|b| b.key == k).cloned() {
            self.action(&b.action);
            return;
        }
        let rtl = self.tree.dir == Direction::Rtl;
        let is = |name: &str| self.keys.is(name, &k);
        let pane_action = [
            ("pane_menu", "pane_menu"),
            ("split_right", "split_pane_right"),
            ("split_down", "split_pane_down"),
            ("close_pane", "close_pane"),
            ("pop_out", "pop_out"),
            ("detach", "detach"),
        ]
        .into_iter()
        .find(|(name, _)| is(name))
        .map(|(_, act)| act);
        if let Some(act) = pane_action {
            return self.action(act);
        }
        if is("focus_next") {
            return self.move_focus(true);
        }
        if is("focus_previous") {
            return self.move_focus(false);
        }
        if is("quit") && !k.is(Code::Esc) {
            return self.user_quit();
        }
        if k.mods != Default::default() {
            return;
        }
        match k.code {
            Code::Down => self.move_focus(true),
            Code::Up => self.move_focus(false),
            Code::Right => self.move_focus(!rtl),
            Code::Left => self.move_focus(rtl),
            Code::Char(c @ '1'..='9') => self.select_workspace(c as usize - '1' as usize),
            _ => {}
        }
    }

    /// Whether the prefix key was just pressed (the status bar shows pane keys).
    pub fn prefix_armed(&self) -> bool {
        self.prefix_armed
    }

    /// Run an action by name: built-ins here, the rest go to the app.
    pub fn action(&mut self, name: &str) {
        match name {
            "quit" => self.user_quit(),
            "detach" => self.detach = true,
            "pop_out" | "detach_focused" => {
                if let Some(p) = self.view_pane.or_else(|| self.current_pane()) {
                    self.toggle_pop(p);
                }
            }
            "focus_next" => self.move_focus(true),
            "focus_previous" => self.move_focus(false),
            "next_theme" => {
                let names: Vec<&str> = style::THEMES.iter().map(|t| t.name).collect();
                let pos = names
                    .iter()
                    .position(|n| *n == self.tree.theme.name)
                    .unwrap_or(0);
                let next = names[(pos + 1) % names.len()];
                let _ = self.tree.set_theme(next);
                self.dirty = true;
                self.emit_app("theme", json!(next));
            }
            "close_modal" => {
                self.close_top_modal();
            }
            "pane_menu" => {
                if let Some(p) = self.current_pane().filter(|p| self.tree.is_typed_pane(*p)) {
                    self.open_menu(p);
                }
            }
            "split_pane_right" | "split_pane_down" => {
                if let Some(p) = self.current_pane() {
                    self.split_pane(p, name == "split_pane_right");
                }
            }
            "close_pane" => {
                if let Some(p) = self.current_pane() {
                    self.close_pane(p);
                }
            }
            other => self.emit_app("action", json!(other)),
        }
    }

    fn close_top_modal(&mut self) -> bool {
        let Some(m) = self.top_modal() else {
            return false;
        };
        let Kind::Modal(mb) = &self.tree.nodes[m].kind else {
            return false;
        };
        if !mb.dismissable {
            return true;
        }
        self.set_visible(m, false);
        self.emit(m, "close", Value::Null);
        true
    }

    fn switch_tab(&mut self, forward: bool) {
        // The tabs around the focus, or else the first tabs on screen.
        let start = self.focus.or_else(|| {
            self.tree.preorder().into_iter().find(|i| {
                matches!(self.tree.nodes[*i].kind, Kind::Tabs(_))
                    && self.tree.nodes[*i].bounds.width > 0
            })
        });
        let mut cur = start;
        while let Some(c) = cur {
            if let Kind::Tabs(t) = &self.tree.nodes[c].kind {
                let count = self.tree.nodes[c]
                    .children
                    .iter()
                    .filter(|k| self.tree.nodes[**k].alive && self.tree.nodes[**k].visible)
                    .count();
                if count > 0 {
                    let next = if forward {
                        (t.active + 1) % count
                    } else {
                        (t.active + count - 1) % count
                    };
                    self.select_tab(c, next);
                }
                return;
            }
            cur = self.tree.nodes[c].parent;
        }
    }

    fn select_tab(&mut self, tabs: usize, k: usize) {
        if let Kind::Tabs(t) = &mut self.tree.nodes[tabs].kind
            && t.active != k
        {
            t.active = k;
            self.dirty = true;
            self.emit(tabs, "change", json!(k));
            // Keyboard focus follows the page (unless it is somewhere else entirely).
            if self.focus.is_none_or(|f| self.within(f, tabs)) {
                let page = self.tree.nodes[tabs]
                    .children
                    .iter()
                    .copied()
                    .filter(|c| self.tree.nodes[*c].alive && self.tree.nodes[*c].visible)
                    .nth(k);
                self.focus = page.and_then(|p| {
                    self.tree.preorder().into_iter().find(|i| {
                        self.within(*i, p)
                            && self.shown(*i)
                            && self.tree.nodes[*i].widget().is_some_and(Widget::focusable)
                    })
                });
            }
        }
    }

    fn scroll_root(&mut self, delta: i32) {
        let max = i32::from(
            self.laid
                .content_rows
                .saturating_sub(self.laid.content.height),
        );
        let s = (i32::from(self.root_scroll) + delta).clamp(0, max.max(0)) as u16;
        if s != self.root_scroll {
            self.root_scroll = s;
            self.dirty = true;
        }
    }

    fn hit(&self, x: u16, y: u16) -> Option<Hit> {
        let (toasts, rects) = (&self.toasts, self.toast_rects());
        for (k, r) in rects.iter().enumerate().take(toasts.len()) {
            if r.contains((x, y).into()) {
                return Some(Hit::Toast(k));
            }
        }
        self.hits
            .iter()
            .rev()
            .find(|(r, _)| r.contains((x, y).into()))
            .map(|(_, h)| *h)
    }

    /// The nearest focusable widget at or above `i`.
    fn focus_target(&self, mut i: usize) -> Option<usize> {
        loop {
            if self.can_focus(i) || self.tree.is_typed_pane(i) {
                return Some(i);
            }
            i = self.tree.nodes[i].parent?;
        }
    }

    fn mouse(&mut self, m: Mouse) {
        let (x, y) = (m.x, m.y);
        if let Some((rect, buttons)) = self.dialog_rects(Rect::new(0, 0, self.size.0, self.size.1))
        {
            let at = buttons.iter().position(|b| b.contains((x, y).into()));
            match m.kind {
                MouseKind::Down(_) => match at {
                    Some(k) => self.dialog_choose(k),
                    None if !rect.contains((x, y).into()) => {
                        let last = buttons.len().saturating_sub(1);
                        self.dialog_choose(last);
                    }
                    None => {}
                },
                MouseKind::Move => {
                    if let Some(k) = at
                        && let Some(d) = &mut self.dialog
                        && d.selected != k
                    {
                        d.selected = k;
                        self.dirty = true;
                    }
                }
                _ => {}
            }
            return;
        }
        // Widgets that want raw mouse events (a shell running a mouse-aware app).
        if self.popup().is_none()
            && self.drag.is_none()
            && let Some(Hit::Node(i)) = self.hit(x, y)
            && let Some(t) = self.focus_target(i)
        {
            let r = self.tree.nodes[t].rect;
            let b = self.tree.nodes[t].bounds;
            let (lx, ly) = (
                (i32::from(x) - r.x).max(0) as u16,
                (i32::from(y) - r.y).max(0) as u16,
            );
            let res = self.tree.nodes[t]
                .kind
                .widget_mut()
                .map(|w| w.mouse(m.kind, lx, ly, b));
            if let Some(res) = res
                && res != Reaction::Ignored
            {
                if matches!(m.kind, MouseKind::Down(_)) {
                    self.set_focus(Some(t));
                }
                self.react(t, res);
                return;
            }
        }
        if self.menu.is_some() {
            if let MouseKind::Down(_) = m.kind {
                let rect = self.menu_rect().unwrap_or_default();
                if rect.contains((x, y).into()) {
                    if let Some(k) = self.menu_item_at(y - rect.y) {
                        let pane = self.menu.as_ref().map_or(0, |m| m.pane);
                        let key = self
                            .menu
                            .as_ref()
                            .map(|m| m.items[k].key.clone())
                            .unwrap_or_default();
                        self.menu = None;
                        self.menu_choose(pane, &key);
                    }
                } else {
                    self.menu = None;
                }
                self.dirty = true;
            } else if m.kind == MouseKind::Move {
                let rect = self.menu_rect().unwrap_or_default();
                if rect.contains((x, y).into())
                    && let Some(k) = self.menu_item_at(y - rect.y)
                    && let Some(menu) = &mut self.menu
                    && menu.hover != k
                {
                    menu.hover = k;
                    self.dirty = true;
                }
            }
            return;
        }
        match m.kind {
            MouseKind::Down(Button::Left) => {
                if let Some((p, rect)) = self.popup() {
                    if rect.contains((x, y).into()) {
                        let r = self.tree.nodes[p]
                            .kind
                            .widget_mut()
                            .map(|w| w.click_popup(x - rect.x, y - rect.y, rect));
                        if let Some(r) = r {
                            self.react(p, r);
                        }
                        return;
                    }
                    if let Some(w) = self.tree.nodes[p].kind.widget_mut() {
                        w.close_popup();
                    }
                    self.dirty = true;
                    if self.tree.nodes[p].bounds.contains((x, y).into()) {
                        return;
                    }
                }
                match self.hit(x, y) {
                    Some(Hit::Divider(d)) => {
                        self.drag = Some(Drag {
                            divider: self.laid.dividers[d],
                        });
                        self.dirty = true;
                    }
                    Some(Hit::Tab(t, k)) => self.select_tab(t, k),
                    Some(Hit::PaneTitle(p)) => {
                        self.set_focus(Some(p));
                        self.focus_into(p);
                        self.open_menu(p);
                    }
                    Some(Hit::PaneAction(p, k)) => match k {
                        0 => self.split_pane(p, true),
                        1 => self.split_pane(p, false),
                        2 => self.toggle_pop(p),
                        3 => self.close_pane(p),
                        _ => self.pop_in_idx(p),
                    },
                    Some(Hit::Backdrop(_)) => {
                        self.close_top_modal();
                    }
                    Some(Hit::Toast(k)) => {
                        if k < self.toasts.len() {
                            self.toasts.remove(k);
                            self.dirty = true;
                        }
                    }
                    Some(Hit::Node(i)) => {
                        let target = self.focus_target(i);
                        if target.is_some() {
                            self.set_focus(target);
                        }
                        if let Some(t) = target {
                            let b = self.tree.nodes[t].bounds;
                            let r = self.tree.nodes[t].rect;
                            let (lx, ly) = (
                                (i32::from(x) - r.x).max(0) as u16,
                                (i32::from(y) - r.y).max(0) as u16,
                            );
                            let res = self.tree.nodes[t]
                                .kind
                                .widget_mut()
                                .map(|w| w.click(lx, ly, b));
                            if let Some(res) = res {
                                self.react(t, res);
                            }
                        }
                    }
                    None => {}
                }
            }
            MouseKind::Up(_) => {
                if self.drag.take().is_some() {
                    self.dirty = true;
                }
            }
            MouseKind::Drag(Button::Left) => {
                if let Some(d) = self.drag {
                    self.drag_divider(d.divider, x, y);
                }
            }
            MouseKind::Move => {
                let (hover, divider, button) = match self.hit(x, y) {
                    Some(Hit::Node(i)) => (self.focus_target(i), None, None),
                    Some(Hit::Divider(d)) => (None, Some(d), None),
                    Some(Hit::PaneAction(p, k)) => (None, None, Some((p, k))),
                    _ => (None, None, None),
                };
                if hover != self.hover
                    || divider != self.hover_divider
                    || button != self.hover_pane_button
                {
                    self.hover = hover;
                    self.hover_divider = divider;
                    self.hover_pane_button = button;
                    self.dirty = true;
                }
            }
            MouseKind::ScrollUp | MouseKind::ScrollDown => {
                let delta = if m.kind == MouseKind::ScrollUp { -3 } else { 3 };
                self.wheel(x, y, delta);
            }
            _ => {}
        }
    }

    fn wheel(&mut self, x: u16, y: u16, delta: i32) {
        if let Some((p, _)) = self.popup() {
            let code = if delta < 0 { Code::Up } else { Code::Down };
            let r = self.tree.nodes[p]
                .kind
                .widget_mut()
                .map(|w| w.key(&Key::plain(code), Rect::default()));
            if let Some(r) = r {
                self.react(p, r);
            }
            return;
        }
        let mut cur = match self.hit(x, y) {
            Some(Hit::Node(i)) => Some(i),
            _ => None,
        };
        while let Some(i) = cur {
            let area = self.tree.nodes[i].bounds;
            let n = &mut self.tree.nodes[i];
            if let Some(w) = n.kind.widget_mut()
                && w.scroll_by(delta, area)
            {
                self.dirty = true;
                return;
            }
            if matches!(n.kind, Kind::Stage(_) | Kind::Modal(_)) && n.content_h > n.rect.h {
                let max = i32::from(n.content_h - n.rect.h);
                let s = (i32::from(n.scroll) + delta).clamp(0, max) as u16;
                if s != n.scroll {
                    n.scroll = s;
                    self.dirty = true;
                    return;
                }
            }
            if matches!(n.kind, Kind::Modal(_)) {
                return;
            }
            cur = n.parent;
        }
        self.scroll_root(delta);
    }

    fn drag_divider(&mut self, d: crate::layout::Divider, x: u16, y: u16) {
        let kids = self.tree.nodes[d.split].children.clone();
        let Some(&a) = kids.get(d.index) else { return };
        let Some(&b) = kids[d.index + 1..]
            .iter()
            .find(|c| self.tree.nodes[**c].visible && self.tree.nodes[**c].alive)
        else {
            return;
        };
        let bi = kids.iter().position(|c| *c == b).unwrap_or(d.index + 1);
        let (ra, rb) = (self.tree.nodes[a].rect, self.tree.nodes[b].rect);
        let (total, size_a) = if d.horizontal {
            let total = i32::from(ra.w) + i32::from(rb.w);
            let na = if ra.x < rb.x {
                i32::from(x) - ra.x
            } else {
                ra.right() - i32::from(x) - 1
            };
            (total, na)
        } else {
            (i32::from(ra.h) + i32::from(rb.h), i32::from(y) - ra.y)
        };
        if total < 4 {
            return;
        }
        let na = size_a.clamp(2, total - 2) as f32;
        if let Kind::Split(s) = &mut self.tree.nodes[d.split].kind {
            let pair = s.ratios.get(d.index).copied().unwrap_or(0.5)
                + s.ratios.get(bi).copied().unwrap_or(0.5);
            let new_a = pair * na / total as f32;
            if let Some(r) = s.ratios.get_mut(d.index) {
                *r = new_a;
            }
            if let Some(r) = s.ratios.get_mut(bi) {
                *r = pair - new_a;
            }
            self.dirty = true;
        }
    }

    // ----- changes from the app ---------------------------------------------

    fn target(&self, target: &str) -> Result<usize> {
        self.tree.require(target)
    }

    /// Change a widget's content or properties: `{"content": ..., "props": {...}}`.
    pub fn update(&mut self, target: &str, patch: &Value) -> Result<()> {
        let i = self.target(target)?;
        let was_visible = self.tree.nodes[i].visible;
        self.tree.update(i, patch)?;
        let now_visible = self.tree.nodes[i].visible;
        if was_visible != now_visible {
            self.after_visibility(i, now_visible);
        }
        self.dirty = true;
        Ok(())
    }

    /// Replace a widget's primary content.
    pub fn load(&mut self, target: &str, content: Value) -> Result<()> {
        self.update(target, &json!({ "content": content }))
    }

    /// Show or hide a widget (modals open and close this way).
    pub fn set_shown(&mut self, target: &str, visible: bool) -> Result<()> {
        let i = self.target(target)?;
        self.set_visible(i, visible);
        Ok(())
    }

    fn set_visible(&mut self, i: usize, visible: bool) {
        if self.tree.nodes[i].visible == visible {
            return;
        }
        self.tree.nodes[i].visible = visible;
        self.tree.nodes[i].spec.props.visible = Some(visible);
        self.after_visibility(i, visible);
        self.dirty = true;
    }

    fn after_visibility(&mut self, i: usize, visible: bool) {
        if !visible {
            self.tree.nodes[i].born = None;
        }
        if matches!(self.tree.nodes[i].kind, Kind::Modal(_)) {
            if visible {
                self.modal_return.push((i, self.focus));
                // Focus the first control inside once laid out.
                let first = self.tree.preorder().into_iter().find(|c| {
                    *c != i
                        && self.within(*c, i)
                        && self.tree.nodes[*c].widget().is_some_and(Widget::focusable)
                });
                self.focus = first;
            } else if let Some(pos) = self.modal_return.iter().rposition(|(m, _)| *m == i) {
                let (_, back) = self.modal_return.remove(pos);
                self.focus = back.filter(|b| self.tree.nodes[*b].alive);
            }
        } else if !visible && self.focus.is_some_and(|f| self.within(f, i)) {
            self.focus = None;
        }
    }

    /// Give keyboard focus to a widget.
    pub fn focus(&mut self, target: &str) -> Result<()> {
        let i = self.target(target)?;
        if !self.can_focus(i) && !self.tree.is_typed_pane(i) {
            return Err(Error::friendly(
                format!("\"{}\" can't take focus", self.tree.nodes[i].id),
                "only interactive widgets (buttons, inputs, lists, panes…) take keyboard focus",
                "focus a button, input, select, table, tree or a pane of a split",
            ));
        }
        self.set_focus(Some(i));
        Ok(())
    }

    /// Split a widget with a new one. `direction`: `"horizontal"` (side by side) or `"vertical"`.
    pub fn split(&mut self, target: &str, spec: &NodeSpec, direction: &str) -> Result<String> {
        let i = self.target(target)?;
        let horizontal = match direction {
            "horizontal" | "right" | "row" => true,
            "vertical" | "down" | "column" => false,
            other => {
                return Err(Error::friendly(
                    format!("Can't split \"{other}\""),
                    "a split places the new widget beside or below the old one",
                    "use direction=\"horizontal\" (side by side) or \"vertical\" (below)",
                ));
            }
        };
        let new = self.tree.split(i, spec, horizontal)?;
        self.dirty = true;
        if self.tree.nodes[new].widget().is_some_and(Widget::focusable) {
            self.focus = Some(new);
        }
        Ok(self.tree.nodes[new].id.clone())
    }

    /// Remove a widget (closing a pane joins the split back together).
    pub fn remove(&mut self, target: &str) -> Result<()> {
        let i = self.target(target)?;
        let next = if self.focus.is_some_and(|f| self.within(f, i)) {
            let list = self.focusables();
            let pos = list.iter().position(|x| self.within(*x, i));
            pos.and_then(|p| {
                list[..p]
                    .iter()
                    .rev()
                    .chain(list[p..].iter())
                    .find(|x| !self.within(**x, i))
                    .copied()
            })
        } else {
            self.focus
        };
        self.tree.remove(i);
        self.frames.remove(&i);
        self.focus = next;
        self.dirty = true;
        Ok(())
    }

    /// Add a widget to a container (or to the app when `parent` is `None`).
    pub fn mount(&mut self, parent: Option<&str>, spec: &NodeSpec) -> Result<String> {
        let p = parent.map(|t| self.target(t)).transpose()?;
        if let Some(p) = p
            && !self.tree.nodes[p].kind.is_container()
        {
            return Err(Error::friendly(
                format!(
                    "Can't add widgets inside a {}",
                    self.tree.nodes[p].kind.name()
                ),
                "only Stage, StageSplit, StageTabs and StageModal hold other widgets",
                "mount into a Stage([...]) instead",
            ));
        }
        let i = self.tree.insert(spec, p)?;
        match p {
            Some(p) => {
                self.tree.nodes[p].children.push(i);
                if let Kind::Split(s) = &mut self.tree.nodes[p].kind {
                    let share = 1.0 / s.ratios.len().max(1) as f32;
                    s.ratios.push(share);
                    let sum: f32 = s.ratios.iter().sum();
                    s.ratios.iter_mut().for_each(|r| *r /= sum);
                }
            }
            None => self.tree.roots.push(i),
        }
        if self.tree.nodes[i].visible && matches!(self.tree.nodes[i].kind, Kind::Modal(_)) {
            self.after_visibility(i, true);
        }
        self.dirty = true;
        Ok(self.tree.nodes[i].id.clone())
    }

    /// Switch the theme while running.
    pub fn set_theme(&mut self, name: &str) -> Result<()> {
        self.tree.set_theme(name)?;
        self.dirty = true;
        Ok(())
    }

    /// Show a short message in the corner for `timeout`.
    pub fn notify(
        &mut self,
        text: &str,
        icon: Option<&str>,
        accent: Option<&str>,
        timeout: Duration,
    ) -> Result<()> {
        let icon = icon.map(crate::widgets::icon).transpose()?;
        let accent = accent
            .map(style::accent)
            .transpose()?
            .unwrap_or(self.tree.theme.accent);
        self.toasts.push((
            Rect::default(),
            Toast {
                text: text.to_string(),
                icon,
                accent,
                born: self.now,
                ttl: timeout.max(Duration::from_millis(500)),
            },
        ));
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
        self.dirty = true;
        Ok(())
    }

    /// A live canvas delivered its new picture.
    pub fn set_image(&mut self, target: &str, image: Image) -> Result<()> {
        let i = self.target(target)?;
        match &mut self.tree.nodes[i].kind {
            Kind::Image(v) => v.image = image,
            _ => {
                return Err(Error::friendly(
                    "Only pictures can take a new image",
                    format!(
                        "\"{}\" is a {}",
                        self.tree.nodes[i].id,
                        self.tree.nodes[i].kind.name()
                    ),
                    "call this on a StageImage or StageCanvas",
                ));
            }
        }
        if let Some(c) = self.frames.get_mut(&i) {
            c.pending = false;
        }
        self.dirty = true;
        Ok(())
    }

    /// A widget's current value (typed text, checked state, selection…).
    pub fn value(&self, target: &str) -> Result<Value> {
        let i = self.target(target)?;
        let n = &self.tree.nodes[i];
        Ok(match &n.kind {
            Kind::Tabs(t) => json!(t.active),
            Kind::Split(s) => json!(s.ratios),
            k => k.widget().map_or(Value::Null, Widget::value),
        })
    }

    /// One widget's JSON (like `tree_json`, for a single node).
    pub fn query(&self, target: &str) -> Option<Value> {
        let i = self.tree.find(target)?;
        fn find(v: &Value, id: &str) -> Option<Value> {
            if v.get("id").and_then(Value::as_str) == Some(id) {
                return Some(v.clone());
            }
            v.get("children")?
                .as_array()?
                .iter()
                .find_map(|c| find(c, id))
        }
        let all = self.tree_json(self.size.0, self.size.1);
        find(&all, &self.tree.nodes[i].id)
    }

    /// `(key, id)` of the focused widget.
    pub fn focused(&self) -> Option<(Option<String>, String)> {
        self.focus.map(|f| {
            (
                self.tree.nodes[f].key.clone(),
                self.tree.nodes[f].id.clone(),
            )
        })
    }

    /// `(key, id)` of the pane holding the focus (the focused widget's ancestor
    /// that sits directly in a split or tabs), or the focused widget itself.
    pub fn focused_pane(&self) -> Option<(Option<String>, String)> {
        let f = self.focus?;
        let mut cur = Some(f);
        while let Some(c) = cur {
            if self.tree.is_pane(c) {
                let n = &self.tree.nodes[c];
                return Some((n.key.clone(), n.id.clone()));
            }
            cur = self.tree.nodes[c].parent;
        }
        self.focused()
    }

    // ----- scripted input (headless runs and agents) -------------------------

    /// Press a key described like `"ctrl+s"`.
    pub fn press(&mut self, key: &str) -> Result<()> {
        let k = Key::parse(key)?;
        self.handle(&Input::Key(k));
        Ok(())
    }

    /// Type text as key presses.
    pub fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let k = match c {
                '\n' => Key::plain(Code::Enter),
                '\t' => Key::plain(Code::Tab),
                c => Key::plain(Code::Char(c)),
            };
            self.handle(&Input::Key(k));
        }
    }

    /// Click the middle of a widget (it must be on screen).
    pub fn click(&mut self, target: &str) -> Result<()> {
        let i = self.target(target)?;
        let b = self.tree.nodes[i].bounds;
        if b.is_empty() {
            return Err(Error::friendly(
                format!("\"{}\" is not on screen", self.tree.nodes[i].id),
                "only visible widgets can be clicked",
                "show it first, scroll to it, or make the screen bigger",
            ));
        }
        self.click_at(b.x + b.width / 2, b.y + b.height / 2);
        Ok(())
    }

    /// Click at a cell.
    pub fn click_at(&mut self, x: u16, y: u16) {
        for kind in [MouseKind::Down(Button::Left), MouseKind::Up(Button::Left)] {
            self.handle(&Input::Mouse(Mouse { kind, x, y }));
        }
    }

    /// Drag with the left button from one cell to another, one cell at a time
    /// (like a hand would): split lines, sliders, 3D models…
    pub fn drag(&mut self, from: (u16, u16), to: (u16, u16)) {
        self.handle(&Input::Mouse(Mouse {
            kind: MouseKind::Down(Button::Left),
            x: from.0,
            y: from.1,
        }));
        let (dx, dy) = (
            i32::from(to.0) - i32::from(from.0),
            i32::from(to.1) - i32::from(from.1),
        );
        let steps = dx.abs().max(dy.abs()).max(1);
        for i in 1..=steps {
            let x = i32::from(from.0) + dx * i / steps;
            let y = i32::from(from.1) + dy * i / steps;
            self.handle(&Input::Mouse(Mouse {
                kind: MouseKind::Drag(Button::Left),
                x: x as u16,
                y: y as u16,
            }));
        }
        self.handle(&Input::Mouse(Mouse {
            kind: MouseKind::Up(Button::Left),
            x: to.0,
            y: to.1,
        }));
    }

    /// Scroll the wheel over a cell (`rows` < 0 scrolls up).
    pub fn scroll_at(&mut self, x: u16, y: u16, rows: i32) {
        let kind = if rows < 0 {
            MouseKind::ScrollUp
        } else {
            MouseKind::ScrollDown
        };
        for _ in 0..(rows.unsigned_abs().div_ceil(3)) {
            self.handle(&Input::Mouse(Mouse { kind, x, y }));
        }
    }

    /// Advance virtual time until entrance animations finish (at most 5 s),
    /// so snapshots show the settled screen.
    pub fn settle(&mut self, cols: u16, rows: u16) {
        let _ = self.frame(cols, rows);
        let mut waited = Duration::ZERO;
        while waited < Duration::from_secs(5) {
            let busy = self.tree.preorder().into_iter().any(|i| {
                let n = &self.tree.nodes[i];
                n.bounds.width > 0
                    && !n.animate.loops()
                    && n.born.is_some_and(|b| {
                        motion::active(
                            n.animate,
                            self.now.saturating_sub(b),
                            self.tree.reduce_motion,
                        )
                    })
            }) || self
                .toasts
                .iter()
                .any(|(_, t)| self.now.saturating_sub(t.born) < motion::ENTER);
            if !busy {
                break;
            }
            self.advance(TICK);
            waited += TICK;
            let _ = self.frame(cols, rows);
        }
    }
}

/// Content rows in a pane menu: its items, and one rule before the actions.
fn menu_rows(m: &crate::panes::Menu) -> usize {
    m.items.len() + usize::from(m.first_action() < m.items.len())
}

/// The content row an item is drawn on.
fn menu_row_of(m: &crate::panes::Menu, k: usize) -> usize {
    let sep = m.first_action();
    k + usize::from(sep < m.items.len() && k >= sep)
}

/// The item on a content row (`None` for the rule).
fn menu_item_of_row(m: &crate::panes::Menu, r: usize) -> Option<usize> {
    let sep = m.first_action();
    let k = if sep < m.items.len() && r >= sep {
        if r == sep {
            return None;
        }
        r - 1
    } else {
        r
    };
    (k < m.items.len()).then_some(k)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(json: &str) -> Session {
        Session::from_json(json).unwrap()
    }

    const POP: &str = r##"{"children":[{"type":"split","children":[
        {"type":"pane","content":"markdown","props":{"id":"a"},"children":[{"type":"markdown","content":"# مَرْحَبًا Hello 日本語 नमस्ते"}]},
        {"type":"pane","content":"notes","props":{"id":"b"},"children":[{"type":"input","props":{"id":"box"}}]}]}]}"##;

    fn key(s: &mut Session, window: Option<usize>, k: &str) {
        s.handle_in(window, &Input::Key(Key::parse(k).unwrap()));
    }

    #[test]
    fn a_pane_pops_out_into_its_own_window_and_back() {
        let mut s = session(POP);
        assert!(s.frame(100, 30).text().contains("Hello"));
        let a = s.tree().require("#a").unwrap();
        s.pop_out("#a").unwrap();
        assert_eq!(s.take_popouts(), ["a"]);
        assert_eq!(s.query("#a").unwrap()["detached"], true);
        let main = s.frame(100, 30).text();
        assert!(
            main.contains("open in its own window") && !main.contains("Hello"),
            "{main}"
        );
        let win = s.pane_frame(a, 60, 16).unwrap().text();
        assert!(win.contains("Hello") && win.contains("日本語"), "{win}");
        assert!(
            win.lines().next().unwrap().contains("↗") && win.contains("Put back"),
            "{win}"
        );
        // Clicking "Bring it back" in the main window puts it back.
        let b = s.placeholder_button(a).unwrap();
        s.click_at(b.x + 1, b.y);
        assert_eq!(s.take_popins(), ["a"]);
        assert!(s.pane_frame(a, 60, 16).is_none());
        assert!(s.frame(100, 30).text().contains("Hello"));
    }

    #[test]
    fn each_window_types_into_its_own_widgets() {
        let mut s = session(POP);
        s.frame(100, 30);
        let b = s.tree().require("#b").unwrap();
        s.focus("#box").unwrap();
        s.press("ctrl+space").unwrap();
        s.press("o").unwrap(); // pop out the focused pane
        assert_eq!(s.take_popouts(), ["b"]);
        s.frame(100, 30);
        s.pane_frame(b, 50, 10).unwrap();
        for c in ["h", "i"] {
            key(&mut s, Some(b), c);
        }
        assert_eq!(s.value("#box").unwrap(), json!("hi"));
        key(&mut s, None, "x"); // the main window doesn't type into it
        assert_eq!(s.value("#box").unwrap(), json!("hi"));
        // Esc leaves the box; Esc again in the pop-out window puts the pane back.
        s.pane_frame(b, 50, 10).unwrap();
        key(&mut s, Some(b), "esc");
        key(&mut s, Some(b), "esc");
        assert!(!s.quit_requested());
        assert_eq!(s.take_popins(), ["b"]);
        assert_eq!(s.value("#box").unwrap(), json!("hi"), "nothing is lost");
        // A window closing on its own also brings the pane back (no window to close).
        s.pane_window_opened(b);
        assert!(s.take_popouts().is_empty(), "the window is already there");
        s.pane_window_closed(b);
        assert!(s.take_popins().is_empty());
        assert_eq!(s.query("#b").unwrap()["detached"], false);
    }

    const ADDON: &str = r##"{"children":[{"type":"split","children":[
        {"type":"pane","content":"pulse","props":{"id":"v"},"children":[{"type":"plugin","content":"pulse-test"}]},
        {"type":"pane","content":"video","props":{"id":"w"},"children":[{"type":"plugin","content":"video"}]},
        {"type":"pane","content":"markdown","props":{"id":"m"},"children":[{"type":"markdown","content":"# مَرْحَبًا Hello 日本語 नमस्ते"}]}]}]}"##;

    #[test]
    fn picture_add_ons_open_where_pictures_are_real() {
        // "pulse-test" is installed (a program that never starts here); "video" is not.
        crate::plugins::pretend_installed("pulse-test");
        let mut s = session(ADDON);
        // A terminal with real pictures: nothing to do.
        s.frame(100, 30);
        assert!(s.take_popouts().is_empty());
        // Blocks here, foot installed: the add-on's pane opens in foot, once.
        s.set_pictures(false);
        s.set_picture_terminal(Some("foot".into()));
        s.frame(100, 30);
        assert_eq!(s.take_popouts(), ["v"], "only the installed add-on moves");
        assert_eq!(s.popout_terminal("v").as_deref(), Some("foot"));
        let v = s.tree().require("#v").unwrap();
        s.pane_window_closed(v);
        s.frame(100, 30);
        assert!(s.take_popouts().is_empty(), "brought back: not again");
        // No such terminal: offer to install one (where we know how).
        let mut s = session(ADDON);
        s.set_pictures(false);
        s.set_picture_terminal(None);
        let text = s.frame(100, 30).text();
        if crate::launch::picture_install().is_some() {
            assert_eq!(s.dialog_open(), Some("pictures"));
            assert!(s.frame(100, 30).text().contains("Real pictures?"), "{text}");
            s.press("esc").unwrap();
            assert_eq!(s.dialog_open(), None);
        }
        assert!(s.take_popouts().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn an_install_runs_in_the_pane_then_the_pane_switches_back() {
        let mut s = session(ADDON);
        s.frame(100, 30);
        let v = s.tree().require("#v").unwrap();
        s.run_install(
            v,
            vec![
                "sh".into(),
                "-c".into(),
                "echo installing مرحبا; exit 0".into(),
            ],
            After::Pictures("markdown".into()),
            "Installing Ghostty",
        );
        assert_eq!(s.query("#v").unwrap()["pane_type"], "installing");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !s.installing.is_empty() && std::time::Instant::now() < deadline {
            s.frame(100, 30);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(s.installing.is_empty(), "the install finished");
        assert_eq!(s.query("#v").unwrap()["pane_type"], "markdown");
        // A failed install leaves its output on screen.
        s.run_install(
            v,
            vec!["sh".into(), "-c".into(), "echo nope; exit 2".into()],
            After::Plugin("video".into()),
            "Installing Video",
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !s.installing.is_empty() && std::time::Instant::now() < deadline {
            s.frame(100, 30);
            std::thread::sleep(Duration::from_millis(20));
        }
        let text = s.frame(100, 30).text();
        assert_eq!(s.query("#v").unwrap()["pane_type"], "installing");
        assert!(text.contains("didn't work"), "{text}");
    }

    #[test]
    fn q_in_an_attached_window_asks_before_quitting() {
        let mut s =
            session(r#"{"children":[{"type":"card","content":"مَرْحَبًا Hello 日本語 नमस्ते"}]}"#);
        s.frame(80, 24);
        // The app's own terminal: q quits right away, as always.
        s.press("q").unwrap();
        assert!(s.quit_requested());

        let mut s =
            session(r#"{"children":[{"type":"card","content":"مَرْحَبًا Hello 日本語 नमस्ते"}]}"#);
        s.set_session_name(Some("night-shift".into()));
        s.set_detachable(true);
        s.frame(80, 24);
        s.press("q").unwrap();
        assert!(!s.quit_requested() && s.leave_prompt_open());
        let text = s.frame(80, 24).text();
        assert!(text.contains("Leave \"night-shift\"?"), "{text}");
        assert!(text.contains("Detach") && text.contains("Quit app") && text.contains("Cancel"));
        // Enter picks the default: detach (the app keeps running).
        s.press("enter").unwrap();
        assert!(s.take_detach() && !s.quit_requested() && !s.leave_prompt_open());
        // Esc cancels; nothing happens.
        s.press("ctrl+c").unwrap();
        s.press("esc").unwrap();
        assert!(!s.take_detach() && !s.quit_requested() && !s.leave_prompt_open());
        // Ctrl+Space then q opens it too; q again really quits.
        s.press("ctrl+space").unwrap();
        s.press("q").unwrap();
        assert!(s.leave_prompt_open());
        s.press("q").unwrap();
        assert!(s.quit_requested());
    }

    #[test]
    fn leave_prompt_works_with_the_mouse_and_arrows() {
        let mut s = session(r#"{"children":[{"type":"text","content":"hi"}]}"#);
        s.set_detachable(true);
        s.frame(80, 24);
        s.press("esc").unwrap(); // nothing focused: Esc means leave
        assert!(s.leave_prompt_open());
        s.press("right").unwrap();
        s.press("right").unwrap();
        s.press("enter").unwrap(); // Cancel
        assert!(!s.leave_prompt_open() && !s.quit_requested() && !s.take_detach());
        s.press("q").unwrap();
        let (_, buttons) = s.dialog_rects(Rect::new(0, 0, 80, 24)).unwrap();
        let quit = buttons[1];
        s.click_at(quit.x + 1, quit.y);
        assert!(s.quit_requested());
    }

    fn mem_image(w: u32, h: u32) -> String {
        let mut p = tiny_skia::Pixmap::new(w, h).unwrap();
        p.fill(tiny_skia::Color::from_rgba8(220, 40, 40, 255));
        crate::gfx::register(crate::gfx::Image::from_pixmap("red", p))
    }

    fn ids(events: &[AppEvent]) -> Vec<(String, String)> {
        events
            .iter()
            .map(|e| (e.id.clone(), e.name.clone()))
            .collect()
    }

    #[test]
    fn tab_moves_focus_and_enter_clicks() {
        let mut s = session(
            r#"{"children":[{"type":"button","content":"One","props":{"id":"one"}},{"type":"button","content":"Two","props":{"id":"two"}}]}"#,
        );
        s.frame(60, 10);
        assert_eq!(s.focused(), None);
        s.press("tab").unwrap();
        assert_eq!(s.focused().unwrap().1, "one");
        s.press("tab").unwrap();
        s.press("enter").unwrap();
        assert_eq!(ids(&s.take_events()), [("two".into(), "click".into())]);
        s.press("shift+tab").unwrap();
        assert_eq!(s.focused().unwrap().1, "one");
    }

    #[test]
    fn arabic_typed_after_english_is_joined_and_in_order() {
        let mut s = session(r#"{"children":[{"type":"input","props":{"id":"name"}}]}"#);
        s.frame(60, 6);
        let typed = "Sara · مَرْحَبًا سارة 日本語 नमस्ते";
        s.type_text(typed);
        let f = s.frame(60, 6);
        // The whole line is one text run in reading order (pixel text where pictures work)…
        assert!(
            f.runs.iter().any(|r| r.logical == typed && r.complex),
            "{:?}",
            f.runs
        );
        // …and the cells hold the joined letters, right-to-left words in visual order.
        let row: String = (0..f.cells.area.height)
            .map(|y| {
                (0..f.cells.area.width)
                    .map(|x| f.cells[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .find(|r| r.contains("Sara"))
            .unwrap();
        let sara = text::bidi::visual(&text::arabic::shape("سارة"), Direction::Rtl);
        assert!(row.contains(&sara), "{row:?} should contain {sara:?}");
        assert!(!row.contains('س'), "unjoined letters: {row:?}");
    }

    #[test]
    fn clicking_and_typing() {
        let mut s = session(
            r#"{"children":[{"type":"input","props":{"id":"name","placeholder":"Name"}},{"type":"checkbox","content":"Agree","props":{"id":"ok"}},{"type":"button","content":"Send","props":{"id":"send"}}]}"#,
        );
        s.frame(60, 16);
        assert_eq!(
            s.focused().unwrap().1,
            "name",
            "the first input is focused at start"
        );
        s.type_text("hi مرحبا");
        s.press("enter").unwrap();
        let ev = s.take_events();
        assert_eq!(ev.last().unwrap().name, "submit");
        assert_eq!(ev.last().unwrap().value, json!("hi مرحبا"));
        s.press("q").unwrap();
        assert!(!s.quit_requested(), "q types into inputs");
        s.frame(60, 16);
        s.click("#ok").unwrap();
        assert_eq!(s.value("#ok").unwrap(), json!(true));
        s.frame(60, 16);
        s.click("#send").unwrap();
        assert_eq!(s.take_events().last().unwrap().name, "click");
        assert_eq!(s.focused().unwrap().1, "send");
    }

    #[test]
    fn quit_keys_and_actions() {
        let mut s = session(
            r#"{"bindings":[{"key":"ctrl+s","action":"save","description":"Save"},{"key":"ctrl+t","action":"next_theme"}],"children":[{"type":"card","content":"x"}]}"#,
        );
        s.frame(40, 10);
        s.press("ctrl+s").unwrap();
        assert_eq!(s.take_events()[0].value, json!("save"));
        s.press("ctrl+t").unwrap();
        assert_eq!(s.tree().theme.name, "nordic-light");
        s.take_events();
        s.press("q").unwrap();
        assert!(s.quit_requested());
        assert_eq!(s.take_events()[0].name, "quit");
    }

    #[test]
    fn modal_traps_focus_and_esc_closes_it() {
        let mut s = session(
            r#"{"children":[{"type":"button","content":"Open","props":{"id":"open"}},{"type":"modal","props":{"id":"m","title":"Sure?"},"children":[{"type":"button","content":"Yes","props":{"id":"yes"}},{"type":"button","content":"No","props":{"id":"no"}}]}]}"#,
        );
        s.frame(60, 20);
        s.set_shown("#m", true).unwrap();
        s.frame(60, 20);
        assert_eq!(s.focused().unwrap().1, "yes");
        s.press("tab").unwrap();
        s.press("tab").unwrap();
        assert_eq!(
            s.focused().unwrap().1,
            "yes",
            "focus stays inside the modal"
        );
        s.press("esc").unwrap();
        assert!(!s.tree().get("m").unwrap().visible);
        assert_eq!(ids(&s.take_events()), [("m".into(), "close".into())]);
        assert!(!s.quit_requested());
    }

    #[test]
    fn modal_hides_pictures_underneath() {
        let img = mem_image(80, 80);
        let mut s = session(&format!(
            r#"{{"children":[{{"type":"image","content":"{img}","props":{{"width":30,"height":10}}}},{{"type":"modal","props":{{"visible":true}},"children":[{{"type":"text","content":"over"}}]}}]}}"#
        ));
        let f = s.frame(60, 20);
        let m = s.tree().nodes[1].rect.clip(f.area());
        for img in &f.images {
            assert!(
                img.area.intersection(m).is_empty(),
                "{:?} under {m:?}",
                img.area
            );
        }
        assert!(f.images.iter().all(|i| i.dim));
    }

    #[test]
    fn split_drag_changes_ratios() {
        let mut s = session(
            r#"{"children":[{"type":"split","children":[{"type":"card","content":"a","props":{"id":"a"}},{"type":"card","content":"b","props":{"id":"b"}}]}]}"#,
        );
        s.frame(81, 20);
        let d = s.laid.dividers[0].rect;
        assert_eq!(d.x, 40);
        for kind in [MouseKind::Down(Button::Left), MouseKind::Drag(Button::Left)] {
            let x = if matches!(kind, MouseKind::Down(_)) {
                40
            } else {
                20
            };
            s.handle(&Input::Mouse(Mouse { kind, x, y: 5 }));
        }
        s.handle(&Input::Mouse(Mouse {
            kind: MouseKind::Up(Button::Left),
            x: 20,
            y: 5,
        }));
        s.frame(81, 20);
        assert_eq!(s.tree().get("a").unwrap().rect.w, 20);
    }

    #[test]
    fn drag_moves_a_split_line() {
        let mut s = session(
            r#"{"children":[{"type":"split","children":[{"type":"card","content":"a","props":{"id":"a"}},{"type":"card","content":"b","props":{"id":"b"}}]}]}"#,
        );
        s.frame(81, 20);
        s.drag((40, 5), (60, 5));
        s.frame(81, 20);
        assert_eq!(s.tree().get("a").unwrap().rect.w, 60);
    }

    #[test]
    fn wheel_scrolls_long_pages() {
        let cards: Vec<String> = (0..8)
            .map(|i| format!(r#"{{"type":"card","content":"card {i}\nline\nline"}}"#))
            .collect();
        let mut s = session(&format!(r#"{{"children":[{}]}}"#, cards.join(",")));
        s.frame(40, 12);
        let first = s.tree().nodes[0].rect.y;
        s.scroll_at(10, 5, 3);
        s.frame(40, 12);
        assert_eq!(s.tree().nodes[0].rect.y, first - 3);
        assert!(s.tree_json(40, 12)["scroll"]["offset"] == 3);
        s.press("end").unwrap();
        s.frame(40, 12);
        let last = s.tree().nodes[7].rect;
        assert!(last.bottom() <= 12, "end shows the last card: {last:?}");
    }

    #[test]
    fn motion_under_a_virtual_clock() {
        let json = r#"{"children":[{"type":"card","content":"hello","props":{"animate":"slide-up","id":"c"}}]}"#;
        let mut a = session(json);
        let f0 = a.frame(40, 10);
        let y_final = a.tree().get("c").unwrap().rect.y as u16;
        let row_text = |f: &Frame, y: u16| {
            crate::view::buffer_to_text(&f.cells)
                .lines()
                .nth(usize::from(y))
                .unwrap_or("")
                .to_string()
        };
        assert!(
            !row_text(&f0, y_final).contains('╭'),
            "still sliding in at t=0"
        );
        assert!(a.next_wake().is_some());
        a.advance(Duration::from_millis(400));
        let f1 = a.frame(40, 10);
        assert!(row_text(&f1, y_final).contains('╭'));
        // Same clock → same pixels.
        let mut b = session(json);
        b.frame(40, 10);
        b.advance(Duration::from_millis(400));
        assert_eq!(b.frame(40, 10).cells, f1.cells);
        assert_eq!(a.next_wake(), None, "idle once settled");
    }

    #[test]
    fn live_canvas_asks_for_frames() {
        let img = mem_image(10, 10);
        let mut s = session(&format!(
            r#"{{"children":[{{"type":"image","content":"{img}","events":["frame"],"props":{{"id":"cv","fps":10}}}}]}}"#
        ));
        s.frame(40, 10);
        let ev = s.take_events();
        assert_eq!(ids(&ev), [("cv".into(), "frame".into())]);
        s.advance(Duration::from_millis(500));
        assert!(
            s.take_events().is_empty(),
            "waits until the picture arrives"
        );
        let img = crate::gfx::Image::from_pixmap("x", tiny_skia::Pixmap::new(4, 4).unwrap());
        s.set_image("#cv", img).unwrap();
        s.advance(Duration::from_millis(100));
        assert_eq!(s.take_events().len(), 1);
    }

    #[test]
    fn rtl_app_mirrors_and_toasts() {
        let mut s = session(
            r#"{"children":[{"type":"bar","content":"جاهز"},{"type":"stage","props":{"direction":"horizontal"},"children":[{"type":"button","content":"أولاً","props":{"id":"a"}},{"type":"button","content":"ثانياً","props":{"id":"b"}}]}]}"#,
        );
        s.notify("تم الحفظ", Some("check"), None, Duration::from_secs(2))
            .unwrap();
        s.settle(60, 12);
        let (a, b) = (
            s.tree().get("a").unwrap().rect,
            s.tree().get("b").unwrap().rect,
        );
        assert!(a.x > b.x, "first button on the right");
        assert!(s.tree_json(60, 12)["toasts"][0] == "تم الحفظ");
        s.advance(Duration::from_secs(3));
        assert!(s.tree_json(60, 12).get("toasts").is_none(), "toasts expire");
    }

    #[test]
    fn focus_follows_the_tab_page() {
        let mut s = session(
            r#"{"children":[{"type":"tabs","children":[
                {"type":"stage","props":{"title":"A"},"children":[{"type":"input","props":{"id":"a"}}]},
                {"type":"stage","props":{"title":"B"},"children":[{"type":"input","props":{"id":"b"}}]}]}]}"#,
        );
        s.frame(60, 16);
        assert_eq!(s.focused().unwrap().1, "a");
        s.press("ctrl+pagedown").unwrap();
        s.frame(60, 16);
        assert_eq!(s.focused().unwrap().1, "b");
        s.type_text("مرحبا");
        assert_eq!(s.value("#b").unwrap(), json!("مرحبا"));
    }

    fn pane_type(s: &Session, id: &str) -> String {
        s.query(&format!("#{id}")).unwrap()["pane_type"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn panes_switch_type_and_restore_stashed_content() {
        let mut s = session(
            r##"{"children":[{"type":"split","children":[
            {"type":"pane","content":"markdown","props":{"id":"a"},"children":[{"type":"markdown","content":"# Notes","props":{"id":"notes"}}]},
            {"type":"pane","content":"log","props":{"id":"b"},"children":[{"type":"log","props":{"id":"logs"}}]}]}]}"##,
        );
        s.frame(100, 30);
        // Open the menu by clicking pane a's title, choose "Chat".
        let a = s.tree().get("a").unwrap().rect;
        s.click_at(a.x as u16 + 2, a.y as u16);
        s.frame(100, 30);
        let menu = s.menu.clone().expect("menu open");
        let chat = menu.items.iter().position(|i| i.key == "chat").unwrap();
        for _ in 0..chat.abs_diff(menu.hover) {
            s.press(if chat > menu.hover { "down" } else { "up" })
                .unwrap();
        }
        s.press("enter").unwrap();
        s.frame(100, 30);
        assert_eq!(pane_type(&s, "a"), "chat");
        assert!(
            !s.query("#notes").unwrap()["on_screen"].as_bool().unwrap(),
            "markdown is stashed"
        );
        let ev = s.take_events();
        assert!(
            ev.iter()
                .any(|e| e.name == "pane" && e.value["type"] == "chat")
        );
        // Switching back restores the same markdown widget.
        let a_idx = s.tree().find("#a").unwrap();
        s.switch_pane(a_idx, "markdown");
        s.frame(100, 30);
        assert!(s.query("#notes").unwrap()["on_screen"].as_bool().unwrap());
        assert_eq!(s.query("#a").unwrap()["stashed"], json!(["chat"]));
    }

    #[test]
    fn prefix_keys_split_and_close_panes() {
        let mut s = session(
            r#"{"children":[{"type":"pane","content":"empty","props":{"id":"p"},"children":[{"type":"text","content":"x"}]}]}"#,
        );
        s.frame(80, 24);
        s.focus("#p").unwrap();
        s.press("ctrl+space").unwrap();
        assert!(s.prefix_armed());
        s.press("|").unwrap();
        s.frame(80, 24);
        let panes = |s: &Session| {
            s.tree()
                .preorder()
                .into_iter()
                .filter(|i| s.tree().is_typed_pane(*i))
                .count()
        };
        assert_eq!(panes(&s), 2);
        // The bar shows pane keys while the prefix is armed.
        s.press("ctrl+space").unwrap();
        assert!(s.hints().iter().any(|(_, d)| d == "Close pane"));
        s.press("x").unwrap();
        s.frame(80, 24);
        assert_eq!(panes(&s), 1);
        assert!(
            !s.quit_requested(),
            "closing one of two panes keeps the app"
        );
        s.press("ctrl+space").unwrap();
        s.press("x").unwrap();
        assert_eq!(panes(&s), 1, "the last pane stays on screen…");
        assert!(s.quit_requested(), "…and the app closes");
    }

    #[test]
    fn the_last_panes_close_button_closes_the_app() {
        let mut s = session(
            r#"{"children":[{"type":"split","children":[
            {"type":"pane","content":"markdown","props":{"id":"a"},"children":[{"type":"markdown","content":"a"}]},
            {"type":"pane","content":"markdown","props":{"id":"b"},"children":[{"type":"markdown","content":"b"}]}]}]}"#,
        );
        let close = |s: &mut Session, id: &str| {
            let i = s.tree().find(&format!("#{id}")).unwrap();
            let x = s.pane_header(i).1[3]; // ⊞ ⊟ ↗ ×
            s.click_at(x.x + 1, x.y);
            s.frame(80, 24);
        };
        s.frame(80, 24);
        close(&mut s, "a");
        assert!(s.tree().get("a").is_none(), "pane a closed");
        assert!(!s.quit_requested());
        close(&mut s, "b");
        assert!(s.quit_requested(), "× on the last pane closes the app");
    }

    const TWO_PANES: &str = r#""children":[{"type":"split","children":[
        {"type":"pane","content":"markdown","props":{"id":"a"},"children":[{"type":"markdown","content":"مَرْحَبًا Hello 日本語 नमस्ते"}]},
        {"type":"pane","content":"markdown","props":{"id":"b"},"children":[{"type":"markdown","content":"b"}]}]}]"#;

    #[test]
    fn keys_can_be_renamed_and_turned_off() {
        let mut s = session(&format!(
            r#"{{"keys":{{"quit":"ctrl+q","prefix":"ctrl+a","close_pane":"w","pane_menu":null}},{TWO_PANES}}}"#
        ));
        s.frame(80, 24);
        s.press("q").unwrap();
        s.press("esc").unwrap();
        s.press("ctrl+c").unwrap();
        assert!(!s.quit_requested(), "the old quit keys are free now");
        assert!(s.hints().iter().any(|(k, d)| k == "Ctrl+Q" && d == "Quit"));
        s.focus("#a").unwrap();
        s.press("ctrl+space").unwrap();
        assert!(!s.prefix_armed(), "Ctrl+Space isn't the prefix any more");
        s.press("ctrl+a").unwrap();
        let hints = s.hints();
        assert!(
            hints.iter().any(|(k, d)| k == "w" && d == "Close pane"),
            "{hints:?}"
        );
        assert!(
            !hints.iter().any(|(_, d)| d == "Pane type"),
            "turned off: not shown"
        );
        s.press("x").unwrap();
        s.frame(80, 24);
        assert!(s.tree().get("a").is_some(), "x does nothing now");
        s.press("ctrl+a").unwrap();
        s.press("space").unwrap();
        assert!(s.menu.is_none(), "the pane menu key is off");
        s.press("ctrl+a").unwrap();
        s.press("w").unwrap();
        s.frame(80, 24);
        assert!(s.tree().get("a").is_none(), "w closes the pane");
        s.press("ctrl+a").unwrap();
        s.press("ctrl+q").unwrap();
        assert!(
            s.quit_requested(),
            "the quit key works after the prefix too"
        );
    }

    #[test]
    fn keys_that_would_lock_people_in_or_clash_are_refused() {
        let err = |keys: &str, bindings: &str| {
            Session::from_json(&format!(
                r#"{{"keys":{keys},"bindings":{bindings},{TWO_PANES}}}"#
            ))
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default()
        };
        assert!(err(r#"{"quit":null}"#, "[]").contains("way to quit"));
        assert!(err(r#"{"prefix":null}"#, "[]").contains("prefix key can't be turned off"));
        assert!(err(r#"{"focus_next":"q"}"#, "[]").contains("two jobs"));
        assert!(err(r#"{"quitt":"q"}"#, "[]").contains("no key called"));
        assert!(
            err("{}", r#"[{"key":"ctrl+space","action":"save"}]"#).contains("is the prefix key")
        );
        // An app's own shortcut may take a built-in key on purpose.
        assert!(err("{}", r#"[{"key":"q","action":"save"}]"#).is_empty());
    }

    #[test]
    fn an_app_can_ask_before_quitting() {
        let mut s = session(&format!(r#"{{"confirm_quit":true,{TWO_PANES}}}"#));
        s.frame(80, 24);
        s.press("q").unwrap();
        assert_eq!(s.dialog_open(), Some("quit"));
        assert!(!s.quit_requested());
        s.press("esc").unwrap();
        assert_eq!(s.dialog_open(), None, "Esc says no");
        assert!(!s.quit_requested());
        s.press("ctrl+c").unwrap();
        assert_eq!(s.dialog_open(), Some("quit"));
        s.press("ctrl+c").unwrap();
        assert!(s.quit_requested(), "a second Ctrl+C says yes");
        // The app quitting itself (app.exit()) doesn't ask.
        let mut s = session(&format!(r#"{{"confirm_quit":true,{TWO_PANES}}}"#));
        s.request_quit();
        assert!(s.quit_requested());
    }

    #[test]
    fn split_then_close_puts_the_sizes_back() {
        let mut s = session(&format!("{{{TWO_PANES}}}"));
        s.frame(100, 24);
        let ratios = |s: &Session| match &s.tree().nodes[s.tree().find("#a").unwrap()]
            .parent
            .map(|p| s.tree().nodes[p].kind.clone())
        {
            Some(Kind::Split(sb)) => sb.ratios.clone(),
            _ => vec![],
        };
        s.focus("#a").unwrap();
        s.press("ctrl+space").unwrap();
        s.press("|").unwrap();
        s.frame(100, 24);
        assert_eq!(ratios(&s).len(), 3);
        // Close the new pane (the focus moved into it).
        s.press("ctrl+space").unwrap();
        s.press("x").unwrap();
        s.frame(100, 24);
        let r = ratios(&s);
        assert!(
            (r[0] - 0.5).abs() < 1e-3 && (r[1] - 0.5).abs() < 1e-3,
            "{r:?}"
        );
    }

    #[test]
    fn a_workspace_tab_keeps_its_name_when_panes_close() {
        let mut s = session(
            r#"{"children":[{"type":"workspaces","children":[
            {"type":"split","props":{"title":"Layout"},"children":[
                {"type":"pane","content":"markdown","props":{"id":"a"},"children":[{"type":"markdown","content":"a"}]},
                {"type":"pane","content":"markdown","props":{"id":"b"},"children":[{"type":"markdown","content":"b"}]}]},
            {"type":"pane","content":"markdown","props":{"id":"c","title":"Notes"},"children":[{"type":"markdown","content":"c"}]}]}]}"#,
        );
        s.frame(100, 24);
        assert_eq!(s.workspace_name(), "Layout");
        s.focus("#a").unwrap();
        s.press("ctrl+space").unwrap();
        s.press("x").unwrap();
        s.frame(100, 24);
        assert_eq!(
            s.workspace_name(),
            "Layout",
            "the pane left keeps the tab's name"
        );
    }

    #[test]
    fn the_pane_menu_stays_on_screen_and_scrolls() {
        let mut s = session(&format!("{{{TWO_PANES}}}"));
        s.frame(60, 10);
        let b = s.tree().find("#b").unwrap();
        s.open_menu(b);
        let f = s.frame(60, 10);
        let rect = s.menu_rect().unwrap();
        assert!(rect.right() <= 60 && rect.bottom() <= 10, "{rect:?}");
        assert!(f.text().contains('▼'), "more below: says so");
        // Move to the last item: the menu scrolls to show it.
        let n = s.menu.as_ref().unwrap().items.len();
        s.menu.as_mut().unwrap().hover = n - 2;
        s.press("down").unwrap();
        s.frame(60, 10);
        assert_eq!(s.menu.as_ref().unwrap().hover, n - 1);
        let m = s.menu.as_ref().unwrap();
        let last = m.items[m.hover].label.clone();
        let hit = (1..rect.height - 1).any(|row| s.menu_item_at(row) == Some(m.hover));
        assert!(hit, "the highlighted {last:?} is on screen");
        assert!(s.frame(60, 10).text().contains('▲'), "more above: says so");
    }

    #[test]
    fn the_bar_offers_a_quit_key_that_works() {
        let mut s = session(
            r#"{"children":[{"type":"input","props":{"id":"i"}},{"type":"button","content":"ok"}]}"#,
        );
        s.frame(80, 20);
        s.focus("#i").unwrap();
        let quit = |s: &Session| {
            s.hints()
                .into_iter()
                .find(|(_, d)| d == "Quit")
                .map(|(k, _)| k)
        };
        assert_eq!(
            quit(&s).as_deref(),
            Some("Ctrl+C"),
            "q would type into the box"
        );
        s.press("esc").unwrap();
        assert_eq!(quit(&s).as_deref(), Some("q"));
    }

    #[test]
    fn long_dialog_lines_wrap_instead_of_being_cut() {
        let mut s = session(&format!("{{{TWO_PANES}}}"));
        s.frame(70, 24);
        let long = "مَرْحَبًا — this add-on plays video files and links from the web, with sound, \
                    日本語 नमस्ते";
        s.ask(
            Ask::Quit,
            "Check".into(),
            vec![long.into()],
            &["OK", "Cancel"],
            "Esc",
        );
        let text = s.frame(70, 24).text();
        assert!(text.contains("with sound"), "{text}");
        assert!(text.contains("नमस्ते"), "the end of the line is there too");
    }

    #[test]
    fn hidden_panes_are_not_shown() {
        let s = session(
            r##"{"children":[{"type":"tabs","children":[
            {"type":"pane","content":"markdown","props":{"id":"a","title":"One"},"children":[
                {"type":"markdown","content":"# Notes","props":{"id":"notes"}},
                {"type":"log","props":{"id":"logs"}}]},
            {"type":"card","content":"two","props":{"id":"two","title":"Two"}}]}]}"##,
        );
        let t = s.tree();
        let shown = |id: &str| t.shown(t.find(&format!("#{id}")).unwrap());
        assert!(
            shown("a") && shown("notes"),
            "the active tab and the pane's type"
        );
        assert!(!shown("logs"), "stashed under another pane type");
        assert!(!shown("two"), "a tab you're not looking at");
    }

    #[cfg(unix)]
    #[test]
    fn terminal_panes_get_every_key_but_the_prefix() {
        let mut s = session(
            r#"{"children":[{"type":"terminal","content":["cat"],"props":{"id":"t"}},{"type":"button","content":"b","props":{"id":"b"}}]}"#,
        );
        s.frame(60, 24);
        s.focus("#t").unwrap();
        s.type_text("q");
        s.press("ctrl+c").unwrap();
        assert!(!s.quit_requested(), "q and Ctrl+C go to the program");
        assert!(
            s.hints()
                .iter()
                .any(|(k, d)| k == "Ctrl+Space Tab" && d == "Leave the shell"),
            "the bar says how to leave"
        );
        s.press("ctrl+space").unwrap();
        s.press("tab").unwrap();
        assert_eq!(
            s.focused().unwrap().1,
            "b",
            "prefix + Tab leaves the terminal"
        );
    }

    #[test]
    fn copy_reaches_the_clipboard() {
        let mut s = session(
            r#"{"children":[{"type":"chat","content":[{"role":"assistant","text":"copy me"}],"props":{"id":"c"}}]}"#,
        );
        s.frame(60, 20);
        s.focus("#c").unwrap();
        s.press("up").unwrap();
        s.press("y").unwrap();
        assert_eq!(s.take_clipboard().as_deref(), Some("copy me"));
        assert!(crate::app::osc52("hi").starts_with("\x1b]52;c;aGk="));
    }

    #[test]
    fn split_and_close_panes_at_runtime() {
        let mut s =
            session(r#"{"children":[{"type":"card","content":"main","props":{"id":"main"}}]}"#);
        s.frame(80, 20);
        let id = s
            .split("#main", &NodeSpec::text("card", "side"), "horizontal")
            .unwrap();
        s.frame(80, 20);
        assert_eq!(s.focused().unwrap().1, id, "the new pane is focused");
        assert_eq!(s.focused_pane().unwrap().1, id);
        s.remove(&format!("#{id}")).unwrap();
        s.frame(80, 20);
        assert_eq!(s.tree().roots.len(), 1);
        assert!(
            s.split("#main", &NodeSpec::text("card", "x"), "sideways")
                .is_err()
        );
    }
}
