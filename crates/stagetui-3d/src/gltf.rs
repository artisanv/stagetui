//! glTF 2.0 — `.gltf` (JSON with `.bin` files or embedded data) and `.glb` (one file).
//!
//! Reads the scene's meshes with their node transforms, normals, texture coordinates,
//! vertex colors, base colors and base color pictures.

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::math::{self, M4, V3, v3};
use crate::model::{Material, Model, Texture};

const GLB_MAGIC: &[u8] = b"glTF";
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

pub fn load(path: &Path) -> Result<Model, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("couldn't read \"{}\": {e}", path.display()))?;
    parse(&bytes, path.parent().unwrap_or(Path::new(".")))
        .map_err(|e| format!("couldn't read \"{}\": {e}", path.display()))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The JSON part and the binary part of a `.glb` (or the whole file for `.gltf`).
fn split(bytes: &[u8]) -> Result<(Value, Option<Vec<u8>>), String> {
    if !bytes.starts_with(GLB_MAGIC) {
        let doc = serde_json::from_slice(bytes).map_err(|e| format!("this is not glTF ({e})"))?;
        return Ok((doc, None));
    }
    let mut at = 12;
    let (mut json, mut bin) = (None, None);
    while let (Some(len), Some(kind)) = (u32_at(bytes, at), u32_at(bytes, at + 4)) {
        let data = bytes
            .get(at + 8..at + 8 + len as usize)
            .ok_or("the .glb file is cut short")?;
        match kind {
            CHUNK_JSON => json = Some(data),
            CHUNK_BIN => bin = Some(data.to_vec()),
            _ => {}
        }
        at += 8 + len as usize;
    }
    let json = json.ok_or("the .glb file has no JSON part")?;
    let doc = serde_json::from_slice(json).map_err(|e| format!("broken JSON in the .glb ({e})"))?;
    Ok((doc, bin))
}

struct Doc<'a> {
    json: &'a Value,
    buffers: Vec<Vec<u8>>,
}

fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn num(v: &Value, key: &str) -> Option<usize> {
    v.get(key).and_then(Value::as_u64).map(|n| n as usize)
}

fn floats(v: &Value, key: &str) -> Option<Vec<f32>> {
    Some(
        v.get(key)?
            .as_array()?
            .iter()
            .filter_map(Value::as_f64)
            .map(|f| f as f32)
            .collect(),
    )
}

/// `data:…;base64,…` or a file next to the model.
fn fetch(uri: &str, dir: &Path) -> Result<Vec<u8>, String> {
    if let Some(rest) = uri.strip_prefix("data:") {
        let (_, data) = rest
            .split_once(";base64,")
            .ok_or("an embedded file isn't base64")?;
        return base64(data).ok_or_else(|| "an embedded file is broken".into());
    }
    let file = percent_decode(uri);
    std::fs::read(dir.join(&file)).map_err(|e| format!("\"{file}\" (next to the model): {e}"))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(h) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(h);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn base64(text: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    }
    let clean: Vec<u8> = text
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
        .collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= value(c)? << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Some(out)
}

