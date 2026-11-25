//! `StageButton`: a rounded pill you can click, or focus and press Enter.

use ratatui_core::layout::Rect;
use ratatui_core::style::{Modifier, Style};
use serde_json::{Map, Value, json};

use super::{Cx, PaintCx, Reaction, Widget, object};
use crate::frame::Frame;
use crate::input::{Code, Key};
use crate::text::{self, Direction};
use crate::{Error, Result};

/// Button looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// Quiet until focused or hovered.
    Secondary,
    /// Filled with the accent color: the main action.
    Primary,
    /// Filled with red: destructive actions.
    Danger,
}

impl Variant {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "secondary" => Self::Secondary,
            "primary" => Self::Primary,
            "danger" => Self::Danger,
            other => {
                return Err(Error::friendly(
                    format!("Unknown button variant \"{other}\""),
                    "buttons come in three looks",
                    "use variant=\"primary\", \"secondary\" or \"danger\"",
                ));
            }
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Secondary => "secondary",
            Self::Primary => "primary",
            Self::Danger => "danger",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Button {
    pub label: String,
    pub icon: Option<&'static str>,
    pub variant: Variant,
}

impl Button {
    fn text(&self) -> String {
        match self.icon {
            Some(i) if self.label.is_empty() => i.to_string(),
            Some(i) => format!("{i} {}", self.label),
            None => self.label.clone(),
        }
    }
}

impl Widget for Button {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let w = text::width(&self.text()) + 4;
        ((w as u16).min(max_width).max(3), 1)
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.width < 3 || area.height == 0 {
            return;
        }
        let t = p.theme;
        let danger = crate::style::accent("rose").unwrap_or(t.accent);
        let active = p.focused || p.hovered;
        let (fill, fg) = match self.variant {
            Variant::Secondary if active => (t.accent, t.bg),
            Variant::Secondary => (t.surface.mix(t.fg, 0.08), t.fg),
            Variant::Primary if active => (t.accent.mix(t.fg, 0.25), t.bg),
            Variant::Primary => (t.accent, t.bg),
            Variant::Danger if active => (danger.mix(t.fg, 0.25), t.bg),
            Variant::Danger => (danger, t.bg),
        };
        let fill = fill.mix(t.fg, p.glow * 0.4);
        let y = area.y + area.height / 2;
        let outside = super::behind(frame, area.x, y, t.bg);
        let cap = Style::new().fg(fill.color()).bg(outside.color());
        let mut body = Style::new().fg(fg.color()).bg(fill.color());
        if p.focused {
            body = body.add_modifier(Modifier::BOLD);
        }
        let (l, r) = (area.left(), area.right() - 1);
        frame.cells[(l, y)].set_symbol("▐").set_style(cap);
        frame.cells[(r, y)].set_symbol("▌").set_style(cap);
        for x in l + 1..r {
            frame.cells[(x, y)].set_symbol(" ").set_style(body);
        }
        let room = area.width.saturating_sub(4);
        let line =
            text::layout(&self.text(), usize::from(room.max(1)), Direction::Ltr).swap_remove(0);
        let lw = (line.width as u16).min(room);
        let x = l + 2 + (room - lw) / 2;
        frame.put_line(x, y, room, &line, fg, fill, p.focused);
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({ "label": self.label, "icon": self.icon, "variant": self.variant.name() }))
    }

    fn focusable(&self) -> bool {
        true
    }

    fn key(&mut self, key: &Key, _area: Rect) -> Reaction {
        if key.is(Code::Enter) || key.is(Code::Char(' ')) {
            Reaction::Emit("click", Value::Null)
        } else {
            Reaction::Ignored
        }
    }

    fn click(&mut self, _x: u16, _y: u16, _area: Rect) -> Reaction {
        Reaction::Emit("click", Value::Null)
    }
}
