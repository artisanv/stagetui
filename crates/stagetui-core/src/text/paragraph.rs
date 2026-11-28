//! Paragraph layout: shape → wrap by cell width → reorder each line.

use std::borrow::Cow;

use super::{Direction, arabic, bidi, first_strong, is_complex, split_graphemes, width};

/// Columns between tab stops: a tab moves on to the next multiple of 4.
pub const TAB_STOP: usize = 4;

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
///
/// Spaces are kept as written — indentation at the start of a paragraph too —
/// except where a line wraps. Tabs become spaces up to the next [`TAB_STOP`].
pub fn layout(text: &str, max_width: usize, dir: Direction) -> Vec<Line> {
    let max = max_width.max(1);
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let para = expand_tabs(para);
        let para_dir = match dir {
            Direction::Rtl => Direction::Rtl,
            Direction::Ltr => first_strong(&para),
        };
        for logical in wrap_words(&para, max) {
            lines.push(line(logical, para_dir));
        }
    }
    lines
}

/// One line, shaped and reordered for cells.
fn line(logical: String, dir: Direction) -> Line {
    let shaped = arabic::shape(&logical);
    let visual = bidi::visual(&shaped, dir);
    Line {
        width: width(&visual),
        complex: is_complex(&logical),
        visual,
        logical,
        dir,
    }
}

/// `s` with each tab replaced by spaces up to the next [`TAB_STOP`] column.
pub fn expand_tabs(s: &str) -> Cow<'_, str> {
    expand_tabs_from(s, 0)
}

/// [`expand_tabs`] for a piece of a line that starts at column `start` (a
/// highlighted token, say), so its tabs stop where the whole line's would.
pub fn expand_tabs_from(s: &str, start: usize) -> Cow<'_, str> {
    if !s.contains('\t') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    let mut col = start;
    for piece in s.split_inclusive('\t') {
        let text = piece.strip_suffix('\t');
        out.push_str(text.unwrap_or(piece));
        col += width(text.unwrap_or(piece));
        if text.is_some() {
            let n = TAB_STOP - col % TAB_STOP;
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        }
    }
    Cow::Owned(out)
}

/// One line of preformatted text, from [`verbatim`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verbatim {
    /// Blank cells before the words (the indentation).
    pub lead: usize,
    /// The words, shaped and reordered; no spaces at either end.
    pub line: Line,
    /// Blank cells after the words.
    pub trail: usize,
}

impl Verbatim {
    /// Cells the whole line takes.
    pub fn width(&self) -> usize {
        self.lead + self.line.width + self.trail
    }
}

/// One line of preformatted text — source code, a diff or log line — exactly as
/// written: never wrapped, every space kept, tabs expanded to [`TAB_STOP`] columns.
///
/// The spaces at either end come back as cell counts, outside the [`Line`]: paint
/// `lead` blank cells, the line, then `trail` blank cells. That keeps indentation
/// on the left when the words read right to left (reordering would carry it to the
/// far end), and keeps spaces out of text drawn as pixels (a proportional font).
///
/// `base` fixes the direction (code reads left to right); `None` follows the
/// words' first strong character, like [`layout`].
pub fn verbatim(text: &str, base: Option<Direction>) -> Verbatim {
    let text = expand_tabs(text);
    let words = text.trim_start_matches(' ');
    let lead = text.len() - words.len();
    let words = words.trim_end_matches(' ');
    let trail = text.len() - lead - words.len();
    let dir = base.unwrap_or_else(|| first_strong(words));
    Verbatim {
        lead,
        line: line(words.to_string(), dir),
        trail,
    }
}

