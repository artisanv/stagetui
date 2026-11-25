//! `StageSelect`: pick one option from a dropdown list.

use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use serde_json::{Map, Value, json};

use super::{Cx, PaintCx, Reaction, Widget, draw_box, fill, object};
use crate::frame::Frame;
use crate::input::{Code, Key};
use crate::text::{self, Direction};

#[derive(Debug, Clone)]
pub struct Select {
    pub options: Vec<String>,
    pub selected: usize,
    pub title: Option<String>,
    pub width: Option<u16>,
    pub open: bool,
    /// Highlighted row while open.
    pub hover: usize,
}

impl Select {
    pub fn new(options: Vec<String>, selected: usize) -> Self {
        let selected = selected.min(options.len().saturating_sub(1));
        Self {
            options,
            selected,
            title: None,
            width: None,
            open: false,
            hover: selected,
        }
    }

    fn current(&self) -> &str {
        self.options.get(self.selected).map_or("", String::as_str)
    }

    fn choose(&mut self, i: usize) -> Reaction {
        self.open = false;
        if i < self.options.len() && i != self.selected {
            self.selected = i;
            self.hover = i;
            Reaction::Emit("change", Value::String(self.options[i].clone()))
        } else {
            Reaction::Redraw
        }
    }

    fn rows_shown(&self) -> u16 {
        self.options.len().min(8) as u16
    }

    fn first_shown(&self) -> usize {
        let rows = usize::from(self.rows_shown());
        self.hover.saturating_sub(rows.saturating_sub(1))
    }
}

impl Widget for Select {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let longest = self
            .options
            .iter()
            .map(|o| text::width(o))
            .max()
            .unwrap_or(4);
        let want = self.width.unwrap_or(longest as u16 + 8);
        (want.min(max_width).max(8), 3)
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.width < 6 || area.height < 3 {
            return;
        }
        let t = p.theme;
        let outside = super::behind(frame, area.x, area.y, t.bg);
        let field = Rect::new(area.x, area.y, area.width, 3);
        fill(frame, field, t.fg, t.surface);
        draw_box(
            frame,
            field,
            true,
            p.border(),
            outside,
            self.title.as_deref(),
            if p.focused { t.accent } else { t.muted },
            p.rtl,
        );
        let room = area.width - 6;
        let line =
            text::layout(self.current(), usize::from(room.max(1)), Direction::Ltr).swap_remove(0);
        let lw = (line.width as u16).min(room);
        let rtl = p.rtl || line.dir == Direction::Rtl;
        let (tx, ax) = if rtl {
            (area.right() - 2 - lw, area.x + 2)
        } else {
            (area.x + 2, area.right() - 3)
        };
        frame.put_line(tx, area.y + 1, room, &line, t.fg, t.surface, false);
        frame.cells[(ax, area.y + 1)]
            .set_symbol(if self.open { "▴" } else { "▾" })
            .set_style(Style::new().fg(t.accent.color()).bg(t.surface.color()));
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({
            "options": self.options,
            "value": self.current(),
            "index": self.selected,
            "open": self.open,
        }))
    }

    fn focusable(&self) -> bool {
        true
    }

    fn key(&mut self, key: &Key, _area: Rect) -> Reaction {
        let n = self.options.len();
        if n == 0 {
            return Reaction::Ignored;
        }
        if self.open {
            return match key.code {
                Code::Up => {
                    self.hover = self.hover.saturating_sub(1);
                    Reaction::Redraw
                }
                Code::Down => {
                    self.hover = (self.hover + 1).min(n - 1);
                    Reaction::Redraw
                }
                Code::Home => {
                    self.hover = 0;
                    Reaction::Redraw
                }
                Code::End => {
                    self.hover = n - 1;
                    Reaction::Redraw
                }
                Code::Enter | Code::Char(' ') => self.choose(self.hover),
                Code::Esc | Code::Tab => {
                    self.open = false;
                    self.hover = self.selected;
                    if key.code == Code::Tab {
                        Reaction::Ignored
                    } else {
                        Reaction::Redraw
                    }
                }
                _ => Reaction::Redraw,
            };
        }
        match key.code {
            Code::Enter | Code::Char(' ') | Code::Down if key.mods == Default::default() => {
                self.open = true;
                self.hover = self.selected;
                Reaction::Redraw
            }
            _ => Reaction::Ignored,
        }
    }

    fn click(&mut self, _x: u16, _y: u16, _area: Rect) -> Reaction {
        self.open = !self.open;
        self.hover = self.selected;
        Reaction::Redraw
    }

    fn value(&self) -> Value {
        Value::String(self.current().to_string())
    }

    fn popup(&self, area: Rect, screen: Rect) -> Option<Rect> {
        if !self.open || self.options.is_empty() {
            return None;
        }
        let h = self.rows_shown() + 2;
        let below = area.y + 3;
        let y = if below + h <= screen.bottom() {
            below
        } else {
            area.y.saturating_sub(h).max(screen.y)
        };
        Some(Rect::new(area.x, y, area.width, h).intersection(screen))
    }

    fn paint_popup(&self, popup: Rect, frame: &mut Frame, p: &PaintCx) {
        let t = p.theme;
        let outside = super::behind(frame, popup.x, popup.y, t.bg);
        fill(frame, popup, t.fg, t.surface);
        draw_box(frame, popup, true, t.accent, outside, None, t.accent, p.rtl);
        let first = self.first_shown();
        let room = popup.width.saturating_sub(4);
        for (row, (i, opt)) in self
            .options
            .iter()
            .enumerate()
            .skip(first)
            .take(usize::from(popup.height.saturating_sub(2)))
            .enumerate()
        {
            let y = popup.y + 1 + row as u16;
            let on = i == self.hover;
            let bg = if on {
                t.accent.mix(t.surface, 0.55)
            } else {
                t.surface
            };
            fill(
                frame,
                Rect::new(popup.x + 1, y, popup.width - 2, 1),
                t.fg,
                bg,
            );
            let line = text::layout(opt, usize::from(room.max(1)), Direction::Ltr).swap_remove(0);
            let lw = (line.width as u16).min(room);
            let x = if p.rtl || line.dir == Direction::Rtl {
                popup.right() - 2 - lw
            } else {
                popup.x + 2
            };
            let fg = if i == self.selected { t.accent } else { t.fg };
            frame.put_line(x, y, room, &line, fg, bg, i == self.selected);
        }
    }

    fn click_popup(&mut self, _x: u16, y: u16, popup: Rect) -> Reaction {
        if y == 0 || y + 1 >= popup.height {
            return Reaction::Redraw;
        }
        self.choose(self.first_shown() + usize::from(y - 1))
    }

    fn close_popup(&mut self) {
        self.open = false;
        self.hover = self.selected;
    }
}
