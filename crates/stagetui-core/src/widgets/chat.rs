//! `StageChat`: a conversation — you, the assistant, tool calls — with
//! streaming Markdown, auto-scroll, selection and copy.

use std::cell::RefCell;
use std::time::Duration;

use ratatui_core::layout::Rect;
use serde_json::{Map, Value, json};

use super::markdown::{Look, MdCache, MdLine, Role, Seg, paint_lines};
use super::tool_call::{ToolCall, tool_from};
use super::{Cx, PaintCx, Reaction, Widget, clamp_scroll, object, scrollbar};
use crate::frame::Frame;
use crate::input::{Code, Key};
use crate::style::Theme;
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct Message {
    pub role: String,
    pub name: Option<String>,
    pub text: String,
    pub tool: Option<ToolCall>,
    cache: RefCell<MdCache>,
}

impl Message {
    fn new(role: &str, text: &str, name: Option<String>, tool: Option<ToolCall>) -> Self {
        Self {
            role: role.to_string(),
            name,
            text: text.to_string(),
            tool,
            cache: RefCell::new(MdCache::default()),
        }
    }

    fn header(&self) -> Option<MdLine> {
        let (icon, label, role) = match self.role.as_str() {
            "user" => ("›", "You", Role::Accent),
            "assistant" => ("✦", "Assistant", Role::Heading),
            "system" => ("ℹ", "System", Role::Muted),
            "tool" => return None,
            other => ("•", other, Role::Muted),
        };
        let label = self.name.clone().unwrap_or_else(|| label.to_string());
        Some(MdLine {
            segs: vec![Seg {
                text: format!("{icon} {label}"),
                role,
                look: Look {
                    bold: true,
                    ..Look::default()
                },
            }],
            ..MdLine::default()
        })
    }

    fn lines(&self, width: u16, t: &Theme, now: Duration) -> Vec<MdLine> {
        if let Some(tool) = &self.tool {
            return tool.lines(width, t, now);
        }
        let mut out: Vec<MdLine> = self.header().into_iter().collect();
        let body_w = width.saturating_sub(2).max(4);
        let mut body = self.cache.borrow_mut().lines(&self.text, body_w, t.dark);
        if body.is_empty() && self.role == "assistant" {
            body.push(MdLine {
                segs: vec![Seg {
                    text: "…".into(),
                    role: Role::Muted,
                    look: Look::default(),
                }],
                ..MdLine::default()
            });
        }
        for mut l in body {
            l.indent += 2;
            if self.role == "system" {
                for s in &mut l.segs {
                    s.role = Role::Muted;
                }
            }
            out.push(l);
        }
        out
    }
}

#[derive(Debug, Clone, Default)]
pub struct Chat {
    pub messages: Vec<Message>,
    pub selected: Option<usize>,
    pub follow: bool,
    pub scroll: usize,
    pub height: Option<u16>,
}

impl Chat {
    pub fn new(content: &Value) -> Result<Self> {
        let mut c = Self {
            follow: true,
            ..Self::default()
        };
        if let Value::Array(items) = content {
            for m in items {
                c.add(m)?;
            }
        }
        Ok(c)
    }

    fn add(&mut self, m: &Value) -> Result<usize> {
        let (role, text) = match m {
            Value::Object(o) => (
                o.get("role").and_then(Value::as_str).unwrap_or("assistant"),
                o.get("text").and_then(Value::as_str).unwrap_or_default(),
            ),
            Value::String(s) => ("user", s.as_str()),
            _ => {
                return Err(Error::friendly(
                    "StageChat got a message it can't read",
                    "messages are {\"role\": ..., \"text\": ...}",
                    "use chat.add(\"assistant\", \"Hello\")",
                ));
            }
        };
        let name = m.get("name").and_then(Value::as_str).map(str::to_string);
        let tool = (role == "tool").then(|| tool_from(m.get("tool").unwrap_or(m)));
        self.messages.push(Message::new(role, text, name, tool));
        Ok(self.messages.len() - 1)
    }

