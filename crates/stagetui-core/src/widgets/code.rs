//! `StageCode`: source code with syntax colors and line numbers.

use std::cell::RefCell;

use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use serde_json::{Map, Value, json};

use super::{Cx, PaintCx, Reaction, Widget, clamp_scroll, draw_box, fill, object, scrollbar};
use crate::frame::Frame;
use crate::input::{Code as KeyCode, Key};
use crate::style::Rgb;
use crate::text::paragraph::{Verbatim, verbatim};
use crate::text::{self, Direction, highlight};

type Highlighted = (u64, bool, Vec<Vec<highlight::Token>>);

/// Paint one line of preformatted text (see [`verbatim`]) at `(x, y)`, at most
/// `max` cells: its spaces as blank cells, its words as text. Returns the cells used.
#[allow(clippy::too_many_arguments)]
pub(crate) fn put_verbatim(
    frame: &mut Frame,
    x: u16,
    y: u16,
    max: u16,
    v: &Verbatim,
    fg: Rgb,
    bg: Rgb,
    bold: bool,
) -> u16 {
    let blank = Style::new().fg(fg.color()).bg(bg.color());
    let mut n = frame.put_str(x, y, &" ".repeat(v.lead), max, blank);
    n += frame.put_line(x + n, y, max - n, &v.line, fg, bg, bold);
    n + frame.put_str(x + n, y, &" ".repeat(v.trail), max - n, blank)
}

#[derive(Debug, Clone)]
pub struct Code {
    pub source: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub line_numbers: bool,
    pub height: Option<u16>,
    pub scroll: usize,
    cache: RefCell<Option<Highlighted>>,
}

impl Code {
    pub fn new(source: String) -> Self {
        Self {
            source,
            language: None,
            title: None,
            line_numbers: true,
            height: None,
            scroll: 0,
            cache: RefCell::new(None),
        }
    }

    fn tokens(&self, dark: bool) -> Vec<Vec<highlight::Token>> {
        let hash = crate::gfx::hash_bytes(format!("{:?}{}", self.language, self.source).as_bytes());
        let mut c = self.cache.borrow_mut();
        if let Some((h, d, lines)) = c.as_ref()
            && *h == hash
            && *d == dark
        {
            return lines.clone();
        }
        let lines = highlight::highlight(
            &self.source,
            self.language.as_deref().or(self.title.as_deref()),
            dark,
        );
        *c = Some((hash, dark, lines.clone()));
        lines
    }

    fn line_count(&self) -> usize {
        self.source.lines().count().max(1)
    }

    fn gutter(&self) -> u16 {
        if self.line_numbers {
            self.line_count().to_string().len() as u16 + 2
        } else {
            1
        }
    }

    fn rows(area: Rect) -> usize {
        usize::from(area.height.saturating_sub(2))
    }

    pub fn carry(&mut self, old: &Code) {
        self.scroll = old.scroll;
    }
}

