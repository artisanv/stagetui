//! iTerm2 inline images (iTerm2, WezTerm, and others that adopted it).
//!
//! Spec: <https://iterm2.com/documentation-images.html>

use super::base64;

/// Show a PNG at the cursor, stretched to `cols × rows` cells.
/// With `move_cursor = false` the cursor stays put (`doNotMoveCursor=1`).
pub fn inline_png(png: &[u8], cols: u16, rows: u16, move_cursor: bool) -> String {
    let stay = if move_cursor {
        ""
    } else {
        ";doNotMoveCursor=1"
    };
    format!(
        "\x1b]1337;File=inline=1;size={};width={cols};height={rows};preserveAspectRatio=0{stay}:{}\x07",
        png.len(),
        base64::encode(png)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format() {
        assert_eq!(
            inline_png(b"abc", 4, 2, false),
            "\x1b]1337;File=inline=1;size=3;width=4;height=2;preserveAspectRatio=0;doNotMoveCursor=1:YWJj\x07"
        );
        assert!(!inline_png(b"abc", 4, 2, true).contains("doNotMoveCursor"));
    }
}
