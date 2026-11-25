//! Built-in widgets.
//!
//! Every widget answers the same questions: how big do you want to be
//! ([`Widget::measure`]), draw yourself ([`Widget::paint`]), describe yourself
//! for agents ([`Widget::describe`]) and — for interactive ones — what happens
//! on a key, click or paste.

mod bar;
mod button;
mod card;
mod checkbox;
mod icons;
mod image_view;
mod input;
mod progress;
mod select;
mod spinner;
mod table;
mod text_block;
mod tree_view;

use std::time::Duration;

pub use bar::Bar;
pub use button::{Button, Variant};
pub use card::Card;
pub use checkbox::Checkbox;
pub use icons::{ICONS, icon, mirror};
pub use image_view::ImageView;
pub use input::Input;
pub use progress::Progress;
pub use select::Select;
pub use spinner::Spinner;
pub use table::Table;
pub use text_block::TextBlock;
pub use tree_view::{TreeItem, TreeView};

use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use serde_json::{Map, Value};

use crate::frame::Frame;
use crate::input::Key;
use crate::style::{Rgb, Theme};

/// Facts about the output that affect layout.
#[derive(Debug, Clone, Copy)]
pub struct Cx {
    /// Size of one terminal cell in pixels.
    pub cell_px: (u32, u32),
}

impl Default for Cx {
    fn default() -> Self {
        Self {
            cell_px: crate::caps::DEFAULT_CELL_PX,
        }
    }
}

/// Everything a widget needs to know while drawing.
#[derive(Debug, Clone, Copy)]
pub struct PaintCx<'a> {
    pub theme: &'a Theme,
    pub cell_px: (u32, u32),
    pub focused: bool,
    pub hovered: bool,
    /// Animation clock.
    pub now: Duration,
    /// Extra border brightness from `pulse` / `pop`, 0..1.
    pub glow: f32,
    /// The layout around this widget is right-to-left.
    pub rtl: bool,
    /// The widget is a pane of a split or tabs: fill the whole area.
    pub pane: bool,
    /// Shortcut hints `(key, description)` for the status bar.
    pub hints: &'a [(String, String)],
}

impl PaintCx<'_> {
    /// Border color for boxed widgets: accent when focused, brighter while glowing.
    pub fn border(&self) -> Rgb {
        let t = self.theme;
        let base = if self.focused {
            t.accent
        } else if self.hovered {
            t.accent.mix(t.border, 0.2)
        } else {
            t.accent.mix(t.border, 0.55)
        };
        base.mix(t.fg, self.glow * 0.7)
    }
}

/// What a widget did with an input.
#[derive(Debug, Clone, PartialEq)]
pub enum Reaction {
    /// Not for me: let the app handle it.
    Ignored,
    /// Handled; redraw.
    Redraw,
    /// Handled; redraw and tell the app (`on_<name>` in Python).
    Emit(&'static str, Value),
}

/// What every widget can do.
pub trait Widget {
    /// Preferred `(width, height)` in cells given a maximum width.
    fn measure(&self, max_width: u16, cx: &Cx) -> (u16, u16);
    /// Draw into `frame` inside `area`.
    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx);
    /// Widget-specific fields for the agent-facing JSON tree.
    fn describe(&self) -> Map<String, Value>;
    /// Can it take keyboard focus?
    fn focusable(&self) -> bool {
        false
    }
    /// Does it take typed text (so `q` types instead of quitting)?
    fn takes_text(&self) -> bool {
        false
    }
    fn key(&mut self, _key: &Key, _area: Rect) -> Reaction {
        Reaction::Ignored
    }
    /// A click at `(x, y)` relative to the widget's top-left corner.
    fn click(&mut self, _x: u16, _y: u16, _area: Rect) -> Reaction {
        Reaction::Ignored
    }
    fn paste(&mut self, _text: &str) -> Reaction {
        Reaction::Ignored
    }
    /// Scroll its own content by `rows`; returns whether anything moved.
    fn scroll_by(&mut self, _rows: i32, _area: Rect) -> bool {
        false
    }
    /// Needs frames even when nothing else changes (spinners).
    fn animating(&self) -> bool {
        false
    }
    /// The current value (`widget.value` in Python).
    fn value(&self) -> Value {
        Value::Null
    }
    /// An open pop-up (a dropdown list) drawn above everything.
    fn popup(&self, _area: Rect, _screen: Rect) -> Option<Rect> {
        None
    }
    fn paint_popup(&self, _popup: Rect, _frame: &mut Frame, _p: &PaintCx) {}
    fn click_popup(&mut self, _x: u16, _y: u16, _popup: Rect) -> Reaction {
        Reaction::Ignored
    }
    fn close_popup(&mut self) {}
}

