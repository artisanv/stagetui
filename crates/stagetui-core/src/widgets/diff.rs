//! `StageDiff`: proposed changes, hunk by hunk, each accepted or rejected.

use ratatui_core::layout::Rect;
use ratatui_core::style::{Modifier, Style};
use serde_json::{Map, Value, json};
use similar::{ChangeTag, DiffOp, TextDiff};

use super::{Cx, PaintCx, Reaction, Widget, clamp_scroll, draw_box, fill, object, scrollbar};
use crate::frame::Frame;
use crate::input::{Code, Key};
use crate::style::Rgb;
use crate::text::{self, Direction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Pending,
    Accepted,
    Rejected,
}

impl Decision {
    fn name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }
}

/// One drawn line of the diff.
#[derive(Debug, Clone, PartialEq)]
enum Row {
    Header {
        hunk: usize,
        text: String,
    },
    Line {
        tag: ChangeTag,
        old: Option<usize>,
        new: Option<usize>,
        text: String,
    },
}

#[derive(Debug, Clone)]
pub struct Diff {
    pub old: String,
    pub new: String,
    pub path: Option<String>,
    pub decisions: Vec<Decision>,
    /// The hunk with the keyboard cursor.
    pub current: usize,
    pub scroll: usize,
    pub height: Option<u16>,
    rows: Vec<Row>,
    /// Hunk index of each changed op, keyed by `(old_index, new_index)`.
    ops: Vec<(DiffOp, Option<usize>)>,
}

impl Diff {
    pub fn new(old: String, new: String, path: Option<String>) -> Self {
        let mut d = Self {
            old,
            new,
            path,
            decisions: Vec::new(),
            current: 0,
            scroll: 0,
            height: None,
            rows: Vec::new(),
            ops: Vec::new(),
        };
        d.compute();
        d
    }

    fn compute(&mut self) {
        let diff = TextDiff::from_lines(&self.old, &self.new);
        let groups = diff.grouped_ops(3);
        let mut rows = Vec::new();
        let mut hunk_of = std::collections::HashMap::new();
        for (h, group) in groups.iter().enumerate() {
            let (Some(first), Some(last)) = (group.first(), group.last()) else {
                continue;
            };
            let (o0, n0) = (first.old_range().start, first.new_range().start);
            let (o1, n1) = (last.old_range().end, last.new_range().end);
            rows.push(Row::Header {
                hunk: h,
                text: format!("@@ -{},{} +{},{} @@", o0 + 1, o1 - o0, n0 + 1, n1 - n0),
            });
            for op in group {
                if op.tag() != similar::DiffTag::Equal {
                    hunk_of.insert((op.old_range().start, op.new_range().start), h);
                }
                for change in diff.iter_changes(op) {
                    rows.push(Row::Line {
                        tag: change.tag(),
                        old: change.old_index(),
                        new: change.new_index(),
                        text: change
                            .value()
                            .trim_end_matches(['\n', '\r'])
                            .replace('\t', "    "),
                    });
                }
            }
        }
        self.ops = diff
            .ops()
            .iter()
            .map(|op| {
                let h = hunk_of
                    .get(&(op.old_range().start, op.new_range().start))
                    .copied();
                (*op, h)
            })
            .collect();
        self.decisions.resize(groups.len(), Decision::Pending);
        self.decisions.truncate(groups.len());
        self.current = self.current.min(groups.len().saturating_sub(1));
        self.rows = rows;
    }

    /// The text with accepted hunks applied (pending and rejected keep the old text).
    pub fn result(&self) -> String {
        let old: Vec<&str> = self.old.split_inclusive('\n').collect();
        let new: Vec<&str> = self.new.split_inclusive('\n').collect();
        let mut out = String::new();
        for (op, hunk) in &self.ops {
            let accepted = hunk.is_some_and(|h| self.decisions.get(h) == Some(&Decision::Accepted));
            let (src, range) = if op.tag() == similar::DiffTag::Equal || !accepted {
                (&old, op.old_range())
            } else {
                (&new, op.new_range())
            };
            for line in &src[range.start.min(src.len())..range.end.min(src.len())] {
                out.push_str(line);
            }
        }
        out
    }

