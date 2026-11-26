//! Unicode Bidirectional Algorithm (UAX #9) → visual order for one line.
//!
//! Levels come from `unicode-bidi`; reversal is done per **grapheme cluster**
//! so harakat stay attached to their letters, and paired punctuation is
//! mirrored in right-to-left runs (rule L4), because terminals do neither.

use unicode_bidi::{BidiInfo, Level};
use unicode_segmentation::UnicodeSegmentation;

use super::Direction;

const MIRRORS: [(char, char); 16] = [
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('<', '>'),
    ('«', '»'),
    ('‹', '›'),
    ('⁅', '⁆'),
    ('⁽', '⁾'),
    ('₍', '₎'),
    ('≤', '≥'),
    ('⟨', '⟩'),
    ('⟪', '⟫'),
    ('〈', '〉'),
    ('《', '》'),
    ('【', '】'),
    ('〔', '〕'),
];

/// The mirrored counterpart of `c` (rule L4), if it has one.
pub fn mirror(c: char) -> Option<char> {
    MIRRORS.iter().find_map(|&(a, b)| {
        if c == a {
            Some(b)
        } else if c == b {
            Some(a)
        } else {
            None
        }
    })
}

/// Reorder one line of (already shaped) logical text into visual order.
pub fn visual(line: &str, base: Direction) -> String {
    if line.is_empty() || (!line.chars().any(super::is_strong_rtl) && base == Direction::Ltr) {
        return line.to_string();
    }
    let level = match base {
        Direction::Rtl => Level::rtl(),
        Direction::Ltr => Level::ltr(),
    };
    let info = BidiInfo::new(line, Some(level));
    let mut out = String::with_capacity(line.len());
    for para in &info.paragraphs {
        let (levels, runs) = info.visual_runs(para, para.range.clone());
        for run in runs {
            let text = &line[run.clone()];
            if levels[run.start].is_rtl() {
                for g in text.graphemes(true).rev() {
                    for c in g.chars() {
                        out.push(mirror(c).unwrap_or(c));
                    }
                }
            } else {
                out.push_str(text);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::arabic::shape;

    #[test]
    fn pure_rtl_is_reversed_by_grapheme() {
        let shaped = shape("مَرْحَباً");
        let v = visual(&shaped, Direction::Rtl);
        // Visual order starts with the last letter (alef + tanween) and every mark follows its base.
        let first: Vec<char> = v.chars().take(2).collect();
        assert_eq!(first, ['\u{FE8E}', '\u{064B}']);
        assert!(!v.starts_with(|c: char| crate::text::arabic::is_transparent(c)));
    }

    #[test]
    fn mixed_ltr_paragraph_keeps_english_in_place() {
        let shaped = shape("Hello مرحبا world");
        let v = visual(&shaped, Direction::Ltr);
        assert!(v.starts_with("Hello ") && v.ends_with(" world"));
        let middle: String = v
            .trim_start_matches("Hello ")
            .trim_end_matches(" world")
            .into();
        assert_eq!(middle, shape("مرحبا").chars().rev().collect::<String>());
    }

    #[test]
    fn numbers_stay_left_to_right_inside_rtl() {
        let v = visual(&shape("عدد 123"), Direction::Rtl);
        assert!(v.starts_with("123 "), "{v:?}");
    }

    #[test]
    fn brackets_are_mirrored_in_rtl() {
        let v = visual(&shape("(مرحبا)"), Direction::Rtl);
        assert!(v.starts_with('(') && v.ends_with(')'), "{v:?}");
    }

    #[test]
    fn ltr_text_untouched() {
        assert_eq!(visual("plain (text)", Direction::Ltr), "plain (text)");
    }
}
