//! Colors, themes and friendly accent names.

use ratatui_core::style::Color;
use serde::Serialize;

use crate::{Error, Result};

/// A 24-bit color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Parse `#rgb` or `#rrggbb`.
    pub fn parse_hex(s: &str) -> Option<Self> {
        let h = s.strip_prefix('#')?;
        let digit = |i: usize, n: usize| u8::from_str_radix(h.get(i..i + n)?, 16).ok();
        match h.len() {
            3 => Some(Self(
                digit(0, 1)? * 17,
                digit(1, 1)? * 17,
                digit(2, 1)? * 17,
            )),
            6 => Some(Self(digit(0, 2)?, digit(2, 2)?, digit(4, 2)?)),
            _ => None,
        }
    }

    /// As a ratatui color.
    pub fn color(self) -> Color {
        Color::Rgb(self.0, self.1, self.2)
    }

    /// Linear mix: `t = 0` → self, `t = 1` → other.
    pub fn mix(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Self(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }

    /// `#rrggbb`
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

/// Friendly accent names.
pub const ACCENTS: [(&str, Rgb); 5] = [
    ("blue", Rgb(0x7a, 0xa2, 0xf7)),
    ("mint", Rgb(0x73, 0xda, 0xca)),
    ("amber", Rgb(0xe0, 0xaf, 0x68)),
    ("rose", Rgb(0xf7, 0x76, 0x8e)),
    ("violet", Rgb(0xbb, 0x9a, 0xf7)),
];

/// Resolve an accent given as a friendly name or hex string.
pub fn accent(value: &str) -> Result<Rgb> {
    if let Some((_, rgb)) = ACCENTS.iter().find(|(n, _)| n.eq_ignore_ascii_case(value)) {
        return Ok(*rgb);
    }
    Rgb::parse_hex(value).ok_or_else(|| {
        Error::friendly(
            format!("Unknown accent color \"{value}\""),
            "accents are a friendly name or a hex color",
            format!(
                "use one of {} or a hex value like \"#7aa2f7\"",
                ACCENTS
                    .iter()
                    .map(|(n, _)| format!("\"{n}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
    })
}

/// A curated color theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Theme {
    pub name: &'static str,
    pub dark: bool,
    pub bg: Rgb,
    pub surface: Rgb,
    pub fg: Rgb,
    pub muted: Rgb,
    pub border: Rgb,
    pub accent: Rgb,
}

/// The 5 curated themes.
pub const THEMES: [Theme; 5] = [
    Theme {
        name: "aurora-dark",
        dark: true,
        bg: Rgb(0x1a, 0x1b, 0x26),
        surface: Rgb(0x22, 0x24, 0x36),
        fg: Rgb(0xc0, 0xca, 0xf5),
        muted: Rgb(0x56, 0x5f, 0x89),
        border: Rgb(0x3b, 0x42, 0x61),
        accent: Rgb(0xbb, 0x9a, 0xf7),
    },
    Theme {
        name: "nordic-light",
        dark: false,
        bg: Rgb(0xec, 0xef, 0xf4),
        surface: Rgb(0xe5, 0xe9, 0xf0),
        fg: Rgb(0x2e, 0x34, 0x40),
        muted: Rgb(0x7b, 0x88, 0xa1),
        border: Rgb(0xc8, 0xd0, 0xdc),
        accent: Rgb(0x5e, 0x81, 0xac),
    },
    Theme {
        name: "velvet-dusk",
        dark: true,
        bg: Rgb(0x1e, 0x17, 0x26),
        surface: Rgb(0x2a, 0x20, 0x35),
        fg: Rgb(0xe8, 0xdf, 0xf5),
        muted: Rgb(0x8a, 0x7a, 0xa0),
        border: Rgb(0x3d, 0x2f, 0x4f),
        accent: Rgb(0xf5, 0xa3, 0xc7),
    },
    Theme {
        name: "paper-soft",
        dark: false,
        bg: Rgb(0xfa, 0xf7, 0xf0),
        surface: Rgb(0xf2, 0xed, 0xe1),
        fg: Rgb(0x3c, 0x38, 0x36),
        muted: Rgb(0x8b, 0x84, 0x78),
        border: Rgb(0xe0, 0xd8, 0xc6),
        accent: Rgb(0xd6, 0x5d, 0x0e),
    },
    Theme {
        name: "hacker-mint",
        dark: true,
        bg: Rgb(0x0b, 0x0f, 0x0c),
        surface: Rgb(0x11, 0x1a, 0x14),
        fg: Rgb(0xb8, 0xf5, 0xd0),
        muted: Rgb(0x4f, 0x7a, 0x5f),
        border: Rgb(0x1f, 0x3a, 0x2a),
        accent: Rgb(0x3d, 0xdc, 0x97),
    },
];

/// Look up a theme by name.
pub fn theme(name: &str) -> Result<Theme> {
    THEMES
        .iter()
        .find(|t| t.name == name)
        .copied()
        .ok_or_else(|| {
            Error::friendly(
                format!("Unknown theme \"{name}\""),
                "termstage ships 5 curated themes",
                format!(
                    "use one of {}",
                    THEMES
                        .iter()
                        .map(|t| format!("\"{}\"", t.name))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex() {
        assert_eq!(Rgb::parse_hex("#7aa2f7"), Some(Rgb(0x7a, 0xa2, 0xf7)));
        assert_eq!(Rgb::parse_hex("#fff"), Some(Rgb(255, 255, 255)));
        assert_eq!(Rgb::parse_hex("7aa2f7"), None);
        assert_eq!(Rgb::parse_hex("#zzzzzz"), None);
        assert_eq!(Rgb(1, 2, 255).hex(), "#0102ff");
    }

    #[test]
    fn accents_and_themes() {
        assert_eq!(accent("Violet").unwrap(), Rgb(0xbb, 0x9a, 0xf7));
        assert_eq!(accent("#000").unwrap(), Rgb(0, 0, 0));
        let err = accent("neon").unwrap_err().to_string();
        assert!(err.contains("\"mint\""), "{err}");
        assert_eq!(theme("paper-soft").unwrap().name, "paper-soft");
        assert!(
            theme("matrix")
                .unwrap_err()
                .to_string()
                .contains("aurora-dark")
        );
    }

    #[test]
    fn mix() {
        assert_eq!(Rgb(0, 0, 0).mix(Rgb(200, 100, 50), 0.5), Rgb(100, 50, 25));
    }
}
