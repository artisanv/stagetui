//! Text: measurement, direction, Arabic shaping, BiDi reordering, layout, fonts and rasterization.
//!
//! Two output paths:
//! - **Cells** (every terminal): Arabic is shaped into presentation forms and
//!   reordered into visual order here, so the terminal only has to draw glyphs.
//! - **Pixels** (graphics terminals and PNG snapshots): complex-script runs are
//!   rendered with `cosmic-text` and the bundled Noto fonts.

pub mod arabic;
mod arabic_table;
pub mod bidi;
pub mod fonts;
pub mod paragraph;
pub mod raster;

pub use paragraph::{Line, layout};

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Width of `s` in terminal cells.
pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Paragraph direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Ltr,
    Rtl,
}

/// Whether `c` is a strong right-to-left character (Hebrew, Arabic, Syriac, Thaana, NKo, …).
pub fn is_strong_rtl(c: char) -> bool {
    matches!(c as u32,
        0x0590..=0x08FF      // Hebrew, Arabic, Syriac, Thaana, NKo, Samaritan, Mandaic, Arabic Ext
        | 0xFB1D..=0xFDFF    // Hebrew & Arabic presentation forms A
        | 0xFE70..=0xFEFF    // Arabic presentation forms B
        | 0x10800..=0x10FFF  // historic RTL scripts
        | 0x1E800..=0x1EFFF) // Adlam, Arabic mathematical symbols, …
}

/// Whether `c` is a strong left-to-right character (rough but sufficient for P2/P3).
pub fn is_strong_ltr(c: char) -> bool {
    c.is_alphabetic() && !is_strong_rtl(c)
}

/// Scripts that terminals usually draw badly in cells (need real shaping).
pub fn needs_shaping(c: char) -> bool {
    matches!(c as u32,
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF // Arabic
        | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF                  // Arabic presentation forms
        | 0x0900..=0x0DFF) // Indic: Devanagari … Sinhala
}

/// Whether a string contains any right-to-left or complex-script characters.
pub fn is_complex(s: &str) -> bool {
    s.chars().any(|c| is_strong_rtl(c) || needs_shaping(c))
}

/// First-strong-character direction (UAX #9 rules P2–P3). Defaults to LTR.
pub fn first_strong(s: &str) -> Direction {
    for c in s.chars() {
        if is_strong_rtl(c) {
            return Direction::Rtl;
        }
        if is_strong_ltr(c) {
            return Direction::Ltr;
        }
    }
    Direction::Ltr
}

/// Resolve a `dir` prop (`"auto" | "ltr" | "rtl"`) against content.
pub fn resolve_dir(dir: &str, content: &str) -> Direction {
    match dir {
        "rtl" => Direction::Rtl,
        "ltr" => Direction::Ltr,
        _ => first_strong(content),
    }
}

/// Split `s` into chunks of at most `max` cells, never splitting a grapheme.
pub fn split_graphemes(s: &str, max: usize) -> Vec<String> {
    let max = max.max(1);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0;
    for g in s.graphemes(true) {
        let w = width(g);
        if cur_w + w > max && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        cur.push_str(g);
        cur_w += w;
    }
    out.push(cur);
    out
}

/// Plain word-wrap (no shaping) to `max` cells. Respects `\n`.
pub fn wrap(text: &str, max: usize) -> Vec<String> {
    layout(text, max, Direction::Ltr)
        .into_iter()
        .map(|l| l.logical)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const ARABIC: &str = "مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج";
    const MIXED: &str = "Hello مرحبا world";
    const JAPANESE: &str = "こんにちは世界";
    const HINDI: &str = "नमस्ते दुनिया";

    #[test]
    fn widths() {
        assert_eq!(width("abc"), 3);
        assert_eq!(width(JAPANESE), 14);
        assert_eq!(width("مَ"), 1); // harakat are zero-width
        assert!(width(ARABIC) < ARABIC.chars().count());
        assert!(width(HINDI) > 0);
    }

    #[test]
    fn direction() {
        assert_eq!(first_strong(ARABIC), Direction::Rtl);
        assert_eq!(first_strong(MIXED), Direction::Ltr);
        assert_eq!(first_strong("123 مرحبا"), Direction::Rtl);
        assert_eq!(first_strong("  "), Direction::Ltr);
        assert_eq!(resolve_dir("rtl", "abc"), Direction::Rtl);
        assert_eq!(resolve_dir("auto", ARABIC), Direction::Rtl);
    }

    #[test]
    fn complexity() {
        assert!(is_complex(ARABIC) && is_complex(HINDI));
        assert!(!is_complex(JAPANESE) && !is_complex("hello"));
    }

    #[test]
    fn wrapping() {
        assert_eq!(wrap("one two three", 7), vec!["one two", "three"]);
        assert_eq!(wrap("a\nb", 10), vec!["a", "b"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        for line in wrap(JAPANESE, 5) {
            assert!(width(&line) <= 5, "{line}");
        }
    }

    #[test]
    fn grapheme_split_keeps_harakat_with_letters() {
        for chunk in split_graphemes("مَرْحَباً", 2) {
            assert!(!chunk.starts_with('\u{064E}'), "{chunk:?}");
        }
    }
}
