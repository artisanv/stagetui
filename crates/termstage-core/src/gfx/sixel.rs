//! Sixel encoder (Windows Terminal ≥ 1.22, foot, xterm, WezTerm, iTerm2, mlterm, …).
//!
//! Pixels with alpha < 50% are left transparent (`P2 = 1`). Images with up to 256
//! distinct colors use an exact palette; others are quantized with NeuQuant.

use std::collections::HashMap;
use std::fmt::Write as _;

use tiny_skia::Pixmap;

/// Encode a pixmap as a complete Sixel escape sequence.
pub fn encode(p: &Pixmap) -> String {
    let (w, h) = (p.width() as usize, p.height() as usize);
    let rgba = super::to_rgba(p);
    let (palette, index) = quantize(&rgba);

    let mut out = String::with_capacity(w * h / 2 + 256);
    out.push_str("\x1bP0;1;0q");
    let _ = write!(out, "\"1;1;{w};{h}");
    for (i, [r, g, b]) in palette.iter().enumerate() {
        let pct = |v: u8| (u32::from(v) * 100 + 127) / 255;
        let _ = write!(out, "#{i};2;{};{};{}", pct(*r), pct(*g), pct(*b));
    }

    let n = palette.len();
    let mut bands = vec![0u8; n * w];
    let mut used = vec![false; n];
    for band_top in (0..h).step_by(6) {
        bands.iter_mut().for_each(|b| *b = 0);
        used.iter_mut().for_each(|u| *u = false);
        for dy in 0..6.min(h - band_top) {
            let y = band_top + dy;
            for x in 0..w {
                if let Some(ci) = index[y * w + x] {
                    let ci = usize::from(ci);
                    bands[ci * w + x] |= 1 << dy;
                    used[ci] = true;
                }
            }
        }
        let mut first = true;
        for ci in (0..n).filter(|&c| used[c]) {
            if !first {
                out.push('$');
            }
            first = false;
            let _ = write!(out, "#{ci}");
            rle(&bands[ci * w..(ci + 1) * w], &mut out);
        }
        out.push('-');
    }
    out.push_str("\x1b\\");
    out
}

fn rle(row: &[u8], out: &mut String) {
    let mut i = 0;
    while i < row.len() {
        let v = row[i];
        let mut j = i + 1;
        while j < row.len() && row[j] == v {
            j += 1;
        }
        let ch = char::from(63 + v);
        let run = j - i;
        if run >= 4 {
            let _ = write!(out, "!{run}{ch}");
        } else {
            for _ in 0..run {
                out.push(ch);
            }
        }
        i = j;
    }
}

/// `(palette, per-pixel palette index or None for transparent)`.
fn quantize(rgba: &[u8]) -> (Vec<[u8; 3]>, Vec<Option<u8>>) {
    let (pixels, _) = rgba.as_chunks::<4>();
    let opaque = |px: &[u8; 4]| px[3] >= 128;
    let mut exact: HashMap<[u8; 3], u8> = HashMap::new();
    let mut palette = Vec::new();
    let mut fits = true;
    for px in pixels.iter().filter(|px| opaque(px)) {
        let key = [px[0], px[1], px[2]];
        if let std::collections::hash_map::Entry::Vacant(slot) = exact.entry(key) {
            if palette.len() == 256 {
                fits = false;
                break;
            }
            slot.insert(palette.len() as u8);
            palette.push(key);
        }
    }
    if fits {
        let idx = pixels
            .iter()
            .map(|px| opaque(px).then(|| exact[&[px[0], px[1], px[2]]]))
            .collect();
        return (palette, idx);
    }
    let nq = color_quant::NeuQuant::new(10, 256, rgba);
    let map = nq.color_map_rgb();
    let palette: Vec<[u8; 3]> = map.as_chunks::<3>().0.to_vec();
    let idx = pixels
        .iter()
        .map(|px| opaque(px).then(|| nq.index_of(px) as u8))
        .collect();
    (palette, idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::Color;

    #[test]
    fn two_by_two_red() {
        let mut p = Pixmap::new(2, 2).unwrap();
        p.fill(Color::from_rgba8(255, 0, 0, 255));
        // Rows 0–1 set → bits 0b000011 → char 63+3 = 'B'.
        assert_eq!(encode(&p), "\x1bP0;1;0q\"1;1;2;2#0;2;100;0;0#0BB-\x1b\\");
    }

    #[test]
    fn rle_and_transparency() {
        let mut p = Pixmap::new(8, 1).unwrap();
        p.fill(Color::from_rgba8(0, 0, 255, 255));
        p.pixels_mut()[0] = tiny_skia::PremultipliedColorU8::TRANSPARENT;
        let s = encode(&p);
        assert!(s.contains("#0?!7@"), "{s:?}"); // 1 empty column then 7 × bit0
    }

    #[test]
    fn many_colors_are_quantized() {
        let mut p = Pixmap::new(64, 64).unwrap();
        for (i, px) in p.pixels_mut().iter_mut().enumerate() {
            let (r, g) = ((i % 64 * 4) as u8, (i / 64 * 4) as u8);
            *px = tiny_skia::PremultipliedColorU8::from_rgba(r, g, 128, 255).unwrap();
        }
        let s = encode(&p);
        let registers = s.matches(";2;").count();
        assert!(registers <= 256 && registers > 16, "{registers}");
        assert_eq!(s.matches('-').count(), 64usize.div_ceil(6));
    }
}
