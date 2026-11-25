//! `ts.show(...)` / `termstage show`: print text cards and images **inline**,
//! right where the cursor is, like `print()` — no full-screen app needed.

use std::path::Path;

use ratatui_core::style::Color;

use crate::caps::{Capabilities, GraphicsProtocol};
use crate::frame::Frame;
use crate::gfx::{self, Fit, Image, blocks, iterm2, kitty, load, sixel};
use crate::render::{ColorMode, Encoder};
use crate::session::Session;
use crate::term::{BIDI_EXPLICIT, BIDI_IMPLICIT};
use crate::tree::{AppSpec, NodeSpec, Props};
use crate::view::Gfx;
use crate::{Error, Result};

/// Options shared by text and image display.
#[derive(Debug, Clone, Default)]
pub struct ShowOptions {
    pub title: Option<String>,
    pub icon: Option<String>,
    pub dir: Option<String>,
    pub theme: Option<String>,
    pub accent: Option<String>,
    /// Size in cells (images; `width` also caps text cards).
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub fit: Option<String>,
}

/// What a string passed to `show` means.
pub fn is_image_file(s: &str) -> bool {
    let p = Path::new(s);
    s.len() < 4096 && load::is_image_path(p) && p.is_file()
}

/// Build the escape sequences that show `content` (an image path or text).
pub fn show(content: &str, opts: &ShowOptions, caps: &Capabilities) -> Result<String> {
    if is_image_file(content) {
        let image = load::from_path(Path::new(content))?;
        show_image(&image, opts, caps)
    } else if load::is_image_path(Path::new(content))
        && !content.contains('\n')
        && content.len() < 4096
    {
        Err(Error::friendly(
            format!("Image \"{content}\" was not found"),
            "the text looks like an image file name, but no such file exists",
            "check the path (it is relative to the folder you run the command from)",
        ))
    } else {
        show_text(content, opts, caps)
    }
}

fn card_session(text: &str, opts: &ShowOptions) -> Result<Session> {
    let spec = AppSpec {
        theme: opts.theme.clone().unwrap_or_else(|| "aurora-dark".into()),
        accent: opts.accent.clone(),
        dir: None,
        bindings: Vec::new(),
        reduce_motion: None,
        prefix: None,
        pane_types: Vec::new(),
        children: vec![NodeSpec {
            kind: "card".into(),
            content: serde_json::Value::String(text.to_string()),
            props: Props {
                id: Some("show".into()),
                title: opts.title.clone(),
                icon: opts.icon.clone(),
                dir: opts.dir.clone(),
                ..Props::default()
            },
            ..NodeSpec::default()
        }],
    };
    Session::from_spec(&spec)
}

/// Render `text` as a card sized to its content.
pub fn card_frame(
    text: &str,
    opts: &ShowOptions,
    max_cols: u16,
    cell_px: (u32, u32),
) -> Result<Frame> {
    let mut session = card_session(text, opts)?;
    session.set_cell_px(cell_px);
    session.set_margin(0);
    let (w, h) = session.natural_size(opts.width.unwrap_or(max_cols).min(max_cols).max(12));
    Ok(session.frame(w, h))
}

fn max_cols(caps: &Capabilities) -> u16 {
    if caps.headless || caps.size.cols == 0 {
        80
    } else {
        caps.size.cols
    }
}

/// A text card, printed inline.
pub fn show_text(text: &str, opts: &ShowOptions, caps: &Capabilities) -> Result<String> {
    let cell_px = (caps.size.cell_width_px, caps.size.cell_height_px);
    let mut frame = card_frame(text, opts, max_cols(caps), cell_px)?;
    if caps.headless {
        return Ok(format!("{}\n", frame.text()));
    }
    // Inline output sits on the user's own terminal background: whatever the card
    // drew on the theme background (border, title) adopts the terminal's color.
    let theme_bg = frame.bg;
    let terminal_bg = caps.background_rgb();
    for run in frame.runs.iter_mut().filter(|r| r.bg == theme_bg) {
        match terminal_bg {
            Some(bg) => run.bg = bg,
            None => run.transparent = true,
        }
    }
    let mut gfx = Gfx::new(caps);
    let mut prepared = gfx.prepare(&frame, true);
    for cell in prepared.cells.content.iter_mut() {
        if cell.bg == theme_bg.color() {
            cell.bg = Color::Reset;
        }
    }
    let color = if caps.truecolor {
        ColorMode::TrueColor
    } else {
        ColorMode::Ansi256
    };
    let mut out = String::new();
    if caps.bidi.terminal_reorders {
        out.push_str(BIDI_EXPLICIT);
    }
    out.push_str(&prepared.pre);
    Encoder { color, sync: false }.inline(&prepared.cells, &mut out);
    // Pictures of text go on top of the lines just printed: save the cursor,
    // move up to each run, draw, and come back.
    let rows = frame.area().height;
    for o in prepared.overlays.iter().flatten() {
        out.push_str("\x1b7");
        out.push_str(&format!("\x1b[{}A\x1b[{}G", rows - o.area.y, o.area.x + 1));
        out.push_str(&o.payload);
        out.push_str("\x1b8");
    }
    if caps.bidi.terminal_reorders {
        out.push_str(BIDI_IMPLICIT);
    }
    Ok(out)
}

