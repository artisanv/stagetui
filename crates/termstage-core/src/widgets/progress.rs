//! `StageProgress`: a smooth progress bar (or a moving one when the amount is unknown).

use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use serde_json::{Map, Value, json};

use super::{Cx, PaintCx, Widget, object};
use crate::frame::Frame;
use crate::text::{self, Direction};

/// Left-aligned eighth blocks, 1/8 … 7/8.
const EIGHTHS: [&str; 7] = ["▏", "▎", "▍", "▌", "▋", "▊", "▉"];

#[derive(Debug, Clone)]
pub struct Progress {
    /// 0.0 – 1.0, or `None` while the amount is unknown.
    pub value: Option<f32>,
    pub label: Option<String>,
    pub width: Option<u16>,
}

impl Progress {
    fn percent(&self) -> Option<String> {
        self.value
            .map(|v| format!("{:>3}%", (v.clamp(0.0, 1.0) * 100.0).round() as u32))
    }
}

impl Widget for Progress {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let label = self.label.as_deref().map_or(0, |l| text::width(l) + 2);
        let want = self.width.unwrap_or((label + 30) as u16);
        (want.min(max_width).max(6), 1)
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.width < 4 || area.height == 0 {
            return;
        }
        let t = p.theme;
        let y = area.y;
        let bg = super::behind(frame, area.x, y, t.bg);
        let rtl = p.rtl
            || self
                .label
                .as_deref()
                .is_some_and(|l| text::first_strong(l) == Direction::Rtl);

        // Layout: label · bar · percent (mirrored in RTL).
        let mut bar = area;
        if let Some(label) = &self.label {
            let line =
                text::layout(label, usize::from(area.width / 2), Direction::Ltr).swap_remove(0);
            let lw = line.width as u16;
            let x = if rtl { area.right() - lw } else { area.x };
            frame.put_line(x, y, lw, &line, t.fg, bg, false);
            bar.width = bar.width.saturating_sub(lw + 2);
            if !rtl {
                bar.x += lw + 2;
            }
        }
        if let Some(pct) = self.percent() {
            let x = if rtl { bar.x } else { bar.right() - 4 };
            frame.put_str(
                x,
                y,
                &pct,
                4,
                Style::new().fg(t.muted.color()).bg(bg.color()),
            );
            bar.width = bar.width.saturating_sub(5);
            if rtl {
                bar.x += 5;
            }
        }
        if bar.width == 0 {
            return;
        }

        let track = t.surface.mix(t.border, 0.5);
        let fill_color = t.accent.mix(t.fg, p.glow * 0.4);
        let w = usize::from(bar.width);
        let track_style = Style::new().fg(track.color()).bg(bg.color());
        for x in bar.left()..bar.right() {
            frame.cells[(x, y)].set_symbol("━").set_style(track_style);
        }
        let on = Style::new().fg(fill_color.color()).bg(bg.color());
        match self.value {
            Some(v) => {
                let eighths = (v.clamp(0.0, 1.0) * (w * 8) as f32).round() as usize;
                let (full, part) = (eighths / 8, eighths % 8);
                for i in 0..full.min(w) {
                    let x = if rtl {
                        bar.right() - 1 - i as u16
                    } else {
                        bar.x + i as u16
                    };
                    frame.cells[(x, y)].set_symbol("█").set_style(on);
                }
                if part > 0 && full < w {
                    // Partial blocks only exist left-aligned; in RTL round to a half block.
                    let (x, sym) = if rtl {
                        (
                            bar.right() - 1 - full as u16,
                            if part >= 4 { "▐" } else { "━" },
                        )
                    } else {
                        (bar.x + full as u16, EIGHTHS[part - 1])
                    };
                    let style = if sym == "━" {
                        track_style
                    } else {
                        on.bg(track.color())
                    };
                    frame.cells[(x, y)].set_symbol(sym).set_style(style);
                }
            }
            None => {
                // A short segment gliding back and forth.
                let seg = (w / 5).clamp(2, 12);
                let span = w.saturating_sub(seg).max(1);
                let phase = (p.now.as_millis() % 2000) as f32 / 1000.0;
                let tri = if phase < 1.0 { phase } else { 2.0 - phase };
                let start = (crate::motion::ease_out_cubic(tri) * span as f32).round() as usize;
                for i in start..(start + seg).min(w) {
                    frame.cells[(bar.x + i as u16, y)]
                        .set_symbol("█")
                        .set_style(on);
                }
            }
        }
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({ "value": self.value, "label": self.label }))
    }

    fn animating(&self) -> bool {
        self.value.is_none()
    }

    fn value(&self) -> Value {
        json!(self.value)
    }
}
