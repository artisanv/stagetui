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
    /// Draw as a picture where the terminal can. Off while the run is moving
    /// (animations) or partly hidden, so the terminal shows the cell version.
    pub pixel: bool,
}

/// An image shown over a cell area.
#[derive(Debug, Clone)]
pub struct Placement {
    /// The visible cells.
    pub area: Rect,
    pub image: Image,
    pub fit: Fit,
    /// Background behind transparent pixels.
    pub bg: Rgb,
    /// The whole picture's cells; larger than `area` when part of it is
    /// scrolled away or covered by a pop-up.
    pub full: Rect,
    /// Darkened (behind a modal).
    pub dim: bool,
}

impl Placement {
    pub fn new(area: Rect, image: Image, fit: Fit, bg: Rgb) -> Self {
        Self {
            area,
            image,
            fit,
            bg,
            full: area,
            dim: false,
        }
    }

    /// Whether this placement shows the whole picture as-is.
    pub fn is_plain(&self) -> bool {
        self.full == self.area && !self.dim
    }

    /// Pixels for exactly `area` (cropped from `full`, darkened if `dim`).
    pub fn pixels(&self, cell_px: (u32, u32)) -> tiny_skia::Pixmap {
        let (cw, ch) = (cell_px.0.max(1), cell_px.1.max(1));
        let whole = self.image.render(
            u32::from(self.full.width) * cw,
            u32::from(self.full.height) * ch,
            self.fit,
        );
        let mut pic = if self.full == self.area {
            whole
        } else {
            let (w, h) = (
                u32::from(self.area.width) * cw,
                u32::from(self.area.height) * ch,
            );
            let mut out = tiny_skia::Pixmap::new(w.max(1), h.max(1))
                .unwrap_or_else(|| tiny_skia::Pixmap::new(1, 1).expect("1x1"));
            out.draw_pixmap(
                -(i32::from(self.area.x - self.full.x) * cw as i32),
                -(i32::from(self.area.y - self.full.y) * ch as i32),
                whole.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                tiny_skia::Transform::identity(),
                None,
            );
            out
        };
        if self.dim {
            for px in pic.pixels_mut() {
                let c = px.demultiply();
                let d = |v: u8| (u16::from(v) * 2 / 5) as u8;
                *px =
                    tiny_skia::ColorU8::from_rgba(d(c.red()), d(c.green()), d(c.blue()), c.alpha())
                        .premultiply();
            }
        }
        pic
    }

