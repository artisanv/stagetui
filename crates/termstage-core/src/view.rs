//! Views: windows onto a [`Session`](crate::session::Session).
//!
//! A Session owns state; a View owns one output target (its terminal's
//! capabilities, size and encoder). This milestone has an in-process terminal
//! View and a headless View; Milestone 5 adds IPC Views for detach/reattach.

use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::Color;

use crate::caps::{Capabilities, GraphicsProtocol};
use crate::frame::{Frame, Placement};
use crate::gfx::{self, Fit, Image, blocks, iterm2, kitty, sixel};
use crate::render::{ColorMode, Encoder, SYNC_BEGIN, SYNC_END};

/// An output target.
pub trait View {
    /// Grid size `(cols, rows)`.
    fn size(&self) -> (u16, u16);
    /// Show a frame.
    fn present(&mut self, frame: &Frame) -> io::Result<()>;
}

/// How complex-script text (Arabic, Hindi) is drawn in a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextMode {
    /// Shaped presentation forms in cells (works everywhere).
    Cells,
    /// Real fonts rendered to pixels (needs Kitty Unicode placeholders).
    Pixels,
}

impl TextMode {
    /// `TERMSTAGE_TEXT_MODE=cells|pixels|auto`. Auto: pixels whenever the terminal can
    /// show real images (Kitty, Sixel, iTerm2), so Arabic never depends on the user's fonts.
    pub fn choose(caps: &Capabilities) -> Self {
        let graphics = caps.graphics.best != GraphicsProtocol::Halfblocks;
        match std::env::var("TERMSTAGE_TEXT_MODE").as_deref() {
            Ok("cells") => Self::Cells,
            _ if graphics => Self::Pixels,
            _ => Self::Cells,
        }
    }
}

type Key = (u64, u16, u16, Fit);

/// Per-terminal graphics state: which images the terminal already holds.
pub struct Gfx {
    protocol: GraphicsProtocol,
    placeholders: bool,
    text_mode: TextMode,
    truecolor: bool,
    tmux: bool,
    cell_px: (u32, u32),
    next_id: u32,
    live: HashMap<Key, u32>,
    text_cache: HashMap<u64, Image>,
    overlays: Vec<(Rect, Key)>,
}

/// One image drawn on top of the cell grid after the cells (Sixel, iTerm2, Kitty direct).
pub struct Overlay {
    pub area: Rect,
    pub payload: String,
}

/// Output of [`Gfx::prepare`].
pub struct Prepared {
    pub cells: Buffer,
    /// Escapes to send before the cells (image uploads, deletions).
    pub pre: String,
    /// Images to draw after the cells, or `None` if unchanged since last frame.
    pub overlays: Option<Vec<Overlay>>,
}

impl Gfx {
    pub fn new(caps: &Capabilities) -> Self {
        Self {
            protocol: caps.graphics.best,
            placeholders: caps.graphics.kitty_placeholders,
            text_mode: TextMode::choose(caps),
            truecolor: caps.truecolor,
            tmux: caps.remote.tmux,
            cell_px: (
                caps.size.cell_width_px.max(1),
                caps.size.cell_height_px.max(1),
            ),
            next_id: 1,
            live: HashMap::new(),
            text_cache: HashMap::new(),
            overlays: Vec::new(),
        }
    }

    pub fn cell_px(&self) -> (u32, u32) {
        self.cell_px
    }

    fn id_for(&mut self) -> u32 {
        let id = self.next_id;
        // Without truecolor, ids must fit in a 256-color index.
        let limit = if self.truecolor { 0x00FF_FFFF } else { 255 };
        self.next_id = if self.next_id >= limit {
            1
        } else {
            self.next_id + 1
        };
        id
    }

    /// Render a complex-script run as a pixel image (cached).
    fn text_image(&mut self, run: &crate::frame::TextRun) -> Image {
        let key = gfx::hash_bytes(
            format!(
                "{}|{:?}|{:?}|{:?}|{}|{}|{:?}|{}",
                run.logical,
                run.fg,
                run.bg,
                run.dir,
                run.bold,
                run.area.width,
                self.cell_px,
                run.transparent
            )
            .as_bytes(),
        );
        let cell_px = self.cell_px;
        self.text_cache
            .entry(key)
            .or_insert_with(|| {
                Image::from_pixmap(&run.logical, gfx::raster::text_run(run, cell_px))
            })
            .clone()
    }