    /// All lines plus, for each, the message it belongs to.
    fn layout(&self, width: u16, t: &Theme, now: Duration) -> (Vec<MdLine>, Vec<Option<usize>>) {
        let mut lines = Vec::new();
        let mut owner = Vec::new();
        for (i, m) in self.messages.iter().enumerate() {
            if i > 0 {
                lines.push(MdLine::default());
                owner.push(None);
            }
            let ml = m.lines(width, t, now);
            owner.extend(std::iter::repeat_n(Some(i), ml.len()));
            lines.extend(ml);
        }
        (lines, owner)
    }

    fn offset(&self, total: usize, visible: usize) -> usize {
        if self.follow {
            total.saturating_sub(visible)
        } else {
            clamp_scroll(self.scroll as i64, total, visible)
        }
    }

    fn message(&mut self, args: &Value) -> Result<&mut Message> {
        let n = self.messages.len();
        let i = args
            .get("index")
            .and_then(Value::as_i64)
            .map_or(n.saturating_sub(1), |i| {
                if i < 0 {
                    (n as i64 + i).max(0) as usize
                } else {
                    i as usize
                }
            });
        self.messages.get_mut(i).ok_or_else(|| {
            Error::friendly(
                "That chat message doesn't exist",
                format!("the chat has {n} messages"),
                "add a message first with chat.add(role, text)",
            )
        })
    }

    pub fn carry(&mut self, old: &Chat) {
        if self.messages.is_empty() {
            self.messages = old.messages.clone();
        }
        self.selected = old.selected;
        self.follow = old.follow;
        self.scroll = old.scroll;
    }
}