    /// A plain placement showing exactly [`Self::pixels`], for protocols that
    /// can't crop: same output, simpler path.
    pub fn flattened(
        &self,
        cell_px: (u32, u32),
        cache: &mut std::collections::HashMap<u64, Image>,
    ) -> Self {
        if self.is_plain() {
            return self.clone();
        }
        let key = crate::gfx::hash_bytes(
            format!(
                "{}|{:?}|{:?}|{:?}|{:?}|{}",
                self.image.id, self.full, self.area, self.fit, cell_px, self.dim
            )
            .as_bytes(),
        );
        let image = cache
            .entry(key)
            .or_insert_with(|| Image::from_pixmap(self.image.name.clone(), self.pixels(cell_px)))
            .clone();
        Self {
            area: self.area,
            image,
            fit: Fit::Stretch,
            bg: self.bg,
            full: self.area,
            dim: false,
        }
    }
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
                pixel: true,
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
        self.images.push(Placement::new(area, image, fit, bg));
    }

    /// A frame where every cell is "nothing painted" (see [`Frame::blit`]).
    pub fn transparent(area: Rect, bg: Rgb) -> Self {
        Self {
            cells: Buffer::empty(area),
            runs: Vec::new(),
            images: Vec::new(),
            bg,
        }
    }

    /// Copy `src` (painted at its own coordinates) into this frame shifted by
    /// `(dx, dy)`, keeping only what falls inside `clip`. Untouched source cells
    /// are skipped, so containers stay see-through. `alpha < 1` fades colors
    /// toward `toward`; `pixels = false` keeps pictures out (cells only).
    #[allow(clippy::too_many_arguments)]
    pub fn blit(
        &mut self,
        src: &Frame,
        dx: i32,
        dy: i32,
        clip: Rect,
        alpha: f32,
        toward: Rgb,
        pixels: bool,
    ) {
        let clip = clip.intersection(self.cells.area);
        if clip.is_empty() {
            return;
        }
        let fade = |c: ratatui_core::style::Color| match c {
            ratatui_core::style::Color::Rgb(r, g, b) if alpha < 1.0 => {
                toward.mix(Rgb(r, g, b), alpha).color()
            }
            other => other,
        };
        let sa = src.cells.area;
        for sy in sa.top()..sa.bottom() {
            let ty = i32::from(sy) + dy;
            if ty < i32::from(clip.top()) || ty >= i32::from(clip.bottom()) {
                continue;
            }
            let mut skip = 0usize;
            for sx in sa.left()..sa.right() {
                let cell = &src.cells[(sx, sy)];
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let w = text::width(cell.symbol()).max(1);
                skip = w - 1;
                if *cell == ratatui_core::buffer::Cell::EMPTY {
                    continue;
                }
                let tx = i32::from(sx) + dx;
                if tx < i32::from(clip.left()) || tx + w as i32 > i32::from(clip.right()) {
                    // A wide character cut by the edge becomes a blank.
                    if tx >= i32::from(clip.left()) && tx < i32::from(clip.right()) {
                        let t = &mut self.cells[(tx as u16, ty as u16)];
                        *t = cell.clone();
                        t.set_symbol(" ");
                        t.bg = fade(cell.bg);
                    }
                    continue;
                }
                let t = &mut self.cells[(tx as u16, ty as u16)];
                *t = cell.clone();
                t.fg = fade(cell.fg);
                t.bg = fade(cell.bg);
            }
        }
        let shift = |r: Rect| -> Option<Rect> {
            let x = i32::from(r.x) + dx;
            let y = i32::from(r.y) + dy;
            if x < 0 || y < 0 || x > i32::from(u16::MAX) || y > i32::from(u16::MAX) {
                return None;
            }
            Some(Rect::new(x as u16, y as u16, r.width, r.height))
        };
        for run in &src.runs {
            let Some(area) = shift(run.area) else {
                continue;
            };
            if area.intersection(clip) != area {
                continue;
            }
            let mut run = run.clone();
            run.area = area;
            run.pixel = run.pixel && pixels && alpha >= 1.0;
            self.runs.push(run);
        }
        if pixels && alpha >= 1.0 {
            for img in &src.images {
                let (Some(area), Some(full)) = (shift(img.area), shift(img.full)) else {
                    continue;
                };
                let visible = area.intersection(clip);
                if visible.is_empty() {
                    continue;
                }
                self.images.push(Placement {
                    area: visible,
                    full,
                    ..img.clone()
                });
            }
        }
    }

    /// Something opaque (a pop-up) now covers `rect`: pictures and pixel text
    /// under it must not show through.
    pub fn occlude(&mut self, rect: Rect) {
        self.runs.retain(|r| r.area.intersection(rect).is_empty());
        let mut kept = Vec::with_capacity(self.images.len());
        for img in self.images.drain(..) {
            let hit = img.area.intersection(rect);
            if hit.is_empty() {
                kept.push(img);
                continue;
            }
            // Up to four strips around the covered part stay visible.
            let a = img.area;
            let strips = [
                Rect::new(a.x, a.y, a.width, hit.y - a.y),
                Rect::new(a.x, hit.bottom(), a.width, a.bottom() - hit.bottom()),
                Rect::new(a.x, hit.y, hit.x - a.x, hit.height),
                Rect::new(hit.right(), hit.y, a.right() - hit.right(), hit.height),
            ];
            for s in strips.into_iter().filter(|s| !s.is_empty()) {
                kept.push(Placement {
                    area: s,
                    ..img.clone()
                });
            }
        }
        self.images = kept;
    }

    /// Darken everything except `keep` (the backdrop behind a modal).
    pub fn dim_except(&mut self, keep: Rect, toward: Rgb, amount: f32) {
        let area = self.cells.area;
        let dim = |c: ratatui_core::style::Color| match c {
            ratatui_core::style::Color::Rgb(r, g, b) => Rgb(r, g, b).mix(toward, amount).color(),
            other => other,
        };
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if keep.contains((x, y).into()) {
                    continue;
                }
                let c = &mut self.cells[(x, y)];
                c.fg = dim(c.fg);
                c.bg = dim(c.bg);
            }
        }
        for run in &mut self.runs {
            if keep.intersection(run.area).is_empty() {
                run.fg = run.fg.mix(toward, amount);
                run.bg = run.bg.mix(toward, amount);
            }
        }
        for img in &mut self.images {
            img.dim = true;
        }
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
