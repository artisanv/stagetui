//! `StageTree`: a folding outline (files, settings, anything nested).

use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use serde_json::{Map, Value, json};

use super::{Cx, PaintCx, Reaction, Widget, clamp_scroll, fill, object, scrollbar};
use crate::frame::Frame;
use crate::input::{Code, Key};
use crate::text::{self, Direction};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct TreeItem {
    pub label: String,
    pub children: Vec<TreeItem>,
    pub expanded: bool,
}

impl TreeItem {
    /// Parse `"label"` or `{"label": ..., "children": [...], "expanded": bool}`.
    pub fn parse(v: &Value) -> Result<Self> {
        match v {
            Value::String(s) => Ok(Self {
                label: s.clone(),
                children: Vec::new(),
                expanded: false,
            }),
            Value::Object(m) => {
                let label = m
                    .get("label")
                    .and_then(Value::as_str)
                    .ok_or_else(bad_item)?
                    .to_string();
                let children = match m.get("children") {
                    Some(Value::Array(a)) => a.iter().map(Self::parse).collect::<Result<_>>()?,
                    None | Some(Value::Null) => Vec::new(),
                    Some(_) => return Err(bad_item()),
                };
                let expanded = m.get("expanded").and_then(Value::as_bool).unwrap_or(false);
                Ok(Self {
                    label,
                    children,
                    expanded,
                })
            }
            _ => Err(bad_item()),
        }
    }
}

fn bad_item() -> Error {
    Error::friendly(
        "StageTree got an item it can't read",
        "each item is a text label, or a dict with \"label\" and optional \"children\"",
        "use e.g. StageTree({\"src\": [\"main.py\", \"app.py\"], \"README.md\": None})",
    )
}

/// One visible line of the outline.
#[derive(Debug, Clone)]
struct Row {
    depth: usize,
    path: Vec<usize>,
    parent: bool,
    expanded: bool,
    label: String,
}

#[derive(Debug, Clone)]
pub struct TreeView {
    pub items: Vec<TreeItem>,
    pub selected: usize,
    pub scroll: usize,
    pub height: Option<u16>,
}

impl TreeView {
    fn rows(&self) -> Vec<Row> {
        fn walk(items: &[TreeItem], depth: usize, path: &mut Vec<usize>, out: &mut Vec<Row>) {
            for (i, it) in items.iter().enumerate() {
                path.push(i);
                out.push(Row {
                    depth,
                    path: path.clone(),
                    parent: !it.children.is_empty(),
                    expanded: it.expanded,
                    label: it.label.clone(),
                });
                if it.expanded {
                    walk(&it.children, depth + 1, path, out);
                }
                path.pop();
            }
        }
        let mut out = Vec::new();
        walk(&self.items, 0, &mut Vec::new(), &mut out);
        out
    }

    fn item_mut(&mut self, path: &[usize]) -> Option<&mut TreeItem> {
        let (first, rest) = path.split_first()?;
        let mut it = self.items.get_mut(*first)?;
        for i in rest {
            it = it.children.get_mut(*i)?;
        }
        Some(it)
    }

    fn labels(&self, path: &[usize]) -> Vec<String> {
        let mut out = Vec::new();
        let mut items = &self.items;
        for i in path {
            if let Some(it) = items.get(*i) {
                out.push(it.label.clone());
                items = &it.children;
            }
        }
        out
    }

    fn visible(&self, area: Rect) -> usize {
        usize::from(area.height).max(1)
    }

    fn select(&mut self, i: usize, area: Rect) -> Reaction {
        let rows = self.rows();
        if rows.is_empty() {
            return Reaction::Ignored;
        }
        let i = i.min(rows.len() - 1);
        let v = self.visible(area);
        if i < self.scroll {
            self.scroll = i;
        } else if i >= self.scroll + v {
            self.scroll = i + 1 - v;
        }
        let changed = i != self.selected;
        self.selected = i;
        if changed {
            Reaction::Emit("select", self.selection_at(&rows))
        } else {
            Reaction::Redraw
        }
    }

    fn selection_at(&self, rows: &[Row]) -> Value {
        match rows.get(self.selected) {
            Some(r) => json!({ "label": r.label, "path": self.labels(&r.path), "leaf": !r.parent }),
            None => Value::Null,
        }
    }

    fn set_expanded(&mut self, i: usize, open: Option<bool>) -> Reaction {
        let rows = self.rows();
        let Some(r) = rows.get(i) else {
            return Reaction::Ignored;
        };
        if !r.parent {
            return Reaction::Emit("submit", self.selection_at(&rows));
        }
        let path = r.path.clone();
        if let Some(it) = self.item_mut(&path) {
            it.expanded = open.unwrap_or(!it.expanded);
        }
        Reaction::Redraw
    }

    /// Keep folds and selection when the items are replaced.
    pub fn carry_state(&mut self, old: &TreeView) {
        fn copy(new: &mut [TreeItem], old: &[TreeItem]) {
            for n in new {
                if let Some(o) = old.iter().find(|o| o.label == n.label) {
                    n.expanded = n.expanded || o.expanded;
                    copy(&mut n.children, &o.children);
                }
            }
        }
        copy(&mut self.items, &old.items);
        self.selected = old.selected.min(self.rows().len().saturating_sub(1));
        self.scroll = old.scroll;
    }
}

