//! Built-in icon names → single-cell Unicode fallbacks.
//!
//! These glyphs are deliberately narrow, non-emoji code points so every terminal
//! renders them in exactly one cell, without needing a Nerd Font.

use crate::{Error, Result};

/// `(name, glyph)` pairs.
pub const ICONS: [(&str, &str); 30] = [
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
    ("send", "➤"),
    ("arrow-right", "→"),
    ("arrow-left", "←"),
    ("plus", "+"),
    ("minus", "−"),
    ("dot", "•"),
    ("circle", "○"),
    ("home", "⌂"),
];

/// The mirrored form of a directional glyph (for right-to-left layouts).
pub fn mirror(glyph: &'static str) -> &'static str {
    match glyph {
        "▶" => "◀",
        "»" => "«",
        "❯" => "❮",
        "→" => "←",
        "←" => "→",
        other => other,
    }
}

/// Resolve an icon name.
pub fn icon(name: &str) -> Result<&'static str> {
    ICONS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, g)| *g)
        .ok_or_else(|| {
            Error::friendly(
                format!("Unknown icon \"{name}\""),
                "icons come from stagetui's built-in set",
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
