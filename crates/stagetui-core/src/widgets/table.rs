//! `StageTable`: rows and columns with a selectable row.

use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use serde_json::{Map, Value, json};

use super::{Cx, PaintCx, Reaction, Widget, clamp_scroll, fill, object, scrollbar};
use crate::frame::Frame;
use crate::input::{Code, Key};
use crate::text::{self, Direction};

const GAP: usize = 3;

#[derive(Debug, Clone)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub selected: Option<usize>,
    pub scroll: usize,
    /// Visible rows (not counting the header); `None` = all.
    pub height: Option<u16>,
}

impl Table {
    fn columns(&self) -> usize {
        self.rows
            .iter()
            .map(Vec::len)
            .chain(std::iter::once(self.headers.len()))
            .max()
            .unwrap_or(0)
    }

    fn widths(&self) -> Vec<usize> {
        let mut w = vec![0usize; self.columns()];
        for row in std::iter::once(&self.headers).chain(&self.rows) {
            for (i, cell) in row.iter().enumerate() {
                w[i] = w[i].max(text::width(cell));
            }
        }
        w
    }

    fn header_rows(&self) -> u16 {
        if self.headers.is_empty() { 0 } else { 2 }
    }

    fn body_rows(&self, area: Rect) -> usize {
        usize::from(area.height.saturating_sub(self.header_rows()))
    }

    fn select(&mut self, i: usize, area: Rect) -> Reaction {
        if self.rows.is_empty() {
            return Reaction::Ignored;
        }
        let i = i.min(self.rows.len() - 1);
        let visible = self.body_rows(area).max(1);
        if i < self.scroll {
            self.scroll = i;
        } else if i >= self.scroll + visible {
            self.scroll = i + 1 - visible;
        }
        if self.selected == Some(i) {
            return Reaction::Redraw;
        }
        self.selected = Some(i);
        Reaction::Emit("select", self.selection())
    }

    fn selection(&self) -> Value {
        match self.selected {
            Some(i) => json!({ "index": i, "row": self.rows.get(i) }),
            None => Value::Null,
        }
    }
}

impl Widget for Table {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let widths = self.widths();
        let w = widths.iter().sum::<usize>() + GAP * widths.len().saturating_sub(1) + 2;
        let rows = self.height.map_or(self.rows.len(), |h| {
            usize::from(h).min(self.rows.len().max(1))
        });
        let h = usize::from(self.header_rows()) + rows.max(1);
        ((w as u16).min(max_width).max(4), h as u16)
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.width < 3 || area.height == 0 {
            return;
        }
        let t = p.theme;
        let bg = super::behind(frame, area.x, area.y, t.bg);
        let mut widths = self.widths();
        // Shrink the widest columns until everything fits.
        let room = usize::from(area.width).saturating_sub(2);
        while widths.iter().sum::<usize>() + GAP * widths.len().saturating_sub(1) > room {
            let Some(m) = widths.iter_mut().max_by_key(|w| **w) else {
                break;
            };
            if *m <= 3 {
                break;
            }
            *m -= 1;
        }
        let rtl = p.rtl;
        let draw_row = |frame: &mut Frame, y: u16, row: &[String], fg, rbg, bold| {
            fill(frame, Rect::new(area.x, y, area.width, 1), fg, rbg);
            let mut offset = 1usize;
            for (i, w) in widths.iter().enumerate() {
                let cell = row.get(i).map_or("", String::as_str);
                let line = text::layout(cell, (*w).max(1), Direction::Ltr).swap_remove(0);
                let lw = line.width.min(*w);
                let start = if rtl {
                    usize::from(area.width) - offset - w
                } else {
                    offset
                };
                let x = area.x
                    + start as u16
                    + if line.dir == Direction::Rtl {
                        (w - lw) as u16
                    } else {
                        0
                    };
                frame.put_line(x, y, *w as u16, &line, fg, rbg, bold);
                offset += w + GAP;
            }
        };

        let mut y = area.y;
        if !self.headers.is_empty() {
            draw_row(frame, y, &self.headers, t.accent, bg, true);
            if area.height > 1 {
                let rule = Style::new().fg(t.border.color()).bg(bg.color());
                for x in area.left()..area.right() {
                    frame.cells[(x, y + 1)].set_symbol("─").set_style(rule);
                }
            }
            y += 2;
        }
        let visible = self.body_rows(area);
        let offset = clamp_scroll(self.scroll as i64, self.rows.len(), visible);
        for (i, row) in self.rows.iter().enumerate().skip(offset).take(visible) {
            let selected = self.selected == Some(i);
            let rbg = if selected && p.focused {
                t.accent.mix(t.surface, 0.55)
            } else if selected {
                t.accent.mix(t.surface, 0.8)
            } else if i % 2 == 1 {
                t.surface
            } else {
                bg
            };
            draw_row(frame, y, row, t.fg, rbg, selected);
            y += 1;
        }
        if self.rows.len() > visible {
            let x = if rtl { area.left() } else { area.right() - 1 };
            let top = area.y + self.header_rows();
            scrollbar(
                frame,
                x,
                top,
                visible as u16,
                offset,
                self.rows.len(),
                t.border,
                t.accent,
            );
        }
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({
            "headers": self.headers,
            "rows": self.rows.len(),
            "selected": self.selected,
        }))
    }

    fn focusable(&self) -> bool {
        true
    }

    fn key(&mut self, key: &Key, area: Rect) -> Reaction {
        if self.rows.is_empty() {
            return Reaction::Ignored;
        }
        let cur = self.selected.unwrap_or(0);
        let page = self.body_rows(area).max(1);
        let target = match key.code {
            Code::Up => cur.saturating_sub(1),
            Code::Down if self.selected.is_none() => 0,
            Code::Down => cur + 1,
            Code::PageUp => cur.saturating_sub(page),
            Code::PageDown => cur + page,
            Code::Home => 0,
            Code::End => self.rows.len() - 1,
            Code::Enter => return Reaction::Emit("submit", self.selection()),
            _ => return Reaction::Ignored,
        };
        self.select(target, area)
    }

    fn click(&mut self, _x: u16, y: u16, area: Rect) -> Reaction {
        let h = self.header_rows();
        if y < h {
            return Reaction::Redraw;
        }
        let offset = clamp_scroll(self.scroll as i64, self.rows.len(), self.body_rows(area));
        self.select(offset + usize::from(y - h), area)
    }

    fn scroll_by(&mut self, rows: i32, area: Rect) -> bool {
        let next = clamp_scroll(
            self.scroll as i64 + i64::from(rows),
            self.rows.len(),
            self.body_rows(area),
        );
        let moved = next != self.scroll;
        self.scroll = next;
        moved
    }

    fn value(&self) -> Value {
        self.selection()
    }
}
