//! A 2D drawing surface (anti-aliased shapes, gradients, real text, images),
//! exposed to Python as `stagetui.Canvas`.

use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint, PathBuilder, Pixmap,
    PixmapPaint, Point, RadialGradient, Rect, Shader, SpreadMode, Stroke, Transform,
};

use super::{Fit, Image};
use crate::style::{self, Rgb};
use crate::text::raster::{self as text_raster, Face, HAlign, TextStyle};
use crate::{Error, Result};

/// Largest canvas side in pixels.
pub const MAX_SIDE: u32 = 8192;

/// Basic color names on top of stagetui's accents.
const NAMED: [(&str, [u8; 4]); 6] = [
    ("white", [255, 255, 255, 255]),
    ("black", [0, 0, 0, 255]),
    ("gray", [128, 128, 128, 255]),
    ("grey", [128, 128, 128, 255]),
    ("red", [0xf7, 0x76, 0x8e, 255]),
    ("transparent", [0, 0, 0, 0]),
];

/// A parsed RGBA color (straight alpha).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    fn sk(self) -> Color {
        Color::from_rgba8(self.0, self.1, self.2, self.3)
    }

    pub fn rgb(self) -> Rgb {
        Rgb(self.0, self.1, self.2)
    }
}

/// Parse `"#rgb"`, `"#rrggbb"`, `"#rrggbbaa"`, an accent name (`"mint"`) or a basic name (`"white"`).
pub fn parse_color(s: &str) -> Result<Rgba> {
    let lower = s.trim().to_ascii_lowercase();
    if let Some((_, c)) = NAMED.iter().find(|(n, _)| *n == lower) {
        return Ok(Rgba(c[0], c[1], c[2], c[3]));
    }
    if let Some(h) = lower.strip_prefix('#')
        && h.len() == 8
        && let (Some(rgb), Ok(a)) = (
            Rgb::parse_hex(&format!("#{}", &h[..6])),
            u8::from_str_radix(&h[6..], 16),
        )
    {
        return Ok(Rgba(rgb.0, rgb.1, rgb.2, a));
    }
    match style::accent(&lower) {
        Ok(c) => Ok(Rgba(c.0, c.1, c.2, 255)),
        Err(_) => Err(Error::friendly(
            format!("Unknown color \"{s}\""),
            "colors are hex codes or friendly names",
            "use a hex color like \"#7aa2f7\" (or \"#7aa2f780\" with transparency), an accent name (\"mint\", \"violet\", …), \"white\", \"black\" or \"transparent\"",
        )),
    }
}

/// A pixel canvas.
pub struct Canvas {
    pixmap: Pixmap,
}

