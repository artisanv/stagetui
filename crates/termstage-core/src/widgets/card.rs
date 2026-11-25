//! `StageCard`: a titled, bordered surface with wrapped text.

use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use serde_json::{Map, Value, json};

use super::{Cx, Paint, object};
use crate::frame::Frame;
use crate::style::Theme;
use crate::text::{self, Direction};

#[derive(Debug, Clone)]
pub struct Card {
    pub content: String,
    pub title: Option<String>,
    pub icon: Option<&'static str>,
    pub rounded: bool,
    /// `(horizontal, vertical)` padding in cells.
    pub padding: (u16, u16),
    pub dir: String,
}

impl Card {
    fn direction(&self) -> Direction {
        text::resolve_dir(&self.dir, &self.content)
    }

    /// Title text in reading order (icon first).
    fn heading(&self) -> Option<String> {
        match (self.icon, self.title.as_deref()) {
            (Some(i), Some(t)) => Some(format!("{i} {t}")),
            (None, Some(t)) => Some(t.to_string()),
            (Some(i), None) => Some(i.to_string()),
            (None, None) => None,
        }
    }

    fn inner_width(&self, width: u16) -> usize {
        usize::from(width.saturating_sub(2 + 2 * self.padding.0)).max(1)
    }

    fn base_dir(&self) -> Direction {
        if self.dir == "rtl" {
            Direction::Rtl
        } else {
            Direction::Ltr
        }
    }
}

impl Paint for Card {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let natural = text::layout(&self.content, usize::MAX / 2, self.base_dir())
            .iter()
            .map(|l| l.width)
            .max()
            .unwrap_or(0);
        let heading = self.heading().map_or(0, |h| {
            text::layout(&h, usize::MAX / 2, Direction::Ltr)[0].width + 4
        });
        let chrome = 2 + 2 * usize::from(self.padding.0);
        let want = (natural + chrome).max(heading + 2).max(12);
        let w = want.min(usize::from(max_width)) as u16;
        let lines = text::layout(&self.content, self.inner_width(w), self.base_dir()).len();
        let h = lines + 2 + 2 * usize::from(self.padding.1);
        (w, h.min(usize::from(u16::MAX)) as u16)
    }

    fn paint(&self, area: Rect, frame: &mut Frame, theme: &Theme, _cx: &Cx) {
        let area = area.intersection(frame.area());
        if area.width < 2 || area.height < 2 {
            return;
        }
        let surface = Style::new().bg(theme.surface.color()).fg(theme.fg.color());
        // The frame sits on the app background so rounded corners don't show a square halo.
        let border = Style::new()
            .bg(theme.bg.color())
            .fg(theme.accent.mix(theme.border, 0.35).color());
        frame.cells.set_style(area, surface);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                frame.cells[(x, y)].set_symbol(" ");
            }
        }

        let (tl, tr, bl, br) = if self.rounded {
            ("╭", "╮", "╰", "╯")
        } else {
            ("┌", "┐", "└", "┘")
        };
        let (l, r, t, b) = (area.left(), area.right() - 1, area.top(), area.bottom() - 1);
        for x in l + 1..r {
            frame.cells[(x, t)].set_symbol("─").set_style(border);
            frame.cells[(x, b)].set_symbol("─").set_style(border);
        }
        for y in t + 1..b {
            frame.cells[(l, y)].set_symbol("│").set_style(border);
            frame.cells[(r, y)].set_symbol("│").set_style(border);
        }
        frame.cells[(l, t)].set_symbol(tl).set_style(border);
        frame.cells[(r, t)].set_symbol(tr).set_style(border);
        frame.cells[(l, b)].set_symbol(bl).set_style(border);
        frame.cells[(r, b)].set_symbol(br).set_style(border);

        let rtl = self.direction() == Direction::Rtl;
        if let Some(h) = self.heading() {
            let room = area.width.saturating_sub(6);
            let title_dir = if rtl { Direction::Rtl } else { Direction::Ltr };
            let line = text::layout(&h, usize::from(room.max(1)), title_dir).swap_remove(0);
            let hw = (line.width as u16).min(room);
            // RTL mirrors the title to the right edge.
            let x = if rtl { r.saturating_sub(2 + hw) } else { l + 3 };
            frame.cells[(x - 1, t)].set_symbol(" ").set_style(border);
            if x + hw < r {
                frame.cells[(x + hw, t)].set_symbol(" ").set_style(border);
            }
            frame.put_line(x, t, room, &line, theme.accent, theme.bg, true);
        }

        let inner_w = self.inner_width(area.width);
        let x0 = l + 1 + self.padding.0;
        let first_row = t + 1 + self.padding.1;
        for (y, line) in (first_row..b).zip(text::layout(&self.content, inner_w, self.base_dir())) {
            let lw = line.width.min(inner_w) as u16;
            let x = if line.dir == Direction::Rtl {
                x0 + inner_w as u16 - lw
            } else {
                x0
            };
            frame.put_line(x, y, inner_w as u16, &line, theme.fg, theme.surface, false);
        }
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({
            "title": self.title,
            "icon": self.icon,
            "text": self.content,
            "dir": if self.direction() == Direction::Rtl { "rtl" } else { "ltr" },
            "rounded": self.rounded,
        }))
    }
}
