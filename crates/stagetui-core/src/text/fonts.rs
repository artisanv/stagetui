//! Fonts: bundled Noto + DejaVu, plus the system's fonts as fallback.
//!
//! The bundled fonts make Latin, Greek, Cyrillic, Arabic and Devanagari look
//! identical on every machine (even a bare Docker image). Everything else —
//! Chinese/Japanese/Korean, Hebrew, Thai, other Indic scripts, emoji — comes from
//! the system's fonts, because those fonts are large. A character no font has is
//! drawn as an empty box; the first time that happens for a script, stagetui
//! says so on stderr (once per process).
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
    /// Scripts this process has already drawn as empty boxes in pictures.
    pub missing_in_pictures: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScriptCoverage {
    pub script: &'static str,
    pub sample: char,
    pub covered: bool,
}

const SAMPLES: [(&str, char); 9] = [
    ("latin", 'A'),
    ("greek", 'Ω'),
    ("cyrillic", 'Ж'),
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
        missing_in_pictures: lock_warned().clone(),
    }
}

/// A writing system some character belongs to, and where to get a font for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Missing {
    /// Its name, as people say it ("Chinese/Japanese", "Hebrew").
    pub script: &'static str,
    /// The font to install, with its Linux package names.
    pub install: &'static str,
}

/// The script of a character that needs a glyph (`None` for spaces, controls and
/// invisible marks such as the direction marks, which no font has to draw).
pub fn script_of(c: char) -> Option<Missing> {
    let invisible = matches!(c as u32,
        0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x2069
        | 0xFE00..=0xFE0F | 0xFEFF | 0xE0000..=0xE0FFF);
    if c.is_whitespace() || c.is_control() || invisible {
        return None;
    }
    let noto = |script, install| Some(Missing { script, install });
    match c as u32 {
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF => noto(
            "Korean",
            "Noto Sans CJK (Linux: fonts-noto-cjk / noto-fonts-cjk)",
        ),
        0x2E80..=0x2FFF
        | 0x3000..=0x30FF
        | 0x3190..=0x9FFF
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFFEF
        | 0x20000..=0x3FFFF => noto(
            "Chinese/Japanese",
            "Noto Sans CJK (Linux: fonts-noto-cjk / noto-fonts-cjk)",
        ),
        0x0590..=0x05FF | 0xFB1D..=0xFB4F => noto(
            "Hebrew",
            "Noto Sans Hebrew (Linux: fonts-noto / noto-fonts)",
        ),
        0x0E00..=0x0E7F => noto("Thai", "Noto Sans Thai (Linux: fonts-noto / noto-fonts)"),
        0x0980..=0x09FF => noto(
            "Bengali",
            "Noto Sans Bengali (Linux: fonts-noto / noto-fonts)",
        ),
        0x0A00..=0x0DFF => noto(
            "Indic",
            "the Noto Sans font for that script (Linux: fonts-noto / noto-fonts)",
        ),
        0x1F000..=0x1FAFF | 0x2600..=0x27BF => noto(
            "emoji",
            "Noto Color Emoji (Linux: fonts-noto-color-emoji / noto-fonts-emoji)",
        ),
        _ => noto(
            "some",
            "a Noto font for those characters (Linux: fonts-noto / noto-fonts)",
        ),
    }
}

/// The plain warning for a script with no font.
pub fn missing_note(m: Missing, system_fonts: bool) -> String {
    let allow = if system_fonts {
        ""
    } else {
        ", or allow system fonts (unset STAGETUI_SYSTEM_FONTS)"
    };
    format!(
        "stagetui: no font for {} text on this computer — it shows as empty boxes in pictures. \
         Install {}{allow}.",
        m.script, m.install
    )
}

fn lock_warned() -> MutexGuard<'static, Vec<&'static str>> {
    static WARNED: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
    WARNED
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Remember that `m` had no font; the first time per script, return the warning
/// to print. Later calls for the same script return `None`.
pub fn note_missing(m: Missing, system_fonts: bool) -> Option<String> {
    let mut warned = lock_warned();
    if warned.contains(&m.script) {
        return None;
    }
    warned.push(m.script);
    Some(missing_note(m, system_fonts))
}

/// Warnings waiting while a full-screen app owns the terminal (`None` = not holding).
fn lock_held() -> MutexGuard<'static, Option<Vec<String>>> {
    static HELD: Mutex<Option<Vec<String>>> = Mutex::new(None);
    HELD.lock().unwrap_or_else(|e| e.into_inner())
}

