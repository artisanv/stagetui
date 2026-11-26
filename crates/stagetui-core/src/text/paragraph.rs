//! Paragraph layout: shape → wrap by cell width → reorder each line.

use super::{Direction, arabic, bidi, first_strong, is_complex, split_graphemes, width};

/// One laid-out line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// What to put in terminal cells, left to right (shaped, visual order).
    pub visual: String,
    /// The same text in reading order, unshaped (for snapshots, copy, pixel rendering).
    pub logical: String,
    /// Width of `visual` in cells.
    pub width: usize,
    /// Paragraph direction of this line.
    pub dir: Direction,
    /// Contains right-to-left or complex-script characters.
    pub complex: bool,
}

/// Lay out `text` to at most `max_width` cells.
///
/// `dir` is the base direction for paragraphs whose own direction cannot be
/// detected (e.g. lines of digits); each paragraph otherwise uses its first strong character.
pub fn layout(text: &str, max_width: usize, dir: Direction) -> Vec<Line> {
    let max = max_width.max(1);
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let para_dir = match dir {
            Direction::Rtl => Direction::Rtl,
            Direction::Ltr if first_strong(para) == Direction::Rtl => Direction::Rtl,
            Direction::Ltr => Direction::Ltr,
        };
        for logical in wrap_words(para, max) {
            let shaped = arabic::shape(&logical);
            let visual = bidi::visual(&shaped, para_dir);
            lines.push(Line {
                width: width(&visual),
                complex: is_complex(&logical),
                visual,
                logical,
                dir: para_dir,
            });
        }
    }
    lines
}

/// Greedy word wrap measured on **shaped** widths (lam-alef is one cell for two letters).
fn wrap_words(para: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    let mut line_w = 0;
    for word in para.split(' ') {
        let ww = width(&arabic::shape(word));
        let sep = usize::from(!line.is_empty());
        if line_w + sep + ww <= max {
            if sep == 1 {
                line.push(' ');
            }
            line.push_str(word);
            line_w += sep + ww;
            continue;
        }
        if !line.is_empty() {
            out.push(std::mem::take(&mut line));
        }
        if ww <= max {
            line.push_str(word);
            line_w = ww;
        } else {
            let mut pieces = split_graphemes(word, max);
            let last = pieces.pop().unwrap_or_default();
            out.extend(pieces);
            line_w = width(&arabic::shape(&last));
            line = last;
        }
    }
    out.push(line);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARABIC: &str = "مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج";

    #[test]
    fn arabic_line_is_shaped_reversed_and_logical_kept() {
        let lines = layout(ARABIC, 80, Direction::Ltr);
        assert_eq!(lines.len(), 1);
        let l = &lines[0];
        assert_eq!(l.dir, Direction::Rtl);
        assert!(l.complex);
        assert_eq!(l.logical, ARABIC);
        assert_ne!(l.visual, ARABIC);
        assert_eq!(l.width, width(&l.visual));
        // Visual text uses presentation forms only (plus marks and spaces).
        assert!(
            l.visual
                .chars()
                .all(|c| c == ' ' || (c as u32) >= 0xFE70 || arabic::is_transparent(c))
        );
    }

    #[test]
    fn wrapping_respects_shaped_width() {
        // "لا" is two letters but one cell once shaped.
        let lines = layout("لا لا لا", 3, Direction::Ltr);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines.iter().all(|l| l.width <= 3));
    }

    #[test]
    fn rtl_lines_wrap_in_reading_order() {
        let lines = layout(ARABIC, 12, Direction::Ltr);
        assert!(lines.len() > 1);
        let rejoined: Vec<&str> = lines.iter().map(|l| l.logical.as_str()).collect();
        assert_eq!(rejoined.join(" "), ARABIC);
        assert!(lines.iter().all(|l| l.width <= 12));
    }

    #[test]
    fn global_scripts_survive() {
        for s in ["こんにちは世界", "नमस्ते दुनिया", "Hello مرحبا world"]
        {
            let lines = layout(s, 80, Direction::Ltr);
            assert_eq!(lines[0].logical, s);
        }
    }
}
