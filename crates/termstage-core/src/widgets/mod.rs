//! Built-in widgets. Milestones 1–2 ship `Card`, `TextBlock` and `ImageView`;
//! the full suite lands in Milestones 3–4.

mod card;
mod icons;
mod image_view;
mod text_block;

pub use card::Card;
pub use icons::{ICONS, icon};
pub use image_view::ImageView;
pub use text_block::TextBlock;

use ratatui_core::layout::Rect;
use serde_json::{Map, Value};

use crate::frame::Frame;
use crate::style::Theme;

/// Facts about the output that affect layout.
#[derive(Debug, Clone, Copy)]
pub struct Cx {
    /// Size of one terminal cell in pixels.
    pub cell_px: (u32, u32),
}

impl Default for Cx {
    fn default() -> Self {
        Self {
            cell_px: crate::caps::DEFAULT_CELL_PX,
        }
    }
}

/// What every widget can do.
pub trait Paint {
    /// Preferred `(width, height)` in cells given a maximum width.
    fn measure(&self, max_width: u16, cx: &Cx) -> (u16, u16);
    /// Draw into `frame` inside `area`.
    fn paint(&self, area: Rect, frame: &mut Frame, theme: &Theme, cx: &Cx);
    /// Widget-specific fields for the agent-facing JSON tree.
    fn describe(&self) -> Map<String, Value>;
}

pub(crate) fn object(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}
