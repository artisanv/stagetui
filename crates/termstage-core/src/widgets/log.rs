//! `StageLog`: a fast, filterable log tail (thousands of lines per second).

use std::collections::VecDeque;

use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use serde_json::{Map, Value, json};

use super::{Cx, PaintCx, Reaction, Widget, clamp_scroll, object, scrollbar};
use crate::frame::Frame;
use crate::input::{Code, Key};
use crate::style::{Rgb, Theme};
use crate::text::{self, Direction};

#[derive(Debug, Clone)]
pub struct Log {
    pub lines: VecDeque<String>,
    pub max_lines: usize,
    pub filter: String,
    pub follow: bool,
    pub scroll: usize,
    pub height: Option<u16>,
    /// Lines dropped from the top since the start.
    pub dropped: usize,
}

impl Log {
    pub fn new(text: &str, max_lines: usize) -> Self {
        let mut l = Self {
            lines: VecDeque::new(),
            max_lines: max_lines.max(10),
            filter: String::new(),
            follow: true,
            scroll: 0,
            height: None,
            dropped: 0,
        };
        l.push(text);
        l
    }

    pub fn push(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        for line in text.trim_end_matches('\n').split('\n') {
            self.lines
                .push_back(line.trim_end_matches('\r').replace('\t', "    "));
        }
        while self.lines.len() > self.max_lines {
            self.lines.pop_front();
            self.dropped += 1;
        }
    }

    fn shown(&self) -> Vec<&str> {
        let f = self.filter.to_lowercase();
        self.lines
            .iter()
            .filter(|l| f.is_empty() || l.to_lowercase().contains(&f))
            .map(String::as_str)
            .collect()
    }

    fn offset(&self, total: usize, visible: usize) -> usize {
        if self.follow {
            total.saturating_sub(visible)
        } else {
            clamp_scroll(self.scroll as i64, total, visible)
        }
    }

    pub fn carry(&mut self, old: &Log) {
        self.follow = old.follow;
        self.scroll = old.scroll;
    }
}

fn level_color(line: &str, t: &Theme) -> Rgb {
    let head: String = line.chars().take(40).collect::<String>().to_uppercase();
    let has = |w: &str| head.contains(w);
    if has("ERROR") || has("FATAL") || has("PANIC") || has("CRITICAL") {
        crate::style::accent("rose").unwrap_or(t.fg)
    } else if has("WARN") {
        crate::style::accent("amber").unwrap_or(t.fg)
    } else if has("DEBUG") || has("TRACE") {
        t.muted
    } else if has("OK") || has("PASS") || has("SUCCESS") {
        crate::style::accent("mint").unwrap_or(t.fg)
    } else {
        t.fg.mix(t.muted, 0.2)
    }
}

impl Widget for Log {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let h = self.height.unwrap_or(self.lines.len().clamp(1, 20) as u16);
        (max_width.min(100), h.max(1))
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.is_empty() {
            return;
        }
        let t = p.theme;
        let bg = super::behind(frame, area.x, area.y, t.bg);
        let shown = self.shown();
        let mut rows = usize::from(area.height);
        let filter_row = !self.filter.is_empty();
        if filter_row {
            rows = rows.saturating_sub(1);
        }
        let first = self.offset(shown.len(), rows);
        let room = area.width.saturating_sub(1);
        for (row, line) in shown.iter().skip(first).take(rows).enumerate() {
            let y = area.y + row as u16;
            let laid = text::layout(line, usize::MAX / 2, Direction::Ltr);
            if let Some(l) = laid.first() {
                frame.put_line(area.x, y, room, l, level_color(line, t), bg, false);
            }
        }
        if filter_row {
            let y = area.bottom() - 1;
            let label = format!(
                " filter: {}  ({} of {} lines) ",
                self.filter,
                shown.len(),
                self.lines.len()
            );
            frame.put_str(
                area.x,
                y,
                &label,
                area.width,
                Style::new().fg(t.bg.color()).bg(t.accent.color()),
            );
        }
        if shown.len() > rows {
            scrollbar(
                frame,
                area.right() - 1,
                area.y,
                rows as u16,
                first,
                shown.len(),
                t.border,
                t.accent,
            );
        }
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({
            "lines": self.lines.len(),
            "dropped": self.dropped,
            "filter": self.filter,
            "follow": self.follow,
            "last": self.lines.back(),
        }))
    }

    fn focusable(&self) -> bool {
        true
    }

    fn key(&mut self, key: &Key, area: Rect) -> Reaction {
        let page = i32::from(area.height.saturating_sub(1).max(1));
        let d = match key.code {
            Code::Up => -1,
            Code::Down => 1,
            Code::PageUp => -page,
            Code::PageDown => page,
            Code::Home => i32::MIN / 2,
            Code::End => {
                self.follow = true;
                return Reaction::Redraw;
            }
            _ => return Reaction::Ignored,
        };
        if self.scroll_by(d, area) {
            Reaction::Redraw
        } else {
            Reaction::Ignored
        }
    }

    fn scroll_by(&mut self, rows: i32, area: Rect) -> bool {
        let total = self.shown().len();
        let visible = usize::from(area.height);
        let cur = self.offset(total, visible);
        let next = clamp_scroll(cur as i64 + i64::from(rows), total, visible);
        self.follow = next + visible >= total;
        self.scroll = next;
        next != cur
    }

    fn call(&mut self, method: &str, args: &Value) -> crate::Result<Value> {
        match method {
            "append" => match args {
                Value::String(s) => self.push(s),
                Value::Array(a) => {
                    for v in a {
                        self.push(v.as_str().unwrap_or_default());
                    }
                }
                _ => {}
            },
            "clear" => {
                self.lines.clear();
                self.follow = true;
            }
            "filter" => {
                self.filter = args.as_str().unwrap_or_default().to_string();
                self.follow = true;
            }
            _ => {
                return Err(crate::Error::friendly(
                    format!("StageLog has no method \"{method}\""),
                    "a log can append lines, clear, and filter",
                    "use log.append(\"line\"), log.clear() or log.filter(\"error\")",
                ));
            }
        }
        Ok(Value::Null)
    }

    fn value(&self) -> Value {
        json!(self.lines.iter().collect::<Vec<_>>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_and_filter() {
        let mut l = Log::new("", 100);
        for i in 0..250 {
            l.call("append", &json!(format!("INFO line {i}\nERROR boom {i}")))
                .unwrap();
        }
        assert_eq!(l.lines.len(), 100);
        assert_eq!(l.dropped, 400);
        l.call("filter", &json!("error")).unwrap();
        assert_eq!(l.shown().len(), 50);
        assert!(l.shown().iter().all(|s| s.starts_with("ERROR")));
    }
}
