//! Cell buffer → minimal ANSI. Frames are wrapped in synchronized output
//! (DEC mode 2026) when the terminal supports it, so updates never tear.

use std::fmt::Write as _;

use ratatui_core::buffer::{Buffer, Cell};
use ratatui_core::style::{Color, Modifier};
use serde::Serialize;

/// How colors are emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    TrueColor,
    Ansi256,
}

pub const SYNC_BEGIN: &str = "\x1b[?2026h";
pub const SYNC_END: &str = "\x1b[?2026l";

/// Encodes frames for one terminal.
#[derive(Debug, Clone, Copy)]
pub struct Encoder {
    pub color: ColorMode,
    pub sync: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Pen {
    fg: Color,
    bg: Color,
    modifier: Modifier,
}

impl Encoder {
    /// Clear the screen and draw `next` completely.
    pub fn full(&self, next: &Buffer, out: &mut String) {
        let blank = Buffer::empty(next.area);
        self.frame(&blank, next, true, out);
    }

    /// Draw only the cells that changed from `prev` to `next` (same size).
    pub fn diff(&self, prev: &Buffer, next: &Buffer, out: &mut String) {
        self.frame(prev, next, false, out);
    }

    fn frame(&self, prev: &Buffer, next: &Buffer, clear: bool, out: &mut String) {
        if self.sync {
            out.push_str(SYNC_BEGIN);
        }
        if clear {
            out.push_str("\x1b[0m\x1b[2J");
        }
        let mut cursor: Option<(u16, u16)> = None;
        let mut pen: Option<Pen> = None;
        for (x, y, cell) in prev.diff(next) {
            if cursor != Some((x, y)) {
                let _ = write!(out, "\x1b[{};{}H", y + 1, x + 1);
            }
            let want = Pen {
                fg: cell.fg,
                bg: cell.bg,
                modifier: cell.modifier,
            };
            if pen != Some(want) {
                self.sgr(want, out);
                pen = Some(want);
            }
            let sym = symbol(cell);
            out.push_str(sym);
            cursor = Some((x + crate::text::width(sym).max(1) as u16, y));
        }
        if pen.is_some() {
            out.push_str("\x1b[0m");
        }
        if self.sync {
            out.push_str(SYNC_END);
        }
    }

    /// Encode a buffer for printing **inline** at the cursor (no absolute
    /// positioning): each row starts at column 0 and ends with a newline.
    pub fn inline(&self, buf: &Buffer, out: &mut String) {
        let width = usize::from(buf.area.width).max(1);
        for row in buf.content().chunks(width) {
            out.push('\r');
            let mut pen: Option<Pen> = None;
            let mut skip = 0usize;
            for cell in row {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let want = Pen {
                    fg: cell.fg,
                    bg: cell.bg,
                    modifier: cell.modifier,
                };
                if pen != Some(want) {
                    self.sgr(want, out);
                    pen = Some(want);
                }
                let sym = symbol(cell);
                out.push_str(sym);
                skip = crate::text::width(sym).saturating_sub(1);
            }
            out.push_str("\x1b[0m\n");
        }
    }

    fn sgr(&self, pen: Pen, out: &mut String) {
        out.push_str("\x1b[0");
        for (flag, code) in [
            (Modifier::BOLD, "1"),
            (Modifier::DIM, "2"),
            (Modifier::ITALIC, "3"),
            (Modifier::UNDERLINED, "4"),
            (Modifier::SLOW_BLINK, "5"),
            (Modifier::REVERSED, "7"),
            (Modifier::HIDDEN, "8"),
            (Modifier::CROSSED_OUT, "9"),
        ] {
            if pen.modifier.contains(flag) {
                out.push(';');
                out.push_str(code);
            }
        }
        self.color(pen.fg, true, out);
        self.color(pen.bg, false, out);
        out.push('m');
    }

