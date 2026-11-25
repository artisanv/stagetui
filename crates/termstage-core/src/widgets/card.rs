//! `StageCard`: a titled, bordered surface with wrapped text.

use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use serde_json::{Map, Value, json};

use super::{Cx, PaintCx, Reaction, Widget, clamp_scroll, draw_box, fill, object, scrollbar};
use crate::frame::Frame;
use crate::input::{Code, Key};
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
    /// A soft shadow one cell to the right and below.
    pub shadow: bool,
    /// First visible text line (when the text is taller than the card).
    pub scroll: usize,
    /// Takes focus (cards that are panes of a split).
    pub selectable: bool,
}

impl Card {
    pub fn new(content: String) -> Self {
        Self {
            content,
            title: None,
            icon: None,
            rounded: true,
            padding: (2, 1),
            dir: "auto".into(),
            shadow: false,
            scroll: 0,
            selectable: false,
        }
    }

    pub fn direction(&self) -> Direction {
        text::resolve_dir(&self.dir, &self.content)
    }

    /// Title text in reading order (icon first).
    fn heading(&self, rtl: bool) -> Option<String> {
        let icon = self.icon.map(|i| if rtl { super::mirror(i) } else { i });
        match (icon, self.title.as_deref()) {
            (Some(i), Some(t)) => Some(format!("{i} {t}")),
            (None, Some(t)) => Some(t.to_string()),
            (Some(i), None) => Some(i.to_string()),
            (None, None) => None,
        }
    }

    fn shadow_cells(&self) -> u16 {
        u16::from(self.shadow)
    }

    fn inner_width(&self, width: u16) -> usize {
        usize::from(width.saturating_sub(2 + 2 * self.padding.0 + self.shadow_cells())).max(1)
    }

    fn base_dir(&self) -> Direction {
        if self.dir == "rtl" {
            Direction::Rtl
        } else {
            Direction::Ltr
        }
    }

    fn lines(&self, width: u16) -> Vec<text::Line> {
        text::layout(&self.content, self.inner_width(width), self.base_dir())
    }

    /// Rows available for text inside `area`.
    fn text_rows(&self, area: Rect) -> usize {
        usize::from(
            area.height
                .saturating_sub(2 + 2 * self.padding.1 + self.shadow_cells()),
        )
    }
}

impl Widget for Card {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let natural = text::layout(&self.content, usize::MAX / 2, self.base_dir())
            .iter()
            .map(|l| l.width)
            .max()
            .unwrap_or(0);
        let heading = self.heading(false).map_or(0, |h| {
            text::layout(&h, usize::MAX / 2, Direction::Ltr)[0].width + 4
        });
        let chrome = 2 + 2 * usize::from(self.padding.0) + usize::from(self.shadow_cells());
        let want = (natural + chrome).max(heading + 2).max(12);
        let w = want.min(usize::from(max_width)) as u16;
        let h = self.lines(w).len()
            + 2
            + 2 * usize::from(self.padding.1)
            + usize::from(self.shadow_cells());
        (w, h.min(usize::from(u16::MAX)) as u16)
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.width < 2 || area.height < 2 {
            return;
        }
        let theme = p.theme;
        let outside = super::behind(frame, area.x, area.y, theme.bg);
        let s = self.shadow_cells();
        let body = Rect::new(
            area.x,
            area.y,
            area.width.saturating_sub(s),
            area.height.saturating_sub(s),
        );
        if s > 0 {
            let shade = outside.mix(crate::style::Rgb(0, 0, 0), 0.45);
            for y in body.top() + 1..area.bottom() {
                for x in body.left() + 2..area.right() {
                    if x >= body.right() || y >= body.bottom() {
                        frame.cells[(x, y)]
                            .set_symbol(" ")
                            .set_style(Style::new().bg(shade.color()));
                    }
                }
            }
        }
        fill(frame, body, theme.fg, theme.surface);

        let rtl = self.direction() == Direction::Rtl || (p.rtl && self.dir == "auto");
        let heading = self.heading(rtl);
        draw_box(
            frame,
            body,
            self.rounded,
            p.border(),
            outside,
            heading.as_deref(),
            theme.accent,
            rtl,
        );

        let inner_w = self.inner_width(area.width);
        let x0 = body.left() + 1 + self.padding.0;
        let first_row = body.top() + 1 + self.padding.1;
        let rows = self.text_rows(area);
        let lines = self.lines(area.width);
        let offset = clamp_scroll(self.scroll as i64, lines.len(), rows);
        for (y, line) in (first_row..first_row + rows as u16).zip(lines.iter().skip(offset)) {
            let lw = line.width.min(inner_w) as u16;
            let x = if line.dir == Direction::Rtl {
                x0 + inner_w as u16 - lw
            } else {
                x0
            };
            frame.put_line(x, y, inner_w as u16, line, theme.fg, theme.surface, false);
        }
        if lines.len() > rows {
            let x = if rtl { body.left() } else { body.right() - 1 };
            scrollbar(
                frame,
                x,
                body.top() + 1,
                body.height.saturating_sub(2),
                offset,
                lines.len(),
                p.border(),
                theme.accent,
            );
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

    fn focusable(&self) -> bool {
        self.selectable
    }

    fn key(&mut self, key: &Key, area: Rect) -> Reaction {
        let step = match key.code {
            Code::Up if key.mods == Default::default() => -1,
            Code::Down if key.mods == Default::default() => 1,
            Code::PageUp => -(self.text_rows(area).max(1) as i32),
            Code::PageDown => self.text_rows(area).max(1) as i32,
            _ => return Reaction::Ignored,
        };
        if self.scroll_by(step, area) {
            Reaction::Redraw
        } else {
            Reaction::Ignored
        }
    }

    fn scroll_by(&mut self, rows: i32, area: Rect) -> bool {
        let total = self.lines(area.width).len();
        let next = clamp_scroll(
            self.scroll as i64 + i64::from(rows),
            total,
            self.text_rows(area),
        );
        let moved = next != self.scroll;
        self.scroll = next;
        moved
    }
}
