//! Wavefront OBJ (with its `.mtl` colors and pictures).

use std::collections::HashMap;
use std::path::Path;

use crate::math::{V3, v3};
use crate::model::{Material, Model, Texture, smooth_normals};

pub fn load(path: &Path) -> Result<Model, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("couldn't read \"{}\": {e}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    parse(&text, path.parent().unwrap_or(Path::new(".")))
        .map_err(|e| format!("couldn't read \"{}\": {e}", path.display()))
}

fn floats<'a>(parts: impl Iterator<Item = &'a str>) -> Vec<f32> {
    parts.filter_map(|p| p.parse::<f32>().ok()).collect()
}

/// An OBJ index (1-based, or negative = counted from the end) → 0-based.
fn index(text: &str, len: usize) -> Option<usize> {
    let i: i64 = text.parse().ok()?;
    let at = if i < 0 { len as i64 + i } else { i - 1 };
    (0..len as i64).contains(&at).then_some(at as usize)
}

pub fn parse(text: &str, dir: &Path) -> Result<Model, String> {
    let mut pos: Vec<V3> = Vec::new();
    let mut pos_color: Vec<Option<[f32; 4]>> = Vec::new();
    let mut tex: Vec<[f32; 2]> = Vec::new();
    let mut nor: Vec<V3> = Vec::new();
    let mut model = Model::default();
    let mut lookup: HashMap<(usize, Option<usize>, Option<usize>), u32> = HashMap::new();
    let mut source: Vec<usize> = Vec::new(); // the `v` each output vertex came from
    let mut has_normals = true;
    let mut named: HashMap<String, u32> = HashMap::new();
    let mut current = 0u32;
    let mut textures: HashMap<String, Option<usize>> = HashMap::new();

    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or_default().trim();
        let mut parts = line.split_whitespace();
        let Some(kind) = parts.next() else { continue };
        match kind {
            "v" => {
                let f = floats(parts);
                if f.len() < 3 {
                    return Err(format!("line {}: a vertex needs x y z", n + 1));
                }
                pos.push(v3(f[0], f[1], f[2]));
                pos_color.push((f.len() >= 6).then(|| [srgb(f[3]), srgb(f[4]), srgb(f[5]), 1.0]));
            }
            "vt" => {
                let f = floats(parts);
                tex.push([
                    f.first().copied().unwrap_or(0.0),
                    1.0 - f.get(1).copied().unwrap_or(0.0),
                ]);
            }
            "vn" => {
                let f = floats(parts);
                if f.len() >= 3 {
                    nor.push(v3(f[0], f[1], f[2]));
                }
            }
            "f" => {
                let mut corners = Vec::new();
                for p in parts {
                    let mut it = p.split('/');
                    let vi = it.next().and_then(|s| index(s, pos.len())).ok_or_else(|| {
                        format!("line {}: face corner \"{p}\" points to no vertex", n + 1)
                    })?;
                    let ti = it.next().and_then(|s| index(s, tex.len()));
                    let ni = it.next().and_then(|s| index(s, nor.len()));
                    has_normals &= ni.is_some();
                    let id = *lookup.entry((vi, ti, ni)).or_insert_with(|| {
                        model.positions.push(pos[vi]);
                        model.uvs.push(ti.map_or([0.0, 0.0], |t| tex[t]));
                        model.normals.push(ni.map_or(V3::default(), |k| nor[k]));
                        model.colors.push(pos_color[vi].unwrap_or([1.0; 4]));
                        source.push(vi);
                        model.positions.len() as u32 - 1
                    });
                    corners.push(id);
                }
                // Polygons become fans of triangles.
                for k in 1..corners.len().saturating_sub(1) {
                    model
                        .triangles
                        .push([corners[0], corners[k], corners[k + 1]]);
                    model.tri_material.push(current);
                }
            }
            "mtllib" => {
                let file = line["mtllib".len()..].trim();
                if let Ok(mtl) = std::fs::read(dir.join(file)) {
                    read_mtl(
                        &String::from_utf8_lossy(&mtl),
                        dir,
                        &mut model,
                        &mut named,
                        &mut textures,
                    );
                }
            }
            "usemtl" => {
                let name = line["usemtl".len()..].trim().to_string();
                current = *named.entry(name).or_insert_with(|| {
                    model.materials.push(Material::default());
                    model.materials.len() as u32 - 1
                });
            }
            _ => {}
        }
    }
    if model.materials.is_empty() {
        model.materials.push(Material::default());
    }
    if !has_normals || model.normals.iter().any(|n| n.len() < 1e-12) {
        // Smooth by the original vertex, so seams in the texture don't show as creases.
        let by_source: Vec<[u32; 3]> = model
            .triangles
            .iter()
            .map(|t| t.map(|i| source[i as usize] as u32))
            .collect();
        let smooth = smooth_normals(&pos, &by_source);
        model.normals = source.iter().map(|&s| smooth[s]).collect();
    }
    Ok(model)
}

