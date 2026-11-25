//! Built-in icon names → single-cell Unicode fallbacks.
//!
//! These glyphs are deliberately narrow, non-emoji code points so every terminal
//! renders them in exactly one cell. Pixel (SVG) icons arrive in Milestone 3.

use crate::{Error, Result};

/// `(name, glyph)` pairs.
pub const ICONS: [(&str, &str); 22] = [
    ("sparkles", "✦"),
    ("check", "✓"),
    ("x", "✗"),
    ("play", "▶"),
    ("pause", "‖"),
    ("stop", "■"),
    ("globe", "◍"),
    ("book", "≡"),
    ("cube", "◆"),
    ("sun", "☼"),
    ("moon", "☾"),
    ("terminal", "❯"),
    ("star", "★"),
    ("bolt", "ϟ"),
    ("info", "ℹ"),
    ("warning", "▲"),
    ("folder", "▣"),
    ("file", "▢"),
    ("chat", "»"),
    ("code", "⌘"),
    ("gear", "⚙"),
    ("heart", "♥"),
];

/// Resolve an icon name.
pub fn icon(name: &str) -> Result<&'static str> {
    ICONS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, g)| *g)
        .ok_or_else(|| {
            Error::friendly(
                format!("Unknown icon \"{name}\""),
                "icons come from termstage's built-in set",
                format!(
                    "use one of: {}",
                    ICONS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
                ),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_one_cell() {
        for (name, glyph) in ICONS {
            assert_eq!(crate::text::width(glyph), 1, "{name} {glyph}");
        }
    }
}