impl Doc<'_> {
    /// The bytes a buffer view covers, and its stride.
    fn view(&self, index: usize) -> Result<(&[u8], Option<usize>), String> {
        let v = arr(self.json, "bufferViews")
            .get(index)
            .ok_or("a missing buffer view")?;
        let buffer = self
            .buffers
            .get(num(v, "buffer").unwrap_or(0))
            .ok_or("a missing buffer")?;
        let start = num(v, "byteOffset").unwrap_or(0);
        let len = num(v, "byteLength").ok_or("a buffer view without a length")?;
        let bytes = buffer
            .get(start..start + len)
            .ok_or("a buffer view past the end of its file")?;
        Ok((bytes, num(v, "byteStride")))
    }

    /// An accessor's values as floats (`count × components`), normalized integers mapped to 0..1.
    fn read(&self, index: usize) -> Result<(Vec<f32>, usize), String> {
        let a = arr(self.json, "accessors")
            .get(index)
            .ok_or("a missing accessor")?;
        let count = num(a, "count").unwrap_or(0);
        let comps = match a.get("type").and_then(Value::as_str).unwrap_or("SCALAR") {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT4" => 16,
            other => return Err(format!("accessor type {other} isn't supported")),
        };
        let kind = num(a, "componentType").unwrap_or(5126);
        let size = match kind {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            other => return Err(format!("component type {other} isn't supported")),
        };
        let normalized = a
            .get("normalized")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut out = vec![0.0f32; count * comps];
        let Some(view) = num(a, "bufferView") else {
            return Ok((out, comps)); // all zeros (sparse data isn't read)
        };
        let (bytes, stride) = self.view(view)?;
        let offset = num(a, "byteOffset").unwrap_or(0);
        let stride = stride.unwrap_or(size * comps);
        for i in 0..count {
            for c in 0..comps {
                let at = offset + i * stride + c * size;
                let b = bytes
                    .get(at..at + size)
                    .ok_or("an accessor past the end of its data")?;
                out[i * comps + c] = match kind {
                    5126 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                    5125 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
                    5123 => {
                        let n = u16::from_le_bytes([b[0], b[1]]) as f32;
                        if normalized { n / 65535.0 } else { n }
                    }
                    5122 => {
                        let n = i16::from_le_bytes([b[0], b[1]]) as f32;
                        if normalized {
                            (n / 32767.0).max(-1.0)
                        } else {
                            n
                        }
                    }
                    5121 => {
                        let n = b[0] as f32;
                        if normalized { n / 255.0 } else { n }
                    }
                    _ => {
                        let n = b[0] as i8 as f32;
                        if normalized { (n / 127.0).max(-1.0) } else { n }
                    }
                };
            }
        }
        Ok((out, comps))
    }

    /// Vertex indices, exactly (no float rounding).
    fn indices(&self, index: usize) -> Result<Vec<u32>, String> {
        let a = arr(self.json, "accessors")
            .get(index)
            .ok_or("a missing accessor")?;
        let count = num(a, "count").unwrap_or(0);
        let kind = num(a, "componentType").unwrap_or(5125);
        let size = match kind {
            5121 => 1,
            5123 => 2,
            5125 => 4,
            other => return Err(format!("index type {other} isn't supported")),
        };
        let view = num(a, "bufferView").ok_or("indices without data")?;
        let (bytes, stride) = self.view(view)?;
        let offset = num(a, "byteOffset").unwrap_or(0);
        let stride = stride.unwrap_or(size);
        (0..count)
            .map(|i| {
                let at = offset + i * stride;
                let b = bytes
                    .get(at..at + size)
                    .ok_or("indices past the end of their data")?;
                Ok(match size {
                    1 => b[0] as u32,
                    2 => u16::from_le_bytes([b[0], b[1]]) as u32,
                    _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                })
            })
            .collect()
    }

    fn image(&self, index: usize, dir: &Path) -> Option<Texture> {
        let img = arr(self.json, "images").get(index)?;
        let bytes = if let Some(uri) = img.get("uri").and_then(Value::as_str) {
            fetch(uri, dir).ok()?
        } else {
            self.view(num(img, "bufferView")?).ok()?.0.to_vec()
        };
        Some(Texture::from_image(image::load_from_memory(&bytes).ok()?))
    }
}

fn local_matrix(node: &Value) -> M4 {
    if let Some(m) = floats(node, "matrix").filter(|m| m.len() == 16) {
        return math::from_columns(&m);
    }
    let t = floats(node, "translation").filter(|t| t.len() == 3);
    let r = floats(node, "rotation").filter(|r| r.len() == 4);
    let s = floats(node, "scale").filter(|s| s.len() == 3);
    let mut m = math::IDENTITY;
    if let Some(t) = t {
        m = math::mul(&m, &math::translate(v3(t[0], t[1], t[2])));
    }
    if let Some(r) = r {
        m = math::mul(&m, &math::rotate_quat([r[0], r[1], r[2], r[3]]));
    }
    if let Some(s) = s {
        m = math::mul(&m, &math::scale(v3(s[0], s[1], s[2])));
    }
    m
}