impl Widget for Chat {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let t = crate::style::THEMES[0];
        let h = self.height.unwrap_or_else(|| {
            (self.layout(max_width, &t, Duration::ZERO).0.len() as u16).clamp(3, 40)
        });
        (max_width, h)
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.is_empty() {
            return;
        }
        let t = p.theme;
        let bg = super::behind(frame, area.x, area.y, t.bg);
        let text_area = Rect {
            width: area.width.saturating_sub(1),
            ..area
        };
        let (mut lines, owner) = self.layout(text_area.width, t, p.now);
        if self.messages.is_empty() {
            let hint = MdLine {
                segs: vec![Seg {
                    text: "No messages yet.".into(),
                    role: Role::Muted,
                    look: Look::default(),
                }],
                ..MdLine::default()
            };
            paint_lines(&[hint], 0, text_area, frame, t, bg);
            return;
        }
        // The selected message gets a bar in the margin.
        if let Some(sel) = self.selected {
            for (l, o) in lines.iter_mut().zip(&owner) {
                if *o == Some(sel) && l.marker.is_none() && l.indent >= 2 {
                    l.marker = Some(("▌".into(), Role::Accent));
                }
            }
        }
        let first = self.offset(lines.len(), usize::from(area.height));
        paint_lines(&lines, first, text_area, frame, t, bg);
        if lines.len() > usize::from(area.height) {
            scrollbar(
                frame,
                area.right() - 1,
                area.y,
                area.height,
                first,
                lines.len(),
                t.border,
                t.accent,
            );
        }
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({
            "messages": self.messages.len(),
            "selected": self.selected,
            "follow": self.follow,
            "last": self.messages.last().map(|m| json!({"role": m.role, "text": m.text})),
        }))
    }

    fn focusable(&self) -> bool {
        true
    }

    fn key(&mut self, key: &Key, area: Rect) -> Reaction {
        let n = self.messages.len();
        if n == 0 {
            return Reaction::Ignored;
        }
        match key.code {
            Code::Up => {
                self.selected = Some(self.selected.map_or(n - 1, |s| s.saturating_sub(1)));
                self.follow = false;
                self.reveal(area);
                Reaction::Redraw
            }
            Code::Down => {
                let next = self.selected.map_or(n - 1, |s| (s + 1).min(n - 1));
                self.selected = Some(next);
                self.reveal(area);
                Reaction::Redraw
            }
            Code::PageUp | Code::PageDown => {
                let page = i32::from(area.height.saturating_sub(1).max(1));
                self.scroll_by(
                    if key.code == Code::PageUp {
                        -page
                    } else {
                        page
                    },
                    area,
                );
                Reaction::Redraw
            }
            Code::End => {
                self.follow = true;
                self.selected = None;
                Reaction::Redraw
            }
            Code::Home => {
                self.follow = false;
                self.scroll = 0;
                Reaction::Redraw
            }
            Code::Char('y') | Code::Char('c') if !key.mods.ctrl => match self.selected {
                Some(s) => Reaction::Copy(self.messages[s].text.clone()),
                None => Reaction::Ignored,
            },
            Code::Enter | Code::Char(' ') => {
                if let Some(s) = self.selected
                    && let Some(tool) = &mut self.messages[s].tool
                {
                    tool.expanded = !tool.expanded;
                    return Reaction::Redraw;
                }
                Reaction::Ignored
            }
            Code::Esc => {
                if self.selected.take().is_some() {
                    self.follow = true;
                    Reaction::Redraw
                } else {
                    Reaction::Ignored
                }
            }
            _ => Reaction::Ignored,
        }
    }

    fn click(&mut self, _x: u16, y: u16, area: Rect) -> Reaction {
        let t = crate::style::THEMES[0];
        let (lines, owner) = self.layout(area.width.saturating_sub(1), &t, Duration::ZERO);
        let first = self.offset(lines.len(), usize::from(area.height));
        let row = first + usize::from(y);
        if let Some(&Some(i)) = owner.get(row) {
            // A tool call's first line is its header: one click opens or closes it,
            // focused or not (like a `StageToolCall` on its own). Its output only selects.
            let header = owner.iter().position(|o| *o == Some(i)) == Some(row);
            if header && let Some(tool) = &mut self.messages[i].tool {
                tool.expanded = !tool.expanded;
            }
            self.selected = Some(i);
            self.follow = false;
            self.scroll = first;
        }
        Reaction::Redraw
    }

    fn scroll_by(&mut self, rows: i32, area: Rect) -> bool {
        let t = crate::style::THEMES[0];
        let total = self
            .layout(area.width.saturating_sub(1), &t, Duration::ZERO)
            .0
            .len();
        let visible = usize::from(area.height);
        let cur = self.offset(total, visible);
        let next = clamp_scroll(cur as i64 + i64::from(rows), total, visible);
        self.follow = next + visible >= total;
        self.scroll = next;
        next != cur
    }

    fn animating(&self) -> bool {
        self.messages
            .iter()
            .any(|m| m.tool.as_ref().is_some_and(ToolCall::running))
    }

    fn call(&mut self, method: &str, args: &Value) -> Result<Value> {
        match method {
            "add" => {
                let i = self.add(args)?;
                Ok(json!(i))
            }
            "append" => {
                let text = args
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let m = self.message(args)?;
                match &mut m.tool {
                    Some(t) => t.output.push_str(&text),
                    None => m.text.push_str(&text),
                }
                Ok(Value::Null)
            }
            "set" => {
                let text = args
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                self.message(args)?.text = text;
                Ok(Value::Null)
            }
            "tool" => {
                let m = self.message(args)?;
                match &mut m.tool {
                    Some(t) => t.call("finish", args),
                    None => Err(Error::friendly(
                        "That message is not a tool call",
                        "only tool messages have a status and output",
                        "add one with chat.add_tool(name, args)",
                    )),
                }
            }
            "clear" => {
                self.messages.clear();
                self.selected = None;
                self.follow = true;
                Ok(Value::Null)
            }
            _ => Err(Error::friendly(
                format!("StageChat has no method \"{method}\""),
                "a chat can add, append to, change and clear messages",
                "use chat.add(role, text), message.append(chunk), chat.clear()",
            )),
        }
    }

    fn value(&self) -> Value {
        json!(
            self.messages
                .iter()
                .map(|m| match &m.tool {
                    Some(t) => json!({"role": "tool", "tool": t.describe()}),
                    None => json!({"role": m.role, "text": m.text}),
                })
                .collect::<Vec<_>>()
        )
    }
}