    fn color(&self, c: Color, fg: bool, out: &mut String) {
        let base = if fg { 30 } else { 40 };
        let ext = if fg { 38 } else { 48 };
        let _ = match c {
            Color::Reset => Ok(()),
            Color::Rgb(r, g, b) => match self.color {
                ColorMode::TrueColor => write!(out, ";{ext};2;{r};{g};{b}"),
                ColorMode::Ansi256 => write!(out, ";{ext};5;{}", rgb_to_256(r, g, b)),
            },
            Color::Indexed(i) => write!(out, ";{ext};5;{i}"),
            named => {
                let (offset, bright) = named_index(named);
                write!(
                    out,
                    ";{}",
                    if bright {
                        base + 60 + offset
                    } else {
                        base + offset
                    }
                )
            }
        };
    }
}

fn symbol(cell: &Cell) -> &str {
    let s = cell.symbol();
    if s.is_empty() { " " } else { s }
}

fn named_index(c: Color) -> (u8, bool) {
    match c {
        Color::Black => (0, false),
        Color::Red => (1, false),
        Color::Green => (2, false),
        Color::Yellow => (3, false),
        Color::Blue => (4, false),
        Color::Magenta => (5, false),
        Color::Cyan => (6, false),
        Color::Gray => (7, false),
        Color::DarkGray => (0, true),
        Color::LightRed => (1, true),
        Color::LightGreen => (2, true),
        Color::LightYellow => (3, true),
        Color::LightBlue => (4, true),
        Color::LightMagenta => (5, true),
        Color::LightCyan => (6, true),
        _ => (7, true),
    }
}

/// Nearest xterm-256 color.
pub fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    let level = |v: u8| -> u8 {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            (v - 35) / 40
        }
    };
    let steps = [0u8, 95, 135, 175, 215, 255];
    let (lr, lg, lb) = (level(r), level(g), level(b));
    let cube = 16 + 36 * lr + 6 * lg + lb;
    let cube_rgb = (steps[lr as usize], steps[lg as usize], steps[lb as usize]);
    let avg = ((u16::from(r) + u16::from(g) + u16::from(b)) / 3) as u8;
    let gray_idx = if avg > 238 {
        23
    } else {
        avg.saturating_sub(3) / 10
    };
    let gray_v = 8 + 10 * gray_idx;
    let dist = |a: (u8, u8, u8)| {
        let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        d(a.0, r) + d(a.1, g) + d(a.2, b)
    };
    if dist((gray_v, gray_v, gray_v)) < dist(cube_rgb) {
        232 + gray_idx
    } else {
        cube
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_core::layout::Rect;
    use ratatui_core::style::Style;

    fn screen_text(p: &vt100::Parser) -> Vec<String> {
        p.screen()
            .rows(0, p.screen().size().1.get())
            .map(|r| r.trim_end().to_string())
            .collect()
    }

    fn buf_text(b: &Buffer) -> Vec<String> {
        crate::view::buffer_to_text(b)
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn sample(lines: &[&str]) -> Buffer {
        let mut b = Buffer::empty(Rect::new(0, 0, 30, lines.len() as u16));
        for (y, l) in lines.iter().enumerate() {
            b.set_string(0, y as u16, l, Style::new().fg(Color::Rgb(200, 100, 50)));
        }
        b
    }

    #[test]
    fn full_then_diff_reproduces_frames_in_a_real_vt_parser() {
        let enc = Encoder {
            color: ColorMode::TrueColor,
            sync: true,
        };
        let a = sample(&["Hello termstage", "مَرْحَباً بِكُمْ", "こんにちは世界", "नमस्ते"]);
        let b = sample(&["Hello TERMSTAGE!", "مرحبا", "世界こんにちは", "done"]);

        let mut parser = vt100::Parser::new(4.try_into().unwrap(), 30.try_into().unwrap(), 0);
        let mut out = String::new();
        enc.full(&a, &mut out);
        parser.process(out.as_bytes());
        assert_eq!(screen_text(&parser), buf_text(&a));

        out.clear();
        enc.diff(&a, &b, &mut out);
        parser.process(out.as_bytes());
        assert_eq!(screen_text(&parser), buf_text(&b));
        assert!(out.starts_with(SYNC_BEGIN) && out.ends_with(SYNC_END));
    }

    #[test]
    fn identical_frames_emit_nothing_but_sync() {
        let enc = Encoder {
            color: ColorMode::TrueColor,
            sync: false,
        };
        let a = sample(&["same"]);
        let mut out = String::new();
        enc.diff(&a, &a.clone(), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn colors() {
        let mut b = Buffer::empty(Rect::new(0, 0, 1, 1));
        b[(0, 0)].set_symbol("x").set_style(
            Style::new()
                .fg(Color::Rgb(255, 0, 0))
                .bg(Color::Indexed(17))
                .add_modifier(Modifier::BOLD),
        );
        let mut t = String::new();
        Encoder {
            color: ColorMode::TrueColor,
            sync: false,
        }
        .full(&b, &mut t);
        assert!(t.contains("\x1b[0;1;38;2;255;0;0;48;5;17m"), "{t:?}");
        let mut a = String::new();
        Encoder {
            color: ColorMode::Ansi256,
            sync: false,
        }
        .full(&b, &mut a);
        assert!(a.contains(";38;5;196;"), "{a:?}");
    }

    #[test]
    fn ansi256_mapping() {
        assert_eq!(rgb_to_256(0, 0, 0), 16);
        assert_eq!(rgb_to_256(255, 255, 255), 231);
        assert_eq!(rgb_to_256(128, 128, 128), 244);
        assert_eq!(rgb_to_256(255, 0, 0), 196);
    }
}
