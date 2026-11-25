//! A Frame is one painted screen: terminal cells plus the extra information
//! needed for pixels and for agents.
//!
//! - `cells` — what goes into the terminal grid (visual order, shaped Arabic).
//! - `runs` — text whose reading order differs from the grid (Arabic, Hebrew,
//!   complex scripts): used for pixel rendering and for readable snapshots.
//! - `images` — pictures to draw over cell areas.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Modifier, Style};

use crate::gfx::{Fit, Image};
use crate::style::Rgb;
use crate::text::{self, Direction, Line};

/// Text that must be read in a different order than it is drawn.
#[derive(Debug, Clone)]
pub struct TextRun {
    /// One row of cells.
    pub area: Rect,
    /// Reading-order, unshaped text.
    pub logical: String,
    pub dir: Direction,
    /// Needs real shaping (Arabic, Indic) — drawn as pixels where possible.
    pub complex: bool,
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
    /// Draw without a background (inline output over the user's own terminal background).
    pub transparent: bool,
}

/// An image shown over a cell area.
#[derive(Debug, Clone)]
pub struct Placement {
    pub area: Rect,
    pub image: Image,
    pub fit: Fit,
    /// Background behind transparent pixels.
    pub bg: Rgb,
}

/// One painted screen.
#[derive(Debug, Clone)]
pub struct Frame {
    pub cells: Buffer,
    pub runs: Vec<TextRun>,
    pub images: Vec<Placement>,
    /// Default background (theme).
    pub bg: Rgb,
}

impl Frame {
    /// A blank frame filled with `bg`.
    pub fn new(area: Rect, bg: Rgb, fg: Rgb) -> Self {
        let mut cells = Buffer::empty(area);
        cells.set_style(area, Style::new().bg(bg.color()).fg(fg.color()));
        Self {
            cells,
            runs: Vec::new(),
            images: Vec::new(),
            bg,
        }
    }

    pub fn area(&self) -> Rect {
        self.cells.area
    }

    /// Write a laid-out [`Line`] at `(x, y)`, at most `max` cells. Returns cells written.
    #[allow(clippy::too_many_arguments)]
    pub fn put_line(
        &mut self,
        x: u16,
        y: u16,
        max: u16,
        line: &Line,
        fg: Rgb,
        bg: Rgb,
        bold: bool,
    ) -> u16 {
        let mut style = Style::new().fg(fg.color()).bg(bg.color());
        if bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        let written = self.put_str(x, y, &line.visual, max, style);
        if written > 0 && (line.complex || line.visual != line.logical) {
            self.runs.push(TextRun {
                area: Rect::new(x, y, written, 1),
                logical: line.logical.clone(),
                dir: line.dir,
                complex: line.complex,
                fg,
                bg,
                bold,
                transparent: false,
            });
        }
        written
    }

    /// Write plain (already visual-order) text. Returns cells written.
    pub fn put_str(&mut self, x: u16, y: u16, s: &str, max: u16, style: Style) -> u16 {
        if max == 0 || !self.cells.area.contains((x, y).into()) {
            return 0;
        }
        let (end, _) = self.cells.set_stringn(x, y, s, usize::from(max), style);
        end.saturating_sub(x)
    }

    /// Show `image` over `area` (cells under it are blanked to `bg`).
    pub fn put_image(&mut self, area: Rect, image: Image, fit: Fit, bg: Rgb) {
        let area = area.intersection(self.cells.area);
        if area.is_empty() {
            return;
        }
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                self.cells[(x, y)].set_symbol(" ").set_bg(bg.color());
            }
        }
        self.images.push(Placement {
            area,
            image,
            fit,
            bg,
        });
    }

    /// The screen as plain text **in reading order** — what an agent should read.
    /// Arabic/Hebrew runs are given back in logical order; images become a label.
    pub fn text(&self) -> String {
        let area = self.cells.area;
        let w = usize::from(area.width);
        let mut rows: Vec<Vec<String>> = self
            .cells
            .content()
            .chunks(w.max(1))
            .map(|row| {
                let mut cols = Vec::with_capacity(w);
                let mut skip = 0usize;
                for cell in row {
                    if skip > 0 {
                        skip -= 1;
                        cols.push(String::new());
                        continue;
                    }
                    let sym = if cell.symbol().is_empty() {
                        " "
                    } else {
                        cell.symbol()
                    };
                    skip = text::width(sym).saturating_sub(1);
                    cols.push(sym.to_string());
                }
                cols
            })
            .collect();
        let mut splice = |r: Rect, s: String| {
            let (y, x) = (usize::from(r.y - area.y), usize::from(r.x - area.x));
            if let Some(row) = rows.get_mut(y) {
                for (i, col) in row
                    .iter_mut()
                    .enumerate()
                    .skip(x)
                    .take(usize::from(r.width))
                {
                    *col = if i == x { s.clone() } else { String::new() };
                }
            }
        };
        for run in &self.runs {
            splice(run.area, run.logical.clone());
        }
        for img in &self.images {
            let label = format!(
                "[image: {} {}×{}]",
                img.image.name, img.image.width, img.image.height
            );
            let pieces = text::split_graphemes(&label, usize::from(img.area.width));
            for (row, piece) in (img.area.y..img.area.bottom()).zip(pieces) {
                splice(Rect::new(img.area.x, row, img.area.width, 1), piece);
            }
        }
        rows.iter()
            .map(|r| r.concat().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtl_runs_read_back_in_logical_order() {
        let mut f = Frame::new(Rect::new(0, 0, 30, 2), Rgb(0, 0, 0), Rgb(255, 255, 255));
        let line = &text::layout("مرحبا بكم", 30, Direction::Ltr)[0];
        let n = f.put_line(2, 1, 30, line, Rgb(255, 255, 255), Rgb(0, 0, 0), false);
        assert_eq!(usize::from(n), line.width);
        assert_eq!(f.runs.len(), 1);
        assert_eq!(f.text(), "\n  مرحبا بكم");
    }

    #[test]
    fn images_become_labels() {
        let mut f = Frame::new(Rect::new(0, 0, 40, 3), Rgb(0, 0, 0), Rgb(255, 255, 255));
        let mut p = tiny_skia::Pixmap::new(64, 32).unwrap();
        p.fill(tiny_skia::Color::WHITE);
        f.put_image(
            Rect::new(1, 0, 30, 3),
            Image::from_pixmap("logo.png", p),
            Fit::Contain,
            Rgb(0, 0, 0),
        );
        assert_eq!(f.text().lines().next().unwrap(), " [image: logo.png 64×32]");
    }
}
