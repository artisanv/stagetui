//! Arabic (and Persian/Urdu) contextual shaping into Unicode presentation forms.
//!
//! Terminals draw one glyph per cell. Most do not join Arabic letters, so we
//! pick the right *form* of each letter ourselves (isolated, final, initial,
//! medial) and the lam-alef ligatures. The output is still in **logical**
//! order; [`super::bidi`] turns it into visual order.

use super::arabic_table::{FORMS, LAM_ALEF};

const LAM: char = '\u{0644}';
const TATWEEL: char = '\u{0640}';
const ZWJ: char = '\u{200D}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Joining {
    /// Joins on both sides (ب).
    Dual,
    /// Joins only to the previous letter (ا د ر و).
    Right,
    /// Forces joining on both sides (tatweel, ZWJ).
    Causing,
    /// Ignored when deciding joins (harakat and other combining marks).
    Transparent,
    /// Breaks joining (spaces, Latin, digits, hamza, ZWNJ).
    None,
}

fn forms(c: char) -> Option<[u32; 4]> {
    FORMS
        .binary_search_by_key(&c, |(k, _)| *k)
        .ok()
        .map(|i| FORMS[i].1)
}

fn lam_alef(alef: char) -> Option<[u32; 2]> {
    LAM_ALEF.iter().find(|(k, _)| *k == alef).map(|(_, f)| *f)
}

/// Combining marks used with Arabic script (plus the generic combining block).
pub fn is_transparent(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F
        | 0x0610..=0x061A
        | 0x064B..=0x065F
        | 0x0670
        | 0x06D6..=0x06DC
        | 0x06DF..=0x06E4
        | 0x06E7..=0x06E8
        | 0x06EA..=0x06ED
        | 0x08D3..=0x08E1
        | 0x08E3..=0x08FF
        | 0xFE20..=0xFE2F)
}

fn joining(c: char) -> Joining {
    if is_transparent(c) {
        return Joining::Transparent;
    }
    if c == TATWEEL || c == ZWJ {
        return Joining::Causing;
    }
    match forms(c) {
        Some(f) if f[2] != 0 => Joining::Dual,
        Some(f) if f[1] != 0 => Joining::Right,
        _ => Joining::None,
    }
}

/// Can `j` connect to the letter after it?
fn joins_next(j: Joining) -> bool {
    matches!(j, Joining::Dual | Joining::Causing)
}

/// Can `j` connect to the letter before it?
fn joins_prev(j: Joining) -> bool {
    matches!(j, Joining::Dual | Joining::Right | Joining::Causing)
}

/// Shape a logical-order string. Non-Arabic text passes through untouched.
pub fn shape(s: &str) -> String {
    if !s.chars().any(|c| forms(c).is_some()) {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let kinds: Vec<Joining> = chars.iter().map(|&c| joining(c)).collect();
    let prev_solid = |i: usize| (0..i).rev().find(|&j| kinds[j] != Joining::Transparent);
    let next_solid = |i: usize| (i + 1..chars.len()).find(|&j| kinds[j] != Joining::Transparent);

    let mut out = String::with_capacity(s.len() + 8);
    let mut i = 0;
    // `joined_prev[i]`: the solid char before i actually connects to i.
    let mut prev_connects_forward = false;
    while i < chars.len() {
        let c = chars[i];
        let kind = kinds[i];
        if kind == Joining::Transparent {
            out.push(c);
            i += 1;
            continue;
        }

        let connect_prev = joins_prev(kind) && prev_solid(i).is_some() && prev_connects_forward;
        let next = next_solid(i);

        // Lam + alef → one ligature glyph.
        if c == LAM
            && let Some(n) = next
            && let Some(lig) = lam_alef(chars[n])
        {
            let form = if connect_prev { lig[1] } else { lig[0] };
            out.push(char::from_u32(form).unwrap_or(c));
            // Keep any marks that sat between lam and alef, then those after alef.
            for &m in &chars[i + 1..n] {
                out.push(m);
            }
            i = n + 1;
            prev_connects_forward = false; // the ligature ends like alef: right-joining
            continue;
        }

        let connect_next = joins_next(kind) && next.is_some_and(|n| joins_prev(kinds[n]));
        match forms(c) {
            Some(f) => {
                let pick = match (connect_prev, connect_next) {
                    (false, false) => f[0],
                    (true, false) => f[1],
                    (false, true) => f[2],
                    (true, true) => f[3],
                };
                let pick = if pick == 0 { f[0] } else { pick };
                out.push(char::from_u32(pick).unwrap_or(c));
            }
            None => out.push(c),
        }
        prev_connects_forward = connect_next;
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cps(s: &str) -> Vec<u32> {
        s.chars().map(|c| c as u32).collect()
    }

    #[test]
    fn marhaba() {
        // م (initial) ر (final) ح (initial) ب (medial) ا (final)
        assert_eq!(
            cps(&shape("مرحبا")),
            [0xFEE3, 0xFEAE, 0xFEA3, 0xFE92, 0xFE8E]
        );
    }

    #[test]
    fn harakat_are_transparent_and_kept() {
        let shaped = shape("مَرْحَباً");
        assert_eq!(
            cps(&shaped),
            [
                0xFEE3, 0x064E, 0xFEAE, 0x0652, 0xFEA3, 0x064E, 0xFE92, 0xFE8E, 0x064B
            ]
        );
    }

    #[test]
    fn lam_alef_ligatures() {
        assert_eq!(cps(&shape("لا")), [0xFEFB]);
        assert_eq!(cps(&shape("سلام")), [0xFEB3, 0xFEFC, 0xFEE1]);
        assert_eq!(cps(&shape("إلى")), [0xFE87, 0xFEDF, 0xFEF0]);
        assert_eq!(cps(&shape("لأن")), [0xFEF7, 0xFEE5]);
    }

    #[test]
    fn isolated_letters_and_words() {
        assert_eq!(cps(&shape("ب")), [0xFE8F]);
        assert_eq!(cps(&shape("ب ب")), [0xFE8F, 0x20, 0xFE8F]);
        // Right-joining letters break the chain: د cannot join forward.
        assert_eq!(cps(&shape("بدب")), [0xFE91, 0xFEAA, 0xFE8F]);
    }

    #[test]
    fn persian_letters() {
        // پ چ گ ی take their Forms-A shapes.
        assert_eq!(cps(&shape("پی")), [0xFB58, 0xFBFD]);
        assert_eq!(cps(&shape("گل")), [0xFB94, 0xFEDE]);
    }

    #[test]
    fn tatweel_and_zwj_force_joins() {
        assert_eq!(cps(&shape("بـ")), [0xFE91, 0x0640]);
        assert_eq!(cps(&shape("ب\u{200D}")), [0xFE91, 0x200D]);
    }

    #[test]
    fn non_arabic_passes_through() {
        assert_eq!(shape("hello 123"), "hello 123");
        assert_eq!(shape("こんにちは"), "こんにちは");
        assert_eq!(cps(&shape("hi مرحبا!")).len(), "hi مرحبا!".chars().count());
    }
}
