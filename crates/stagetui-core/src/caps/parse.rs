//! Pure parser for terminal query replies.
//!
//! The probe writes a batch of queries followed by Primary Device Attributes
//! (DA1, `CSI c`). Every terminal answers DA1, and terminals answer in order,
//! so the DA1 reply marks the end of the batch. Unrelated bytes (e.g. a key the
//! user pressed during the probe) are ignored.

/// Everything we understood from the terminal's replies.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Replies {
    /// XTVERSION (`DCS > | text ST`), e.g. `kitty(0.39.1)`.
    pub xtversion: Option<String>,
    /// Kitty graphics query: `Some(true)` = OK, `Some(false)` = error reply.
    pub kitty_graphics: Option<bool>,
    /// Kitty keyboard protocol flags (`CSI ? flags u`).
    pub kitty_keyboard: Option<u32>,
    /// DECRPM replies: `(mode, value)`; value 1/2/3 = supported, 0/4 = not.
    pub modes: Vec<(u32, u32)>,
    /// Cell size in pixels `(width, height)` from `CSI 16 t`.
    pub cell_px: Option<(u32, u32)>,
    /// Text-area size in pixels `(width, height)` from `CSI 14 t`.
    pub window_px: Option<(u32, u32)>,
    /// Primary Device Attributes parameters (`CSI ? … c`). `4` = Sixel.
    pub da1: Option<Vec<u32>>,
    /// Default background color from OSC 11 (`rgb:RRRR/GGGG/BBBB`).
    pub background: Option<(u8, u8, u8)>,
}

impl Replies {
    /// The DECRPM value reported for `mode`, if any.
    pub fn mode(&self, mode: u32) -> Option<u32> {
        self.modes.iter().find(|(m, _)| *m == mode).map(|(_, v)| *v)
    }

    /// Whether DECRPM reported `mode` as recognised (set, reset, or permanently set).
    pub fn mode_supported(&self, mode: u32) -> bool {
        matches!(self.mode(mode), Some(1..=3))
    }
}

/// The image id used by the Kitty graphics probe (`a=q`).
pub const KITTY_PROBE_ID: u32 = 31;

/// Parse every reply found in `bytes`.
pub fn parse(bytes: &[u8]) -> Replies {
    let mut out = Replies::default();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != 0x1b {
            i += 1;
            continue;
        }
        match bytes.get(i + 1) {
            Some(b'P') => match string_until_st(bytes, i + 2) {
                Some((body, end)) => {
                    handle_dcs(body, &mut out);
                    i = end;
                }
                None => break,
            },
            Some(b']') => match string_until_st(bytes, i + 2) {
                Some((body, end)) => {
                    handle_osc(body, &mut out);
                    i = end;
                }
                None => break,
            },
            Some(b'_') => match string_until_st(bytes, i + 2) {
                Some((body, end)) => {
                    handle_apc(body, &mut out);
                    i = end;
                }
                None => break,
            },
            Some(b'[') => match csi(bytes, i + 2) {
                Some((seq, end)) => {
                    handle_csi(&seq, &mut out);
                    i = end;
                }
                None => i += 2,
            },
            _ => i += 1,
        }
    }
    out
}

/// True once the DA1 reply (our end-of-batch marker) has arrived.
pub fn has_da1(bytes: &[u8]) -> bool {
    parse(bytes).da1.is_some()
}

/// A string-type control sequence body, terminated by ST (`ESC \`) or BEL.
fn string_until_st(bytes: &[u8], start: usize) -> Option<(&[u8], usize)> {
    let mut j = start;
    while j < bytes.len() {
        match bytes[j] {
            0x07 => return Some((&bytes[start..j], j + 1)),
            0x1b if bytes.get(j + 1) == Some(&b'\\') => return Some((&bytes[start..j], j + 2)),
            _ => j += 1,
        }
    }
    None
}

struct Csi {
    private: Option<u8>,
    params: Vec<u32>,
    intermediates: Vec<u8>,
    final_byte: u8,
}