/// Greedy word wrap measured on **shaped** widths (lam-alef is one cell for two letters).
///
/// Spaces at the start of the paragraph stay (they are its indentation); the one
/// space where a line breaks is dropped.
fn wrap_words(para: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    let mut line_w = 0;
    // Glue the indentation to the first word, so it is measured, kept, and only
    // ever split like a long word.
    let body = para.trim_start_matches(' ');
    let indent = &para[..para.len() - body.len()];
    let mut words = body.split(' ');
    let first = words.next().map(|w| format!("{indent}{w}"));
    for word in first.iter().map(String::as_str).chain(words) {
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

    /// Every script the tests must cover, after some indentation.
    const SCRIPTS: [&str; 5] = [
        ARABIC,
        "Hello مَرْحَباً world",
        "こんにちは 世界",
        "你好 世界",
        "नमस्ते दुनिया",
    ];

    #[test]
    fn leading_spaces_and_space_only_text_are_kept() {
        let one = |s: &str| layout(s, usize::MAX / 2, Direction::Ltr).swap_remove(0);
        assert_eq!(one("    return x").visual, "    return x");
        assert_eq!(one("    return x").width, 12);
        assert_eq!(one(" ").visual, " ", "a single space token");
        assert_eq!(one("    ").width, 4, "spaces only");
        assert_eq!(one("a  b ").visual, "a  b ", "inner and trailing spaces");
        for s in SCRIPTS {
            let indented = format!("  {s}");
            let l = one(&indented);
            assert_eq!(l.logical, indented, "{s}");
            assert_eq!(l.width, one(s).width + 2, "{s}");
        }
        // Left to right: the indentation is at the left of the cells.
        assert!(one("  こんにちは").visual.starts_with("  こ"));
        assert!(one("  नमस्ते").visual.starts_with("  न"));
        // Right to left: it is at the right, where an RTL line starts.
        let rtl = one("  مَرْحَباً");
        assert_eq!(rtl.dir, Direction::Rtl);
        assert!(rtl.visual.ends_with("  ") && !rtl.visual.starts_with(' '));
    }

    #[test]
    fn indentation_wraps_with_the_first_word() {
        let lines = layout("    one two three", 10, Direction::Ltr);
        let text: Vec<&str> = lines.iter().map(|l| l.logical.as_str()).collect();
        assert_eq!(text, ["    one", "two three"]);
        // Wider than the line: split like a long word, never lost.
        let lines = layout("        x", 4, Direction::Ltr);
        assert_eq!(lines.iter().map(|l| l.width).sum::<usize>(), 9);
        let lines = layout("  مَرْحَباً بِكُمْ فِي", 9, Direction::Ltr);
        assert!(lines[0].logical.starts_with("  مَ"), "{lines:?}");
        assert!(lines.iter().all(|l| l.width <= 9));
    }

    #[test]
    fn tabs_expand_to_four_column_stops() {
        assert_eq!(expand_tabs("\tx"), "    x");
        assert_eq!(expand_tabs("ab\tc"), "ab  c");
        assert_eq!(expand_tabs("abcd\te"), "abcd    e");
        assert_eq!(expand_tabs("\t\tx\t"), "        x   ");
        assert_eq!(
            expand_tabs("日本\tx"),
            "日本    x",
            "wide characters count two"
        );
        assert_eq!(expand_tabs("مَرْحَباً\tx"), "مَرْحَباً   x");
        assert_eq!(
            expand_tabs("नम\tx").chars().filter(|c| *c == ' ').count(),
            2
        );
        assert!(matches!(expand_tabs("no tabs"), Cow::Borrowed(_)));
        // A piece starting at column 2 stops where the whole line would.
        assert_eq!(expand_tabs_from("\tx", 2), "  x");
        assert_eq!(
            format!("ab{}", expand_tabs_from("\tx", 2)),
            expand_tabs("ab\tx")
        );
        let l = layout("\tdef f():\n\t\treturn 1", 80, Direction::Ltr);
        assert_eq!(l[0].visual, "    def f():");
        assert_eq!(l[1].visual, "        return 1");
    }

    #[test]
    fn verbatim_keeps_every_space_and_indents_on_the_left() {
        let v = verbatim("    return x  ", None);
        assert_eq!(
            (v.lead, v.line.visual.as_str(), v.trail),
            (4, "return x", 2)
        );
        assert_eq!(v.width(), 14);
        let v = verbatim("\tdef  f():", None);
        assert_eq!((v.lead, v.line.visual.as_str()), (4, "def  f():"));
        let v = verbatim("   ", None);
        assert_eq!((v.lead, v.line.width, v.trail), (3, 0, 0));
        for s in SCRIPTS {
            let v = verbatim(&format!("    {s} "), None);
            assert_eq!((v.lead, v.trail), (4, 1), "{s}");
            assert_eq!(v.line.logical, s);
            assert_eq!(v.line.width, width(&arabic::shape(s)), "{s}");
        }
        // A right-to-left line keeps its indentation on the left …
        let v = verbatim("    # مَرْحَباً", None);
        assert_eq!(v.lead, 4);
        assert_eq!(v.line.dir, Direction::Rtl);
        assert!(v.line.visual.ends_with('#'), "{:?}", v.line.visual);
        // … and code can insist on left to right: the `#` stays first.
        let v = verbatim("# مَرْحَباً", Some(Direction::Ltr));
        assert_eq!(v.line.dir, Direction::Ltr);
        assert!(v.line.visual.starts_with("# "), "{:?}", v.line.visual);
        assert_eq!(v.line.logical, "# مَرْحَباً");
    }
}
