//! `StageToolCall` (a collapsible "the agent ran a tool" card) and
//! `StagePermission` (approve / always allow / deny).

use std::time::Duration;

use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use serde_json::{Map, Value, json};

use super::markdown::{Look, MdLine, Role, Seg, paint_lines};
use super::{Cx, PaintCx, Reaction, Widget, draw_box, fill, object};
use crate::frame::Frame;
use crate::input::{Code, Key};
use crate::style::{Rgb, Theme};
use crate::text::{self, Direction};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const MAX_OUTPUT_LINES: usize = 30;

/// A tool the agent called.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub name: String,
    pub args: Value,
    /// `"running"`, `"done"` or `"error"`.
    pub status: String,
    pub output: String,
    pub expanded: bool,
}

fn seg(text: impl Into<String>, role: Role, bold: bool) -> Seg {
    Seg {
        text: text.into(),
        role,
        look: Look {
            bold,
            ..Look::default()
        },
    }
}

/// One line summary of JSON arguments: `path="a.py", lines=3`.
pub fn summary(args: &Value, max: usize) -> String {
    let s = match args {
        Value::Object(m) => m
            .iter()
            .map(|(k, v)| match v {
                Value::String(s) => format!("{k}={s:?}"),
                other => format!("{k}={other}"),
            })
            .collect::<Vec<_>>()
            .join(", "),
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let s = s.replace('\n', " ");
    if s.chars().count() > max {
        format!(
            "{}…",
            s.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    } else {
        s
    }
}

impl ToolCall {
    pub fn status_color(&self, t: &Theme) -> Rgb {
        match self.status.as_str() {
            "done" | "ok" => crate::style::accent("mint").unwrap_or(t.accent),
            "error" | "failed" => crate::style::accent("rose").unwrap_or(t.accent),
            _ => t.accent,
        }
    }

    fn icon(&self, now: Duration) -> &'static str {
        match self.status.as_str() {
            "done" | "ok" => "✓",
            "error" | "failed" => "✗",
            _ => SPINNER[(now.as_millis() / 80) as usize % SPINNER.len()],
        }
    }

    pub fn running(&self) -> bool {
        !matches!(self.status.as_str(), "done" | "ok" | "error" | "failed")
    }

    /// The card as text lines `width` cells wide.
    pub fn lines(&self, width: u16, t: &Theme, now: Duration) -> Vec<MdLine> {
        let color = self.status_color(t);
        let room = usize::from(width)
            .saturating_sub(text::width(&self.name) + 6)
            .max(8);
        let mut out = vec![MdLine {
            segs: vec![
                seg(format!("{} ", self.icon(now)), Role::Raw(color), true),
                seg(self.name.clone(), Role::Text, true),
                seg(
                    format!("  {}", summary(&self.args, room)),
                    Role::Muted,
                    false,
                ),
                seg(
                    if self.expanded { "  ▾" } else { "  ▸" },
                    Role::Muted,
                    false,
                ),
            ],
            ..MdLine::default()
        }];
        if !self.expanded {
            return out;
        }
        let body = |text: &str, role: Role, out: &mut Vec<MdLine>| {
            let lines: Vec<&str> = text.lines().collect();
            for l in lines.iter().take(MAX_OUTPUT_LINES) {
                let cut: String =
                    text::split_graphemes(l, usize::from(width.saturating_sub(4)).max(1))
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                out.push(MdLine {
                    indent: 2,
                    marker: Some(("│".into(), Role::Raw(color))),
                    segs: vec![seg(cut, role, false)],
                    ..MdLine::default()
                });
            }
            if lines.len() > MAX_OUTPUT_LINES {
                out.push(MdLine {
                    indent: 2,
                    marker: Some(("│".into(), Role::Raw(color))),
                    segs: vec![seg(
                        format!("… {} more lines", lines.len() - MAX_OUTPUT_LINES),
                        Role::Muted,
                        false,
                    )],
                    ..MdLine::default()
                });
            }
        };
        if !self.args.is_null() {
            let pretty = serde_json::to_string_pretty(&self.args).unwrap_or_default();
            body(&pretty, Role::Muted, &mut out);
        }
        if !self.output.is_empty() {
            body(&self.output, Role::Text, &mut out);
        } else if self.running() {
            body("running…", Role::Muted, &mut out);
        }
        out
    }

    pub fn call(&mut self, method: &str, args: &Value) -> crate::Result<Value> {
        match method {
            "finish" | "set" => {
                if let Some(s) = args.get("status").and_then(Value::as_str) {
                    self.status = s.to_string();
                }
                if let Some(o) = args.get("output").and_then(Value::as_str) {
                    self.output = o.to_string();
                }
                if let Some(e) = args.get("expanded").and_then(Value::as_bool) {
                    self.expanded = e;
                }
                Ok(Value::Null)
            }
            "append" => {
                self.output.push_str(args.as_str().unwrap_or_default());
                Ok(Value::Null)
            }
            _ => Err(crate::Error::friendly(
                format!("StageToolCall has no method \"{method}\""),
                "a tool call can be finished or get more output",
                "use tool.finish(output, ok=True) or tool.append(\"more output\")",
            )),
        }
    }

    pub fn describe(&self) -> Value {
        json!({ "name": self.name, "args": self.args, "status": self.status, "output": self.output, "expanded": self.expanded })
    }
}

impl Widget for ToolCall {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let t = crate::style::THEMES[0];
        let h = self.lines(max_width, &t, Duration::ZERO).len();
        (max_width.min(90), h as u16)
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.is_empty() {
            return;
        }
        let bg = super::behind(frame, area.x, area.y, p.theme.bg);
        let mut lines = self.lines(area.width, p.theme, p.now);
        if p.focused
            && let Some(first) = lines.first_mut()
        {
            first.segs[1].role = Role::Accent;
        }
        paint_lines(&lines, 0, area, frame, p.theme, bg);
    }

    fn describe(&self) -> Map<String, Value> {
        object(ToolCall::describe(self))
    }

    fn focusable(&self) -> bool {
        true
    }

    fn key(&mut self, key: &Key, _area: Rect) -> Reaction {
        if key.is(Code::Enter) || key.is(Code::Char(' ')) {
            self.expanded = !self.expanded;
            Reaction::Emit("toggle", Value::Bool(self.expanded))
        } else {
            Reaction::Ignored
        }
    }

    fn click(&mut self, _x: u16, y: u16, _area: Rect) -> Reaction {
        if y == 0 {
            self.expanded = !self.expanded;
            return Reaction::Emit("toggle", Value::Bool(self.expanded));
        }
        Reaction::Redraw
    }

    fn animating(&self) -> bool {
        self.running()
    }

    fn call(&mut self, method: &str, args: &Value) -> crate::Result<Value> {
        ToolCall::call(self, method, args)
    }

    fn value(&self) -> Value {
        ToolCall::describe(self)
    }
}

