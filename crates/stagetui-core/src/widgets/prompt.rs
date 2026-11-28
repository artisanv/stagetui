//! `StagePrompt`: where you talk to the agent. Multi-line editing, history
//! (↑ / ↓), `/` commands and `@` file mentions with a completion list, paste.

use std::cell::RefCell;

use ratatui_core::layout::Rect;
use ratatui_core::style::{Modifier, Style};
use serde_json::{Map, Value, json};
use unicode_segmentation::UnicodeSegmentation;

use super::{Cx, PaintCx, Reaction, Widget, draw_box, fill, object};
use crate::frame::Frame;
use crate::input::{Code, Key};
use crate::text::{self, Direction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Command,
    Mention,
}

#[derive(Debug, Clone)]
struct Completion {
    kind: Kind,
    /// Grapheme column where the `/` or `@` token starts.
    start: usize,
    /// `(insert, label, detail)`
    items: Vec<(String, String, String)>,
    hover: usize,
    /// The user picked `hover` with ↑ / ↓ since the last edit.
    moved: bool,
}

#[derive(Debug, Clone)]
pub struct Prompt {
    pub lines: Vec<String>,
    pub row: usize,
    /// Cursor column in graphemes.
    pub col: usize,
    pub placeholder: String,
    pub title: Option<String>,
    pub history: Vec<String>,
    hist: Option<usize>,
    draft: Option<Vec<String>>,
    /// `(name, description)` for `/` commands.
    pub commands: Vec<(String, String)>,
    /// Files offered after `@`; `None` = the files in the current folder.
    pub mentions: Option<Vec<String>>,
    pub max_lines: u16,
    completion: Option<Completion>,
    scanned: RefCell<Option<Vec<String>>>,
}

fn graphemes(s: &str) -> Vec<&str> {
    s.graphemes(true).collect()
}

/// Files under the current folder (skipping hidden and build folders), for `@`.
fn scan_files() -> Vec<String> {
    const SKIP: [&str; 7] = [
        "node_modules",
        "target",
        "__pycache__",
        "dist",
        "build",
        ".venv",
        "venv",
    ];
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(".")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || SKIP.contains(&name.as_str()) {
                continue;
            }
            let path = e.path();
            let rel = path
                .strip_prefix(".")
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                if stack.len() < 10_000 {
                    stack.push(path);
                }
            } else {
                out.push(rel);
                if out.len() >= 5000 {
                    return out;
                }
            }
        }
    }
    out
}

