//! Headless rasterizer: turn a [`Frame`] into a PNG-ready pixmap, exactly as a
//! good terminal would draw it — with crisp box drawing and properly shaped
//! Arabic/Hindi (drawn as real text, not presentation forms).

use std::collections::HashMap;

use ratatui_core::style::{Color, Modifier};
use tiny_skia::{Paint, PathBuilder, Pixmap, PixmapPaint, Rect as SkRect, Stroke, Transform};

use crate::frame::{Frame, TextRun};
use crate::style::Rgb;
use crate::text::raster::{self as text_raster, Face, HAlign, TextStyle};
use crate::text::{self, Direction};

/// Convert a ratatui color to RGB (`Reset` → `default`).
pub fn rgb(c: Color, default: Rgb) -> Rgb {
    match c {
        Color::Rgb(r, g, b) => Rgb(r, g, b),
        Color::Indexed(i) => indexed(i),
        Color::Reset => default,
        Color::Black => Rgb(0, 0, 0),
        Color::Red => Rgb(205, 49, 49),
        Color::Green => Rgb(13, 188, 121),
        Color::Yellow => Rgb(229, 229, 16),
        Color::Blue => Rgb(36, 114, 200),
        Color::Magenta => Rgb(188, 63, 188),
        Color::Cyan => Rgb(17, 168, 205),
        Color::Gray => Rgb(229, 229, 229),
        Color::DarkGray => Rgb(102, 102, 102),
        Color::LightRed => Rgb(241, 76, 76),
        Color::LightGreen => Rgb(35, 209, 139),
        Color::LightYellow => Rgb(245, 245, 67),
        Color::LightBlue => Rgb(59, 142, 234),
        Color::LightMagenta => Rgb(214, 112, 214),
        Color::LightCyan => Rgb(41, 184, 219),
        Color::White => Rgb(255, 255, 255),
    }
}

fn indexed(i: u8) -> Rgb {
    match i {
        16..=231 => {
            let n = i - 16;
            let v = |x: u8| if x == 0 { 0 } else { 55 + x * 40 };
            Rgb(v(n / 36), v(n / 6 % 6), v(n % 6))
        }
        232..=255 => {
            let g = 8 + (i - 232) * 10;
            Rgb(g, g, g)
        }
        _ => rgb(
            [
                Color::Black,
                Color::Red,
                Color::Green,
                Color::Yellow,
                Color::Blue,
                Color::Magenta,
                Color::Cyan,
                Color::Gray,
                Color::DarkGray,
                Color::LightRed,
                Color::LightGreen,
                Color::LightYellow,
                Color::LightBlue,
                Color::LightMagenta,
                Color::LightCyan,
                Color::White,
            ][usize::from(i)],
            Rgb(0, 0, 0),
        ),
    }
}

fn fill(p: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: Rgb) {
    if let Some(r) = SkRect::from_xywh(x, y, w, h) {
        let mut paint = Paint::default();
        paint.set_color_rgba8(c.0, c.1, c.2, 255);
        p.fill_rect(r, &paint, Transform::identity(), None);
    }
}

