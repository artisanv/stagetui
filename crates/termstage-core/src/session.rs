//! A Session owns app state (the widget tree). Views render it.

use ratatui_core::layout::Rect;
use serde_json::Value;

use crate::Result;
use crate::frame::Frame;
use crate::layout;
use crate::tree::{AppSpec, Tree};
use crate::widgets::Cx;

/// App state, independent of any terminal.
#[derive(Debug, Clone)]
pub struct Session {
    tree: Tree,
    cx: Cx,
    /// Horizontal breathing room (cells) kept free around content.
    margin: u16,
}

impl Session {
    /// Build from a JSON app description (the Python → Rust boundary).
    pub fn from_json(json: &str) -> Result<Self> {
        let spec: AppSpec = serde_json::from_str(json)?;
        Self::from_spec(&spec)
    }

    /// Build from a typed app description (Rust users).
    pub fn from_spec(spec: &AppSpec) -> Result<Self> {
        Ok(Self {
            tree: Tree::build(spec)?,
            cx: Cx::default(),
            margin: 4,
        })
    }

    /// The widget tree.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// Tell the session how big a terminal cell is in pixels (affects image sizes).
    pub fn set_cell_px(&mut self, cell_px: (u32, u32)) {
        self.cx.cell_px = (cell_px.0.max(1), cell_px.1.max(1));
    }

    pub fn cell_px(&self) -> (u32, u32) {
        self.cx.cell_px
    }

    /// Cells kept free left+right of content (default 4; 0 for content-sized renders).
    pub fn set_margin(&mut self, margin: u16) {
        self.margin = margin;
    }

    /// Lay out and paint a frame of `cols × rows`.
    pub fn frame(&mut self, cols: u16, rows: u16) -> Frame {
        let area = Rect::new(0, 0, cols, rows);
        let theme = self.tree.theme;
        let mut frame = Frame::new(area, theme.bg, theme.fg);
        let max_w = cols.saturating_sub(self.margin).max(1);
        let cx = self.cx;
        let sizes: Vec<_> = self
            .tree
            .nodes
            .iter()
            .map(|n| n.paint().measure(max_w, &cx))
            .collect();
        let stack = layout::center_stack(area, &sizes, 1);
        for (node, rect) in self.tree.nodes.iter_mut().zip(stack.rects) {
            node.bounds = rect;
            if node.visible && rect.width > 0 && rect.height > 0 {
                node.paint().paint(rect, &mut frame, &node.theme, &cx);
            }
        }
        if stack.hidden > 0 {
            let more = if stack.hidden == 1 {
                "1 more widget".to_string()
            } else {
                format!("{} more widgets", stack.hidden)
            };
            let hint = format!("↓ {more} below — make the window taller to see everything");
            let line = crate::text::layout(&hint, usize::from(cols), crate::text::Direction::Ltr)
                .swap_remove(0);
            let x = (cols.saturating_sub(line.width as u16)) / 2;
            frame.put_line(
                x,
                rows.saturating_sub(1),
                cols,
                &line,
                theme.muted,
                theme.bg,
                false,
            );
        }
        frame
    }

    /// The natural size of the content (cells), capped at `max_cols`. Used to crop
    /// single-card renders (`termstage render "text"`, `ts.show("text")`).
    pub fn natural_size(&self, max_cols: u16) -> (u16, u16) {
        let cx = self.cx;
        let sizes: Vec<_> = self
            .tree
            .nodes
            .iter()
            .map(|n| n.paint().measure(max_cols, &cx))
            .collect();
        let w = sizes.iter().map(|s| s.0).max().unwrap_or(1);
        let h: u16 = sizes.iter().map(|s| s.1).sum::<u16>() + sizes.len().saturating_sub(1) as u16;
        (w.max(1), h.max(1))
    }

    /// The agent-facing JSON tree (bounds reflect the last frame).
    pub fn tree_json(&self, cols: u16, rows: u16) -> Value {
        self.tree.to_json(cols, rows)
    }
}
