//! A Session owns app state: the widget tree, focus, scrolling, animations and
//! the events waiting for the app. Views render it; input changes it.
//!
//! Everything here is deterministic given the inputs and the clock, so headless
//! runs (virtual clock) and real terminals behave the same.

use std::collections::HashMap;
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
    frames: HashMap<usize, FrameClock>,
    modal_return: Vec<(usize, Option<usize>)>,
    autofocused: bool,
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
            frames: HashMap::new(),
            modal_return: Vec::new(),
            autofocused: false,
        })
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
        self.dirty || self.next_wake().is_some_and(|w| w.is_zero())
    }

    /// Mark the screen as needing a redraw.
    pub fn touch(&mut self) {
        self.dirty = true;
    }

    /// The user asked to quit (q, Esc, Ctrl+C or a `quit` action).
    pub fn quit_requested(&self) -> bool {
        self.quit
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
                self.focus = self
                    .focusables()
                    .into_iter()
                    .find(|i| self.tree.nodes[*i].widget().is_some_and(Widget::takes_text));
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

    fn hints(&self) -> Vec<(String, String)> {
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
        for &m in &self.laid.modals {
            self.paint_modal(m, &mut frame, &hints);
        }
        self.paint_toasts(&mut frame);
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
                let bg = crate::widgets::behind(target, area.x, area.y, theme.bg);
                if area.height > 1 {
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
                    if on && area.height > 1 {
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
    /// single-card renders (`termstage render "text"`, `ts.show("text")`).
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
                    && n.widget().is_some_and(Widget::focusable)
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
        }
    }

    fn key(&mut self, k: Key) {
        if k.mods.ctrl && k.code == Code::Char('c') {
            self.request_quit();
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
                        self.request_quit();
                    }
                }
            }
            Code::Char('q') if plain => self.request_quit(),
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

    /// Run an action by name: built-ins here, the rest go to the app.
    pub fn action(&mut self, name: &str) {
        match name {
            "quit" => self.request_quit(),
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
            if self.tree.nodes[i].widget().is_some_and(Widget::focusable) {
                return Some(i);
            }
            i = self.tree.nodes[i].parent?;
        }
    }

    fn mouse(&mut self, m: Mouse) {
        let (x, y) = (m.x, m.y);
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
                let (hover, divider) = match self.hit(x, y) {
                    Some(Hit::Node(i)) => (self.focus_target(i), None),
                    Some(Hit::Divider(d)) => (None, Some(d)),
                    _ => (None, None),
                };
                if hover != self.hover || divider != self.hover_divider {
                    self.hover = hover;
                    self.hover_divider = divider;
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
        if !self.tree.nodes[i].widget().is_some_and(Widget::focusable) {
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
