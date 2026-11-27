//! STL (3D printing): binary or text. Faces are drawn flat, like the printed part.

use std::path::Path;

use crate::math::{V3, v3};
use crate::model::Model;

pub fn load(path: &Path) -> Result<Model, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("couldn't read \"{}\": {e}", path.display()))?;
    parse(&bytes).map_err(|e| format!("couldn't read \"{}\": {e}", path.display()))
}

pub fn parse(bytes: &[u8]) -> Result<Model, String> {
    let corners = if is_binary(bytes) {
        binary(bytes)?
    } else {
        text(&String::from_utf8_lossy(bytes))?
    };
    let mut m = Model::default();
    for tri in corners.as_chunks::<3>().0 {
        let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]).norm();
        let base = m.positions.len() as u32;
        for &p in tri {
            m.positions.push(p);
            m.normals.push(n);
        }
        m.triangles.push([base, base + 1, base + 2]);
    }
    Ok(m)
}

/// Binary files have an 80-byte header and a triangle count that matches the size.
/// (Some binary files also start with "solid", so the size decides.)
fn is_binary(bytes: &[u8]) -> bool {
    if bytes.len() < 84 {
        return false;
    }
    let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    count.checked_mul(50).and_then(|b| b.checked_add(84)) == Some(bytes.len())
}

fn binary(bytes: &[u8]) -> Result<Vec<V3>, String> {
    let f =
        |at: usize| f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let count = (bytes.len() - 84) / 50;
    let mut out = Vec::with_capacity(count * 3);
    for t in 0..count {
        let at = 84 + t * 50 + 12; // skip the stored normal
        for c in 0..3 {
            let p = at + c * 12;
            out.push(v3(f(p), f(p + 4), f(p + 8)));
        }
    }
    Ok(out)
}

fn text(s: &str) -> Result<Vec<V3>, String> {
    if !s.trim_start().starts_with("solid") {
        return Err("this is not an STL file".into());
    }
    let mut out = Vec::new();
    let mut words = s.split_whitespace();
    while let Some(w) = words.next() {
        if w == "vertex" {
            let mut xyz = [0.0f32; 3];
            for c in &mut xyz {
                *c = words
                    .next()
                    .and_then(|n| n.parse().ok())
                    .ok_or("a vertex needs three numbers")?;
            }
            out.push(v3(xyz[0], xyz[1], xyz[2]));
        }
    }
    out.truncate(out.len() / 3 * 3);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_stl() {
        let s = "solid t\nfacet normal 0 0 0\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
        let m = parse(s.as_bytes()).unwrap();
        assert_eq!(m.triangles.len(), 1);
        assert_eq!(
            m.normals[0],
            v3(0.0, 0.0, 1.0),
            "made from the corners, not the (empty) stored one"
        );
    }

    #[test]
    fn binary_stl_even_when_it_starts_with_solid() {
        let mut b = b"solid but actually binary".to_vec();
        b.resize(80, 0);
        b.extend(2u32.to_le_bytes());
        for t in 0..2 {
            b.extend([0u8; 12]);
            for p in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, t as f32]] {
                for c in p {
                    b.extend(c.to_le_bytes());
                }
            }
            b.extend([0u8; 2]);
        }
        let m = parse(&b).unwrap();
        assert_eq!(m.triangles.len(), 2);
        assert_eq!(m.positions[5], v3(0.0, 1.0, 1.0));
        assert!(parse(b"hello").is_err());
    }
}