impl Widget for TreeView {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let rows = self.rows();
        let w = rows
            .iter()
            .map(|r| r.depth * 2 + 2 + text::width(&r.label))
            .max()
            .unwrap_or(8)
            + 2;
        let h = self.height.map_or(rows.len(), usize::from).max(1);
        ((w as u16).min(max_width).max(6), h as u16)
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.width < 4 || area.height == 0 {
            return;
        }
        let t = p.theme;
        let bg = super::behind(frame, area.x, area.y, t.bg);
        let rows = self.rows();
        let v = self.visible(area);
        let offset = clamp_scroll(self.scroll as i64, rows.len(), v);
        for (line_no, (i, r)) in rows.iter().enumerate().skip(offset).take(v).enumerate() {
            let y = area.y + line_no as u16;
            let selected = i == self.selected;
            let rbg = if selected && p.focused {
                t.accent.mix(t.surface, 0.55)
            } else if selected {
                t.surface
            } else {
                bg
            };
            fill(frame, Rect::new(area.x, y, area.width, 1), t.fg, rbg);
            let marker = match (r.parent, r.expanded, p.rtl) {
                (true, true, _) => "▾",
                (true, false, false) => "▸",
                (true, false, true) => "◂",
                (false, ..) => "•",
            };
            let indent = (r.depth * 2 + 1) as u16;
            let room = area.width.saturating_sub(indent + 3);
            let line =
                text::layout(&r.label, usize::from(room.max(1)), Direction::Ltr).swap_remove(0);
            let lw = (line.width as u16).min(room);
            let (mx, lx) = if p.rtl {
                let mx = area.right().saturating_sub(indent + 1);
                (mx, mx.saturating_sub(1 + lw))
            } else {
                (area.x + indent, area.x + indent + 2)
            };
            let marker_color = if r.parent { t.accent } else { t.muted };
            frame.cells[(mx, y)]
                .set_symbol(marker)
                .set_style(Style::new().fg(marker_color.color()).bg(rbg.color()));
            let fg = if r.parent {
                t.fg
            } else {
                t.fg.mix(t.muted, 0.3)
            };
            frame.put_line(lx, y, room, &line, fg, rbg, selected || r.parent);
        }
        if rows.len() > v {
            let x = if p.rtl { area.left() } else { area.right() - 1 };
            scrollbar(
                frame,
                x,
                area.y,
                v as u16,
                offset,
                rows.len(),
                t.border,
                t.accent,
            );
        }
    }

    fn describe(&self) -> Map<String, Value> {
        let rows = self.rows();
        object(json!({
            "rows": rows.iter().map(|r| json!({"label": r.label, "depth": r.depth, "expanded": r.expanded})).collect::<Vec<_>>(),
            "selected": self.selection_at(&rows),
        }))
    }

    fn focusable(&self) -> bool {
        true
    }

    fn key(&mut self, key: &Key, area: Rect) -> Reaction {
        let rows = self.rows();
        if rows.is_empty() {
            return Reaction::Ignored;
        }
        let cur = self.selected.min(rows.len() - 1);
        // In RTL the outline is mirrored, so "open" is the left arrow.
        let (open_key, close_key) = (Code::Right, Code::Left);
        let flip = |c: Code| match c {
            Code::Left => Code::Right,
            Code::Right => Code::Left,
            c => c,
        };
        let pressed = if self.rtl_hint() {
            flip(key.code)
        } else {
            key.code
        };
        match pressed {
            Code::Up => self.select(cur.saturating_sub(1), area),
            Code::Down => self.select(cur + 1, area),
            Code::Home => self.select(0, area),
            Code::End => self.select(rows.len() - 1, area),
            Code::PageUp => self.select(cur.saturating_sub(self.visible(area)), area),
            Code::PageDown => self.select(cur + self.visible(area), area),
            c if c == open_key && rows[cur].parent && !rows[cur].expanded => {
                self.set_expanded(cur, Some(true))
            }
            c if c == close_key && rows[cur].parent && rows[cur].expanded => {
                self.set_expanded(cur, Some(false))
            }
            c if c == close_key && rows[cur].depth > 0 => {
                let parent = rows[..cur]
                    .iter()
                    .rposition(|r| r.depth + 1 == rows[cur].depth)
                    .unwrap_or(0);
                self.select(parent, area)
            }
            Code::Enter | Code::Char(' ') => self.set_expanded(cur, None),
            _ => Reaction::Ignored,
        }
    }

    fn click(&mut self, _x: u16, y: u16, area: Rect) -> Reaction {
        let rows = self.rows().len();
        let offset = clamp_scroll(self.scroll as i64, rows, self.visible(area));
        let i = offset + usize::from(y);
        if i >= rows {
            return Reaction::Redraw;
        }
        if i == self.selected {
            return self.set_expanded(i, None);
        }
        let r = self.select(i, area);
        if self.rows()[i].parent {
            let _ = self.set_expanded(i, None);
        }
        r
    }

    fn scroll_by(&mut self, rows: i32, area: Rect) -> bool {
        let next = clamp_scroll(
            self.scroll as i64 + i64::from(rows),
            self.rows().len(),
            self.visible(area),
        );
        let moved = next != self.scroll;
        self.scroll = next;
        moved
    }

    fn value(&self) -> Value {
        self.selection_at(&self.rows())
    }
}

impl TreeView {
    /// Arrow keys follow the mirrored layout when the tree is right-to-left.
    fn rtl_hint(&self) -> bool {
        self.items
            .first()
            .is_some_and(|i| text::first_strong(&i.label) == Direction::Rtl)
    }
}
