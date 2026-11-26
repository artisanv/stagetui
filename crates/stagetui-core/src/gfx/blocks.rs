//! Half-block fallback: two pixels per cell using `▀` / `▄` with the
//! foreground and background colors. Works in every truecolor terminal.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::Color;

use super::{Fit, Image};
use crate::style::Rgb;

const UPPER_HALF: &str = "▀";
const LOWER_HALF: &str = "▄";

/// What shows through transparent pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backdrop {
    /// A known color (inside an app: the theme background). Edges are blended smoothly.
    Color(Rgb),
    /// The user's own terminal background (inline `show`): transparent pixels stay untouched.
    Terminal,
}

/// Draw `image` into `area` of `buf` as half-blocks.
pub fn draw(buf: &mut Buffer, area: Rect, image: &Image, fit: Fit, backdrop: Backdrop) {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    let p = image.render(u32::from(area.width), u32::from(area.height) * 2, fit);
    // `None` = transparent (only possible with `Backdrop::Terminal`).
    let px = |x: u32, y: u32| -> Option<Color> {
        let c = p.pixel(x, y)?.demultiply();
        let rgb = Rgb(c.red(), c.green(), c.blue());
        match backdrop {
            Backdrop::Color(bg) => Some(rgb.mix(bg, 1.0 - f32::from(c.alpha()) / 255.0).color()),
            Backdrop::Terminal => (c.alpha() >= 128).then(|| rgb.color()),
        }
    };
    for row in 0..area.height {
        for col in 0..area.width {
            let (x, y) = (u32::from(col), u32::from(row) * 2);
            let cell = &mut buf[(area.x + col, area.y + row)];
            match (px(x, y), px(x, y + 1)) {
                (Some(top), Some(bottom)) => {
                    cell.set_symbol(UPPER_HALF);
                    cell.fg = top;
                    cell.bg = bottom;
                }
                (Some(top), None) => {
                    cell.set_symbol(UPPER_HALF);
                    cell.fg = top;
                    cell.bg = Color::Reset;
                }
                (None, Some(bottom)) => {
                    cell.set_symbol(LOWER_HALF);
                    cell.fg = bottom;
                    cell.bg = Color::Reset;
                }
                (None, None) => {
                    cell.set_symbol(" ");
                    cell.fg = Color::Reset;
                    cell.bg = Color::Reset;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::{Pixmap, PremultipliedColorU8};

    fn two_pixels(top: Option<(u8, u8, u8)>, bottom: Option<(u8, u8, u8)>) -> Image {
        let mut p = Pixmap::new(1, 2).unwrap();
        let px = |c: Option<(u8, u8, u8)>| match c {
            Some((r, g, b)) => PremultipliedColorU8::from_rgba(r, g, b, 255).unwrap(),
            None => PremultipliedColorU8::TRANSPARENT,
        };
        p.pixels_mut()[0] = px(top);
        p.pixels_mut()[1] = px(bottom);
        Image::from_pixmap("t", p)
    }

    fn cell(img: &Image, backdrop: Backdrop) -> (String, Color, Color) {
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        let area = buf.area;
        draw(&mut buf, area, img, Fit::Stretch, backdrop);
        let c = &buf[(0, 0)];
        (c.symbol().to_string(), c.fg, c.bg)
    }

    #[test]
    fn top_and_bottom_pixels_become_fg_and_bg() {
        let img = two_pixels(Some((255, 0, 0)), Some((0, 0, 255)));
        let expected = (
            UPPER_HALF.to_string(),
            Color::Rgb(255, 0, 0),
            Color::Rgb(0, 0, 255),
        );
        assert_eq!(cell(&img, Backdrop::Terminal), expected);
        assert_eq!(cell(&img, Backdrop::Color(Rgb(0, 0, 0))), expected);
    }

    #[test]
    fn transparency_shows_the_terminal_background() {
        let red = Color::Rgb(255, 0, 0);
        assert_eq!(
            cell(&two_pixels(None, Some((255, 0, 0))), Backdrop::Terminal),
            (LOWER_HALF.into(), red, Color::Reset)
        );
        assert_eq!(
            cell(&two_pixels(Some((255, 0, 0)), None), Backdrop::Terminal),
            (UPPER_HALF.into(), red, Color::Reset)
        );
        assert_eq!(
            cell(&two_pixels(None, None), Backdrop::Terminal),
            (" ".into(), Color::Reset, Color::Reset)
        );
    }

    #[test]
    fn transparency_blends_into_a_known_color() {
        let (_, fg, bg) = cell(&two_pixels(None, None), Backdrop::Color(Rgb(10, 20, 30)));
        assert_eq!((fg, bg), (Color::Rgb(10, 20, 30), Color::Rgb(10, 20, 30)));
    }
}