/// "The agent wants to run X — allow?"
#[derive(Debug, Clone)]
pub struct Permission {
    /// Draw its own frame (off inside a modal, which has one).
    pub boxed: bool,
    pub question: String,
    pub tool: Option<String>,
    pub args: Value,
    pub choice: usize,
    pub decided: Option<&'static str>,
}

const CHOICES: [(&str, &str, &str); 3] = [
    ("y", "Allow", "allow"),
    ("a", "Always allow", "always"),
    ("n", "Deny", "deny"),
];

impl Permission {
    fn buttons(&self) -> Vec<(u16, u16)> {
        let mut x = 2u16;
        CHOICES
            .iter()
            .map(|(k, label, _)| {
                let w = text::width(&format!(" {k}  {label} ")) as u16;
                let r = (x, w);
                x += w + 2;
                r
            })
            .collect()
    }

    fn decide(&mut self, i: usize) -> Reaction {
        self.choice = i;
        let v = CHOICES[i].2;
        self.decided = Some(v);
        Reaction::Emit("decide", Value::String(v.into()))
    }
}

impl Widget for Permission {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let q = text::width(&self.question) + 8;
        let b: u16 = self.buttons().last().map_or(30, |(x, w)| x + w + 2);
        let w = (q as u16).max(b).max(40);
        let lines = text::layout(
            &self.question,
            usize::from(w.min(max_width).saturating_sub(6)),
            Direction::Ltr,
        )
        .len();
        (
            w.min(max_width),
            (lines + 5 + usize::from(self.tool.is_some())) as u16,
        )
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.width < 10 || area.height < 4 {
            return;
        }
        let t = p.theme;
        let amber = crate::style::accent("amber").unwrap_or(t.accent);
        let outside = super::behind(frame, area.x, area.y, t.bg);
        fill(frame, area, t.fg, t.surface);
        let border = if p.focused {
            amber
        } else {
            amber.mix(t.border, 0.4)
        };
        if self.boxed {
            draw_box(
                frame,
                area,
                true,
                border,
                outside,
                Some("▲ Permission needed"),
                amber,
                p.rtl,
            );
        }
        let mut y = area.y + 1;
        let room = area.width.saturating_sub(4);
        for line in text::layout(&self.question, usize::from(room), Direction::Ltr) {
            if y >= area.bottom() - 1 {
                break;
            }
            let x = if line.dir == Direction::Rtl {
                area.right() - 2 - (line.width as u16).min(room)
            } else {
                area.x + 2
            };
            frame.put_line(x, y, room, &line, t.fg, t.surface, true);
            y += 1;
        }
        if let Some(tool) = &self.tool {
            let s = format!("{tool}  {}", summary(&self.args, usize::from(room)));
            let line = text::layout(&s, usize::from(room), Direction::Ltr).swap_remove(0);
            frame.put_line(
                area.x + 2,
                y,
                room,
                &line,
                t.accent.mix(t.fg, 0.3),
                t.surface,
                false,
            );
            y += 1;
        }
        y += 1;
        if y >= area.bottom() - 1 {
            return;
        }
        for (i, ((x, w), (k, label, _))) in self.buttons().into_iter().zip(CHOICES).enumerate() {
            let on = i == self.choice;
            let color = if i == 2 {
                crate::style::accent("rose").unwrap_or(t.accent)
            } else {
                t.accent
            };
            let (fg, bg) = if on {
                (t.bg, color)
            } else {
                (t.fg, t.surface.mix(t.border, 0.5))
            };
            let label = format!(" {k}  {label} ");
            frame.put_str(
                area.x + x,
                y,
                &label,
                w,
                Style::new().fg(fg.color()).bg(bg.color()),
            );
        }
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({ "question": self.question, "tool": self.tool, "decided": self.decided }))
    }

    fn focusable(&self) -> bool {
        true
    }

    fn key(&mut self, key: &Key, _area: Rect) -> Reaction {
        match key.code {
            Code::Char('y') => self.decide(0),
            Code::Char('a') => self.decide(1),
            Code::Char('n') => self.decide(2),
            Code::Left => {
                self.choice = self.choice.saturating_sub(1);
                Reaction::Redraw
            }
            Code::Right => {
                self.choice = (self.choice + 1).min(2);
                Reaction::Redraw
            }
            Code::Enter | Code::Char(' ') => self.decide(self.choice),
            _ => Reaction::Ignored,
        }
    }

    fn click(&mut self, x: u16, y: u16, area: Rect) -> Reaction {
        if y + 2 != area.height {
            return Reaction::Redraw;
        }
        for (i, (bx, w)) in self.buttons().into_iter().enumerate() {
            if x >= bx && x < bx + w {
                return self.decide(i);
            }
        }
        Reaction::Redraw
    }

    fn value(&self) -> Value {
        json!(self.decided)
    }
}