pub fn parse(bytes: &[u8], dir: &Path) -> Result<Model, String> {
    let (json, bin) = split(bytes)?;
    for ext in arr(&json, "extensionsRequired")
        .iter()
        .filter_map(Value::as_str)
    {
        if ext.contains("draco") || ext.contains("meshopt") {
            return Err(format!(
                "it is compressed ({ext}), which 3D can't read yet — export it without compression"
            ));
        }
    }
    let mut buffers = Vec::new();
    for b in arr(&json, "buffers") {
        buffers.push(match b.get("uri").and_then(Value::as_str) {
            Some(uri) => fetch(uri, dir)?,
            None => bin.clone().ok_or("a buffer is missing")?,
        });
    }
    let doc = Doc {
        json: &json,
        buffers,
    };
    let mut model = Model::default();

    // Materials: base color and its picture.
    let mut images: HashMap<usize, Option<usize>> = HashMap::new();
    for m in arr(&json, "materials") {
        let pbr = m.get("pbrMetallicRoughness");
        let color = pbr
            .and_then(|p| floats(p, "baseColorFactor"))
            .filter(|c| c.len() == 4)
            .map_or([1.0; 4], |c| [c[0], c[1], c[2], c[3]]);
        let texture = pbr
            .and_then(|p| p.get("baseColorTexture"))
            .and_then(|t| num(t, "index"))
            .and_then(|t| arr(&json, "textures").get(t))
            .and_then(|t| num(t, "source"))
            .and_then(|src| {
                *images.entry(src).or_insert_with(|| {
                    let tex = doc.image(src, dir)?;
                    model.textures.push(tex);
                    Some(model.textures.len() - 1)
                })
            });
        model.materials.push(Material { color, texture });
    }
    let default_material = model.materials.len() as u32;
    model.materials.push(Material {
        color: [0.8, 0.8, 0.8, 1.0],
        texture: None,
    });

    // The scene's nodes, or every mesh when there is no scene.
    let nodes = arr(&json, "nodes");
    let scene = num(&json, "scene").unwrap_or(0);
    let roots: Vec<usize> = match arr(&json, "scenes").get(scene) {
        Some(s) => arr(s, "nodes")
            .iter()
            .filter_map(|n| n.as_u64().map(|n| n as usize))
            .collect(),
        None => {
            let children: Vec<usize> = nodes
                .iter()
                .flat_map(|n| {
                    arr(n, "children")
                        .iter()
                        .filter_map(|c| c.as_u64().map(|c| c as usize))
                })
                .collect();
            (0..nodes.len()).filter(|i| !children.contains(i)).collect()
        }
    };
    let mut stack: Vec<(usize, M4, usize)> =
        roots.into_iter().map(|r| (r, math::IDENTITY, 0)).collect();
    if nodes.is_empty() {
        for mesh in 0..arr(&json, "meshes").len() {
            add_mesh(&doc, mesh, &math::IDENTITY, default_material, &mut model)?;
        }
    }
    while let Some((index, parent, depth)) = stack.pop() {
        let Some(node) = nodes.get(index) else {
            continue;
        };
        if depth > 64 {
            return Err("its nodes loop back on themselves".into());
        }
        let world = math::mul(&parent, &local_matrix(node));
        if let Some(mesh) = num(node, "mesh") {
            add_mesh(&doc, mesh, &world, default_material, &mut model)?;
        }
        for c in arr(node, "children").iter().filter_map(Value::as_u64) {
            stack.push((c as usize, world, depth + 1));
        }
    }
    // Normals the file left out: made from the triangles.
    if model.normals.iter().any(|n| n.len() < 1e-12) {
        let smooth = crate::model::smooth_normals(&model.positions, &model.triangles);
        for (n, s) in model.normals.iter_mut().zip(smooth) {
            if n.len() < 1e-12 {
                *n = s;
            }
        }
    }
    Ok(model)
}