fn csi(bytes: &[u8], start: usize) -> Option<(Csi, usize)> {
    let mut j = start;
    let mut private = None;
    if let Some(&b) = bytes.get(j)
        && matches!(b, b'<' | b'=' | b'>' | b'?')
    {
        private = Some(b);
        j += 1;
    }
    let params_start = j;
    while j < bytes.len() && (0x30..=0x3f).contains(&bytes[j]) {
        j += 1;
    }
    let params_raw = &bytes[params_start..j];
    let inter_start = j;
    while j < bytes.len() && (0x20..=0x2f).contains(&bytes[j]) {
        j += 1;
    }
    let intermediates = bytes[inter_start..j].to_vec();
    let final_byte = *bytes.get(j)?;
    if !(0x40..=0x7e).contains(&final_byte) {
        return None;
    }
    let params = std::str::from_utf8(params_raw)
        .ok()?
        .split(';')
        .map(|p| {
            p.split(':')
                .next()
                .unwrap_or("")
                .parse::<u32>()
                .unwrap_or(0)
        })
        .collect();
    Some((
        Csi {
            private,
            params,
            intermediates,
            final_byte,
        },
        j + 1,
    ))
}

fn handle_csi(seq: &Csi, out: &mut Replies) {
    let p = |n: usize| seq.params.get(n).copied().unwrap_or(0);
    match (seq.private, seq.intermediates.as_slice(), seq.final_byte) {
        (Some(b'?'), [], b'u') => out.kitty_keyboard = Some(p(0)),
        (Some(b'?'), b"$", b'y') => out.modes.push((p(0), p(1))),
        (Some(b'?'), [], b'c') => out.da1 = Some(seq.params.clone()),
        (None, [], b't') if p(0) == 6 && p(1) > 0 && p(2) > 0 => out.cell_px = Some((p(2), p(1))),
        (None, [], b't') if p(0) == 4 && p(1) > 0 && p(2) > 0 => out.window_px = Some((p(2), p(1))),
        _ => {}
    }
}

fn handle_dcs(body: &[u8], out: &mut Replies) {
    if let Some(rest) = body.strip_prefix(b">|") {
        out.xtversion = Some(String::from_utf8_lossy(rest).trim().to_string());
    }
}

fn handle_osc(body: &[u8], out: &mut Replies) {
    let text = String::from_utf8_lossy(body);
    if let Some(spec) = text.strip_prefix("11;") {
        out.background = parse_x_color(spec);
    }
}

/// Parse an X11 color spec `rgb:R/G/B` (1–4 hex digits per channel) to 8-bit RGB.
pub fn parse_x_color(spec: &str) -> Option<(u8, u8, u8)> {
    let rest = spec.trim().strip_prefix("rgb:")?;
    let mut parts = rest.split('/').map(|h| {
        let v = u32::from_str_radix(h, 16).ok()?;
        let max = (1u32 << (4 * h.len().clamp(1, 4))) - 1;
        Some(((v * 255 + max / 2) / max) as u8)
    });
    Some((parts.next()??, parts.next()??, parts.next()??))
}

fn handle_apc(body: &[u8], out: &mut Replies) {
    let Some(rest) = body.strip_prefix(b"G") else {
        return;
    };
    let text = String::from_utf8_lossy(rest);
    let (params, message) = text.split_once(';').unwrap_or((&text, ""));
    let is_probe = params
        .split(',')
        .filter_map(|kv| kv.split_once('='))
        .any(|(k, v)| k == "i" && v.parse::<u32>().ok() == Some(KITTY_PROBE_ID));
    if is_probe {
        out.kitty_graphics = Some(message == "OK");
    }
}

