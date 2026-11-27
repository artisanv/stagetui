//! `StageInput`: a one-line text box (Arabic and other scripts welcome).

use ratatui_core::layout::Rect;
use ratatui_core::style::{Modifier, Style};
use serde_json::{Map, Value, json};
use unicode_segmentation::UnicodeSegmentation;

use super::{Cx, PaintCx, Reaction, Widget, draw_box, fill, object};
use crate::frame::Frame;
use crate::input::{Code, Key};
use crate::text::{self, Direction};

#[derive(Debug, Clone)]
pub struct Input {
    pub value: String,
    pub placeholder: String,
    pub title: Option<String>,
    pub password: bool,
    pub width: Option<u16>,
    /// Cursor position in graphemes.
    pub cursor: usize,
    pub dir: String,
}

impl Input {
    pub fn new(value: String) -> Self {
        let cursor = value.graphemes(true).count();
        Self {
            value,
            placeholder: String::new(),
            title: None,
            password: false,
            width: None,
            cursor,
            dir: "auto".into(),
        }
    }

    fn len(&self) -> usize {
        self.value.graphemes(true).count()
    }

    /// Byte offset of grapheme `i`.
    fn byte_at(&self, i: usize) -> usize {
        self.value
            .grapheme_indices(true)
            .nth(i)
            .map_or(self.value.len(), |(b, _)| b)
    }

    fn shown(&self) -> String {
        if self.password {
            "•".repeat(self.len())
        } else {
            self.value.clone()
        }
    }

    fn rtl(&self) -> bool {
        text::resolve_dir(&self.dir, &self.value) == Direction::Rtl
            || (self.value.is_empty() && self.dir == "rtl")
    }

    fn insert(&mut self, s: &str) -> Reaction {
        let clean: String = s.chars().filter(|c| !c.is_control()).collect();
        if clean.is_empty() {
            return Reaction::Redraw;
        }
        let at = self.byte_at(self.cursor);
        self.value.insert_str(at, &clean);
        self.cursor += clean.graphemes(true).count();
        self.changed()
    }

    fn changed(&self) -> Reaction {
        Reaction::Emit("change", Value::String(self.value.clone()))
    }
}

impl Widget for Input {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let want = self.width.unwrap_or_else(|| {
            let text = text::width(&self.value).max(text::width(&self.placeholder));
            (text as u16 + 6).clamp(24, 48)
        });
        (want.min(max_width).max(6), 3)
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.width < 4 || area.height < 3 {
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
            self.rtl() || p.rtl,
        );
        let inner = usize::from(area.width - 4);
        let y = area.y + 1;
        let x0 = area.x + 2;
        let rtl = self.rtl();

        if self.value.is_empty() {
            let line = text::layout(&self.placeholder, inner.max(1), Direction::Ltr).swap_remove(0);
            let x = if rtl || (p.rtl && self.dir != "ltr") {
                x0 + (inner - line.width.min(inner)) as u16
            } else {
                x0
            };
            frame.put_line(x, y, inner as u16, &line, t.muted, t.surface, false);
            if p.focused {
                let cx = if rtl || (p.rtl && self.dir != "ltr") {
                    x0 + inner as u16 - 1
                } else {
                    x0
                };
                cursor(frame, cx, y, t);
            }
            return;
        }

        let shown = self.shown();
        let graphemes: Vec<&str> = shown.graphemes(true).collect();
        let prefix_w: usize = graphemes[..self.cursor.min(graphemes.len())]
            .iter()
            .map(|g| text::width(g))
            .sum();
        if rtl {
            // Right-to-left: show the end of the text, cursor measured from the right edge.
            let line = text::layout(&shown, usize::MAX / 2, Direction::Rtl).swap_remove(0);
            if line.width < inner {
                let x = x0 + (inner - line.width) as u16;
                frame.put_line(x, y, inner as u16, &line, t.fg, t.surface, false);
            } else {
                // Too long: show the newest part, which is on the left (cells only).
                let head = text::split_graphemes(&line.visual, inner - 1)
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                let style = Style::new().fg(t.fg.color()).bg(t.surface.color());
                frame.put_str(x0 + 1, y, &head, inner as u16 - 1, style);
            }
            if p.focused {
                let from_right = prefix_w.min(inner.saturating_sub(1)) as u16;
                cursor(
                    frame,
                    (x0 + inner as u16).saturating_sub(1 + from_right),
                    y,
                    t,
                );
            }
            return;
        }