/// An image, printed inline at the best quality the terminal supports.
pub fn show_image(image: &Image, opts: &ShowOptions, caps: &Capabilities) -> Result<String> {
    let fit = match &opts.fit {
        Some(f) => Fit::parse(f)?,
        None => Fit::Contain,
    };
    if caps.headless {
        return Ok(format!(
            "[image: {} {}×{}]\n",
            image.name, image.width, image.height
        ));
    }
    let cell_px = (
        caps.size.cell_width_px.max(1),
        caps.size.cell_height_px.max(1),
    );
    let max_rows = caps.size.rows.saturating_sub(2).clamp(4, 40);
    let (cols, rows) = match (opts.width, opts.height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => {
            let h = (f32::from(w) * cell_px.0 as f32 * image.height as f32
                / image.width.max(1) as f32
                / cell_px.1 as f32)
                .round();
            (w, (h as u16).max(1))
        }
        (None, Some(h)) => {
            let w = (f32::from(h) * cell_px.1 as f32 * image.width as f32
                / image.height.max(1) as f32
                / cell_px.0 as f32)
                .round();
            ((w as u16).max(1), h)
        }
        (None, None) => image.cells(cell_px, max_cols(caps), max_rows),
    };
    let render = || {
        image.render(
            u32::from(cols) * cell_px.0,
            u32::from(rows) * cell_px.1,
            fit,
        )
    };
    let tmux = caps.remote.tmux;
    let id = ((image.id as u32) & 0x00FF_FFFF).max(1);
    let out = match caps.graphics.best {
        GraphicsProtocol::Kitty if caps.graphics.kitty_placeholders => {
            let png = gfx::png(&render())?;
            let (r, g, b) = kitty::id_color(id);
            let mut s = kitty::transmit_png(id, &png, tmux);
            s.push_str(&kitty::virtual_placement(id, cols, rows, tmux));
            for row in 0..rows.min(kitty::MAX_PLACEHOLDER_CELLS) {
                s.push_str(&format!("\r\x1b[38;2;{r};{g};{b}m"));
                for col in 0..cols.min(kitty::MAX_PLACEHOLDER_CELLS) {
                    s.push_str(&kitty::placeholder(row, col));
                }
                s.push_str("\x1b[39m\n");
            }
            s
        }
        GraphicsProtocol::Kitty => format!(
            "{}\n",
            kitty::display_png(id, &gfx::png(&render())?, cols, rows, true, tmux)
        ),
        GraphicsProtocol::Iterm2 => format!(
            "{}\n",
            iterm2::inline_png(&gfx::png(&render())?, cols, rows, true)
        ),
        GraphicsProtocol::Sixel => format!("{}\n", sixel::encode(&render())),
        GraphicsProtocol::Halfblocks => {
            let mut frame = Frame::new(
                ratatui_core::layout::Rect::new(0, 0, cols, rows),
                crate::style::Rgb(0, 0, 0),
                crate::style::Rgb(255, 255, 255),
            );
            let area = frame.cells.area;
            blocks::draw(
                &mut frame.cells,
                area,
                image,
                fit,
                blocks::Backdrop::Terminal,
            );
            let color = if caps.truecolor {
                ColorMode::TrueColor
            } else {
                ColorMode::Ansi256
            };
            let mut s = String::new();
            Encoder { color, sync: false }.inline(&frame.cells, &mut s);
            s
        }
    };
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::{Env, Grid, ProbeInfo, TtyInfo, parse, resolve};

    fn caps(replies: &[u8], env: &[(&str, &str)], headless: bool) -> Capabilities {
        resolve(
            &Env::from_pairs(env.iter().copied()),
            TtyInfo {
                stdin: true,
                stdout: true,
            },
            headless.then(|| "test".to_string()),
            &parse::parse(replies),
            Grid {
                cols: 80,
                rows: 24,
                width_px: 800,
                height_px: 480,
            },
            ProbeInfo::default(),
        )
    }

    #[test]
    fn headless_text_is_readable() {
        let out = show_text(
            "مرحبا بكم",
            &ShowOptions {
                title: Some("Hi".into()),
                ..Default::default()
            },
            &caps(b"", &[], true),
        )
        .unwrap();
        assert!(out.contains("مرحبا بكم") && out.contains("Hi"), "{out}");
    }

    #[test]
    fn vte_gets_explicit_bidi_mode() {
        let out = show_text(
            "مرحبا",
            &ShowOptions::default(),
            &caps(b"\x1b[?65;1;9c", &[("VTE_VERSION", "8002")], false),
        )
        .unwrap();
        assert!(out.starts_with(BIDI_EXPLICIT) && out.ends_with(BIDI_IMPLICIT));
        assert!(
            out.contains('\u{FEE3}'),
            "Arabic is sent as presentation forms"
        );
    }

    #[test]
    fn kitty_text_uses_pixels_for_arabic() {
        let c = caps(
            b"\x1bP>|kitty(0.39.1)\x1b\\\x1b_Gi=31;OK\x1b\\\x1b[?62c",
            &[],
            false,
        );
        let out = show_text("مرحبا", &ShowOptions::default(), &c).unwrap();
        assert!(out.contains("a=t,f=100") && out.contains(kitty::PLACEHOLDER));
    }

    #[test]
    fn images_per_protocol() {
        let mut p = tiny_skia::Pixmap::new(40, 40).unwrap();
        p.fill(tiny_skia::Color::from_rgba8(0, 128, 255, 255));
        let img = Image::from_pixmap("dot.png", p);
        let opts = ShowOptions::default();
        let kitty = show_image(
            &img,
            &opts,
            &caps(
                b"\x1bP>|kitty(0.39.1)\x1b\\\x1b_Gi=31;OK\x1b\\\x1b[?62c",
                &[],
                false,
            ),
        )
        .unwrap();
        assert!(kitty.contains("a=p,U=1") && kitty.contains(kitty::PLACEHOLDER));
        let sixel = show_image(&img, &opts, &caps(b"\x1b[?62;4c", &[], false)).unwrap();
        assert!(sixel.starts_with("\x1bP0;1;0q"));
        let it2 = show_image(
            &img,
            &opts,
            &caps(b"", &[("TERM_PROGRAM", "iTerm.app")], false),
        )
        .unwrap();
        assert!(it2.starts_with("\x1b]1337;File="));
        let blocks = show_image(&img, &opts, &caps(b"", &[], false)).unwrap();
        assert!(blocks.contains('▀'));
        let headless = show_image(&img, &opts, &caps(b"", &[], true)).unwrap();
        assert_eq!(headless, "[image: dot.png 40×40]\n");
    }

    #[test]
    fn sixel_terminals_get_arabic_as_pictures_above_the_card() {
        let c = caps(
            b"\x1bP>|foot(1.28.0)\x1b\\\x1b]11;rgb:2020/2020/2020\x1b\\\x1b[?62;4c",
            &[],
            false,
        );
        assert_eq!(c.background.as_deref(), Some("#202020"));
        let out = show_text(
            "مرحبا",
            &ShowOptions {
                title: Some("عربي".into()),
                ..Default::default()
            },
            &c,
        )
        .unwrap();
        assert!(
            out.contains("\x1b7") && out.contains("\x1bP0;1;0q") && out.contains("\x1b8"),
            "{out:?}"
        );
        // Nothing on the theme background leaks into the user's terminal.
        assert!(
            !out.contains("48;2;26;27;38"),
            "theme background must not be printed inline"
        );
    }

    #[test]
    fn missing_image_file_is_explained() {
        let e = show(
            "nope/cat.png",
            &ShowOptions::default(),
            &caps(b"", &[], true),
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("was not found"), "{e}");
    }
}
