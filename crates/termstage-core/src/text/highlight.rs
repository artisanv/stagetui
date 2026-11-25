//! Syntax highlighting with `syntect` (pure-Rust regex engine, bundled grammars).

use std::sync::OnceLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

use crate::style::Rgb;

/// One colored piece of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub text: String,
    pub color: Rgb,
    pub bold: bool,
}

fn syntaxes() -> &'static SyntaxSet {
    static S: OnceLock<SyntaxSet> = OnceLock::new();
    S.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn themes() -> &'static ThemeSet {
    static T: OnceLock<ThemeSet> = OnceLock::new();
    T.get_or_init(ThemeSet::load_defaults)
}

fn theme(dark: bool) -> &'static Theme {
    let set = themes();
    let name = if dark {
        "base16-ocean.dark"
    } else {
        "InspiredGitHub"
    };
    set.themes
        .get(name)
        .or_else(|| set.themes.values().next())
        .expect("syntect ships default themes")
}

/// Find a grammar from a language name (`"python"`, `"rs"`), a file name
/// (`"main.py"`), or the first line (`#!/bin/bash`).
fn syntax(lang: Option<&str>, code: &str) -> &'static SyntaxReference {
    let set = syntaxes();
    let by_name = lang.and_then(|l| {
        let l = l.trim();
        let ext = l.rsplit('.').next().unwrap_or(l);
        set.find_syntax_by_token(l)
            .or_else(|| set.find_syntax_by_extension(ext))
            .or_else(|| set.find_syntax_by_token(&l.to_lowercase()))
    });
    by_name
        .or_else(|| set.find_syntax_by_first_line(code.lines().next().unwrap_or("")))
        .unwrap_or_else(|| set.find_syntax_plain_text())
}

/// The language name termstage would use for `lang` / `code` (for agents).
pub fn language(lang: Option<&str>, code: &str) -> String {
    syntax(lang, code).name.clone()
}

/// Highlight `code` line by line. `dark` picks a palette for dark or light themes.
pub fn highlight(code: &str, lang: Option<&str>, dark: bool) -> Vec<Vec<Token>> {
    let mut h = HighlightLines::new(syntax(lang, code), theme(dark));
    let mut out = Vec::new();
    for line in LinesWithEndings::from(code) {
        let tokens = match h.highlight_line(line, syntaxes()) {
            Ok(ranges) => ranges
                .into_iter()
                .filter_map(|(style, text)| {
                    let text = text.trim_end_matches(['\n', '\r']).replace('\t', "    ");
                    (!text.is_empty()).then(|| Token {
                        text,
                        color: Rgb(style.foreground.r, style.foreground.g, style.foreground.b),
                        bold: style.font_style.contains(FontStyle::BOLD),
                    })
                })
                .collect(),
            Err(_) => vec![Token {
                text: line.trim_end_matches(['\n', '\r']).to_string(),
                color: if dark {
                    Rgb(0xc0, 0xc5, 0xce)
                } else {
                    Rgb(0x33, 0x33, 0x33)
                },
                bold: false,
            }],
        };
        out.push(tokens);
    }
    if code.ends_with('\n') || code.is_empty() {
        // `LinesWithEndings` doesn't report a trailing empty line; keep line counts honest.
        if code.is_empty() {
            out.push(Vec::new());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_languages_and_colors_tokens() {
        assert_eq!(language(Some("python"), ""), "Python");
        assert_eq!(language(Some("main.rs"), ""), "Rust");
        assert_eq!(
            language(None, "#!/bin/bash\necho hi"),
            "Bourne Again Shell (bash)"
        );
        assert_eq!(language(Some("nope-lang"), "x"), "Plain Text");
        let lines = highlight("def f(x):\n    return 'مرحبا'\n", Some("py"), true);
        assert_eq!(lines.len(), 2);
        let colors: std::collections::HashSet<_> = lines[0].iter().map(|t| t.color).collect();
        assert!(colors.len() > 1, "keywords and names get different colors");
        let text: String = lines[1].iter().map(|t| t.text.as_str()).collect();
        assert_eq!(text, "    return 'مرحبا'");
    }
}