/// Parse `{"name", "args", "status", "output"}` into a tool call.
pub fn tool_from(v: &Value) -> ToolCall {
    ToolCall {
        name: v
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("tool")
            .to_string(),
        args: v.get("args").cloned().unwrap_or(Value::Null),
        status: v
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("running")
            .to_string(),
        output: v
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        expanded: v.get("expanded").and_then(Value::as_bool).unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_card_lines() {
        let t = crate::style::THEMES[0];
        let mut tc = tool_from(&json!({"name": "read_file", "args": {"path": "main.py"}}));
        assert_eq!(tc.lines(60, &t, Duration::ZERO).len(), 1, "collapsed");
        assert!(tc.running());
        tc.call(
            "finish",
            &json!({"status": "done", "output": "line 1\nline 2"}),
        )
        .unwrap();
        tc.expanded = true;
        let lines = tc.lines(60, &t, Duration::ZERO);
        assert!(
            lines[0]
                .plain()
                .starts_with("✓ read_file  path=\"main.py\"")
        );
        assert!(lines.iter().any(|l| l.plain() == "line 2"));
    }

    #[test]
    fn permission_keys() {
        let mut p = Permission {
            boxed: true,
            question: "Run rm -rf build?".into(),
            tool: Some("bash".into()),
            args: json!({"cmd": "rm -rf build"}),
            choice: 0,
            decided: None,
        };
        assert_eq!(
            p.key(&Key::plain(Code::Char('n')), Rect::default()),
            Reaction::Emit("decide", json!("deny"))
        );
        p.key(&Key::plain(Code::Left), Rect::default());
        assert_eq!(
            p.key(&Key::plain(Code::Enter), Rect::default()),
            Reaction::Emit("decide", json!("always"))
        );
    }
}