    fn header_row(&self, hunk: usize) -> usize {
        self.rows
            .iter()
            .position(|r| matches!(r, Row::Header { hunk: h, .. } if *h == hunk))
            .unwrap_or(0)
    }

    fn visible_rows(area: Rect) -> usize {
        usize::from(area.height.saturating_sub(2))
    }

    fn reveal(&mut self, area: Rect) {
        let row = self.header_row(self.current);
        let v = Self::visible_rows(area).max(1);
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll + v {
            self.scroll = row + 1 - v.min(row + 1);
        }
    }

    fn decide(&mut self, d: Decision) -> Reaction {
        if self.decisions.is_empty() {
            return Reaction::Ignored;
        }
        self.decisions[self.current] = d;
        let hunk = self.current;
        // Move on to the next undecided hunk.
        if let Some(next) =
            (0..self.decisions.len()).find(|i| *i > hunk && self.decisions[*i] == Decision::Pending)
        {
            self.current = next;
        }
        Reaction::Emit("decide", json!({ "hunk": hunk, "decision": d.name() }))
    }

    pub fn carry(&mut self, old: &Diff) {
        if self.old == old.old && self.new == old.new {
            self.decisions = old.decisions.clone();
            self.current = old.current;
        }
        self.scroll = old.scroll.min(self.rows.len());
    }
}

fn tint(base: Rgb, color: Rgb, dark: bool) -> Rgb {
    base.mix(color, if dark { 0.16 } else { 0.22 })
}

