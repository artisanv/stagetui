//! `StageCheckbox`: a labeled on/off box.

use ratatui_core::layout::Rect;
use serde_json::{Map, Value, json};

use super::{Cx, PaintCx, Reaction, Widget, object};
use crate::frame::Frame;
use crate::input::{Code, Key};
use crate::text::{self, Direction};

#[derive(Debug, Clone)]
pub struct Checkbox {
    pub label: String,
    pub checked: bool,
}

impl Checkbox {
    fn toggle(&mut self) -> Reaction {
        self.checked = !self.checked;
        Reaction::Emit("change", Value::Bool(self.checked))
    }
}

impl Widget for Checkbox {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        (
            (text::width(&self.label) as u16 + 4).min(max_width).max(3),
            1,
        )
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.width < 3 || area.height == 0 {
            return;
        }
        let t = p.theme;
        let y = area.y;
        let bg = super::behind(frame, area.x, y, t.bg);
        let rtl = p.rtl || text::first_strong(&self.label) == Direction::Rtl;
        let box_x = if rtl { area.right() - 3 } else { area.x };
        let edge = if p.focused || p.hovered {
            t.accent
        } else {
            t.muted
        };
        let style = |c: crate::style::Rgb| {
            ratatui_core::style::Style::new()
                .fg(c.color())
                .bg(bg.color())
        };
        frame.cells[(box_x, y)]
            .set_symbol("[")
            .set_style(style(edge));
        frame.cells[(box_x + 1, y)]
            .set_symbol(if self.checked { "✓" } else { " " })
            .set_style(style(t.accent).add_modifier(ratatui_core::style::Modifier::BOLD));
        frame.cells[(box_x + 2, y)]
            .set_symbol("]")
            .set_style(style(edge));
        let room = area.width.saturating_sub(4);
        if room == 0 {
            return;
        }
        let line = text::layout(&self.label, usize::from(room), Direction::Ltr).swap_remove(0);
        let lw = (line.width as u16).min(room);
        let x = if rtl { box_x - 1 - lw } else { area.x + 4 };
        let fg = if p.focused { t.accent } else { t.fg };
        frame.put_line(x, y, room, &line, fg, bg, p.focused);
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({ "label": self.label, "checked": self.checked }))
    }

    fn focusable(&self) -> bool {
        true
    }

    fn key(&mut self, key: &Key, _area: Rect) -> Reaction {
        if key.is(Code::Enter) || key.is(Code::Char(' ')) {
            self.toggle()
        } else {
            Reaction::Ignored
        }
    }

    fn click(&mut self, _x: u16, _y: u16, _area: Rect) -> Reaction {
        self.toggle()
    }

    fn value(&self) -> Value {
        Value::Bool(self.checked)
    }
}
