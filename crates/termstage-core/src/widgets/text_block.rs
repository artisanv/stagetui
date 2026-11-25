//! `StageText`: plain wrapped text on the app background.

use ratatui_core::layout::Rect;
use serde_json::{Map, Value, json};

use super::{Cx, Paint, object};
use crate::frame::Frame;
use crate::style::Theme;
use crate::text::{self, Direction};

#[derive(Debug, Clone)]
pub struct TextBlock {
    pub content: String,
    pub dir: String,
}

impl TextBlock {
    fn base_dir(&self) -> Direction {
        if self.dir == "rtl" {
            Direction::Rtl
        } else {
            Direction::Ltr
        }
    }
}

impl Paint for TextBlock {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let natural = text::layout(&self.content, usize::MAX / 2, self.base_dir())
            .iter()
            .map(|l| l.width)
            .max()
            .unwrap_or(0);
        let w = natural.clamp(1, usize::from(max_width.max(1))) as u16;
        let h = text::layout(&self.content, usize::from(w), self.base_dir()).len();
        (w, h.min(usize::from(u16::MAX)) as u16)
    }

    fn paint(&self, area: Rect, frame: &mut Frame, theme: &Theme, _cx: &Cx) {
        let area = area.intersection(frame.area());
        for (y, line) in (area.top()..area.bottom()).zip(text::layout(
            &self.content,
            usize::from(area.width.max(1)),
            self.base_dir(),
        )) {
            let lw = (line.width as u16).min(area.width);
            let x = if line.dir == Direction::Rtl {
                area.right() - lw
            } else {
                area.left()
            };
            frame.put_line(x, y, area.width, &line, theme.muted, theme.bg, false);
        }
    }

    fn describe(&self) -> Map<String, Value> {
        let rtl = text::resolve_dir(&self.dir, &self.content) == Direction::Rtl;
        object(json!({ "text": self.content, "dir": if rtl { "rtl" } else { "ltr" } }))
    }
}