    /// Turn a frame into cells + escapes for this terminal.
    pub fn prepare(&mut self, frame: &Frame, allow_overlays: bool) -> Prepared {
        let mut cells = frame.cells.clone();
        let mut placements: Vec<Placement> = frame.images.clone();
        if self.text_mode == TextMode::Pixels {
            let last_row = frame.area().bottom().saturating_sub(1);
            let placeholders = self.protocol == GraphicsProtocol::Kitty && self.placeholders;
            for run in frame.runs.iter().filter(|r| r.complex) {
                // An overlay image on the last row could scroll the screen: keep cells there.
                if !placeholders && run.area.y >= last_row && frame.area().height > 1 {
                    continue;
                }
                let image = self.text_image(run);
                placements.push(Placement {
                    area: run.area,
                    image,
                    fit: Fit::Stretch,
                    bg: run.bg,
                });
            }
        }
        let mut pre = String::new();
        let kitty_placeholders = self.protocol == GraphicsProtocol::Kitty && self.placeholders;
        let overlay_protocol = !kitty_placeholders && self.protocol != GraphicsProtocol::Halfblocks;

        if kitty_placeholders {
            let mut used = HashSet::new();
            for pl in &placements {
                let area = Rect {
                    width: pl.area.width.min(kitty::MAX_PLACEHOLDER_CELLS),
                    height: pl.area.height.min(kitty::MAX_PLACEHOLDER_CELLS),
                    ..pl.area
                };
                let key = (pl.image.id, area.width, area.height, pl.fit);
                let id = match self.live.get(&key) {
                    Some(id) => *id,
                    None => {
                        let id = self.id_for();
                        let pic = pl.image.render(
                            u32::from(area.width) * self.cell_px.0,
                            u32::from(area.height) * self.cell_px.1,
                            pl.fit,
                        );
                        if let Ok(png) = gfx::png(&pic) {
                            pre.push_str(&kitty::transmit_png(id, &png, self.tmux));
                            pre.push_str(&kitty::virtual_placement(
                                id,
                                area.width,
                                area.height,
                                self.tmux,
                            ));
                        }
                        self.live.insert(key, id);
                        id
                    }
                };
                used.insert(key);
                let fg = if self.truecolor {
                    let (r, g, b) = kitty::id_color(id);
                    Color::Rgb(r, g, b)
                } else {
                    Color::Indexed(id as u8)
                };
                for row in 0..area.height {
                    for col in 0..area.width {
                        if let Some(cell) = cells.cell_mut((area.x + col, area.y + row)) {
                            cell.set_symbol(&kitty::placeholder(row, col));
                            cell.fg = fg;
                            cell.bg = pl.bg.color();
                        }
                    }
                }
            }
            let stale: Vec<Key> = self
                .live
                .keys()
                .filter(|k| !used.contains(*k))
                .copied()
                .collect();
            for key in stale {
                if let Some(id) = self.live.remove(&key) {
                    pre.push_str(&kitty::delete(id, self.tmux));
                }
            }
            return Prepared {
                cells,
                pre,
                overlays: None,
            };
        }

        if !overlay_protocol || !allow_overlays {
            for pl in &placements {
                blocks::draw(
                    &mut cells,
                    pl.area,
                    &pl.image,
                    pl.fit,
                    blocks::Backdrop::Color(pl.bg),
                );
            }
            return Prepared {
                cells,
                pre,
                overlays: None,
            };
        }

        // Overlay protocols: re-send all images only when the set changed.
        let rows = cells.area.bottom();
        let now: Vec<(Rect, Key)> = placements
            .iter()
            .map(|pl| {
                let mut area = pl.area;
                // Sixel at the last row can scroll the whole screen: keep one row free.
                if self.protocol == GraphicsProtocol::Sixel
                    && area.bottom() >= rows
                    && area.height > 1
                {
                    area.height -= 1;
                }
                (area, (pl.image.id, area.width, area.height, pl.fit))
            })
            .collect();
        if now == self.overlays {
            return Prepared {
                cells,
                pre,
                overlays: None,
            };
        }
        if self.protocol == GraphicsProtocol::Kitty {
            for (_, key) in &self.overlays {
                if let Some(id) = self.live.remove(key) {
                    pre.push_str(&kitty::delete(id, self.tmux));
                }
            }
        }
        let mut overlays = Vec::new();
        for (pl, (area, key)) in placements.iter().zip(&now) {
            let pic = pl.image.render(
                u32::from(area.width) * self.cell_px.0,
                u32::from(area.height) * self.cell_px.1,
                pl.fit,
            );
            let payload = match self.protocol {
                GraphicsProtocol::Sixel => sixel::encode(&pic),
                GraphicsProtocol::Iterm2 => gfx::png(&pic)
                    .map(|png| iterm2::inline_png(&png, area.width, area.height, false))
                    .unwrap_or_default(),
                _ => {
                    let id = self.id_for();
                    self.live.insert(*key, id);
                    gfx::png(&pic)
                        .map(|png| {
                            kitty::display_png(id, &png, area.width, area.height, false, self.tmux)
                        })
                        .unwrap_or_default()
                }
            };
            overlays.push(Overlay {
                area: *area,
                payload,
            });
        }
        self.overlays = now;
        Prepared {
            cells,
            pre,
            overlays: Some(overlays),
        }
    }
}

