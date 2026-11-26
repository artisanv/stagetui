//! `StageSpinner`: "working on it…" with a gentle braille spinner.

use ratatui_core::layout::Rect;
use ratatui_core::style::{Modifier, Style};
use serde_json::{Map, Value, json};

use super::{Cx, PaintCx, Widget, object};
use crate::frame::Frame;
use crate::text::{self, Direction};

const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// Milliseconds per spinner frame.
pub const FRAME_MS: u128 = 80;

#[derive(Debug, Clone)]
pub struct Spinner {
    pub label: String,
}

impl Widget for Spinner {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let w = if self.label.is_empty() {
            1
        } else {
            text::width(&self.label) + 2
        };
        ((w as u16).min(max_width).max(1), 1)
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.is_empty() {
            return;
        }
        let t = p.theme;
        let bg = super::behind(frame, area.x, area.y, t.bg);
        let glyph = FRAMES[(p.now.as_millis() / FRAME_MS) as usize % FRAMES.len()];
        let rtl = p.rtl || text::first_strong(&self.label) == Direction::Rtl;
        let sx = if rtl { area.right() - 1 } else { area.x };
        frame.cells[(sx, area.y)].set_symbol(glyph).set_style(
            Style::new()
                .fg(t.accent.color())
                .bg(bg.color())
                .add_modifier(Modifier::BOLD),
        );
        let room = area.width.saturating_sub(2);
        if room == 0 || self.label.is_empty() {
            return;
        }
        let line = text::layout(&self.label, usize::from(room), Direction::Ltr).swap_remove(0);
        let lw = (line.width as u16).min(room);
        let x = if rtl { sx - 1 - lw } else { area.x + 2 };
        frame.put_line(x, area.y, room, &line, t.fg, bg, false);
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({ "label": self.label }))
    }

    fn animating(&self) -> bool {
        true
    }
}