impl Canvas {
    /// A transparent canvas of `width × height` pixels.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(Error::friendly(
                format!("A canvas of {width}×{height} pixels is not possible"),
                format!("each side must be between 1 and {MAX_SIDE} pixels"),
                "pick a size like width=640, height=360",
            ));
        }
        let pixmap = Pixmap::new(width, height).ok_or_else(|| {
            Error::friendly(
                "The canvas is too large",
                "there is not enough memory",
                "use a smaller size",
            )
        })?;
        Ok(Self { pixmap })
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    pub fn pixmap(&self) -> &Pixmap {
        &self.pixmap
    }

    /// Snapshot as an [`Image`].
    pub fn to_image(&self, name: &str) -> Image {
        Image::from_pixmap(name, self.pixmap.clone())
    }

    /// Fill everything with `color`.
    pub fn clear(&mut self, color: Rgba) {
        self.pixmap.fill(color.sk());
    }

    fn paint(color: Rgba) -> Paint<'static> {
        let mut p = Paint::default();
        p.set_color(color.sk());
        p.anti_alias = true;
        p
    }

    fn stroke(width: f32) -> Stroke {
        Stroke {
            width: width.max(0.1),
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        }
    }

    fn fill_and_stroke(
        &mut self,
        path: &tiny_skia::Path,
        fill: Option<Rgba>,
        stroke: Option<Rgba>,
        width: f32,
    ) {
        if let Some(f) = fill {
            self.pixmap.fill_path(
                path,
                &Self::paint(f),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        if let Some(s) = stroke {
            self.pixmap.stroke_path(
                path,
                &Self::paint(s),
                &Self::stroke(width),
                Transform::identity(),
                None,
            );
        }
    }

    /// A rectangle, optionally with rounded corners.
    #[allow(clippy::too_many_arguments)]
    pub fn rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        fill: Option<Rgba>,
        stroke: Option<Rgba>,
        width: f32,
    ) {
        if let Some(path) = rounded_rect(x, y, w, h, radius) {
            self.fill_and_stroke(&path, fill, stroke, width);
        }
    }

    /// A circle.
    pub fn circle(
        &mut self,
        cx: f32,
        cy: f32,
        r: f32,
        fill: Option<Rgba>,
        stroke: Option<Rgba>,
        width: f32,
    ) {
        if let Some(path) = PathBuilder::from_circle(cx, cy, r.max(0.01)) {
            self.fill_and_stroke(&path, fill, stroke, width);
        }
    }

    /// A straight line.
    pub fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: Rgba, width: f32) {
        self.polyline(&[(x1, y1), (x2, y2)], false, None, Some(color), width);
    }

    /// Connected line segments (`closed` joins the last point to the first).
    pub fn polyline(
        &mut self,
        points: &[(f32, f32)],
        closed: bool,
        fill: Option<Rgba>,
        stroke: Option<Rgba>,
        width: f32,
    ) {
        let mut pb = PathBuilder::new();
        for (i, &(x, y)) in points.iter().enumerate() {
            if i == 0 {
                pb.move_to(x, y)
            } else {
                pb.line_to(x, y)
            }
        }
        if closed {
            pb.close();
        }
        if let Some(path) = pb.finish() {
            self.fill_and_stroke(&path, fill, stroke, width);
        }
    }

    fn stops(colors: &[Rgba]) -> Vec<GradientStop> {
        let n = colors.len().max(2) - 1;
        colors
            .iter()
            .enumerate()
            .map(|(i, c)| GradientStop::new(i as f32 / n as f32, c.sk()))
            .collect()
    }

    /// A rectangle filled with a linear gradient (left→right, or top→bottom if `vertical`).
    #[allow(clippy::too_many_arguments)]
    pub fn gradient_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        colors: &[Rgba],
        vertical: bool,
        radius: f32,
    ) -> Result<()> {
        let end = if vertical {
            Point::from_xy(x, y + h)
        } else {
            Point::from_xy(x + w, y)
        };
        let shader = LinearGradient::new(
            Point::from_xy(x, y),
            end,
            Self::stops(&two(colors)?),
            SpreadMode::Pad,
            Transform::identity(),
        );
        self.fill_shader(rounded_rect(x, y, w, h, radius), shader);
        Ok(())
    }

    /// A circle filled with a radial gradient from the center outwards.
    pub fn gradient_circle(&mut self, cx: f32, cy: f32, r: f32, colors: &[Rgba]) -> Result<()> {
        let c = Point::from_xy(cx, cy);
        let shader = RadialGradient::new(
            c,
            0.0,
            c,
            r.max(0.01),
            Self::stops(&two(colors)?),
            SpreadMode::Pad,
            Transform::identity(),
        );
        self.fill_shader(PathBuilder::from_circle(cx, cy, r.max(0.01)), shader);
        Ok(())
    }

    fn fill_shader(&mut self, path: Option<tiny_skia::Path>, shader: Option<Shader<'static>>) {
        if let (Some(path), Some(shader)) = (path, shader) {
            let paint = Paint {
                shader,
                anti_alias: true,
                ..Paint::default()
            };
            self.pixmap.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    /// Text with real shaping (Arabic joins, Hindi conjuncts). `(x, y)` is the top-left
    /// corner (or top-right for `align = Right`, top-center for `Center`).
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        s: &str,
        x: f32,
        y: f32,
        size: f32,
        color: Rgba,
        bold: bool,
        align: HAlign,
        max_width: Option<f32>,
    ) {
        let style = TextStyle {
            face: Face::Sans,
            size_px: size.clamp(1.0, 1000.0),
            bold,
            color: color.rgb(),
            alpha: color.3,
        };
        let (w, _) = text_raster::measure(s, &style);
        let box_w = max_width.unwrap_or(w).max(1.0);
        let left = match align {
            HAlign::Left => x,
            HAlign::Center => x - box_w / 2.0,
            HAlign::Right => x - box_w,
        };
        text_raster::draw(
            &mut self.pixmap,
            s,
            left,
            y,
            &style,
            Some(box_w + 1.0),
            align,
        );
    }

    /// Draw an image into the box `x, y, w, h`.
    pub fn image(&mut self, image: &Image, x: f32, y: f32, w: f32, h: f32, fit: Fit) {
        let p = image.render(w.max(1.0) as u32, h.max(1.0) as u32, fit);
        self.pixmap.draw_pixmap(
            x as i32,
            y as i32,
            p.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }

    /// Encode as PNG.
    pub fn png(&self) -> Result<Vec<u8>> {
        super::png(&self.pixmap)
    }
}

fn two(colors: &[Rgba]) -> Result<Vec<Rgba>> {
    match colors.len() {
        0 => Err(Error::friendly(
            "A gradient needs colors",
            "no colors were given",
            "pass at least two, e.g. colors=[\"#7aa2f7\", \"#bb9af7\"]",
        )),
        1 => Ok(vec![colors[0], colors[0]]),
        _ => Ok(colors.to_vec()),
    }
}

/// Rounded-rectangle path (`radius = 0` → plain rectangle).
pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<tiny_skia::Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = radius.clamp(0.0, w.min(h) / 2.0);
    if r <= 0.0 {
        return Some(PathBuilder::from_rect(Rect::from_xywh(x, y, w, h)?));
    }
    // Cubic approximation of a quarter circle.
    let k = r * 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(c: &Canvas, x: u32, y: u32) -> [u8; 4] {
        let p = c.pixmap().pixel(x, y).unwrap().demultiply();
        [p.red(), p.green(), p.blue(), p.alpha()]
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#ff000080").unwrap(), Rgba(255, 0, 0, 128));
        assert_eq!(parse_color("Mint").unwrap(), Rgba(0x73, 0xda, 0xca, 255));
        assert_eq!(parse_color("transparent").unwrap().3, 0);
        let e = parse_color("plaid").unwrap_err().to_string();
        assert!(e.contains("Unknown color") && e.contains("fix:"));
    }

    #[test]
    fn shapes_and_gradients() {
        let mut c = Canvas::new(100, 60).unwrap();
        c.clear(parse_color("#0f172a").unwrap());
        c.rect(
            10.0,
            10.0,
            30.0,
            20.0,
            6.0,
            Some(parse_color("mint").unwrap()),
            None,
            1.0,
        );
        assert_eq!(at(&c, 25, 20), [0x73, 0xda, 0xca, 255]);
        assert_eq!(at(&c, 10, 10)[..3], [0x0f, 0x17, 0x2a]); // rounded corner left untouched
        c.circle(
            70.0,
            30.0,
            10.0,
            Some(parse_color("white").unwrap()),
            None,
            1.0,
        );
        assert_eq!(at(&c, 70, 30), [255, 255, 255, 255]);
        c.gradient_rect(
            0.0,
            50.0,
            100.0,
            10.0,
            &[parse_color("black").unwrap(), parse_color("white").unwrap()],
            false,
            0.0,
        )
        .unwrap();
        assert!(at(&c, 2, 55)[0] < 30 && at(&c, 97, 55)[0] > 225);
        assert!(c.gradient_circle(50.0, 30.0, 5.0, &[]).is_err());
    }

    #[test]
    fn text_draws_arabic() {
        let mut c = Canvas::new(200, 40).unwrap();
        c.text(
            "مرحبا بكم",
            190.0,
            5.0,
            22.0,
            parse_color("white").unwrap(),
            true,
            HAlign::Right,
            None,
        );
        let ink = c.pixmap().pixels().iter().filter(|p| p.alpha() > 0).count();
        assert!(ink > 100);
        assert!(c.png().unwrap().starts_with(b"\x89PNG"));
    }

    #[test]
    fn bad_sizes() {
        assert!(Canvas::new(0, 10).is_err());
        assert!(Canvas::new(MAX_SIDE + 1, 10).is_err());
    }
}