/// A real terminal.
pub struct TerminalView<W: Write> {
    out: W,
    prev: Option<Buffer>,
    encoder: Encoder,
    sync: bool,
    size: (u16, u16),
    gfx: Gfx,
    scratch: String,
}

impl<W: Write> TerminalView<W> {
    pub fn new(out: W, caps: &Capabilities, size: (u16, u16)) -> Self {
        let color = if caps.truecolor {
            ColorMode::TrueColor
        } else {
            ColorMode::Ansi256
        };
        Self {
            out,
            prev: None,
            encoder: Encoder { color, sync: false },
            sync: caps.sync_output,
            size,
            gfx: Gfx::new(caps),
            scratch: String::new(),
        }
    }

    /// The terminal was resized: next frame is a full redraw.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.size = (cols, rows);
        self.prev = None;
    }

    pub fn cell_px(&self) -> (u32, u32) {
        self.gfx.cell_px()
    }
}

impl<W: Write> View for TerminalView<W> {
    fn size(&self) -> (u16, u16) {
        self.size
    }

    fn present(&mut self, frame: &Frame) -> io::Result<()> {
        let prepared = self.gfx.prepare(frame, true);
        let full = prepared.overlays.is_some()
            || self
                .prev
                .as_ref()
                .is_none_or(|p| p.area != prepared.cells.area);
        self.scratch.clear();
        if self.sync {
            self.scratch.push_str(SYNC_BEGIN);
        }
        self.scratch.push_str(&prepared.pre);
        match (&self.prev, full) {
            (Some(prev), false) => self.encoder.diff(prev, &prepared.cells, &mut self.scratch),
            _ => self.encoder.full(&prepared.cells, &mut self.scratch),
        }
        if let Some(overlays) = &prepared.overlays {
            for o in overlays {
                self.scratch
                    .push_str(&format!("\x1b[{};{}H", o.area.y + 1, o.area.x + 1));
                self.scratch.push_str(&o.payload);
            }
        }
        if self.sync {
            self.scratch.push_str(SYNC_END);
        }
        let only_sync = self.scratch.len()
            == if self.sync {
                SYNC_BEGIN.len() + SYNC_END.len()
            } else {
                0
            };
        if !only_sync {
            self.out.write_all(self.scratch.as_bytes())?;
            self.out.flush()?;
        }
        self.prev = Some(prepared.cells);
        Ok(())
    }
}

/// An offscreen view for CI, tests and agents.
#[derive(Debug, Default)]
pub struct HeadlessView {
    size: (u16, u16),
    last: Option<Frame>,
}

impl HeadlessView {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            size: (cols, rows),
            last: None,
        }
    }

    /// The last frame as plain text in reading order.
    pub fn text(&self) -> String {
        self.last.as_ref().map(Frame::text).unwrap_or_default()
    }

    /// The last frame.
    pub fn frame(&self) -> Option<&Frame> {
        self.last.as_ref()
    }
}

impl View for HeadlessView {
    fn size(&self) -> (u16, u16) {
        self.size
    }

    fn present(&mut self, frame: &Frame) -> io::Result<()> {
        self.last = Some(frame.clone());
        Ok(())
    }
}

