//! Pixels: images, the 2D canvas, terminal graphics protocols and the headless rasterizer.
//!
//! Everything is an RGBA [`Pixmap`] (premultiplied, from `tiny-skia`). SVGs are
//! kept as vectors and rendered at the exact size they are shown, so they stay crisp.

pub mod base64;
pub mod blocks;
pub mod canvas;
mod diacritics;
pub mod iterm2;
pub mod kitty;
pub mod load;
pub mod raster;
pub mod sixel;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

pub use tiny_skia::Pixmap;
use tiny_skia::{FilterQuality, PixmapPaint, Transform};

use crate::{Error, Result};

/// Where an image's pixels come from.
#[derive(Clone)]
pub enum Source {
    Raster(Arc<Pixmap>),
    Svg(Arc<resvg::usvg::Tree>),
}

/// How an image fills the area it is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Fit {
    /// Whole image visible, aspect ratio kept (letterboxed).
    #[default]
    Contain,
    /// Area fully covered, aspect ratio kept (edges cropped).
    Cover,
    /// Stretched to the area.
    Stretch,
}

impl Fit {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "contain" => Ok(Self::Contain),
            "cover" => Ok(Self::Cover),
            "stretch" => Ok(Self::Stretch),
            other => Err(Error::friendly(
                format!("Unknown fit \"{other}\""),
                "fit controls how an image fills its space",
                "use \"contain\" (show everything), \"cover\" (fill, crop edges) or \"stretch\"",
            )),
        }
    }
}

/// A decoded image with a stable content id.
#[derive(Clone)]
pub struct Image {
    /// Hash of the source bytes (same file → same id → cached by terminals).
    pub id: u64,
    pub name: String,
    pub source: Source,
    /// Natural size in pixels.
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Image({:?} {}x{} #{:016x})",
            self.name, self.width, self.height, self.id
        )
    }
}

impl Image {
    /// Wrap an existing pixmap (e.g. a canvas).
    pub fn from_pixmap(name: impl Into<String>, pixmap: Pixmap) -> Self {
        let id = hash_bytes(pixmap.data());
        let (width, height) = (pixmap.width(), pixmap.height());
        Self {
            id,
            name: name.into(),
            source: Source::Raster(Arc::new(pixmap)),
            width,
            height,
        }
    }

    /// Render into an exact `w × h` pixmap using `fit`; transparent where not covered.
    pub fn render(&self, w: u32, h: u32, fit: Fit) -> Pixmap {
        let (w, h) = (w.max(1), h.max(1));
        let mut out = Pixmap::new(w, h).unwrap_or_else(|| Pixmap::new(1, 1).expect("1x1 pixmap"));
        let (nw, nh) = (self.width.max(1) as f32, self.height.max(1) as f32);
        let (sx, sy) = match fit {
            Fit::Stretch => (w as f32 / nw, h as f32 / nh),
            Fit::Contain => {
                let s = (w as f32 / nw).min(h as f32 / nh);
                (s, s)
            }
            Fit::Cover => {
                let s = (w as f32 / nw).max(h as f32 / nh);
                (s, s)
            }
        };
        let dx = (w as f32 - nw * sx) / 2.0;
        let dy = (h as f32 - nh * sy) / 2.0;
        let transform = Transform::from_row(sx, 0.0, 0.0, sy, dx, dy);
        match &self.source {
            Source::Svg(tree) => resvg::render(tree, transform, &mut out.as_mut()),
            Source::Raster(p) => {
                let paint = PixmapPaint {
                    quality: FilterQuality::Bicubic,
                    ..PixmapPaint::default()
                };
                out.draw_pixmap(0, 0, p.as_ref().as_ref(), &paint, transform, None);
            }
        }
        out
    }

    /// Cells needed to show this image at natural size, capped to `max_cols × max_rows`
    /// while keeping the aspect ratio.
    pub fn cells(&self, cell_px: (u32, u32), max_cols: u16, max_rows: u16) -> (u16, u16) {
        let (cw, ch) = (cell_px.0.max(1) as f32, cell_px.1.max(1) as f32);
        let mut cols = (self.width as f32 / cw).ceil().max(1.0);
        let mut rows = (self.height as f32 / ch).ceil().max(1.0);
        let scale = (f32::from(max_cols.max(1)) / cols)
            .min(f32::from(max_rows.max(1)) / rows)
            .min(1.0);
        if scale < 1.0 {
            cols = (cols * scale).floor().max(1.0);
            rows = (rows * scale).floor().max(1.0);
        }
        (cols as u16, rows as u16)
    }
}

/// Stable 64-bit hash of bytes (deterministic across runs).
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

fn store() -> &'static Mutex<HashMap<u64, Image>> {
    static STORE: OnceLock<Mutex<HashMap<u64, Image>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Keep an in-memory image so app descriptions can refer to it as `mem:<id>`.
pub fn register(image: Image) -> String {
    let key = format!("mem:{:016x}", image.id);
    store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(image.id, image);
    key
}

/// Look up `mem:<id>` or load a file path.
pub fn resolve(source: &str) -> Result<Image> {
    if let Some(hex) = source.strip_prefix("mem:") {
        let id = u64::from_str_radix(hex, 16).ok();
        let found = id.and_then(|id| {
            store()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&id)
                .cloned()
        });
        return found.ok_or_else(|| {
            Error::friendly(
                format!("In-memory image \"{source}\" was not found"),
                "it was never registered in this process",
                "pass a file path, bytes, or a Canvas to StageImage",
            )
        });
    }
    load::from_path(std::path::Path::new(source))
}

/// Straight (non-premultiplied) RGBA bytes of a pixmap.
pub fn to_rgba(p: &Pixmap) -> Vec<u8> {
    p.pixels()
        .iter()
        .flat_map(|c| {
            let d = c.demultiply();
            [d.red(), d.green(), d.blue(), d.alpha()]
        })
        .collect()
}

/// Encode as PNG.
pub fn png(p: &Pixmap) -> Result<Vec<u8>> {
    p.encode_png().map_err(|e| {
        Error::friendly(
            "Could not encode a PNG",
            e.to_string(),
            "this is a bug; please report it",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::Color;

    fn solid(w: u32, h: u32) -> Image {
        let mut p = Pixmap::new(w, h).unwrap();
        p.fill(Color::from_rgba8(255, 0, 0, 255));
        Image::from_pixmap("red", p)
    }

    #[test]
    fn contain_letterboxes() {
        let img = solid(100, 50);
        let out = img.render(100, 100, Fit::Contain);
        let alpha_at = |x: u32, y: u32| out.pixel(x, y).unwrap().alpha();
        assert_eq!(alpha_at(50, 5), 0); // top band transparent
        assert_eq!(alpha_at(50, 50), 255);
        let cover = img.render(100, 100, Fit::Cover);
        assert_eq!(cover.pixel(50, 5).unwrap().alpha(), 255);
    }

    #[test]
    fn cells_keep_aspect_and_cap() {
        let img = solid(400, 200);
        assert_eq!(img.cells((10, 20), 100, 100), (40, 10));
        let (c, r) = img.cells((10, 20), 20, 100);
        assert!(c <= 20 && r <= 5);
    }

    #[test]
    fn store_roundtrip() {
        let key = register(solid(3, 3));
        assert!(key.starts_with("mem:"));
        assert_eq!(resolve(&key).unwrap().width, 3);
        assert!(resolve("mem:00000000deadbeef").is_err());
    }
}