fn read_mtl(
    text: &str,
    dir: &Path,
    model: &mut Model,
    named: &mut HashMap<String, u32>,
    textures: &mut HashMap<String, Option<usize>>,
) {
    let mut current: Option<usize> = None;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("newmtl") => {
                let name = line["newmtl".len()..].trim().to_string();
                let id = *named.entry(name).or_insert_with(|| {
                    model.materials.push(Material {
                        color: [0.8, 0.8, 0.8, 1.0],
                        texture: None,
                    });
                    model.materials.len() as u32 - 1
                });
                current = Some(id as usize);
            }
            Some("Kd") => {
                let f = floats(parts);
                if let (Some(m), true) = (current, f.len() >= 3) {
                    model.materials[m].color = [srgb(f[0]), srgb(f[1]), srgb(f[2]), 1.0];
                }
            }
            Some("map_Kd") => {
                // The file name is last (options like `-s 1 1 1` may come first).
                let Some(file) = line.split_whitespace().last() else {
                    continue;
                };
                let tex = *textures.entry(file.to_string()).or_insert_with(|| {
                    let img = image::open(dir.join(file.replace('\\', "/"))).ok()?;
                    model.textures.push(Texture::from_image(img));
                    Some(model.textures.len() - 1)
                });
                if let Some(m) = current {
                    model.materials[m].texture = tex;
                    if tex.is_some() {
                        model.materials[m].color = [1.0; 4];
                    }
                }
            }
            _ => {}
        }
    }
}

/// OBJ/MTL colors are written as they look (sRGB); drawing works in linear light.
fn srgb(c: f32) -> f32 {
    c.clamp(0.0, 1.0).powf(2.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &str = "# a square, as one quad and with negative indices
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
vt 0 0
vt 1 0
vt 1 1
vt 0 1
f 1/1 2/2 3/3 4/4
f -4 -2 -1
";

    #[test]
    fn quads_and_negative_indices() {
        let m = parse(SQUARE, Path::new(".")).unwrap();
        assert_eq!(
            m.triangles.len(),
            3,
            "a quad is two triangles, plus one more"
        );
        // No normals in the file: they are made, facing +Z.
        assert!(
            m.normals.iter().all(|n| (n.z - 1.0).abs() < 1e-5),
            "{:?}",
            m.normals
        );
        assert_eq!(
            m.uvs[1],
            [1.0, 1.0],
            "v is flipped (OBJ counts from the bottom)"
        );
    }

    #[test]
    fn materials_and_vertex_colors() {
        let dir = std::env::temp_dir().join(format!("stagetui-3d-obj-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("m.mtl"),
            "newmtl red\nKd 1 0 0\nnewmtl blue\nKd 0 0 1\n",
        )
        .unwrap();
        let text = "mtllib m.mtl\nv 0 0 0 1 1 0\nv 1 0 0\nv 0 1 0\nvn 0 0 1\nusemtl blue\nf 1//1 2//1 3//1\nusemtl red\nf 3//1 2//1 1//1\n";
        let m = parse(text, &dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(m.materials.len(), 2);
        assert_eq!(
            m.materials[m.tri_material[0] as usize].color,
            [0.0, 0.0, 1.0, 1.0]
        );
        assert_eq!(
            m.materials[m.tri_material[1] as usize].color,
            [1.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(m.colors[0], [1.0, 1.0, 0.0, 1.0], "the yellow vertex");
        assert_eq!(m.normals[0], v3(0.0, 0.0, 1.0));
    }

    #[test]
    fn broken_faces_say_where() {
        let e = parse("v 0 0 0\nf 1 2 3\n", Path::new(".")).unwrap_err();
        assert!(e.contains("line 2"), "{e}");
    }
}
