//! Fonts: bundled Noto + DejaVu, plus the system's fonts as fallback.
//!
//! The bundled fonts make Latin, Arabic and Devanagari look identical on every
//! machine (even a bare Docker image). Chinese/Japanese/Korean come from the
//! system because those fonts are large.
//!
//! Set `STAGETUI_SYSTEM_FONTS=0` to use only bundled fonts (fully deterministic).

use std::sync::{Mutex, MutexGuard, OnceLock};

use cosmic_text::{FontSystem, SwashCache, fontdb};
use serde::Serialize;

/// Monospace family used for terminal cells (has every Arabic presentation form).
pub const MONO: &str = "DejaVu Sans Mono";
/// Proportional family for Latin/Greek/Cyrillic pixel text.
pub const SANS: &str = "Noto Sans";
/// Arabic-script pixel text.
pub const ARABIC: &str = "Noto Sans Arabic";
/// Devanagari pixel text.
pub const DEVANAGARI: &str = "Noto Sans Devanagari";

macro_rules! font {
    ($file:literal) => {
        (
            $file,
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/", $file)).as_slice(),
        )
    };
}

/// `(file name, bytes)` of every bundled font.
pub const BUNDLED: [(&str, &[u8]); 7] = [
    font!("DejaVuSansMono.ttf"),
    font!("DejaVuSansMono-Bold.ttf"),
    font!("NotoSans-Regular.ttf"),
    font!("NotoSans-Bold.ttf"),
    font!("NotoSansArabic-Regular.ttf"),
    font!("NotoSansArabic-Bold.ttf"),
    font!("NotoSansDevanagari-Regular.ttf"),
];

/// Font system + glyph cache, shared process-wide.
pub struct Fonts {
    pub system: FontSystem,
    pub cache: SwashCache,
    pub system_fonts_loaded: bool,
}

fn build() -> Fonts {
    let mut db = fontdb::Database::new();
    for (_, data) in BUNDLED {
        db.load_font_data(data.to_vec());
    }
    let system_fonts = std::env::var("STAGETUI_SYSTEM_FONTS").as_deref() != Ok("0");
    if system_fonts {
        db.load_system_fonts();
    }
    db.set_monospace_family(MONO);
    db.set_sans_serif_family(SANS);
    let system = FontSystem::new_with_locale_and_db("en-US".into(), db);
    Fonts {
        system,
        cache: SwashCache::new(),
        system_fonts_loaded: system_fonts,
    }
}

/// Lock the shared fonts (built on first use).
pub fn lock() -> MutexGuard<'static, Fonts> {
    static FONTS: OnceLock<Mutex<Fonts>> = OnceLock::new();
    FONTS
        .get_or_init(|| Mutex::new(build()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Which writing systems have at least one font with glyphs.
#[derive(Debug, Clone, Serialize)]
pub struct Coverage {
    pub bundled: Vec<&'static str>,
    pub system_fonts: bool,
    pub faces: usize,
    pub scripts: Vec<ScriptCoverage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScriptCoverage {
    pub script: &'static str,
    pub sample: char,
    pub covered: bool,
}

const SAMPLES: [(&str, char); 7] = [
    ("latin", 'A'),
    ("arabic", 'ب'),
    ("devanagari", 'क'),
    ("chinese", '中'),
    ("japanese", 'あ'),
    ("korean", '한'),
    ("hebrew", 'ש'),
];

/// Check font coverage for common scripts.
pub fn coverage() -> Coverage {
    let mut fonts = lock();
    let ids: Vec<_> = fonts
        .system
        .db()
        .faces()
        .map(|f| (f.id, f.weight))
        .collect();
    let mut scripts: Vec<ScriptCoverage> = SAMPLES
        .iter()
        .map(|&(script, sample)| ScriptCoverage {
            script,
            sample,
            covered: false,
        })
        .collect();
    for (id, weight) in &ids {
        if scripts.iter().all(|s| s.covered) {
            break;
        }
        if let Some(font) = fonts.system.get_font(*id, *weight) {
            let charmap = font.as_swash().charmap();
            for s in scripts.iter_mut().filter(|s| !s.covered) {
                s.covered = charmap.map(s.sample) != 0;
            }
        }
    }
    Coverage {
        bundled: BUNDLED.iter().map(|(n, _)| *n).collect(),
        system_fonts: fonts.system_fonts_loaded,
        faces: ids.len(),
        scripts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_fonts_cover_latin_arabic_devanagari() {
        let c = coverage();
        assert!(c.faces >= BUNDLED.len());
        for script in ["latin", "arabic", "devanagari"] {
            assert!(
                c.scripts.iter().any(|s| s.script == script && s.covered),
                "{script} missing"
            );
        }
    }
}
