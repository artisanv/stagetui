//! `StageImage` / `StageCanvas`: a picture (PNG, JPEG, GIF, WebP, BMP, SVG, or a canvas).

use ratatui_core::layout::Rect;
use serde_json::{Map, Value, json};

use super::{Cx, PaintCx, Widget, object};
use crate::frame::Frame;
use crate::gfx::{Fit, Image};

#[derive(Debug, Clone)]
pub struct ImageView {
    pub image: Image,
    /// Where it came from (path or `mem:<id>`), reported to agents.
    pub source: String,
    /// Requested size in cells; `None` = natural size.
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub fit: Fit,
}

impl Widget for ImageView {
    fn measure(&self, max_width: u16, cx: &Cx) -> (u16, u16) {
        match (self.width, self.height) {
            (Some(w), Some(h)) => (w.min(max_width), h),
            (Some(w), None) => {
                let w = w.min(max_width);
                let px_w = f32::from(w) * cx.cell_px.0 as f32;
                let px_h = px_w * self.image.height as f32 / self.image.width.max(1) as f32;
                (
                    w,
                    (px_h / cx.cell_px.1.max(1) as f32).round().max(1.0) as u16,
                )
            }
            (None, Some(h)) => {
                let px_h = f32::from(h) * cx.cell_px.1 as f32;
                let px_w = px_h * self.image.width as f32 / self.image.height.max(1) as f32;
                (
                    ((px_w / cx.cell_px.0.max(1) as f32).round().max(1.0) as u16).min(max_width),
                    h,
                )
            }
            (None, None) => self.image.cells(cx.cell_px, max_width, 40),
        }
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let bg = super::behind(frame, area.x, area.y, p.theme.bg);
        frame.put_image(area, self.image.clone(), self.fit, bg);
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({
            "source": self.source,
            "name": self.image.name,
            "pixels": { "w": self.image.width, "h": self.image.height },
            "fit": format!("{:?}", self.fit).to_lowercase(),
        }))
    }
}
