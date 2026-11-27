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
use crate::layout::{self, Laid, MARGIN};
use crate::motion::{self, Effect};
use crate::style::{self, Rgb};
use crate::text::{self, Direction};
use crate::tree::{AppSpec, IRect, Kind, NodeSpec, Tree};
use crate::widgets::{Card, Cx, PaintCx, Reaction, Widget, draw_box, fill, scrollbar};
use crate::workspace::Layout;

/// The buttons of the "Leave?" prompt.
const LEAVE_BUTTONS: [&str; 3] = ["Detach", "Quit app", "Cancel"];
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
    /// A pane header button: 0 split right, 1 split down, 2 close.
    PaneAction(usize, u8),
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
    /// The open "Leave?" prompt: the selected button (Detach, Quit app, Cancel).
    leave: Option<usize>,
    frames: HashMap<usize, FrameClock>,
    modal_return: Vec<(usize, Option<usize>)>,
    autofocused: bool,
    /// The prefix key (default Ctrl+Space) was pressed: the next key is for the app.
    prefix_armed: bool,
    prefix: Key,
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
        Ok(Self {
            tree: Tree::build(spec)?,
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
            leave: None,
            frames: HashMap::new(),
            modal_return: Vec::new(),
            autofocused: false,
            prefix_armed: false,
            prefix: Key::parse(spec.prefix.as_deref().unwrap_or("ctrl+space"))?,
            clipboard: None,
            menu: None,
            hover_pane_button: None,
        })
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
        self.leave.is_some()
    }

    /// Someone pressed q / Ctrl+C / Esc to leave. In an attached window, ask
    /// first: detach (the app keeps running) or quit the app.
    fn user_quit(&mut self) {
        if self.detachable && !self.quit {
            self.leave = Some(0);
            self.menu = None;
            self.dirty = true;
        } else {
            self.request_quit();
        }
    }

    fn leave_key(&mut self, k: Key) {
        let Some(sel) = self.leave else { return };
        let choice = match k.code {
            Code::Char('c') if k.mods.ctrl => Some(1),
            Code::Enter | Code::Char(' ') => Some(sel),
            Code::Char('d') => Some(0),
            Code::Char('q') => Some(1),
            Code::Esc | Code::Char('c') | Code::Char('n') => Some(2),
            Code::Right | Code::Down => {
                self.leave = Some((sel + 1) % 3);
                None
            }
            Code::Tab => {
                self.leave = Some(if k.mods.shift {
                    (sel + 2) % 3
                } else {
                    (sel + 1) % 3
                });
                None
            }
            Code::Left | Code::Up => {
                self.leave = Some((sel + 2) % 3);
                None
            }
            _ => None,
        };
        self.dirty = true;
        if let Some(c) = choice {
            self.leave_choose(c);
        }
    }

    fn leave_choose(&mut self, choice: usize) {
        self.leave = None;
        self.dirty = true;
        match choice {
            0 => self.detach = true,
            1 => self.request_quit(),
            _ => {}
        }
    }

    /// The prompt's box and its three buttons, centered on `screen`.
    fn leave_rects(&self, screen: Rect) -> Option<(Rect, [Rect; 3])> {
        self.leave?;
        let w = 50.min(screen.width);
        let h = 9.min(screen.height);
        let rect = Rect::new(
            screen.x + (screen.width - w) / 2,
            screen.y + (screen.height - h) / 2,
            w,
            h,
        );
        let widths = LEAVE_BUTTONS.map(|l| l.chars().count() as u16 + 4);
        let total: u16 = widths.iter().sum::<u16>() + 2 * 2;
        let mut x = rect.x + rect.width.saturating_sub(total) / 2;
        let y = (rect.y + 5).min(rect.bottom().saturating_sub(3));
        let mut buttons = [Rect::default(); 3];
        for (b, w) in buttons.iter_mut().zip(widths) {
            *b = Rect::new(x, y, w, 1).intersection(rect);
            x += w + 2;
        }
        Some((rect, buttons))
    }

    fn paint_leave(&self, frame: &mut Frame) {
        let Some(sel) = self.leave else { return };
        let Some((rect, buttons)) = self.leave_rects(frame.area()) else {
            return;
        };
        let t = &self.tree.theme;
        frame.dim_except(Rect::default(), t.bg, 0.55);
        frame.occlude(rect);
        fill(frame, rect, t.fg, t.surface);
        let title = match &self.session_name {
            Some(n) => format!("Leave \"{n}\"?"),
            None => "Leave this app?".to_string(),
        };
        draw_box(
            frame,
            rect,
            true,
            t.accent,
            t.bg,
            Some(&title),
            t.accent,
            false,
        );
        let inner = rect.width.saturating_sub(6);
        let text = Style::new().fg(t.fg.color()).bg(t.surface.color());
        let muted = Style::new().fg(t.muted.color()).bg(t.surface.color());
        frame.put_str(
            rect.x + 3,
            rect.y + 2,
            "Detach: it keeps running, come back later.",
            inner,
            text,
        );
        frame.put_str(
            rect.x + 3,
            rect.y + 3,
            "Quit: the app stops for every window.",
            inner,
            text,
        );
        for (k, (b, label)) in buttons.iter().zip(LEAVE_BUTTONS).enumerate() {
            let (fg, bg) = if k == sel {
                (t.bg, t.accent)
            } else {
                (t.fg, t.surface.mix(t.fg, 0.12))
            };
            fill(frame, *b, fg, bg);
            let style = Style::new().fg(fg.color()).bg(bg.color());
            frame.put_str(b.x + 2, b.y, label, b.width.saturating_sub(2), style);
        }
        let keys = "d detach · q quit · Esc cancel · ← → choose";
        frame.put_str(
            rect.x + 3,
            rect.bottom().saturating_sub(2),
            keys,
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
        {
            self.focus = None;
        }
        let frame = self.paint(cols, rows);
        self.dirty = false;
        self.tick();
        frame
    }

    /// Bounds, birth times and hit regions from the fresh layout.
    fn prepass(&mut self, cols: u16, rows: u16) {
        let screen = Rect::new(0, 0, cols, rows);
        self.hits.clear();
        for n in &mut self.tree.nodes {
            n.bounds = Rect::default();
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
        for n in &mut self.tree.nodes {
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
        let buttons = if w >= title_w + 10 {
            (0..3)
                .map(|k| Rect::new(x + w - 9 + 3 * k, y, 3, 1))
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
            let glyph = ["⊞", "⊟", "×"][k];
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
        let w = m
            .items
            .iter()
            .map(|i| text::width(&i.label))
            .max()
            .unwrap_or(10) as u16
            + 6;
        let h = m.items.len() as u16 + 3;
        let x = pane.rect.x.max(0) as u16;
        let below = (pane.rect.y + 1).max(0) as u16;
        let y = if below + h <= rows {
            below
        } else {
            rows.saturating_sub(h)
        };
        Some(Rect::new(x, y, w.min(cols), h).intersection(screen))
    }

    /// Menu row → item index (the separator row has none).
    fn menu_item_at(&self, row: u16) -> Option<usize> {
        let m = self.menu.as_ref()?;
        let sep = m.first_action();
        let r = usize::from(row.checked_sub(1)?);
        let idx = if r < sep {
            r
        } else if r == sep {
            return None;
        } else {
            r - 1
        };
        (idx < m.items.len()).then_some(idx)
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
        let sep = m.first_action();
        let mut y = rect.y + 1;
        for (k, item) in m.items.iter().enumerate() {
            if k == sep {
                let rule = Style::new().fg(t.border.color()).bg(t.surface.color());
                for x in rect.x + 1..rect.right().saturating_sub(1) {
                    frame.cells[(x, y)].set_symbol("─").set_style(rule);
                }
                y += 1;
            }
            if y >= rect.bottom().saturating_sub(1) {
                break;
            }
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
            y += 1;
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
        } else if let Some(spec) = crate::panes::builtin_content(name).filter(|_| !custom) {
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
        for (pane, kind) in &applied.switch {
            self.switch_pane(*pane, kind);
        }
        for (pane, kind, args) in &applied.fresh {
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
            let _ = self.notify(
                "This is the last pane",
                Some("info"),
                None,
                Duration::from_secs(2),
            );
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
        if self.leave.is_some() {
            return [
                ("Enter", "Choose"),
                ("d", "Detach"),
                ("q", "Quit app"),
                ("Esc", "Cancel"),
            ]
            .iter()
            .map(|(k, d)| ((*k).to_string(), (*d).to_string()))
            .collect();
        }
        if self.prefix_armed {
            return [
                ("Tab", "Next pane"),
                ("|", "Split →"),
                ("-", "Split ↓"),
                ("x", "Close pane"),
                ("Space", "Pane type"),
                ("d", "Detach"),
                ("1-9", "Workspace"),
                ("q", "Quit"),
            ]
            .iter()
            .filter(|(k, _)| *k != "1-9" || self.workspace_tabs().is_some())
            .map(|(k, d)| ((*k).to_string(), (*d).to_string()))
            .collect();
        }
        // A shell keeps every key: say how to get out.
        let in_shell = self.focus.is_some_and(|f| {
            self.tree.nodes[f]
                .widget()
                .is_some_and(Widget::captures_keys)
        });
        if in_shell {
            let prefix = self.prefix.label();
            return vec![
                (format!("{prefix} Tab"), "Leave the shell".into()),
                (format!("{prefix} q"), "Quit".into()),
            ];
        }
        let mut out: Vec<(String, String)> = self
            .tree
            .bindings
            .iter()
            .filter_map(|b| Some((b.key.label(), b.description.clone()?)))
            .collect();
        if !self.focusables().is_empty() {
            out.push(("Tab".into(), "Next".into()));
        }
        out.push(("q".into(), "Quit".into()));
        out
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
        self.paint_menu(&mut frame);
        for &m in &self.laid.modals {
            self.paint_modal(m, &mut frame, &hints);
        }
        self.paint_toasts(&mut frame);
        self.paint_leave(&mut frame);
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
            Kind::Pane(_) => self.paint_pane_header(i, target, area, off),
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
        self.tree
            .preorder()
            .into_iter()
            .filter(|i| {
                let n = &self.tree.nodes[*i];
                n.visible
                    && n.bounds.width > 0
                    && self.can_focus(*i)
                    && scope.is_none_or(|m| self.within(*i, m))
                    && (scope.is_some() || !self.in_modal(*i))
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
        if self.leave.is_some() {
            self.leave_key(k);
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
        if k == self.prefix {
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
        if k.mods.ctrl && k.code == Code::Char('c') {
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
        // 3. Built-in keys.
        let rtl = self.tree.dir == Direction::Rtl;
        let plain = k.mods == Default::default();
        match k.code {
            Code::Tab => self.move_focus(!k.mods.shift),
            Code::Esc if plain => {
                if !self.close_top_modal() {
                    if self.focus.is_some() {
                        self.set_focus(None);
                    } else {
                        self.user_quit();
                    }
                }
            }
            Code::Char('q') if plain => self.user_quit(),
            Code::PageDown | Code::PageUp if k.mods.ctrl => {
                self.switch_tab(k.code == Code::PageDown)
            }
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
        match k.code {
            Code::Tab => self.move_focus(!k.mods.shift),
            Code::Down => self.move_focus(true),
            Code::Up => self.move_focus(false),
            Code::Right => self.move_focus(!rtl),
            Code::Left => self.move_focus(rtl),
            Code::Char('|') | Code::Char('\\') => self.action("split_pane_right"),
            Code::Char('-') => self.action("split_pane_down"),
            Code::Char('x') => self.action("close_pane"),
            Code::Char('d') => self.action("detach"),
            Code::Char(c @ '1'..='9') => self.select_workspace(c as usize - '1' as usize),
            Code::Char(' ') | Code::Enter => self.action("pane_menu"),
            Code::Char('q') => self.user_quit(),
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
        if let Some((rect, buttons)) = self.leave_rects(Rect::new(0, 0, self.size.0, self.size.1)) {
            let at = buttons.iter().position(|b| b.contains((x, y).into()));
            match m.kind {
                MouseKind::Down(_) => match at {
                    Some(k) => self.leave_choose(k),
                    None if !rect.contains((x, y).into()) => self.leave_choose(2),
                    None => {}
                },
                MouseKind::Move => {
                    if let Some(k) = at
                        && self.leave != Some(k)
                    {
                        self.leave = Some(k);
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
                        _ => self.close_pane(p),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn session(json: &str) -> Session {
        Session::from_json(json).unwrap()
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
        let (_, buttons) = s.leave_rects(Rect::new(0, 0, 80, 24)).unwrap();
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
        s.press("ctrl+space").unwrap();
        s.press("x").unwrap();
        assert_eq!(panes(&s), 1, "the last pane stays");
        assert!(s.tree_json(80, 24)["toasts"][0] == "This is the last pane");
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