fn add_mesh(
    doc: &Doc,
    mesh: usize,
    world: &M4,
    default_material: u32,
    model: &mut Model,
) -> Result<(), String> {
    let Some(mesh) = arr(doc.json, "meshes").get(mesh) else {
        return Ok(());
    };
    let nm = math::normal_matrix(world);
    for prim in arr(mesh, "primitives") {
        let mode = num(prim, "mode").unwrap_or(4);
        if !(4..=6).contains(&mode) {
            continue; // points and lines
        }
        let attrs = prim
            .get("attributes")
            .ok_or("a mesh part without attributes")?;
        let Some(pos_at) = num(attrs, "POSITION") else {
            continue;
        };
        let (pos, _) = doc.read(pos_at)?;
        let count = pos.len() / 3;
        let base = model.positions.len() as u32;
        for p in pos.as_chunks::<3>().0 {
            model
                .positions
                .push(math::point(world, v3(p[0], p[1], p[2])));
        }
        match num(attrs, "NORMAL").map(|i| doc.read(i)).transpose()? {
            Some((n, 3)) if n.len() == count * 3 => {
                for n in n.as_chunks::<3>().0 {
                    model
                        .normals
                        .push(math::apply3(&nm, v3(n[0], n[1], n[2])).norm());
                }
            }
            _ => model
                .normals
                .extend(std::iter::repeat_n(V3::default(), count)),
        }
        match num(attrs, "TEXCOORD_0").map(|i| doc.read(i)).transpose()? {
            Some((t, 2)) if t.len() == count * 2 => {
                model.uvs.extend(t.as_chunks::<2>().0.iter().copied());
            }
            _ => model.uvs.extend(std::iter::repeat_n([0.0, 0.0], count)),
        }
        match num(attrs, "COLOR_0").map(|i| doc.read(i)).transpose()? {
            Some((c, k @ 3..=4)) if c.len() == count * k => {
                model.colors.extend(
                    c.chunks_exact(k)
                        .map(|c| [c[0], c[1], c[2], if k == 4 { c[3] } else { 1.0 }]),
                );
            }
            _ => model.colors.extend(std::iter::repeat_n([1.0; 4], count)),
        }
        let idx: Vec<u32> = match num(prim, "indices") {
            Some(i) => doc.indices(i)?,
            None => (0..count as u32).collect(),
        };
        let material = num(prim, "material").map_or(default_material, |m| m as u32);
        let mut push = |a: u32, b: u32, c: u32| {
            model.triangles.push([base + a, base + b, base + c]);
            model.tri_material.push(material);
        };
        match mode {
            4 => idx
                .as_chunks::<3>()
                .0
                .iter()
                .for_each(|t| push(t[0], t[1], t[2])),
            5 => {
                for i in 2..idx.len() {
                    if i % 2 == 0 {
                        push(idx[i - 2], idx[i - 1], idx[i]);
                    } else {
                        push(idx[i - 1], idx[i - 2], idx[i]);
                    }
                }
            }
            _ => (2..idx.len()).for_each(|i| push(idx[0], idx[i - 1], idx[i])),
        }
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A small glTF triangle (two materials' worth of data), as `.gltf` JSON with embedded data.
    pub fn triangle_gltf() -> String {
        // 3 positions (VEC3 float) + 3 indices (u16) + 2 bytes padding.
        let mut bin = Vec::new();
        for p in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for c in p {
                bin.extend(c.to_le_bytes());
            }
        }
        for i in [0u16, 1, 2] {
            bin.extend(i.to_le_bytes());
        }
        bin.extend([0u8; 2]);
        let b64 = encode(&bin);
        format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "nodes":[{{"children":[1],"translation":[10,0,0]}},{{"mesh":0,"scale":[2,2,2]}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"indices":1,"material":0}}]}}],
            "materials":[{{"pbrMetallicRoughness":{{"baseColorFactor":[1,0,0,1]}}}}],
            "buffers":[{{"byteLength":{},"uri":"data:application/octet-stream;base64,{b64}"}}],
            "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":36}},{{"buffer":0,"byteOffset":36,"byteLength":6}}],
            "accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}},
                         {{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}}]}}"#,
            bin.len()
        )
    }

    pub fn encode(b: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for c in b.chunks(3) {
            let n = (c[0] as u32) << 16
                | (*c.get(1).unwrap_or(&0) as u32) << 8
                | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                if i <= c.len() {
                    s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    s.push('=');
                }
            }
        }
        s
    }

    #[test]
    fn base64_round_trip() {
        for data in [&b""[..], b"a", b"ab", b"abc", "مرحبا 日本語".as_bytes()] {
            assert_eq!(base64(&encode(data)).unwrap(), data);
        }
        assert_eq!(percent_decode("my%20model.bin"), "my model.bin");
    }

    #[test]
    fn gltf_with_nodes_and_materials() {
        let m = parse(triangle_gltf().as_bytes(), Path::new(".")).unwrap();
        assert_eq!(m.triangles, vec![[0, 1, 2]]);
        // Child scale 2, then parent moves it by 10.
        assert_eq!(m.positions[1], v3(12.0, 0.0, 0.0));
        assert_eq!(
            m.materials[m.tri_material[0] as usize].color,
            [1.0, 0.0, 0.0, 1.0]
        );
        assert!(
            (m.normals[0].z - 1.0).abs() < 1e-5,
            "made from the triangle"
        );
    }

    #[test]
    fn glb_is_the_same_in_one_file() {
        let json = triangle_gltf();
        let v: Value = serde_json::from_str(&json).unwrap();
        let uri = v["buffers"][0]["uri"].as_str().unwrap();
        let bin = base64(uri.split_once(",").unwrap().1).unwrap();
        let json = json.replace(&format!(r#","uri":"{uri}""#), "");
        let mut j = json.into_bytes();
        while j.len() % 4 != 0 {
            j.push(b' ');
        }
        let mut glb = b"glTF".to_vec();
        glb.extend(2u32.to_le_bytes());
        glb.extend(((12 + 8 + j.len() + 8 + bin.len()) as u32).to_le_bytes());
        glb.extend((j.len() as u32).to_le_bytes());
        glb.extend(CHUNK_JSON.to_le_bytes());
        glb.extend(&j);
        glb.extend((bin.len() as u32).to_le_bytes());
        glb.extend(CHUNK_BIN.to_le_bytes());
        glb.extend(&bin);
        let m = parse(&glb, Path::new(".")).unwrap();
        assert_eq!(m.triangles.len(), 1);
        assert_eq!(m.positions[2], v3(10.0, 2.0, 0.0));
    }

    #[test]
    fn compressed_models_say_so() {
        let e = parse(
            br#"{"asset":{"version":"2.0"},"extensionsRequired":["KHR_draco_mesh_compression"]}"#,
            Path::new("."),
        )
        .unwrap_err();
        assert!(e.contains("compress"), "{e}");
        assert!(
            parse(b"not json", Path::new("."))
                .unwrap_err()
                .contains("not glTF")
        );
    }
}