/// Split an XTVERSION string into `(name, version)`.
///
/// Handles `kitty(0.39.1)`, `WezTerm 20240203-110809-5046fc22`, `ghostty 1.2.0`, `tmux 3.5a`.
pub fn split_xtversion(s: &str) -> (String, Option<String>) {
    let s = s.trim();
    if let Some((name, rest)) = s.split_once('(') {
        let version = rest.trim_end_matches(')').trim();
        return (
            name.trim().to_string(),
            (!version.is_empty()).then(|| version.to_string()),
        );
    }
    match s.split_once(' ') {
        Some((name, version)) => (name.to_string(), Some(version.trim().to_string())),
        None => (s.to_string(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Recorded / representative replies from real terminals.
    const KITTY: &[u8] = b"\x1bP>|kitty(0.39.1)\x1b\\\x1b_Gi=31;OK\x1b\\\x1b[?0u\x1b[?2026;2$y\x1b[6;21;10t\x1b[4;840;1200t\x1b[?62;c";
    const GHOSTTY: &[u8] = b"\x1bP>|ghostty 1.2.0\x1b\\\x1b_Gi=31;OK\x1b\\\x1b[?0u\x1b[?2026;2$y\x1b[6;19;9t\x1b[?62;22c";
    const WEZTERM: &[u8] = b"\x1bP>|WezTerm 20240203-110809-5046fc22\x1b\\\x1b_Gi=31;OK\x1b\\\x1b[?2026;2$y\x1b[?65;4;6;18;22c";
    const FOOT: &[u8] =
        b"\x1bP>|foot(1.21.0)\x1b\\\x1b[?0u\x1b[?2026;2$y\x1b[6;20;10t\x1b[?62;4;22c";
    const XTERM: &[u8] =
        b"\x1bP>|XTerm(397)\x1b\\\x1b[?2026;0$y\x1b[?64;1;2;4;6;9;15;16;17;18;21;22;28c";
    const WINDOWS_TERMINAL: &[u8] = b"\x1b[?2026;2$y\x1b[?61;4;6;7;14;21;22;23;24;28;32;42c";
    const VTE: &[u8] = b"\x1bP>|VTE(8000)\x1b\\\x1b[?2026;2$y\x1b[?65;1;9c";
    const KITTY_GRAPHICS_ERROR: &[u8] = b"\x1b_Gi=31;ENOTSUPPORTED:no\x1b\\\x1b[?62c";

    #[test]
    fn kitty() {
        let r = parse(KITTY);
        assert_eq!(r.xtversion.as_deref(), Some("kitty(0.39.1)"));
        assert_eq!(r.kitty_graphics, Some(true));
        assert_eq!(r.kitty_keyboard, Some(0));
        assert!(r.mode_supported(2026));
        assert_eq!(r.cell_px, Some((10, 21)));
        assert_eq!(r.window_px, Some((1200, 840)));
        assert_eq!(r.da1, Some(vec![62, 0]));
    }

    #[test]
    fn ghostty_and_wezterm() {
        let g = parse(GHOSTTY);
        assert_eq!(
            split_xtversion(g.xtversion.as_deref().unwrap()),
            ("ghostty".into(), Some("1.2.0".into()))
        );
        assert_eq!(g.kitty_graphics, Some(true));
        let w = parse(WEZTERM);
        assert_eq!(
            split_xtversion(w.xtversion.as_deref().unwrap()).0,
            "WezTerm"
        );
        assert!(w.da1.unwrap().contains(&4));
        assert_eq!(w.kitty_keyboard, None);
    }

    #[test]
    fn sixel_terminals() {
        assert!(parse(FOOT).da1.unwrap().contains(&4));
        assert!(parse(XTERM).da1.unwrap().contains(&4));
        let wt = parse(WINDOWS_TERMINAL);
        assert!(wt.da1.as_ref().unwrap().contains(&4));
        assert!(wt.mode_supported(2026));
    }

    #[test]
    fn unsupported_mode_and_graphics_error() {
        assert!(!parse(XTERM).mode_supported(2026));
        assert_eq!(parse(KITTY_GRAPHICS_ERROR).kitty_graphics, Some(false));
        assert!(!parse(VTE).da1.unwrap().contains(&4));
    }

    #[test]
    fn noise_and_truncation_are_ignored() {
        let mut bytes = b"q\x1b[A".to_vec();
        bytes.extend_from_slice(KITTY);
        bytes.extend_from_slice(b"\x1bP>|unterminated");
        let r = parse(&bytes);
        assert_eq!(r.kitty_graphics, Some(true));
        assert!(has_da1(&bytes));
        assert!(!has_da1(b"\x1b[?62"));
    }

    #[test]
    fn bel_terminated_strings() {
        let r = parse(b"\x1bP>|tmux 3.5a\x07\x1b[?1;2c");
        assert_eq!(
            split_xtversion(r.xtversion.as_deref().unwrap()),
            ("tmux".into(), Some("3.5a".into()))
        );
    }

    #[test]
    fn background_color_reply() {
        let r = parse(b"\x1b]11;rgb:1a1a/1b1b/2626\x1b\\\x1b[?62c");
        assert_eq!(r.background, Some((0x1a, 0x1b, 0x26)));
        let r = parse(b"\x1b]11;rgb:ff/80/00\x07\x1b[?62c");
        assert_eq!(r.background, Some((255, 128, 0)));
        assert_eq!(parse_x_color("rgb:f/0/8"), Some((255, 0, 136)));
        assert_eq!(parse_x_color("#123456"), None);
    }

    #[test]
    fn xtversion_shapes() {
        assert_eq!(
            split_xtversion("XTerm(397)"),
            ("XTerm".into(), Some("397".into()))
        );
        assert_eq!(split_xtversion("mystery"), ("mystery".into(), None));
    }
}