impl Widget for Diff {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let longest = self
            .rows
            .iter()
            .map(|r| match r {
                Row::Header { text, .. } => text::width(text) + 24,
                Row::Line { text, .. } => text::width(text),
            })
            .max()
            .unwrap_or(20);
        let w = longest as u16 + 14;
        let h = self.height.unwrap_or(self.rows.len().max(1) as u16 + 2);
        (w.min(max_width).max(20), h.max(3))
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.width < 10 || area.height < 3 {
            return;
        }
        let t = p.theme;
        let outside = super::behind(frame, area.x, area.y, t.bg);
        let bg = t.surface;
        fill(frame, area, t.fg, bg);
        let added = crate::style::accent("mint").unwrap_or(t.accent);
        let removed = crate::style::accent("rose").unwrap_or(t.accent);
        let accepted = self
            .decisions
            .iter()
            .filter(|d| **d == Decision::Accepted)
            .count();
        let title = format!(
            "{} · {} of {} accepted",
            self.path.as_deref().unwrap_or("changes"),
            accepted,
            self.decisions.len()
        );
        draw_box(
            frame,
            area,
            true,
            p.border(),
            outside,
            Some(&title),
            t.accent,
            false,
        );
        if self.rows.is_empty() {
            let line = text::layout("No changes", 20, Direction::Ltr).swap_remove(0);
            frame.put_line(
                area.x + 2,
                area.y + 1,
                area.width - 4,
                &line,
                t.muted,
                bg,
                false,
            );
            return;
        }
        let v = Self::visible_rows(area);
        let first = clamp_scroll(self.scroll as i64, self.rows.len(), v);
        let inner = Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2);
        let digits = self
            .old
            .lines()
            .count()
            .max(self.new.lines().count())
            .max(1)
            .to_string()
            .len() as u16;
        for (row, r) in self.rows.iter().skip(first).take(v).enumerate() {
            let y = inner.y + row as u16;
            match r {
                Row::Header { hunk, text: h } => {
                    let on = *hunk == self.current && p.focused;
                    let hbg = if on {
                        t.accent.mix(bg, 0.6)
                    } else {
                        bg.mix(t.border, 0.4)
                    };
                    fill(frame, Rect::new(inner.x, y, inner.width, 1), t.fg, hbg);
                    let st = Style::new().fg(t.accent.color()).bg(hbg.color());
                    let mut x = inner.x + 1;
                    x += frame.put_str(x, y, h, inner.width.saturating_sub(2), st);
                    let d = self
                        .decisions
                        .get(*hunk)
                        .copied()
                        .unwrap_or(Decision::Pending);
                    let (label, color) = match d {
                        Decision::Pending => ("pending", t.muted),
                        Decision::Accepted => ("✓ accepted", added),
                        Decision::Rejected => ("✗ rejected", removed),
                    };
                    let chip = format!(" {label} ");
                    x += 2;
                    x += frame.put_str(
                        x,
                        y,
                        &chip,
                        inner.right().saturating_sub(x),
                        Style::new().fg(t.bg.color()).bg(color.color()),
                    );
                    if on {
                        let hint = "  a accept · r reject";
                        frame.put_str(
                            x,
                            y,
                            hint,
                            inner.right().saturating_sub(x),
                            Style::new()
                                .fg(t.fg.mix(t.muted, 0.4).color())
                                .bg(hbg.color()),
                        );
                    }
                }
                Row::Line {
                    tag,
                    old,
                    new,
                    text: line,
                } => {
                    let (sign, lbg, fg) = match tag {
                        ChangeTag::Delete => {
                            ("-", tint(bg, removed, t.dark), removed.mix(t.fg, 0.35))
                        }
                        ChangeTag::Insert => ("+", tint(bg, added, t.dark), added.mix(t.fg, 0.35)),
                        ChangeTag::Equal => (" ", bg, t.fg.mix(t.muted, 0.3)),
                    };
                    fill(frame, Rect::new(inner.x, y, inner.width, 1), fg, lbg);
                    let num = |n: Option<usize>| {
                        n.map_or(" ".repeat(usize::from(digits)), |n| {
                            format!("{:>w$}", n + 1, w = usize::from(digits))
                        })
                    };
                    let gutter = format!("{} {} {} ", num(*old), num(*new), sign);
                    let gw = gutter.chars().count() as u16;
                    frame.put_str(
                        inner.x,
                        y,
                        &gutter,
                        gw,
                        Style::new().fg(t.muted.color()).bg(lbg.color()),
                    );
                    if let Some(c) = frame.cells.cell_mut((inner.x + gw - 2, y)) {
                        c.fg = fg.color();
                        c.modifier.insert(Modifier::BOLD);
                    }
                    let x = inner.x + gw;
                    let laid = text::layout(line, usize::MAX / 2, Direction::Ltr);
                    if let Some(l) = laid.first() {
                        frame.put_line(x, y, inner.right().saturating_sub(x), l, fg, lbg, false);
                    }
                }
            }
        }
        if self.rows.len() > v {
            scrollbar(
                frame,
                area.right() - 1,
                inner.y,
                v as u16,
                first,
                self.rows.len(),
                p.border(),
                t.accent,
            );
        }
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({
            "path": self.path,
            "hunks": self.decisions.len(),
            "decisions": self.decisions.iter().map(|d| d.name()).collect::<Vec<_>>(),
            "current": self.current,
        }))
    }

    fn focusable(&self) -> bool {
        true
    }

    fn key(&mut self, key: &Key, area: Rect) -> Reaction {
        let n = self.decisions.len();
        let r = match key.code {
            Code::Char('a') | Code::Char('y') => self.decide(Decision::Accepted),
            Code::Char('r') | Code::Char('n') if !key.mods.ctrl => self.decide(Decision::Rejected),
            Code::Char('A') => {
                self.decisions
                    .iter_mut()
                    .for_each(|d| *d = Decision::Accepted);
                Reaction::Emit("decide", json!({ "hunk": "all", "decision": "accepted" }))
            }
            Code::Tab if key.mods.shift => return Reaction::Ignored,
            Code::Char('j') | Code::Char(']') if n > 0 => {
                self.current = (self.current + 1).min(n - 1);
                Reaction::Redraw
            }
            Code::Char('k') | Code::Char('[') => {
                self.current = self.current.saturating_sub(1);
                Reaction::Redraw
            }
            Code::Up | Code::Down | Code::PageUp | Code::PageDown => {
                let page = Self::visible_rows(area).max(1) as i32;
                let d = match key.code {
                    Code::Up => -1,
                    Code::Down => 1,
                    Code::PageUp => -page,
                    _ => page,
                };
                return if self.scroll_by(d, area) {
                    Reaction::Redraw
                } else {
                    Reaction::Ignored
                };
            }
            _ => return Reaction::Ignored,
        };
        if key.code == Code::Char('A')
            && let Some(next) = (0..n).find(|i| self.decisions[*i] == Decision::Pending)
        {
            self.current = next;
        }
        self.reveal(area);
        r
    }

    fn click(&mut self, _x: u16, y: u16, area: Rect) -> Reaction {
        let v = Self::visible_rows(area);
        let first = clamp_scroll(self.scroll as i64, self.rows.len(), v);
        let idx = first + usize::from(y.saturating_sub(1));
        // Clicking a line selects its hunk.
        let hunk = self.rows[..=idx.min(self.rows.len().saturating_sub(1))]
            .iter()
            .rev()
            .find_map(|r| match r {
                Row::Header { hunk, .. } => Some(*hunk),
                Row::Line { .. } => None,
            });
        if let Some(h) = hunk {
            self.current = h;
        }
        Reaction::Redraw
    }

    fn scroll_by(&mut self, rows: i32, area: Rect) -> bool {
        let next = clamp_scroll(
            self.scroll as i64 + i64::from(rows),
            self.rows.len(),
            Self::visible_rows(area),
        );
        let moved = next != self.scroll;
        self.scroll = next;
        moved
    }

    fn call(&mut self, method: &str, args: &Value) -> crate::Result<Value> {
        let d = match method {
            "accept_all" => Decision::Accepted,
            "reject_all" => Decision::Rejected,
            "decide" => {
                let h = args.get("hunk").and_then(Value::as_u64).unwrap_or(0) as usize;
                let d = match args.get("decision").and_then(Value::as_str) {
                    Some("accepted" | "accept") => Decision::Accepted,
                    Some("rejected" | "reject") => Decision::Rejected,
                    _ => Decision::Pending,
                };
                if let Some(slot) = self.decisions.get_mut(h) {
                    *slot = d;
                }
                return Ok(self.value());
            }
            _ => {
                return Err(crate::Error::friendly(
                    format!("StageDiff has no method \"{method}\""),
                    "a diff can accept or reject hunks",
                    "use diff.accept_all(), diff.reject_all() or diff.decide(hunk, \"accept\")",
                ));
            }
        };
        self.decisions.iter_mut().for_each(|x| *x = d);
        Ok(self.value())
    }

    fn value(&self) -> Value {
        json!({
            "decisions": self.decisions.iter().map(|d| d.name()).collect::<Vec<_>>(),
            "pending": self.decisions.iter().filter(|d| **d == Decision::Pending).count(),
            "result": self.result(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n";
    const NEW: &str = "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nK\nl\nm\n";

    #[test]
    fn hunks_and_results() {
        let mut d = Diff::new(OLD.into(), NEW.into(), Some("x.txt".into()));
        assert_eq!(d.decisions.len(), 2, "two separate hunks");
        assert_eq!(d.result(), OLD, "nothing accepted yet");
        d.decisions[0] = Decision::Accepted;
        assert_eq!(d.result(), OLD.replace("\nb\n", "\nB\n"));
        d.decisions[1] = Decision::Accepted;
        assert_eq!(d.result(), NEW);
        d.decisions[0] = Decision::Rejected;
        assert_eq!(d.result(), "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nK\nl\nm\n");
    }

    #[test]
    fn keys_decide_and_move() {
        let mut d = Diff::new(OLD.into(), NEW.into(), None);
        let area = Rect::new(0, 0, 60, 20);
        let r = d.key(&Key::plain(Code::Char('a')), area);
        assert!(matches!(r, Reaction::Emit("decide", _)));
        assert_eq!(d.current, 1, "moves to the next pending hunk");
        d.key(&Key::plain(Code::Char('r')), area);
        assert_eq!(d.decisions, vec![Decision::Accepted, Decision::Rejected]);
        assert_eq!(d.value()["pending"], 0);
    }
}