impl Chat {
    fn reveal(&mut self, area: Rect) {
        let Some(sel) = self.selected else { return };
        let t = crate::style::THEMES[0];
        let (lines, owner) = self.layout(area.width.saturating_sub(1), &t, Duration::ZERO);
        let Some(start) = owner.iter().position(|o| *o == Some(sel)) else {
            return;
        };
        let visible = usize::from(area.height);
        let cur = self.offset(lines.len(), visible);
        self.follow = false;
        if start < cur {
            self.scroll = start;
        } else if start >= cur + visible {
            self.scroll = start + 1 - visible.min(start + 1);
        } else {
            self.scroll = cur;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_messages_and_tools() {
        let mut c = Chat::new(&json!([{"role": "user", "text": "Fix the bug"}])).unwrap();
        let i = c
            .call("add", &json!({"role": "assistant", "text": ""}))
            .unwrap();
        for chunk in ["Sure", ", **here**", " is the fix:\n\n```py\nx = 1\n```"] {
            c.call("append", &json!({"index": i, "text": chunk}))
                .unwrap();
        }
        let t = crate::style::THEMES[0];
        let (lines, _) = c.layout(40, &t, Duration::ZERO);
        let text: Vec<String> = lines.iter().map(MdLine::plain).collect();
        assert_eq!(text[0], "› You");
        assert!(text.iter().any(|l| l == "✦ Assistant"));
        assert!(text.iter().any(|l| l.contains("Sure, here is the fix:")));
        assert!(text.iter().any(|l| l.contains("x = 1")));
        let tool = c
            .call(
                "add",
                &json!({"role": "tool", "tool": {"name": "bash", "args": {"cmd": "ls"}}}),
            )
            .unwrap();
        assert!(c.animating());
        c.call(
            "tool",
            &json!({"index": tool, "status": "done", "output": "a.py"}),
        )
        .unwrap();
        assert!(!c.animating());
        c.selected = Some(1);
        assert!(
            matches!(c.key(&Key::plain(Code::Char('y')), Rect::new(0, 0, 40, 10)), Reaction::Copy(s) if s.starts_with("Sure"))
        );
    }

    #[test]
    fn one_click_opens_a_tool_call_even_when_the_chat_is_not_focused() {
        let spec = json!({"children": [
            {"type": "input", "props": {"id": "in"}},
            {"type": "chat", "props": {"id": "chat", "height": 12}, "content": [
                {"role": "user", "text": "مَرْحَباً — Hello مرحبا · こんにちは 你好 · नमस्ते"},
                {"role": "tool", "tool": {"name": "bash", "args": {"cmd": "ls"}, "status": "done",
                    "output": "  مَرْحَباً.txt\n  日本.txt\n  नमस्ते.txt"}},
            ]},
        ]});
        let mut s = crate::session::Session::from_json(&spec.to_string()).unwrap();
        let screen = |s: &mut crate::session::Session| s.frame(70, 20).text();
        let text = screen(&mut s);
        let (y, row) = text
            .lines()
            .enumerate()
            .find(|(_, l)| l.contains("✓ bash"))
            .unwrap();
        let x = crate::text::width(&row[..row.find("bash").unwrap()]) as u16;
        assert_ne!(s.tree_json(70, 20)["focus"], "chat");
        s.click_at(x, y as u16);
        let text = screen(&mut s);
        assert_eq!(
            s.tree_json(70, 20)["focus"],
            "chat",
            "the click focuses the chat"
        );
        assert!(
            text.contains("▾") && text.contains("│   日本.txt"),
            "and opens the tool call:\n{text}"
        );
        assert!(text.contains("│   مَرْحَباً.txt"));
        assert!(text.contains("│   नमस्ते.txt"));
        // Clicking its output only selects; clicking the header again closes it.
        let out = text.lines().position(|l| l.contains("日本.txt")).unwrap();
        s.click_at(x, out as u16);
        assert!(screen(&mut s).contains("日本.txt"), "still open");
        s.click_at(x, y as u16);
        let text = screen(&mut s);
        assert!(
            text.contains("▸") && !text.contains("日本.txt"),
            "closed:\n{text}"
        );
    }
}
