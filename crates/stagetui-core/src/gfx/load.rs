//! Decode images from files or bytes: PNG, JPEG, GIF (first frame), WebP and SVG.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use resvg::usvg;
use tiny_skia::{IntSize, Pixmap};

use super::{Image, Source, hash_bytes};
use crate::{Error, Result};

/// File extensions stagetui treats as images.
pub const IMAGE_EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "webp", "svg", "svgz", "bmp"];

/// Whether `path` looks like an image file by its extension.
pub fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Load an image file.
pub fn from_path(path: &Path) -> Result<Image> {
    let bytes = std::fs::read(path).map_err(|e| {
        Error::friendly(
            format!("Could not open image \"{}\"", path.display()),
            e.to_string(),
            "check the path is correct and the file exists (paths are relative to where you run Python)",
        )
    })?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    from_bytes(&bytes, &name)
}

/// Decode image bytes. `name` is only used for messages and snapshots.
pub fn from_bytes(bytes: &[u8], name: &str) -> Result<Image> {
    let id = hash_bytes(bytes);
    if looks_like_svg(bytes, name) {
        let tree = usvg::Tree::from_data(bytes, svg_options()).map_err(|e| {
            Error::friendly(
                format!("Could not read SVG \"{name}\""),
                e.to_string(),
                "make sure the file is a valid SVG (open it in a browser to check)",
            )
        })?;
        let size = tree.size().to_int_size();
        return Ok(Image {
            id,
            name: name.into(),
            width: size.width(),
            height: size.height(),
            source: Source::Svg(Arc::new(tree)),
        });
    }
    let decoded = image::load_from_memory(bytes).map_err(|e| {
        Error::friendly(
            format!("Could not decode image \"{name}\""),
            e.to_string(),
            "use a PNG, JPEG, GIF, WebP, BMP or SVG file",
        )
    })?;
    let rgba = decoded.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut data = rgba.into_raw();
    for px in data.as_chunks_mut::<4>().0 {
        let a = u16::from(px[3]);
        for c in &mut px[..3] {
            *c = ((u16::from(*c) * a + 127) / 255) as u8;
        }
    }
    let size = IntSize::from_wh(w, h).ok_or_else(|| {
        Error::friendly(
            format!("Image \"{name}\" is empty"),
            "it has zero width or height",
            "use a different image",
        )
    })?;
    let pixmap = Pixmap::from_vec(data, size).ok_or_else(|| {
        Error::friendly(
            format!("Image \"{name}\" is too large"),
            "it does not fit in memory",
            "resize it first",
        )
    })?;
    Ok(Image {
        id,
        name: name.into(),
        width: w,
        height: h,
        source: Source::Raster(Arc::new(pixmap)),
    })
}

fn looks_like_svg(bytes: &[u8], name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".svg") || lower.ends_with(".svgz") {
        return true;
    }
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return false; // gzip without an .svgz name: let the raster decoder report it
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]);
    let head = head.trim_start_matches('\u{feff}').trim_start();
    head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg"))
}

/// SVG options with stagetui's bundled fonts, so SVG text renders everywhere.
fn svg_options() -> &'static usvg::Options<'static> {
    static OPTS: OnceLock<usvg::Options<'static>> = OnceLock::new();
    OPTS.get_or_init(|| {
        let mut opts = usvg::Options::default();
        let db = opts.fontdb_mut();
        for (_, data) in crate::text::fonts::BUNDLED {
            db.load_font_data(data.to_vec());
        }
        db.set_sans_serif_family(crate::text::fonts::SANS);
        db.set_monospace_family(crate::text::fonts::MONO);
        opts
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#7aa2f7"/></svg>"##;

    #[test]
    fn svg_from_bytes() {
        let img = from_bytes(SVG.as_bytes(), "box.svg").unwrap();
        assert_eq!((img.width, img.height), (40, 20));
        let p = img.render(80, 40, super::super::Fit::Contain);
        assert_eq!(p.pixel(40, 20).unwrap().alpha(), 255);
    }

    #[test]
    fn png_roundtrip() {
        let mut p = Pixmap::new(4, 2).unwrap();
        p.fill(tiny_skia::Color::from_rgba8(10, 20, 30, 255));
        let png = p.encode_png().unwrap();
        let img = from_bytes(&png, "x.png").unwrap();
        assert_eq!((img.width, img.height), (4, 2));
    }

    #[test]
    fn friendly_errors() {
        let e = from_path(Path::new("/definitely/not/here.png"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("Could not open image") && e.contains("fix:"));
        let e = from_bytes(b"not an image", "junk.bin")
            .unwrap_err()
            .to_string();
        assert!(e.contains("Could not decode"));
        let e = from_bytes(b"<svg nonsense", "bad.svg")
            .unwrap_err()
            .to_string();
        assert!(e.contains("Could not read SVG"));
    }

    #[test]
    fn image_paths() {
        assert!(is_image_path(Path::new("a/b/Photo.JPG")));
        assert!(!is_image_path(Path::new("notes.txt")));
    }
}