/// Render a buffer as plain text (visual order, as the cells hold it).
pub fn buffer_to_text(buf: &Buffer) -> String {
    let mut out = String::new();
    let width = usize::from(buf.area.width);
    for (row_idx, row) in buf.content().chunks(width.max(1)).enumerate() {
        if row_idx > 0 {
            out.push('\n');
        }
        let mut line = String::new();
        let mut skip = 0usize;
        for cell in row {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let sym = cell.symbol();
            let sym = if sym.is_empty() { " " } else { sym };
            line.push_str(sym);
            skip = crate::text::width(sym).saturating_sub(1);
        }
        out.push_str(line.trim_end());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::{Env, Grid, ProbeInfo, TtyInfo, parse, resolve};
    use crate::style::Rgb;
    use ratatui_core::style::Style;

    fn caps(replies: &[u8]) -> Capabilities {
        resolve(
            &Env::default(),
            TtyInfo {
                stdin: true,
                stdout: true,
            },
            None,
            &parse::parse(replies),
            Grid {
                cols: 40,
                rows: 10,
                width_px: 400,
                height_px: 200,
            },
            ProbeInfo::default(),
        )
    }

    fn frame_with_image() -> Frame {
        let mut f = Frame::new(Rect::new(0, 0, 40, 10), Rgb(0, 0, 0), Rgb(255, 255, 255));
        let mut p = tiny_skia::Pixmap::new(20, 20).unwrap();
        p.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        f.put_image(
            Rect::new(2, 2, 4, 2),
            Image::from_pixmap("red", p),
            Fit::Contain,
            Rgb(0, 0, 0),
        );
        f
    }

    #[test]
    fn text_skips_wide_char_continuations() {
        let mut b = Buffer::empty(Rect::new(0, 0, 10, 2));
        b.set_string(0, 0, "世界ab", Style::new());
        b.set_string(0, 1, "مرحبا", Style::new());
        assert_eq!(buffer_to_text(&b), "世界ab\nمرحبا");
    }

    #[test]
    fn kitty_placeholders_upload_once_and_fill_cells() {
        let c = caps(b"\x1bP>|kitty(0.39.1)\x1b\\\x1b_Gi=31;OK\x1b\\\x1b[?62c");
        let mut g = Gfx::new(&c);
        let f = frame_with_image();
        let first = g.prepare(&f, true);
        assert!(first.pre.contains("a=t,f=100") && first.pre.contains("a=p,U=1"));
        assert!(first.cells[(2, 2)].symbol().starts_with(kitty::PLACEHOLDER));
        let second = g.prepare(&f, true);
        assert!(second.pre.is_empty(), "image must not be re-sent");
        let empty = Frame::new(Rect::new(0, 0, 40, 10), Rgb(0, 0, 0), Rgb(255, 255, 255));
        assert!(
            g.prepare(&empty, true).pre.contains("a=d,d=I"),
            "unused images are deleted"
        );
    }

    #[test]
    fn sixel_overlays_resend_only_on_change() {
        let c = caps(b"\x1b[?62;4c");
        let mut g = Gfx::new(&c);
        let f = frame_with_image();
        let first = g.prepare(&f, true);
        assert!(
            first.overlays.as_ref().unwrap()[0]
                .payload
                .starts_with("\x1bP0;1;0q")
        );
        assert!(g.prepare(&f, true).overlays.is_none());
    }

    #[test]
    fn halfblocks_draw_into_cells() {
        let c = caps(b"\x1b[?62c");
        let mut g = Gfx::new(&c);
        let p = g.prepare(&frame_with_image(), true);
        assert_eq!(p.cells[(3, 2)].symbol(), "▀");
        assert!(p.overlays.is_none() && p.pre.is_empty());
    }

    #[test]
    fn terminal_view_writes_full_then_diff() {
        let c = caps(b"\x1b[?62c");
        let mut out = Vec::new();
        {
            let mut v = TerminalView::new(&mut out, &c, (40, 10));
            let f = frame_with_image();
            v.present(&f).unwrap();
            v.present(&f).unwrap();
        }
        let s = String::from_utf8(out).unwrap();
        assert_eq!(
            s.matches("\x1b[2J").count(),
            1,
            "second identical frame writes nothing"
        );
    }
}
