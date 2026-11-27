//! A model ready to draw: triangles with normals, colors and (optionally) textures.

use std::collections::HashSet;
use std::path::Path;

use crate::math::{V3, v3};

/// A picture on the surface, stored as sRGB bytes.
#[derive(Debug, Clone)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<[u8; 4]>,
}

impl Texture {
    /// Largest side kept: bigger pictures are scaled down (a pane is never that big).
    pub const MAX: u32 = 1024;

    pub fn from_image(img: image::DynamicImage) -> Texture {
        let img = if img.width().max(img.height()) > Self::MAX {
            img.resize(Self::MAX, Self::MAX, image::imageops::FilterType::Triangle)
        } else {
            img
        };
        let rgba = img.to_rgba8();
        Texture {
            width: rgba.width(),
            height: rgba.height(),
            rgba: rgba.pixels().map(|p| p.0).collect(),
        }
    }

    /// The color at `uv` (repeating), as sRGB bytes.
    pub fn sample(&self, u: f32, v: f32) -> [u8; 4] {
        let fx = u - u.floor();
        let fy = v - v.floor();
        let x = ((fx * self.width as f32) as u32).min(self.width - 1);
        let y = ((fy * self.height as f32) as u32).min(self.height - 1);
        self.rgba[(y * self.width + x) as usize]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    /// Linear RGBA.
    pub color: [f32; 4],
    pub texture: Option<usize>,
}

/// A soft clay color for models that bring none (STL, plain OBJ).
pub const CLAY: [f32; 4] = [0.62, 0.60, 0.58, 1.0];

impl Default for Material {
    fn default() -> Self {
        Material {
            color: CLAY,
            texture: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Model {
    pub positions: Vec<V3>,
    pub normals: Vec<V3>,
    pub uvs: Vec<[f32; 2]>,
    /// Linear RGBA per vertex.
    pub colors: Vec<[f32; 4]>,
    pub triangles: Vec<[u32; 3]>,
    /// Material of each triangle.
    pub tri_material: Vec<u32>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    /// Bottom of the model after `fit` (where the floor grid goes).
    pub floor: f32,
}

/// Which way is up in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Up {
    Y,
    Z,
}

impl Model {
    /// Fill in what the file left out: normals (smooth), texture coordinates, colors, materials.
    pub fn finish(&mut self) {
        let n = self.positions.len();
        if self.normals.len() != n {
            self.normals = smooth_normals(&self.positions, &self.triangles);
        }
        self.uvs.resize(n, [0.0, 0.0]);
        self.colors.resize(n, [1.0; 4]);
        if self.materials.is_empty() {
            self.materials.push(Material::default());
        }
        self.tri_material.resize(self.triangles.len(), 0);
        let last = self.materials.len() as u32 - 1;
        for m in &mut self.tri_material {
            *m = (*m).min(last);
        }
        // Drop triangles that point outside the vertex list or are not finite.
        let ok = |t: &[u32; 3]| t.iter().all(|&i| (i as usize) < n);
        let keep: Vec<bool> = self.triangles.iter().map(ok).collect();
        if keep.iter().any(|k| !k) {
            let mut i = 0;
            self.triangles.retain(|_| {
                i += 1;
                keep[i - 1]
            });
            let mut j = 0;
            self.tri_material.retain(|_| {
                j += 1;
                keep[j - 1]
            });
        }
    }

    pub fn bounds(&self) -> (V3, V3) {
        let mut lo = v3(f32::MAX, f32::MAX, f32::MAX);
        let mut hi = v3(f32::MIN, f32::MIN, f32::MIN);
        for p in self.positions.iter().filter(|p| p.is_finite()) {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        (lo, hi)
    }

    /// Turn Z-up files upright, then center the model and make it about 2 units across.
    pub fn fit(&mut self, up: Up) {
        if up == Up::Z {
            for p in &mut self.positions {
                *p = v3(p.x, p.z, -p.y);
            }
            for nr in &mut self.normals {
                *nr = v3(nr.x, nr.z, -nr.y);
            }
        }
        let (lo, hi) = self.bounds();
        if lo.x > hi.x {
            return;
        }
        let center = (lo + hi) * 0.5;
        let radius = ((hi - lo) * 0.5).len().max(1e-12);
        let s = 1.0 / radius;
        for p in &mut self.positions {
            *p = (*p - center) * s;
        }
        self.floor = (lo.y - center.y) * s;
    }

    /// Each edge once (for the wireframe).
    pub fn edges(&self) -> Vec<[u32; 2]> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for t in &self.triangles {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let key = (a.min(b), a.max(b));
                if seen.insert(key) {
                    out.push([key.0, key.1]);
                }
            }
        }
        out
    }
}

/// Area-weighted normals, shared by triangles that share a vertex.
pub fn smooth_normals(positions: &[V3], triangles: &[[u32; 3]]) -> Vec<V3> {
    let mut acc = vec![V3::default(); positions.len()];
    for t in triangles {
        let [a, b, c] = t.map(|i| i as usize);
        if a.max(b).max(c) >= positions.len() {
            continue;
        }
        let n = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
        if n.is_finite() {
            for i in [a, b, c] {
                acc[i] = acc[i] + n;
            }
        }
    }
    acc.into_iter().map(V3::norm).collect()
}

/// Open a model by its file name: glTF (`.gltf`, `.glb`), OBJ or STL.
pub fn load(path: &Path, up: Option<Up>) -> Result<(Model, &'static str), String> {
    let name = path.display().to_string();
    if !path.exists() {
        return Err(format!("\"{name}\" was not found"));
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let (mut model, format, default_up) = match ext.as_str() {
        "gltf" | "glb" => (crate::gltf::load(path)?, "glTF", Up::Y),
        "obj" => (crate::obj::load(path)?, "OBJ", Up::Y),
        "stl" => (crate::stl::load(path)?, "STL", Up::Z),
        _ => {
            return Err(format!(
                "\"{name}\": 3D can show glTF (.gltf, .glb), OBJ and STL models — export it as one of those"
            ));
        }
    };
    model.finish();
    if model.triangles.is_empty() {
        return Err(format!("\"{name}\" has no triangles to show"));
    }
    model.fit(up.unwrap_or(default_up));
    Ok((model, format))
}

#[cfg(test)]
pub fn cube() -> Model {
    let p = [
        v3(-1.0, -1.0, -1.0),
        v3(1.0, -1.0, -1.0),
        v3(1.0, 1.0, -1.0),
        v3(-1.0, 1.0, -1.0),
        v3(-1.0, -1.0, 1.0),
        v3(1.0, -1.0, 1.0),
        v3(1.0, 1.0, 1.0),
        v3(-1.0, 1.0, 1.0),
    ];
    let quads = [
        [0, 3, 2, 1],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [2, 3, 7, 6],
        [1, 2, 6, 5],
        [0, 4, 7, 3],
    ];
    let mut m = Model {
        positions: p.to_vec(),
        ..Model::default()
    };
    for q in quads {
        m.triangles.push([q[0], q[1], q[2]]);
        m.triangles.push([q[0], q[2], q[3]]);
    }
    m.finish();
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_centers_and_sizes() {
        let mut m = cube();
        for p in &mut m.positions {
            *p = *p * 10.0 + v3(100.0, 0.0, 0.0);
        }
        m.fit(Up::Y);
        let (lo, hi) = m.bounds();
        assert!(((lo + hi) * 0.5).len() < 1e-4);
        assert!((((hi - lo) * 0.5).len() - 1.0).abs() < 1e-4);
        assert!((m.floor - lo.y).abs() < 1e-5);
        assert_eq!(m.edges().len(), 18, "12 cube edges + 6 diagonals");
        // Normals point away from the middle.
        for (p, n) in m.positions.iter().zip(&m.normals) {
            assert!(p.dot(*n) > 0.0);
        }
    }

    #[test]
    fn z_up_files_stand_up() {
        let mut m = Model {
            positions: vec![v3(0.0, 0.0, 0.0), v3(0.0, 0.0, 10.0), v3(1.0, 0.0, 0.0)],
            triangles: vec![[0, 1, 2]],
            ..Model::default()
        };
        m.finish();
        m.fit(Up::Z);
        let (lo, hi) = m.bounds();
        assert!(hi.y - lo.y > hi.z - lo.z, "tall along Y now");
    }

    #[test]
    fn bad_triangles_are_dropped() {
        let mut m = Model {
            positions: vec![V3::default(); 3],
            triangles: vec![[0, 1, 2], [0, 1, 9]],
            ..Model::default()
        };
        m.finish();
        assert_eq!(m.triangles.len(), 1);
        assert_eq!(m.tri_material.len(), 1);
    }

    #[test]
    fn unknown_files_get_a_clear_message() {
        let dir = std::env::temp_dir().join(format!("stagetui-3d-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("model.fbx");
        std::fs::write(&f, b"x").unwrap();
        let e = load(&f, None).unwrap_err();
        assert!(e.contains("glTF") && e.contains("STL"), "{e}");
        assert!(
            load(&dir.join("missing.obj"), None)
                .unwrap_err()
                .contains("not found")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