/// While a full-screen app is on the terminal, printing would scribble over it:
/// `hold_warnings(true)` keeps font warnings back, `hold_warnings(false)` (once
/// the terminal is restored) prints the ones that came up meanwhile.
pub fn hold_warnings(hold: bool) {
    let queued = {
        let mut held = lock_held();
        if hold {
            held.get_or_insert_with(Vec::new);
            return;
        }
        held.take().unwrap_or_default()
    };
    for note in queued {
        eprintln!("{note}");
    }
}

/// `c` was drawn as an empty box: say so on stderr, once per script per process.
pub fn warn_missing(c: char, system_fonts: bool) {
    if let Some(m) = script_of(c)
        && let Some(note) = note_missing(m, system_fonts)
    {
        match lock_held().as_mut() {
            Some(queue) => queue.push(note),
            None => eprintln!("{note}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_fonts_cover_latin_arabic_devanagari() {
        let c = coverage();
        assert!(c.faces >= BUNDLED.len());
        for script in ["latin", "greek", "cyrillic", "arabic", "devanagari"] {
            assert!(
                c.scripts.iter().any(|s| s.script == script && s.covered),
                "{script} missing"
            );
        }
        // Every script is reported, found or not.
        let names: Vec<&str> = c.scripts.iter().map(|s| s.script).collect();
        for script in ["chinese", "japanese", "korean", "hebrew"] {
            assert!(names.contains(&script), "{script} not reported");
        }
        let json = serde_json::to_value(&c).unwrap();
        assert!(json["scripts"][0]["covered"].is_boolean());
        assert!(json["missing_in_pictures"].is_array());
    }

    #[test]
    fn characters_map_to_the_script_that_needs_a_font() {
        let name = |c| script_of(c).map(|m| m.script);
        assert_eq!(name('中'), Some("Chinese/Japanese"));
        assert_eq!(name('你'), Some("Chinese/Japanese"));
        assert_eq!(name('あ'), Some("Chinese/Japanese"));
        assert_eq!(name('カ'), Some("Chinese/Japanese"));
        assert_eq!(name('한'), Some("Korean"));
        assert_eq!(name('ש'), Some("Hebrew"));
        assert_eq!(name('ก'), Some("Thai"));
        assert_eq!(name('😀'), Some("emoji"));
        // Nothing to draw: never a warning.
        for c in [' ', '\t', '\u{200E}', '\u{200F}', '\u{200D}', '\u{FEFF}'] {
            assert_eq!(name(c), None, "{c:?}");
        }
        // Bundled scripts still have a name (in case a build lacks the font).
        for c in ['م', 'َ', 'न', 'A'] {
            assert!(name(c).is_some());
        }
    }

    #[test]
    fn the_warning_says_what_why_and_fix_once_per_script() {
        let m = Missing {
            script: "test-script-once",
            install: "Noto Sans Test (Linux: fonts-noto-test)",
        };
        let first = note_missing(m, false).expect("the first time, a warning");
        assert!(first.starts_with("stagetui: no font for test-script-once text"));
        assert!(first.contains("empty boxes in pictures"));
        assert!(first.contains("Install Noto Sans Test"));
        assert!(first.contains("unset STAGETUI_SYSTEM_FONTS"));
        assert!(!first.contains('\n'), "one line");
        assert_eq!(note_missing(m, false), None, "only once");
        assert_eq!(note_missing(m, true), None, "only once, whatever the fonts");
        assert!(coverage().missing_in_pictures.contains(&"test-script-once"));
        // With system fonts on, there is nothing to unset.
        let cjk = script_of('中').unwrap();
        let note = missing_note(cjk, true);
        assert!(note.contains("Noto Sans CJK (Linux: fonts-noto-cjk / noto-fonts-cjk)"));
        assert!(!note.contains("STAGETUI_SYSTEM_FONTS"));
        assert!(
            missing_note(cjk, false)
                .ends_with("or allow system fonts (unset STAGETUI_SYSTEM_FONTS).")
        );
    }

    #[test]
    fn warnings_wait_while_a_full_screen_app_runs() {
        hold_warnings(true);
        let m = Missing {
            script: "test-script-held",
            install: "Noto Sans Test",
        };
        let note = note_missing(m, true).unwrap();
        lock_held().as_mut().unwrap().push(note.clone());
        assert!(lock_held().as_ref().unwrap().contains(&note), "kept back");
        hold_warnings(false);
        assert!(lock_held().is_none(), "printed once the app is gone");
        // `warn_missing` queues while held (other tests may add theirs too).
        hold_warnings(true);
        warn_missing('\u{10FFFE}', true);
        assert!(lock_held().is_some());
        hold_warnings(false);
        assert!(lock_held().is_none());
    }
}