impl Widget for Code {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let longest = self
            .source
            .lines()
            .map(|l| text::width(&text::paragraph::expand_tabs(l)))
            .max()
            .unwrap_or(0);
        let w = longest as u16 + self.gutter() + 4;
        let h = self.height.unwrap_or(self.line_count() as u16 + 2);
        (w.min(max_width).max(12), h.max(3))
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.width < 6 || area.height < 3 {
            return;
        }
        let t = p.theme;
        let outside = super::behind(frame, area.x, area.y, t.bg);
        let bg = super::markdown::code_bg(t, t.surface);
        fill(frame, area, t.fg, bg);
        let lang = highlight::language(
            self.language.as_deref().or(self.title.as_deref()),
            &self.source,
        );
        let heading = match &self.title {
            Some(title) => format!("{title} · {lang}"),
            None => lang,
        };
        draw_box(
            frame,
            area,
            true,
            p.border(),
            outside,
            Some(&heading),
            t.accent,
            false,
        );
        let tokens = self.tokens(t.dark);
        let rows = Self::rows(area);
        let first = clamp_scroll(self.scroll as i64, tokens.len(), rows);
        let gutter = self.gutter();
        let x0 = area.x + 1;
        let right = area.right() - 1;
        let muted = Style::new().fg(t.muted.color()).bg(bg.color());
        for (row, (n, line)) in tokens.iter().enumerate().skip(first).take(rows).enumerate() {
            let y = area.y + 1 + row as u16;
            if self.line_numbers {
                let num = format!("{:>w$} ", n + 1, w = usize::from(gutter) - 2);
                frame.put_str(x0, y, &num, gutter, muted);
            }
            let mut x = x0 + gutter;
            for tok in line {
                if x >= right {
                    break;
                }
                // Each token is a piece of a left-to-right line of code.
                let v = verbatim(&tok.text, Some(Direction::Ltr));
                x += put_verbatim(frame, x, y, right - x, &v, tok.color, bg, tok.bold);
            }
        }
        if tokens.len() > rows {
            scrollbar(
                frame,
                right,
                area.y + 1,
                rows as u16,
                first,
                tokens.len(),
                p.border(),
                t.accent,
            );
        }
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({
            "text": self.source,
            "language": highlight::language(self.language.as_deref().or(self.title.as_deref()), &self.source),
            "title": self.title,
            "lines": self.line_count(),
        }))
    }

    fn focusable(&self) -> bool {
        true
    }

    fn key(&mut self, key: &Key, area: Rect) -> Reaction {
        let page = Self::rows(area).max(1) as i32;
        let d = match key.code {
            KeyCode::Up => -1,
            KeyCode::Down => 1,
            KeyCode::PageUp => -page,
            KeyCode::PageDown => page,
            KeyCode::Home => i32::MIN / 2,
            KeyCode::End => i32::MAX / 2,
            _ => return Reaction::Ignored,
        };
        if self.scroll_by(d, area) {
            Reaction::Redraw
        } else {
            Reaction::Ignored
        }
    }

    fn scroll_by(&mut self, rows: i32, area: Rect) -> bool {
        let next = clamp_scroll(
            self.scroll as i64 + i64::from(rows),
            self.line_count(),
            Self::rows(area),
        );
        let moved = next != self.scroll;
        self.scroll = next;
        moved
    }

    fn value(&self) -> Value {
        Value::String(self.source.clone())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::json;

    use crate::frame::Frame;
    use crate::session::Session;
    use crate::text::Direction;
    use crate::text::paragraph::verbatim;

    /// Render one widget headless: the frame, and each row of cells as a string.
    pub(crate) fn render(widget: serde_json::Value, cols: u16, rows: u16) -> (Frame, Vec<String>) {
        let spec = json!({ "children": [widget] }).to_string();
        let frame = Session::from_json(&spec).unwrap().frame(cols, rows);
        let area = frame.area();
        let cells = (area.top()..area.bottom())
            .map(|y| {
                (area.left()..area.right())
                    .map(|x| frame.cells[(x, y)].symbol())
                    .collect()
            })
            .collect();
        (frame, cells)
    }

    /// The cells a left-to-right code line shows: words in visual order after the indent.
    fn shown(line: &str) -> String {
        let v = verbatim(line, Some(Direction::Ltr));
        format!("{}{}", " ".repeat(v.lead), v.line.visual)
    }

    const SOURCE: &str = "def f(x: int) -> int:\n    if x:\n\treturn x + 1  # done\n        s = \"مَرْحَباً بِكُمْ\"\n    t = \"Hello مرحبا world\"\n    j = \"こんにちは 世界 你好\"\n    h = \"नमस्ते दुनिया\"\n    # مَرْحَباً";

    #[test]
    fn code_keeps_every_space_between_and_before_tokens() {
        for language in [json!("python"), json!(null)] {
            let (frame, cells) = render(
                json!({"type": "code", "content": SOURCE, "props": {"language": language}}),
                70,
                14,
            );
            let text = frame.text();
            let want = [
                "def f(x: int) -> int:",
                "    if x:",
                "    return x + 1  # done",
                "        s = \"مَرْحَباً بِكُمْ\"",
                "    t = \"Hello مرحبا world\"",
                "    j = \"こんにちは 世界 你好\"",
                "    h = \"नमस्ते दुनिया\"",
                "    # مَرْحَباً",
            ];
            for line in want {
                assert!(
                    text.contains(line),
                    "{language}: {line:?} missing in\n{text}"
                );
            }
            // On screen (not just in the text read back): the indentation is at the
            // left, and Arabic is shaped and reversed after it, the `#` first.
            let comment = shown("    # مَرْحَباً");
            assert!(comment.starts_with("    # ") && comment != "    # مَرْحَباً");
            assert!(
                cells.iter().any(|r| r.contains(&comment)),
                "{language}: {cells:#?}"
            );
            assert!(cells.iter().any(|r| r.contains("    return x + 1  # done")));
            assert!(cells.iter().any(|r| r.contains("    j = \"こ")));
            assert!(cells.iter().any(|r| r.contains("    h = \"न")));
            // Pixel text runs hold only the words, never the spaces around them.
            assert!(frame.runs.iter().all(|r| r.logical.trim() == r.logical));
            assert!(
                frame
                    .runs
                    .iter()
                    .any(|r| r.logical.contains("مَرْحَباً") && r.dir == Direction::Ltr)
            );
        }
    }

    #[test]
    fn verbatim_lines_paint_their_spaces_as_cells() {
        let theme = crate::style::THEMES[0];
        let mut frame = Frame::new(
            ratatui_core::layout::Rect::new(0, 0, 30, 1),
            theme.bg,
            theme.fg,
        );
        let v = verbatim("  مَرْحَباً  x ", None);
        let n = super::put_verbatim(&mut frame, 1, 0, 29, &v, theme.fg, theme.bg, false);
        assert_eq!(usize::from(n), v.width());
        assert_eq!(frame.runs.len(), 1);
        assert_eq!(
            frame.runs[0].area.x, 3,
            "the run starts after the indentation"
        );
        assert_eq!(frame.text(), "   مَرْحَباً  x");
        // Clipped: never more than `max` cells.
        let mut frame = Frame::new(
            ratatui_core::layout::Rect::new(0, 0, 30, 1),
            theme.bg,
            theme.fg,
        );
        assert_eq!(
            super::put_verbatim(&mut frame, 0, 0, 3, &v, theme.fg, theme.bg, false),
            3
        );
    }
}