pub(crate) fn object(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// Fill `area` with spaces in `bg`.
pub(crate) fn fill(frame: &mut Frame, area: Rect, fg: Rgb, bg: Rgb) {
    let area = area.intersection(frame.area());
    frame
        .cells
        .set_style(area, Style::new().fg(fg.color()).bg(bg.color()));
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            frame.cells[(x, y)].set_symbol(" ");
        }
    }
}

/// The background color already painted at `(x, y)` (what's behind a widget).
pub(crate) fn behind(frame: &Frame, x: u16, y: u16, fallback: Rgb) -> Rgb {
    match frame.cells.cell((x, y)).map(|c| c.bg) {
        Some(ratatui_core::style::Color::Rgb(r, g, b)) => Rgb(r, g, b),
        _ => fallback,
    }
}

/// Draw a box border. `title` (already laid out) goes on the top edge,
/// on the right in RTL.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_box(
    frame: &mut Frame,
    area: Rect,
    rounded: bool,
    color: Rgb,
    outside: Rgb,
    title: Option<&str>,
    title_color: Rgb,
    rtl: bool,
) {
    let area = area.intersection(frame.area());
    if area.width < 2 || area.height < 2 {
        return;
    }
    let style = Style::new().fg(color.color()).bg(outside.color());
    let (tl, tr, bl, br) = if rounded {
        ("╭", "╮", "╰", "╯")
    } else {
        ("┌", "┐", "└", "┘")
    };
    let (l, r, t, b) = (area.left(), area.right() - 1, area.top(), area.bottom() - 1);
    for x in l + 1..r {
        frame.cells[(x, t)].set_symbol("─").set_style(style);
        frame.cells[(x, b)].set_symbol("─").set_style(style);
    }
    for y in t + 1..b {
        frame.cells[(l, y)].set_symbol("│").set_style(style);
        frame.cells[(r, y)].set_symbol("│").set_style(style);
    }
    frame.cells[(l, t)].set_symbol(tl).set_style(style);
    frame.cells[(r, t)].set_symbol(tr).set_style(style);
    frame.cells[(l, b)].set_symbol(bl).set_style(style);
    frame.cells[(r, b)].set_symbol(br).set_style(style);

    if let Some(h) = title {
        let room = area.width.saturating_sub(6);
        if room == 0 {
            return;
        }
        let dir = if rtl {
            crate::text::Direction::Rtl
        } else {
            crate::text::Direction::Ltr
        };
        let line = crate::text::layout(h, usize::from(room), dir).swap_remove(0);
        let hw = (line.width as u16).min(room);
        let x = if rtl { r.saturating_sub(2 + hw) } else { l + 3 };
        frame.cells[(x - 1, t)].set_symbol(" ").set_style(style);
        if x + hw < r {
            frame.cells[(x + hw, t)].set_symbol(" ").set_style(style);
        }
        frame.put_line(x, t, room, &line, title_color, outside, true);
    }
}

/// A vertical scrollbar along `x` for rows `top..top+height`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn scrollbar(
    frame: &mut Frame,
    x: u16,
    top: u16,
    height: u16,
    offset: usize,
    total: usize,
    track: Rgb,
    thumb: Rgb,
) {
    if height == 0 || total <= usize::from(height) {
        return;
    }
    let h = usize::from(height);
    let size = (h * h / total).clamp(1, h);
    let max_off = total - h;
    let pos = ((h - size) * offset.min(max_off))
        .checked_div(max_off)
        .unwrap_or(0);
    for i in 0..h {
        let y = top + i as u16;
        if let Some(c) = frame.cells.cell_mut((x, y)) {
            let on = i >= pos && i < pos + size;
            c.set_symbol(if on { "┃" } else { "│" });
            c.fg = if on { thumb.color() } else { track.color() };
        }
    }
}

/// Clamp a scroll offset.
pub(crate) fn clamp_scroll(offset: i64, total: usize, visible: usize) -> usize {
    offset.clamp(0, total.saturating_sub(visible) as i64) as usize
}
