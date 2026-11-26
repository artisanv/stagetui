//! Render text to pixels with real shaping (cosmic-text + bundled fonts).
//!
//! Text is split into script runs so Arabic always uses Noto Sans Arabic,
//! Devanagari uses Noto Sans Devanagari, and everything else uses the
//! requested family — with system fonts as fallback for anything missing.

use cosmic_text::{Align, Attrs, Buffer, Color, Family, Metrics, Shaping, Weight};
use tiny_skia::Pixmap;

use super::fonts::{self, ARABIC, DEVANAGARI, MONO, SANS};
use crate::style::Rgb;

/// Which family to prefer for non-Arabic, non-Devanagari text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    Sans,
    Mono,
}

/// How to draw a piece of text.
#[derive(Debug, Clone, Copy)]
pub struct TextStyle {
    pub face: Face,
    pub size_px: f32,
    pub bold: bool,
    pub color: Rgb,
    pub alpha: u8,
}

impl TextStyle {
    pub fn new(size_px: f32, color: Rgb) -> Self {
        Self {
            face: Face::Sans,
            size_px,
            bold: false,
            color,
            alpha: 255,
        }
    }
}

/// Horizontal alignment inside `max_width`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    Arabic,
    Devanagari,
    Other,
}

fn script_of(c: char) -> Option<Script> {
    match c as u32 {
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => {
            Some(Script::Arabic)
        }
        0x0900..=0x097F | 0xA8E0..=0xA8FF => Some(Script::Devanagari),
        // Spaces, digits and punctuation inherit the surrounding run.
        _ if c.is_whitespace() || c.is_ascii_digit() || c.is_ascii_punctuation() => None,
        _ => Some(Script::Other),
    }
}

/// Split into `(text, script)` runs.
fn runs(text: &str) -> Vec<(&str, Script)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut current: Option<Script> = None;
    for (i, c) in text.char_indices() {
        if let Some(s) = script_of(c) {
            match current {
                Some(cur) if cur != s => {
                    out.push((&text[start..i], cur));
                    start = i;
                    current = Some(s);
                }
                None => current = Some(s),
                _ => {}
            }
        }
    }
    if start < text.len() {
        out.push((&text[start..], current.unwrap_or(Script::Other)));
    }
    out
}

fn attrs(style: &TextStyle, script: Script) -> Attrs<'static> {
    let family = match (script, style.face) {
        (Script::Arabic, _) => Family::Name(ARABIC),
        (Script::Devanagari, _) => Family::Name(DEVANAGARI),
        (Script::Other, Face::Mono) => Family::Name(MONO),
        (Script::Other, Face::Sans) => Family::Name(SANS),
    };
    let weight = if style.bold {
        Weight::BOLD
    } else {
        Weight::NORMAL
    };
    Attrs::new().family(family).weight(weight)
}

fn buffer(
    fonts: &mut fonts::Fonts,
    text: &str,
    style: &TextStyle,
    max_width: Option<f32>,
    align: HAlign,
) -> Buffer {
    let metrics = Metrics::new(style.size_px, style.size_px * 1.3);
    let mut buf = Buffer::new(&mut fonts.system, metrics);
    buf.set_size(max_width, None);
    let spans: Vec<(&str, Attrs)> = runs(text)
        .into_iter()
        .map(|(t, s)| (t, attrs(style, s)))
        .collect();
    let align = match align {
        HAlign::Left => None,
        HAlign::Center => Some(Align::Center),
        HAlign::Right => Some(Align::Right),
    };
    buf.set_rich_text(
        spans,
        &attrs(style, Script::Other),
        Shaping::Advanced,
        align,
    );
    buf.shape_until_scroll(&mut fonts.system, false);
    buf
}

/// `(width, height)` in pixels of `text` laid out without wrapping.
pub fn measure(text: &str, style: &TextStyle) -> (f32, f32) {
    let mut fonts = fonts::lock();
    let buf = buffer(&mut fonts, text, style, None, HAlign::Left);
    let mut w: f32 = 0.0;
    let mut h: f32 = 0.0;
    for run in buf.layout_runs() {
        w = w.max(run.line_w);
        h = run.line_top + run.line_height;
    }
    (w, h)
}

/// Draw `text` with its top-left at `(x, y)`, wrapping at `max_width` if given.
pub fn draw(
    pixmap: &mut Pixmap,
    text: &str,
    x: f32,
    y: f32,
    style: &TextStyle,
    max_width: Option<f32>,
    align: HAlign,
) {
    let mut guard = fonts::lock();
    let fonts = &mut *guard;
    let mut buf = buffer(fonts, text, style, max_width, align);
    let color = Color::rgba(style.color.0, style.color.1, style.color.2, style.alpha);
    let (ox, oy) = (x.round() as i32, y.round() as i32);
    let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);
    let data = pixmap.data_mut();
    buf.draw(
        &mut fonts.system,
        &mut fonts.cache,
        color,
        |gx, gy, w, h, c| {
            let a = u32::from(c.a());
            if a == 0 {
                return;
            }
            for yy in (oy + gy).max(0)..(oy + gy + h as i32).min(ph) {
                for xx in (ox + gx).max(0)..(ox + gx + w as i32).min(pw) {
                    blend(data, (yy * pw + xx) as usize, c.r(), c.g(), c.b(), a);
                }
            }
        },
    );
}

/// Source-over blend of a straight-alpha color into premultiplied RGBA8.
#[inline]
pub(crate) fn blend(data: &mut [u8], px: usize, r: u8, g: u8, b: u8, a: u32) {
    let i = px * 4;
    let inv = 255 - a;
    let mul = |s: u8, d: u8| ((u32::from(s) * a + u32::from(d) * inv + 127) / 255) as u8;
    data[i] = mul(r, data[i]);
    data[i + 1] = mul(g, data[i + 1]);
    data[i + 2] = mul(b, data[i + 2]);
    data[i + 3] = ((a * 255 + u32::from(data[i + 3]) * inv + 127) / 255) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ink(p: &Pixmap) -> usize {
        p.data().chunks(4).filter(|px| px[3] > 0).count()
    }

    #[test]
    fn script_runs() {
        let r = runs("Hello مرحبا 123 नमस्ते");
        let kinds: Vec<_> = r.iter().map(|(_, s)| *s).collect();
        assert!(kinds.contains(&Script::Arabic) && kinds.contains(&Script::Devanagari));
        assert_eq!(
            r.iter().map(|(t, _)| *t).collect::<String>(),
            "Hello مرحبا 123 नमस्ते"
        );
    }

    #[test]
    fn draws_arabic_and_measures() {
        let style = TextStyle::new(24.0, Rgb(255, 255, 255));
        let (w, h) = measure("مَرْحَباً بِكُمْ", &style);
        assert!(w > 40.0 && h > 20.0, "{w}x{h}");
        let mut p = Pixmap::new(300, 50).unwrap();
        draw(&mut p, "مَرْحَباً بِكُمْ", 0.0, 0.0, &style, None, HAlign::Left);
        assert!(ink(&p) > 200);
    }

    #[test]
    fn draws_every_bundled_script() {
        for s in ["Hello", "نص عربي", "नमस्ते", "ΑΒΓ Привет"] {
            let mut p = Pixmap::new(200, 40).unwrap();
            draw(
                &mut p,
                s,
                0.0,
                0.0,
                &TextStyle::new(20.0, Rgb(0, 0, 0)),
                None,
                HAlign::Left,
            );
            assert!(ink(&p) > 30, "{s} drew nothing");
        }
    }
}
