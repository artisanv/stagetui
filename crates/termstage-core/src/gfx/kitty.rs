//! Kitty graphics protocol encoder.
//!
//! Two ways to show an image:
//! - **Unicode placeholders** (Kitty ≥ 0.28, Ghostty): transmit once, create a
//!   *virtual* placement, then write placeholder characters into cells. The
//!   image becomes part of the text grid: it scrolls, clips, diffs and works in tmux.
//! - **Direct placement** (`a=T`): draw at the cursor (other Kitty-compatible terminals).
//!
//! Spec: <https://sw.kovidgoyal.net/kitty/graphics-protocol/>

use super::base64;
use super::diacritics::DIACRITICS;

/// The placeholder character (Supplementary Private Use Area-B).
pub const PLACEHOLDER: char = '\u{10EEEE}';
/// Bytes of base64 per escape-sequence chunk (protocol maximum is 4096).
const CHUNK: usize = 4096;

/// Wrap one escape in tmux passthrough when needed.
fn wrap(seq: String, tmux: bool) -> String {
    if tmux {
        crate::caps::tmux_passthrough(&seq)
    } else {
        seq
    }
}

/// Send `payload` (already-encoded image bytes) with control keys `keys`,
/// split into protocol-sized chunks.
fn chunked(keys: &str, payload: &[u8], tmux: bool) -> String {
    let b64 = base64::encode(payload);
    let mut out = String::with_capacity(b64.len() + 64);
    let chunks: Vec<&str> = if b64.is_empty() {
        vec![""]
    } else {
        b64.as_bytes()
            .chunks(CHUNK)
            .map(|c| std::str::from_utf8(c).unwrap_or_default())
            .collect()
    };
    let last = chunks.len() - 1;
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i < last);
        let seq = if i == 0 {
            format!("\x1b_G{keys},m={more};{chunk}\x1b\\")
        } else {
            format!("\x1b_Gm={more};{chunk}\x1b\\")
        };
        out.push_str(&wrap(seq, tmux));
    }
    out
}

/// Transmit a PNG and keep it in the terminal under `id` (no display yet).
pub fn transmit_png(id: u32, png: &[u8], tmux: bool) -> String {
    chunked(&format!("a=t,f=100,t=d,i={id},q=2"), png, tmux)
}

/// Create a virtual placement for Unicode placeholders, `cols × rows` cells.
pub fn virtual_placement(id: u32, cols: u16, rows: u16, tmux: bool) -> String {
    wrap(
        format!("\x1b_Ga=p,U=1,i={id},c={cols},r={rows},q=2\x1b\\"),
        tmux,
    )
}

/// Transmit and display a PNG at the cursor, scaled to `cols × rows` cells.
/// `move_cursor = false` keeps the cursor where it was (`C=1`).
pub fn display_png(
    id: u32,
    png: &[u8],
    cols: u16,
    rows: u16,
    move_cursor: bool,
    tmux: bool,
) -> String {
    let c = if move_cursor { "" } else { ",C=1" };
    chunked(
        &format!("a=T,f=100,t=d,i={id},c={cols},r={rows}{c},q=2"),
        png,
        tmux,
    )
}

/// Delete an image and free its data.
pub fn delete(id: u32, tmux: bool) -> String {
    wrap(format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\"), tmux)
}

/// The text for one placeholder cell at `(row, col)` of an image.
pub fn placeholder(row: u16, col: u16) -> String {
    let d = |n: u16| {
        DIACRITICS
            .get(usize::from(n))
            .copied()
            .unwrap_or(DIACRITICS[0])
    };
    let mut s = String::with_capacity(12);
    s.push(PLACEHOLDER);
    s.push(d(row));
    s.push(d(col));
    s
}

/// Largest image size (cells per side) addressable with placeholder diacritics.
pub const MAX_PLACEHOLDER_CELLS: u16 = DIACRITICS.len() as u16;

/// The foreground color that encodes `id` for placeholder cells (24-bit).
pub fn id_color(id: u32) -> (u8, u8, u8) {
    ((id >> 16) as u8, (id >> 8) as u8, id as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_payload_is_one_chunk() {
        let s = transmit_png(7, b"abc", false);
        assert_eq!(s, "\x1b_Ga=t,f=100,t=d,i=7,q=2,m=0;YWJj\x1b\\");
    }

    #[test]
    fn large_payload_is_chunked() {
        let payload = vec![0u8; 6000]; // 8000 base64 chars → 2 chunks
        let s = transmit_png(1, &payload, false);
        assert_eq!(s.matches("\x1b_G").count(), 2);
        assert!(s.contains(",m=1;") && s.contains("\x1b_Gm=0;"));
        for part in s.split("\x1b\\").filter(|p| !p.is_empty()) {
            let data = part.split_once(';').unwrap().1;
            assert!(data.len() <= CHUNK);
        }
    }

    #[test]
    fn placements_and_deletes() {
        assert_eq!(
            virtual_placement(5, 10, 4, false),
            "\x1b_Ga=p,U=1,i=5,c=10,r=4,q=2\x1b\\"
        );
        assert_eq!(delete(5, false), "\x1b_Ga=d,d=I,i=5,q=2\x1b\\");
        assert!(
            display_png(2, b"x", 3, 2, false, false)
                .starts_with("\x1b_Ga=T,f=100,t=d,i=2,c=3,r=2,C=1,q=2,m=0;")
        );
    }

    #[test]
    fn tmux_passthrough_doubles_escapes() {
        let s = delete(5, true);
        assert!(s.starts_with("\x1bPtmux;\x1b\x1b_G"));
        assert!(s.ends_with("\x1b\x1b\\\x1b\\"));
    }

    #[test]
    fn placeholder_cells() {
        let p = placeholder(0, 2);
        let chars: Vec<char> = p.chars().collect();
        assert_eq!(chars, [PLACEHOLDER, '\u{0305}', '\u{030E}']);
        assert_eq!(crate::text::width(&p), 1);
        assert_eq!(id_color(0x0001_0203), (1, 2, 3));
        assert_eq!(MAX_PLACEHOLDER_CELLS, 297);
    }
}