impl Prompt {
    pub fn new(text: &str) -> Self {
        let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        let row = lines.len() - 1;
        let col = graphemes(&lines[row]).len();
        Self {
            lines,
            row,
            col,
            placeholder: String::new(),
            title: None,
            history: Vec::new(),
            hist: None,
            draft: None,
            commands: Vec::new(),
            mentions: None,
            max_lines: 8,
            completion: None,
            scanned: RefCell::new(None),
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    fn set_text(&mut self, text: &str) {
        self.lines = text.split('\n').map(str::to_string).collect();
        self.row = self.lines.len() - 1;
        self.col = graphemes(&self.lines[self.row]).len();
        self.completion = None;
    }

    fn byte(&self, row: usize, col: usize) -> usize {
        self.lines[row]
            .grapheme_indices(true)
            .nth(col)
            .map_or(self.lines[row].len(), |(b, _)| b)
    }

    fn line_len(&self, row: usize) -> usize {
        graphemes(&self.lines[row]).len()
    }

    fn insert(&mut self, s: &str) -> Reaction {
        let s: String = s
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .chars()
            .filter(|c| *c == '\n' || !c.is_control())
            .collect();
        for (i, part) in s.split('\n').enumerate() {
            if i > 0 {
                let at = self.byte(self.row, self.col);
                let rest = self.lines[self.row].split_off(at);
                self.lines.insert(self.row + 1, rest);
                self.row += 1;
                self.col = 0;
            }
            let at = self.byte(self.row, self.col);
            self.lines[self.row].insert_str(at, part);
            self.col += graphemes(part).len();
        }
        self.changed()
    }

    fn changed(&mut self) -> Reaction {
        self.hist = None;
        self.update_completion();
        Reaction::Emit("change", Value::String(self.text()))
    }

    fn mention_source(&self) -> Vec<String> {
        if let Some(m) = &self.mentions {
            return m.clone();
        }
        self.scanned
            .borrow_mut()
            .get_or_insert_with(scan_files)
            .clone()
    }

    fn update_completion(&mut self) {
        let line = &self.lines[self.row];
        let g = graphemes(line);
        let col = self.col.min(g.len());
        let start = g[..col]
            .iter()
            .rposition(|s| s.chars().all(char::is_whitespace))
            .map_or(0, |p| p + 1);
        let token: String = g[start..col].concat();
        let prev = self.completion.as_ref().map(|c| c.hover).unwrap_or(0);
        self.completion = if let Some(q) = token
            .strip_prefix('/')
            .filter(|_| self.row == 0 && start == 0 && !self.commands.is_empty())
        {
            let items: Vec<_> = self
                .commands
                .iter()
                .filter(|(n, _)| n.starts_with(q))
                .map(|(n, d)| (format!("/{n}"), format!("/{n}"), d.clone()))
                .collect();
            (!items.is_empty()).then_some(Completion {
                kind: Kind::Command,
                start,
                items,
                hover: 0,
                moved: false,
            })
        } else if let Some(q) = token.strip_prefix('@') {
            let q = q.to_lowercase();
            let mut hits: Vec<(bool, String)> = self
                .mention_source()
                .into_iter()
                .filter(|f| f.to_lowercase().contains(&q))
                .map(|f| {
                    let name_hit = f
                        .rsplit('/')
                        .next()
                        .is_some_and(|n| n.to_lowercase().starts_with(&q));
                    (!name_hit, f)
                })
                .collect();
            hits.sort();
            let items: Vec<_> = hits
                .into_iter()
                .take(8)
                .map(|(_, f)| (format!("@{f}"), f, String::new()))
                .collect();
            (!items.is_empty()).then_some(Completion {
                kind: Kind::Mention,
                start,
                items,
                hover: 0,
                moved: false,
            })
        } else {
            None
        };
        if let Some(c) = &mut self.completion {
            c.hover = prev.min(c.items.len() - 1);
        }
    }

    /// What is typed is already a whole command (or file): the one picked with
    /// ↑ / ↓, or else any in the list. Enter then sends it instead of completing it.
    fn typed_in_full(&self) -> bool {
        let Some(c) = &self.completion else {
            return false;
        };
        let g = graphemes(&self.lines[self.row]);
        let typed: String = g[c.start.min(g.len())..self.col.min(g.len())].concat();
        if c.moved {
            c.items[c.hover].0 == typed
        } else {
            c.items.iter().any(|(insert, _, _)| *insert == typed)
        }
    }

    fn accept(&mut self) -> Reaction {
        let Some(c) = self.completion.take() else {
            return Reaction::Ignored;
        };
        let insert = format!("{} ", c.items[c.hover].0);
        let (a, b) = (self.byte(self.row, c.start), self.byte(self.row, self.col));
        self.lines[self.row].replace_range(a..b, &insert);
        self.col = c.start + graphemes(&insert).len();
        let r = Reaction::Emit("change", Value::String(self.text()));
        self.completion = None;
        r
    }

    fn submit(&mut self) -> Reaction {
        let text = self.text();
        if !text.trim().is_empty() && self.history.last() != Some(&text) {
            self.history.push(text.clone());
        }
        self.set_text("");
        self.hist = None;
        self.draft = None;
        Reaction::Emit("submit", Value::String(text))
    }

    fn history_step(&mut self, back: bool) -> Reaction {
        if self.history.is_empty() {
            return Reaction::Ignored;
        }
        let next = match (self.hist, back) {
            (None, true) => {
                self.draft = Some(self.lines.clone());
                Some(self.history.len() - 1)
            }
            (None, false) => return Reaction::Ignored,
            (Some(0), true) => Some(0),
            (Some(i), true) => Some(i - 1),
            (Some(i), false) if i + 1 < self.history.len() => Some(i + 1),
            (Some(_), false) => None,
        };
        match next {
            Some(i) => {
                let t = self.history[i].clone();
                self.set_text(&t);
            }
            None => {
                let d = self.draft.take().unwrap_or_else(|| vec![String::new()]);
                self.set_text(&d.join("\n"));
            }
        }
        self.hist = next;
        Reaction::Redraw
    }

    /// Visual rows: `(line, first grapheme, last grapheme)` wrapped at `w` cells.
    fn rows(&self, w: usize) -> Vec<(usize, usize, usize)> {
        let w = w.max(2);
        let mut out = Vec::new();
        for (li, line) in self.lines.iter().enumerate() {
            let g = graphemes(line);
            let mut start = 0;
            let mut used = 0;
            for (i, s) in g.iter().enumerate() {
                let sw = text::width(s).max(1);
                if used + sw > w {
                    out.push((li, start, i));
                    start = i;
                    used = 0;
                }
                used += sw;
            }
            out.push((li, start, g.len()));
        }
        out
    }

    fn cursor_row(&self, rows: &[(usize, usize, usize)]) -> usize {
        rows.iter()
            .rposition(|(l, s, _)| *l == self.row && *s <= self.col)
            .unwrap_or(0)
    }

    fn inner_width(area: Rect) -> usize {
        usize::from(area.width.saturating_sub(4))
    }

    pub fn carry(&mut self, old: &Prompt) {
        self.history = old.history.clone();
        if self.lines == old.lines {
            self.row = old.row;
            self.col = old.col;
        }
    }
}

impl Widget for Prompt {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let rows = self.rows(usize::from(max_width.saturating_sub(4))).len() as u16;
        (max_width, rows.clamp(1, self.max_lines.max(1)) + 2)
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.width < 6 || area.height < 3 {
            return;
        }
        let t = p.theme;
        let outside = super::behind(frame, area.x, area.y, t.bg);
        fill(frame, area, t.fg, t.surface);
        draw_box(
            frame,
            area,
            true,
            p.border(),
            outside,
            self.title.as_deref(),
            if p.focused { t.accent } else { t.muted },
            p.rtl,
        );
        let inner_w = Self::inner_width(area);
        let x0 = area.x + 2;
        let visible = usize::from(area.height - 2);
        let style = Style::new().fg(t.fg.color()).bg(t.surface.color());
        if self.text().is_empty() {
            let line =
                text::layout(&self.placeholder, inner_w.max(1), Direction::Ltr).swap_remove(0);
            let x = if line.dir == Direction::Rtl {
                x0 + (inner_w - line.width.min(inner_w)) as u16
            } else {
                x0
            };
            frame.put_line(
                x,
                area.y + 1,
                inner_w as u16,
                &line,
                t.muted,
                t.surface,
                false,
            );
            if p.focused {
                let cx = if line.dir == Direction::Rtl {
                    x0 + inner_w as u16 - 1
                } else {
                    x0
                };
                cursor(frame, cx, area.y + 1, t);
            }
            return;
        }
        let rows = self.rows(inner_w);
        let cur = self.cursor_row(&rows);
        let first = cur
            .saturating_sub(visible.saturating_sub(1))
            .min(rows.len().saturating_sub(visible));
        for (vi, (li, s, e)) in rows.iter().enumerate().skip(first).take(visible) {
            let y = area.y + 1 + (vi - first) as u16;
            let g = graphemes(&self.lines[*li]);
            let piece: String = g[*s..*e].concat();
            let rtl = text::first_strong(&self.lines[*li]) == Direction::Rtl;
            if rtl {
                let line = text::layout(&piece, usize::MAX / 2, Direction::Rtl).swap_remove(0);
                let lw = line.width.min(inner_w) as u16;
                frame.put_line(
                    x0 + inner_w as u16 - lw,
                    y,
                    inner_w as u16,
                    &line,
                    t.fg,
                    t.surface,
                    false,
                );
            } else {
                let mut x = x0;
                for gr in &g[*s..*e] {
                    let w = text::width(gr).max(1) as u16;
                    x += frame.put_str(x, y, gr, w, style);
                }
            }
            if p.focused && vi == cur {
                let before: usize = g[*s..self.col.min(*e).max(*s)]
                    .iter()
                    .map(|x| text::width(x).max(1))
                    .sum();
                let cx = if rtl {
                    (x0 + inner_w as u16).saturating_sub(1 + before as u16)
                } else {
                    x0 + before.min(inner_w.saturating_sub(1)) as u16
                };
                cursor(frame, cx, y, t);
            }
        }
        if rows.len() > visible {
            let more = format!(" {} lines ", self.lines.len());
            let x = area.right().saturating_sub(more.len() as u16 + 2);
            frame.put_str(
                x,
                area.bottom() - 1,
                &more,
                more.len() as u16,
                Style::new().fg(t.muted.color()).bg(outside.color()),
            );
        }
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({
            "value": self.text(),
            "placeholder": self.placeholder,
            "history": self.history.len(),
            "completing": self.completion.as_ref().map(|c| c.items.iter().map(|i| i.1.clone()).collect::<Vec<_>>()),
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
        let open = self.completion.is_some();
        match key.code {
            Code::Enter if open && !self.typed_in_full() => self.accept(),
            Code::Tab if open && !key.mods.shift => self.accept(),
            Code::Enter if key.mods.shift || key.mods.alt => self.insert("\n"),
            Code::Char('j') if ctrl => self.insert("\n"),
            Code::Enter => self.submit(),
            Code::Esc if open => {
                self.completion = None;
                Reaction::Redraw
            }
            Code::Up if open => {
                if let Some(c) = &mut self.completion {
                    c.hover = c.hover.saturating_sub(1);
                    c.moved = true;
                }
                Reaction::Redraw
            }
            Code::Down if open => {
                if let Some(c) = &mut self.completion {
                    c.hover = (c.hover + 1).min(c.items.len() - 1);
                    c.moved = true;
                }
                Reaction::Redraw
            }
            Code::Up if self.row > 0 => {
                self.row -= 1;
                self.col = self.col.min(self.line_len(self.row));
                Reaction::Redraw
            }
            Code::Up => self.history_step(true),
            Code::Down if self.row + 1 < self.lines.len() => {
                self.row += 1;
                self.col = self.col.min(self.line_len(self.row));
                Reaction::Redraw
            }
            Code::Down => self.history_step(false),
            Code::Backspace => {
                if self.col > 0 {
                    let (a, b) = (
                        self.byte(self.row, self.col - 1),
                        self.byte(self.row, self.col),
                    );
                    self.lines[self.row].replace_range(a..b, "");
                    self.col -= 1;
                } else if self.row > 0 {
                    let line = self.lines.remove(self.row);
                    self.row -= 1;
                    self.col = self.line_len(self.row);
                    self.lines[self.row].push_str(&line);
                } else {
                    return Reaction::Redraw;
                }
                self.changed()
            }
            Code::Delete => {
                if self.col < self.line_len(self.row) {
                    let (a, b) = (
                        self.byte(self.row, self.col),
                        self.byte(self.row, self.col + 1),
                    );
                    self.lines[self.row].replace_range(a..b, "");
                } else if self.row + 1 < self.lines.len() {
                    let next = self.lines.remove(self.row + 1);
                    self.lines[self.row].push_str(&next);
                } else {
                    return Reaction::Redraw;
                }
                self.changed()
            }
            Code::Left | Code::Right => {
                let rtl = text::first_strong(&self.lines[self.row]) == Direction::Rtl;
                let forward = (key.code == Code::Right) != rtl;
                if forward {
                    if self.col < self.line_len(self.row) {
                        self.col += 1;
                    } else if self.row + 1 < self.lines.len() {
                        self.row += 1;
                        self.col = 0;
                    }
                } else if self.col > 0 {
                    self.col -= 1;
                } else if self.row > 0 {
                    self.row -= 1;
                    self.col = self.line_len(self.row);
                }
                self.update_completion();
                Reaction::Redraw
            }
            Code::Home => {
                self.col = 0;
                Reaction::Redraw
            }
            Code::Char('a') if ctrl => {
                self.col = 0;
                Reaction::Redraw
            }
            Code::End => {
                self.col = self.line_len(self.row);
                Reaction::Redraw
            }
            Code::Char('e') if ctrl => {
                self.col = self.line_len(self.row);
                Reaction::Redraw
            }
            Code::Char('u') if ctrl => {
                self.set_text("");
                self.changed()
            }
            Code::Char('w') if ctrl => {
                let g = graphemes(&self.lines[self.row]);
                let col = self.col.min(g.len());
                let mut start = col;
                while start > 0 && g[start - 1].trim().is_empty() {
                    start -= 1;
                }
                while start > 0 && !g[start - 1].trim().is_empty() {
                    start -= 1;
                }
                let (a, b) = (self.byte(self.row, start), self.byte(self.row, col));
                self.lines[self.row].replace_range(a..b, "");
                self.col = start;
                self.changed()
            }
            _ => Reaction::Ignored,
        }
    }

    fn click(&mut self, _x: u16, _y: u16, _area: Rect) -> Reaction {
        Reaction::Redraw
    }

    fn paste(&mut self, text: &str) -> Reaction {
        self.insert(text)
    }

    fn value(&self) -> Value {
        Value::String(self.text())
    }

    fn popup(&self, area: Rect, screen: Rect) -> Option<Rect> {
        let c = self.completion.as_ref()?;
        let w = c
            .items
            .iter()
            .map(|(_, l, d)| text::width(l) + if d.is_empty() { 0 } else { text::width(d) + 3 })
            .max()
            .unwrap_or(10) as u16
            + 4;
        let h = c.items.len() as u16 + 2;
        let w = w.min(screen.width);
        // Prompts usually sit at the bottom: open upwards when there is no room below.
        let y = if area.bottom() + h <= screen.bottom() {
            area.bottom()
        } else {
            area.y.saturating_sub(h)
        };
        Some(Rect::new(area.x + 1, y, w, h).intersection(screen))
    }

    fn paint_popup(&self, popup: Rect, frame: &mut Frame, p: &PaintCx) {
        let Some(c) = &self.completion else { return };
        let t = p.theme;
        let outside = super::behind(frame, popup.x, popup.y, t.bg);
        fill(frame, popup, t.fg, t.surface);
        let title = match c.kind {
            Kind::Command => "commands",
            Kind::Mention => "files",
        };
        draw_box(
            frame,
            popup,
            true,
            t.accent,
            outside,
            Some(title),
            t.muted,
            false,
        );
        for (i, (_, label, detail)) in c.items.iter().enumerate() {
            let y = popup.y + 1 + i as u16;
            if y >= popup.bottom() - 1 {
                break;
            }
            let on = i == c.hover;
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
            let mut x = popup.x + 2;
            let room = popup.right().saturating_sub(x + 1);
            x += frame.put_str(
                x,
                y,
                label,
                room,
                Style::new()
                    .fg(if on { t.fg } else { t.accent }.color())
                    .bg(bg.color()),
            );
            if !detail.is_empty() {
                let d = format!("  {detail}");
                frame.put_str(
                    x,
                    y,
                    &d,
                    popup.right().saturating_sub(x + 1),
                    Style::new().fg(t.muted.color()).bg(bg.color()),
                );
            }
        }
    }

    fn click_popup(&mut self, _x: u16, y: u16, popup: Rect) -> Reaction {
        if y == 0 || y + 1 >= popup.height {
            return Reaction::Redraw;
        }
        if let Some(c) = &mut self.completion {
            c.hover = usize::from(y - 1).min(c.items.len() - 1);
        }
        self.accept()
    }

    fn close_popup(&mut self) {
        self.completion = None;
    }

    fn call(&mut self, method: &str, args: &Value) -> crate::Result<Value> {
        match method {
            "set" => {
                self.set_text(args.as_str().unwrap_or_default());
                Ok(Value::Null)
            }
            "clear" => {
                self.set_text("");
                Ok(Value::Null)
            }
            _ => Err(crate::Error::friendly(
                format!("StagePrompt has no method \"{method}\""),
                "a prompt's text can be set or cleared",
                "use prompt.update(\"text\") or prompt.clear()",
            )),
        }
    }
}

fn cursor(frame: &mut Frame, x: u16, y: u16, t: &crate::style::Theme) {
    if let Some(c) = frame.cells.cell_mut((x, y)) {
        if c.symbol().is_empty() {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn press(p: &mut Prompt, k: &str) -> Reaction {
        p.key(&Key::parse(k).unwrap(), Rect::new(0, 0, 40, 5))
    }

    fn typed(p: &mut Prompt, s: &str) {
        for c in s.chars() {
            p.key(&Key::plain(Code::Char(c)), Rect::default());
        }
    }

    #[test]
    fn multiline_submit_and_history() {
        let mut p = Prompt::new("");
        typed(&mut p, "hello مرحبا");
        press(&mut p, "ctrl+j");
        typed(&mut p, "second");
        assert_eq!(p.text(), "hello مرحبا\nsecond");
        assert_eq!(
            press(&mut p, "enter"),
            Reaction::Emit("submit", json!("hello مرحبا\nsecond"))
        );
        assert_eq!(p.text(), "");
        typed(&mut p, "draft");
        press(&mut p, "up");
        assert_eq!(p.text(), "hello مرحبا\nsecond", "history");
        press(&mut p, "down");
        press(&mut p, "down");
        press(&mut p, "down");
        assert_eq!(p.text(), "draft", "back to the draft");
        press(&mut p, "ctrl+w");
        assert_eq!(p.text(), "");
    }

    #[test]
    fn commands_and_mentions_complete() {
        let mut p = Prompt::new("");
        p.commands = vec![
            ("help".into(), "Show help".into()),
            ("clear".into(), "Clear".into()),
        ];
        p.mentions = Some(vec![
            "src/main.py".into(),
            "src/app.py".into(),
            "README.md".into(),
        ]);
        typed(&mut p, "/he");
        assert_eq!(p.completion.as_ref().unwrap().items.len(), 1);
        press(&mut p, "tab");
        assert_eq!(p.text(), "/help ");
        typed(&mut p, "look at @app");
        press(&mut p, "enter");
        assert_eq!(p.text(), "/help look at @src/app.py ");
        assert!(matches!(
            press(&mut p, "enter"),
            Reaction::Emit("submit", _)
        ));
    }

    #[test]
    fn enter_sends_a_command_typed_in_full() {
        let commands = [
            "help",
            "helpme",
            "مُسَاعَدَة",
            "ヘルプ",
            "帮助",
            "मदद",
            "run-تشغيل",
        ];
        let prompt = || {
            let mut p = Prompt::new("");
            p.commands = commands
                .iter()
                .map(|c| (c.to_string(), String::new()))
                .collect();
            p.mentions = Some(vec!["src/app.py".into(), "src/app.py.bak".into()]);
            p
        };
        for name in commands {
            let mut p = prompt();
            typed(&mut p, &format!("/{name}"));
            assert!(p.completion.is_some(), "{name}: the list is open");
            assert_eq!(
                press(&mut p, "enter"),
                Reaction::Emit("submit", json!(format!("/{name}"))),
                "{name}: one Enter runs it"
            );
            assert_eq!(p.text(), "");
        }
        // A partial command: Enter still picks the highlighted one.
        let mut p = prompt();
        typed(&mut p, "/मद");
        assert!(matches!(
            press(&mut p, "enter"),
            Reaction::Emit("change", _)
        ));
        assert_eq!(p.text(), "/मदद ");
        // Tab always completes, even a command typed in full.
        let mut p = prompt();
        typed(&mut p, "/help");
        press(&mut p, "tab");
        assert_eq!(p.text(), "/help ");
        // "/help" is typed in full although "/helpme" also matches …
        let mut p = prompt();
        typed(&mut p, "/help");
        assert_eq!(p.completion.as_ref().unwrap().items.len(), 2);
        assert!(matches!(
            press(&mut p, "enter"),
            Reaction::Emit("submit", _)
        ));
        // … but picking "/helpme" with ↓ means that one.
        let mut p = prompt();
        typed(&mut p, "/help");
        press(&mut p, "down");
        press(&mut p, "enter");
        assert_eq!(p.text(), "/helpme ");
        // A file typed in full is sent too; a partial one is completed.
        let mut p = prompt();
        typed(&mut p, "see @src/app.py");
        assert_eq!(
            press(&mut p, "enter"),
            Reaction::Emit("submit", json!("see @src/app.py"))
        );
        let mut p = prompt();
        typed(&mut p, "見て @app");
        press(&mut p, "enter");
        assert_eq!(p.text(), "見て @src/app.py ");
    }

    #[test]
    fn wrapping_rows() {
        let mut p = Prompt::new("");
        typed(&mut p, &"x".repeat(25));
        assert_eq!(p.rows(10).len(), 3);
        assert_eq!(p.cursor_row(&p.rows(10)), 2);
    }
}
