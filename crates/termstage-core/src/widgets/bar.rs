//! `StageBar`: a status line with the app's keyboard shortcuts.

use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use serde_json::{Map, Value, json};

use super::{Cx, PaintCx, Widget, fill, object};
use crate::frame::Frame;
use crate::text::{self, Direction};

#[derive(Debug, Clone)]
pub struct Bar {
    /// Status text shown at the start of the bar.
    pub text: String,
    /// Show shortcut hints.
    pub hints: bool,
}

impl Widget for Bar {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        (max_width, 1)
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.is_empty() {
            return;
        }
        let t = p.theme;
        let bg = t.surface;
        let y = area.y;
        fill(frame, Rect::new(area.x, y, area.width, 1), t.fg, bg);
        let rtl = p.rtl;
        let mut used = 0u16;
        if !self.text.is_empty() {
            let room = area.width.saturating_sub(2);
            let line =
                text::layout(&self.text, usize::from(room.max(1)), Direction::Ltr).swap_remove(0);
            let lw = (line.width as u16).min(room);
            let x = if rtl {
                area.right() - 1 - lw
            } else {
                area.x + 1
            };
            frame.put_line(x, y, room, &line, t.fg, bg, false);
            used = lw + 2;
        }
        if !self.hints {
            return;
        }
        // Hints from the far end: " key  description "; stop when out of room.
        let chip = Style::new()
            .fg(t.bg.color())
            .bg(t.accent.mix(t.surface, 0.15).color());
        let desc = Style::new()
            .fg(t.muted.mix(t.fg, 0.35).color())
            .bg(bg.color());
        let mut pieces: Vec<(String, String)> = Vec::new();
        let mut width = 0u16;
        for (key, d) in p.hints {
            let w = text::width(key) as u16 + 2 + text::width(d) as u16 + 3;
            if used + width + w > area.width {
                break;
            }
            width += w;
            pieces.push((key.clone(), d.clone()));
        }
        let mut x = if rtl {
            area.x + 1
        } else {
            area.right().saturating_sub(width + 1)
        };
        for (key, d) in pieces {
            let k = format!(" {key} ");
            x += frame.put_str(x, y, &k, text::width(&k) as u16, chip);
            let dd = format!(" {d}  ");
            x += frame.put_str(x, y, &dd, text::width(&dd) as u16, desc);
        }
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({ "text": self.text }))
    }
}