        // Left-to-right: scroll so the cursor stays visible.
        let offset = prefix_w.saturating_sub(inner.saturating_sub(1));
        let mut col = 0usize;
        let mut visible = String::new();
        for g in &graphemes {
            let w = text::width(g).max(1);
            if col >= offset && col + w - offset <= inner {
                visible.push_str(g);
            }
            col += w;
        }
        if text::is_complex(&visible) {
            // Arabic, Hindi… inside left-to-right text ("Sara · سارة"): join the letters and
            // put right-to-left words in their visual order, like everywhere else.
            let visual = text::bidi::visual(&text::arabic::shape(&visible), Direction::Ltr);
            let line = text::Line {
                width: text::width(&visual),
                visual,
                dir: Direction::Ltr,
                complex: true,
                logical: visible,
            };
            frame.put_line(x0, y, inner as u16, &line, t.fg, t.surface, false);
        } else {
            let style = Style::new().fg(t.fg.color()).bg(t.surface.color());
            let mut x = x0;
            for g in visible.graphemes(true) {
                let w = text::width(g).max(1);
                frame.put_str(x, y, g, w as u16, style);
                x += w as u16;
            }
        }
        if p.focused {
            cursor(frame, x0 + (prefix_w - offset) as u16, y, t);
        }
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({
            "value": if self.password { Value::String("•".repeat(self.len())) } else { Value::String(self.value.clone()) },
            "placeholder": self.placeholder,
            "title": self.title,
            "cursor": self.cursor,
        }))
    }

    fn focusable(&self) -> bool {
        true
    }

    fn takes_text(&self) -> bool {
        true
    }

    fn key(&mut self, key: &Key, _area: Rect) -> Reaction {
        if let Some(c) = key.text() {
            return self.insert(&c.to_string());
        }
        let ctrl = key.mods.ctrl;
        match key.code {
            Code::Enter if !ctrl => Reaction::Emit("submit", Value::String(self.value.clone())),
            Code::Backspace if self.cursor > 0 => {
                let (a, b) = (self.byte_at(self.cursor - 1), self.byte_at(self.cursor));
                self.value.replace_range(a..b, "");
                self.cursor -= 1;
                self.changed()
            }
            Code::Delete if self.cursor < self.len() => {
                let (a, b) = (self.byte_at(self.cursor), self.byte_at(self.cursor + 1));
                self.value.replace_range(a..b, "");
                self.changed()
            }
            Code::Char('u') if ctrl => {
                self.value.clear();
                self.cursor = 0;
                self.changed()
            }
            // In right-to-left text the arrow keys move visually.
            Code::Left | Code::Right => {
                let forward = (key.code == Code::Right) != self.rtl();
                if forward && self.cursor < self.len() {
                    self.cursor += 1;
                } else if !forward && self.cursor > 0 {
                    self.cursor -= 1;
                }
                Reaction::Redraw
            }
            Code::Home | Code::Char('a') if key.code == Code::Home || ctrl => {
                self.cursor = 0;
                Reaction::Redraw
            }
            Code::End | Code::Char('e') if key.code == Code::End || ctrl => {
                self.cursor = self.len();
                Reaction::Redraw
            }
            Code::Backspace | Code::Delete => Reaction::Redraw,
            _ => Reaction::Ignored,
        }
    }

    fn click(&mut self, _x: u16, _y: u16, _area: Rect) -> Reaction {
        self.cursor = self.len();
        Reaction::Redraw
    }

    fn paste(&mut self, text: &str) -> Reaction {
        self.insert(&text.replace(['\r', '\n'], " "))
    }

    fn value(&self) -> Value {
        Value::String(self.value.clone())
    }
}

fn cursor(frame: &mut Frame, x: u16, y: u16, t: &crate::style::Theme) {
    if let Some(c) = frame.cells.cell_mut((x, y)) {
        if c.symbol().is_empty() || c.symbol() == " " {
            c.set_symbol(" ");
        }
        c.set_style(
            Style::new()
                .bg(t.accent.color())
                .fg(t.bg.color())
                .add_modifier(Modifier::BOLD),
        );
    }
}