/// Draw box-drawing and block characters geometrically so lines join seamlessly.
fn draw_box(p: &mut Pixmap, sym: &str, x: f32, y: f32, cw: f32, ch: f32, fg: Rgb) -> bool {
    let (cx, cy) = (x + cw / 2.0, y + ch / 2.0);
    let t = (ch / 14.0).max(1.0);
    let mut paint = Paint::default();
    paint.set_color_rgba8(fg.0, fg.1, fg.2, 255);
    paint.anti_alias = true;
    let heavy = matches!(sym, "━" | "┃");
    let stroke = Stroke {
        width: if heavy { t * 2.2 } else { t },
        ..Stroke::default()
    };
    let line = |p: &mut Pixmap, x1: f32, y1: f32, x2: f32, y2: f32| {
        let mut pb = PathBuilder::new();
        pb.move_to(x1, y1);
        pb.line_to(x2, y2);
        if let Some(path) = pb.finish() {
            p.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    };
    let arc = |p: &mut Pixmap, from: (f32, f32), ctrl: (f32, f32), to: (f32, f32)| {
        let mut pb = PathBuilder::new();
        pb.move_to(from.0, from.1);
        pb.quad_to(ctrl.0, ctrl.1, to.0, to.1);
        if let Some(path) = pb.finish() {
            p.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    };
    let (l, r, t0, b) = (x, x + cw, y, y + ch);
    match sym {
        "─" | "━" => line(p, l, cy, r, cy),
        "│" | "┃" => line(p, cx, t0, cx, b),
        "┌" => {
            line(p, cx, cy, r, cy);
            line(p, cx, cy, cx, b)
        }
        "┐" => {
            line(p, l, cy, cx, cy);
            line(p, cx, cy, cx, b)
        }
        "└" => {
            line(p, cx, t0, cx, cy);
            line(p, cx, cy, r, cy)
        }
        "┘" => {
            line(p, cx, t0, cx, cy);
            line(p, l, cy, cx, cy)
        }
        "╭" => arc(p, (cx, b), (cx, cy), (r, cy)),
        "╮" => arc(p, (l, cy), (cx, cy), (cx, b)),
        "╰" => arc(p, (cx, t0), (cx, cy), (r, cy)),
        "╯" => arc(p, (cx, t0), (cx, cy), (l, cy)),
        "▀" => fill(p, x, y, cw, (ch / 2.0).ceil(), fg),
        "▄" => fill(p, x, y + ch / 2.0, cw, ch / 2.0, fg),
        "█" => fill(p, x, y, cw, ch, fg),
        "▌" => fill(p, x, y, (cw / 2.0).ceil(), ch, fg),
        "▐" => fill(p, x + cw / 2.0, y, cw / 2.0, ch, fg),
        // Left-aligned eighth blocks ▏▎▍▋▊▉ (▌ above).
        "▏" | "▎" | "▍" | "▋" | "▊" | "▉" => {
            let eighths = match sym {
                "▏" => 1.0,
                "▎" => 2.0,
                "▍" => 3.0,
                "▋" => 5.0,
                "▊" => 6.0,
                _ => 7.0,
            };
            fill(p, x, y, (cw * eighths / 8.0).round(), ch, fg)
        }
        _ => return false,
    }
    true
}

/// Rasterize a frame. `cell_px` = size of one cell in pixels.
pub fn render(frame: &Frame, cell_px: (u32, u32)) -> Pixmap {
    let area = frame.area();
    let (cw, ch) = (cell_px.0.max(1), cell_px.1.max(1));
    let (w, h) = (
        u32::from(area.width).max(1) * cw,
        u32::from(area.height).max(1) * ch,
    );
    let mut p = Pixmap::new(w, h).unwrap_or_else(|| Pixmap::new(1, 1).expect("1x1"));
    let (cwf, chf) = (cw as f32, ch as f32);

    // Cells covered by pixel-drawn text runs or images: paint background only.
    let mut covered = vec![false; usize::from(area.width) * usize::from(area.height)];
    let mut cover = |r: ratatui_core::layout::Rect| {
        for y in r.top()..r.bottom() {
            for x in r.left()..r.right() {
                covered
                    [usize::from(y - area.y) * usize::from(area.width) + usize::from(x - area.x)] =
                    true;
            }
        }
    };
    for run in frame.runs.iter().filter(|r| r.complex && r.pixel) {
        cover(run.area);
    }
    for img in &frame.images {
        cover(img.area);
    }

    let mono_size = chf * 0.74;
    let mut glyphs: HashMap<(String, bool), Pixmap> = HashMap::new();
    let width = usize::from(area.width);
    for (i, cell) in frame.cells.content().iter().enumerate() {
        let (cx, cy) = ((i % width) as f32 * cwf, (i / width) as f32 * chf);
        let mut fg = rgb(cell.fg, Rgb(230, 230, 230));
        let mut bg = rgb(cell.bg, frame.bg);
        if cell.modifier.contains(Modifier::REVERSED) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if cell.modifier.contains(Modifier::DIM) {
            fg = fg.mix(bg, 0.45);
        }
        fill(&mut p, cx, cy, cwf, chf, bg);
        let sym = cell.symbol();
        if covered[i] || sym.is_empty() || sym == " " {
            continue;
        }
        if draw_box(&mut p, sym, cx, cy, cwf, chf, fg) {
            continue;
        }
        let bold = cell.modifier.contains(Modifier::BOLD);
        let cells = text::width(sym).max(1) as u32;
        let mask = glyphs.entry((sym.to_string(), bold)).or_insert_with(|| {
            let mut m = Pixmap::new(cw * cells, ch).expect("glyph pixmap");
            let style = TextStyle {
                face: Face::Mono,
                size_px: mono_size,
                bold,
                color: Rgb(255, 255, 255),
                alpha: 255,
            };
            text_raster::draw(
                &mut m,
                sym,
                0.0,
                (chf - mono_size * 1.3) / 2.0,
                &style,
                None,
                HAlign::Left,
            );
            m
        });
        tint(&mut p, mask, cx as i32, cy as i32, fg);
        if cell.modifier.contains(Modifier::UNDERLINED) {
            fill(&mut p, cx, cy + chf - 2.0, cwf * cells as f32, 1.0, fg);
        }
    }

    // Complex-script runs as real shaped text.
    for run in frame.runs.iter().filter(|r| r.complex && r.pixel) {
        let (x, y) = (
            i32::from(run.area.x - area.x) * cw as i32,
            i32::from(run.area.y - area.y) * ch as i32,
        );
        let pic = text_run(run, (cw, ch));
        p.draw_pixmap(
            x,
            y,
            pic.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }

    for img in &frame.images {
        let (x, y) = (
            i32::from(img.area.x - area.x) * cw as i32,
            i32::from(img.area.y - area.y) * ch as i32,
        );
        let pic = img.pixels((cw, ch));
        p.draw_pixmap(
            x,
            y,
            pic.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
    p
}

/// Render one text run (reading-order text, real fonts) to exactly its cells' pixel size.
pub fn text_run(run: &TextRun, cell_px: (u32, u32)) -> Pixmap {
    let (cw, ch) = (cell_px.0.max(1), cell_px.1.max(1));
    let (w, h) = (u32::from(run.area.width).max(1) * cw, ch);
    let mut p = Pixmap::new(w, h).unwrap_or_else(|| Pixmap::new(1, 1).expect("1x1"));
    let (box_w, chf) = (w as f32, ch as f32);
    if !run.transparent {
        fill(&mut p, 0.0, 0.0, box_w, chf, run.bg);
    }
    let mut style = TextStyle {
        face: Face::Sans,
        size_px: chf * 0.7,
        bold: run.bold,
        color: run.fg,
        alpha: 255,
    };
    let (tw, _) = text_raster::measure(&run.logical, &style);
    if tw > box_w {
        style.size_px *= box_w / tw;
    }
    let ty = (chf - style.size_px * 1.3) / 2.0;
    let align = if run.dir == Direction::Rtl {
        HAlign::Right
    } else {
        HAlign::Left
    };
    text_raster::draw(&mut p, &run.logical, 0.0, ty, &style, Some(box_w), align);
    p
}

/// Blend a white-on-transparent mask onto `p` at `(x, y)` in color `fg`.
fn tint(p: &mut Pixmap, mask: &Pixmap, x: i32, y: i32, fg: Rgb) {
    let (pw, ph) = (p.width() as i32, p.height() as i32);
    let mw = mask.width() as i32;
    let data = p.data_mut();
    for (i, px) in mask.pixels().iter().enumerate() {
        let a = u32::from(px.alpha());
        if a == 0 {
            continue;
        }
        let (dx, dy) = (x + i as i32 % mw, y + i as i32 / mw);
        if dx < 0 || dy < 0 || dx >= pw || dy >= ph {
            continue;
        }
        text_raster::blend(data, (dy * pw + dx) as usize, fg.0, fg.1, fg.2, a);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;

    #[test]
    fn renders_a_card_with_arabic() {
        let mut s = Session::from_json(
            r#"{"children":[{"type":"card","content":"مَرْحَباً بِكُمْ — Hello","props":{"title":"Hi","icon":"sparkles"}}]}"#,
        )
        .unwrap();
        let frame = s.frame(40, 8);
        let p = render(&frame, (10, 20));
        assert_eq!((p.width(), p.height()), (400, 160));
        let theme = crate::style::theme("aurora-dark").unwrap();
        let non_bg = p.pixels().iter().filter(|c| {
            let c = c.demultiply();
            Rgb(c.red(), c.green(), c.blue()) != theme.bg
                && Rgb(c.red(), c.green(), c.blue()) != theme.surface
        });
        assert!(non_bg.count() > 500, "text and borders should be drawn");
    }

    #[test]
    fn indexed_colors() {
        assert_eq!(indexed(16), Rgb(0, 0, 0));
        assert_eq!(indexed(231), Rgb(255, 255, 255));
        assert_eq!(indexed(232), Rgb(8, 8, 8));
    }
}
