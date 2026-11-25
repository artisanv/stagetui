//! `termstage.Canvas` — Python handle to the Rust canvas.

use std::sync::Mutex;

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use termstage_core::Error;
use termstage_core::gfx::canvas::{Canvas, Rgba, parse_color};
use termstage_core::gfx::{self, Fit, Image, load};
use termstage_core::text::raster::HAlign;

use crate::to_py;

/// A picture you draw on with code: shapes, gradients, text (Arabic too) and images.
///
/// Coordinates are in pixels; `(0, 0)` is the top-left corner.
#[pyclass(name = "Canvas", module = "termstage", frozen)]
pub struct PyCanvas {
    inner: Mutex<Canvas>,
}

impl PyCanvas {
    pub fn snapshot(&self, name: &str) -> Image {
        self.lock().to_image(name)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Canvas> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn color(py: Python<'_>, s: Option<&str>) -> PyResult<Option<Rgba>> {
    s.map(parse_color).transpose().map_err(|e| to_py(py, &e))
}

fn colors(py: Python<'_>, list: &[String]) -> PyResult<Vec<Rgba>> {
    list.iter()
        .map(|c| parse_color(c).map_err(|e| to_py(py, &e)))
        .collect()
}

#[pymethods]
impl PyCanvas {
    #[new]
    #[pyo3(signature = (width = 640, height = 360, background = None))]
    fn new(py: Python<'_>, width: u32, height: u32, background: Option<&str>) -> PyResult<Self> {
        let mut canvas = Canvas::new(width, height).map_err(|e| to_py(py, &e))?;
        if let Some(bg) = color(py, background)? {
            canvas.clear(bg);
        }
        Ok(Self {
            inner: Mutex::new(canvas),
        })
    }

    /// Width in pixels.
    #[getter]
    fn width(&self) -> u32 {
        self.lock().width()
    }

    /// Height in pixels.
    #[getter]
    fn height(&self) -> u32 {
        self.lock().height()
    }

    /// Fill the whole canvas with one color.
    #[pyo3(signature = (color))]
    fn clear(&self, py: Python<'_>, color: &str) -> PyResult<()> {
        let c = parse_color(color).map_err(|e| to_py(py, &e))?;
        self.lock().clear(c);
        Ok(())
    }

    /// A rectangle. Use `radius` for rounded corners.
    #[pyo3(signature = (x, y, width, height, *, fill = None, stroke = None, stroke_width = 1.0, radius = 0.0))]
    #[allow(clippy::too_many_arguments)]
    fn rect(
        &self,
        py: Python<'_>,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        fill: Option<&str>,
        stroke: Option<&str>,
        stroke_width: f32,
        radius: f32,
    ) -> PyResult<()> {
        let (f, s) = (color(py, fill)?, color(py, stroke)?);
        let f = if f.is_none() && s.is_none() {
            Some(Rgba(255, 255, 255, 255))
        } else {
            f
        };
        self.lock()
            .rect(x, y, width, height, radius, f, s, stroke_width);
        Ok(())
    }

    /// A rectangle with rounded corners (same as `rect(..., radius=...)`).
    #[pyo3(signature = (x, y, width, height, radius = 12.0, *, fill = None, stroke = None, stroke_width = 1.0))]
    #[allow(clippy::too_many_arguments)]
    fn rounded_rect(
        &self,
        py: Python<'_>,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
        fill: Option<&str>,
        stroke: Option<&str>,
        stroke_width: f32,
    ) -> PyResult<()> {
        self.rect(py, x, y, width, height, fill, stroke, stroke_width, radius)
    }

    /// A circle centered at `(x, y)`.
    #[pyo3(signature = (x, y, radius, *, fill = None, stroke = None, stroke_width = 1.0))]
    #[allow(clippy::too_many_arguments)]
    fn circle(
        &self,
        py: Python<'_>,
        x: f32,
        y: f32,
        radius: f32,
        fill: Option<&str>,
        stroke: Option<&str>,
        stroke_width: f32,
    ) -> PyResult<()> {
        let (f, s) = (color(py, fill)?, color(py, stroke)?);
        let f = if f.is_none() && s.is_none() {
            Some(Rgba(255, 255, 255, 255))
        } else {
            f
        };
        self.lock().circle(x, y, radius, f, s, stroke_width);
        Ok(())
    }

    /// A straight line from `(x1, y1)` to `(x2, y2)`.
    #[pyo3(signature = (x1, y1, x2, y2, *, color = "white", width = 2.0))]
    #[allow(clippy::too_many_arguments)]
    fn line(
        &self,
        py: Python<'_>,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: &str,
        width: f32,
    ) -> PyResult<()> {
        let c = parse_color(color).map_err(|e| to_py(py, &e))?;
        self.lock().line(x1, y1, x2, y2, c, width);
        Ok(())
    }

    /// Lines through a list of `(x, y)` points — great for charts.
    #[pyo3(signature = (points, *, color = "white", width = 2.0, fill = None, closed = false))]
    fn polyline(
        &self,
        py: Python<'_>,
        points: Vec<(f32, f32)>,
        color: Option<&str>,
        width: f32,
        fill: Option<&str>,
        closed: bool,
    ) -> PyResult<()> {
        let (s, f) = (self::color(py, color)?, self::color(py, fill)?);
        self.lock()
            .polyline(&points, closed || f.is_some(), f, s, width);
        Ok(())
    }

    /// A rectangle filled with a smooth color blend (left→right, or top→bottom with `vertical=True`).
    #[pyo3(signature = (x, y, width, height, *, colors, vertical = false, radius = 0.0))]
    #[allow(clippy::too_many_arguments)]
    fn gradient_rect(
        &self,
        py: Python<'_>,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        colors: Vec<String>,
        vertical: bool,
        radius: f32,
    ) -> PyResult<()> {
        let cs = self::colors(py, &colors)?;
        self.lock()
            .gradient_rect(x, y, width, height, &cs, vertical, radius)
            .map_err(|e| to_py(py, &e))
    }

    /// A circle filled with a blend from the center outwards.
    #[pyo3(signature = (x, y, radius, *, colors))]
    fn gradient_circle(
        &self,
        py: Python<'_>,
        x: f32,
        y: f32,
        radius: f32,
        colors: Vec<String>,
    ) -> PyResult<()> {
        let cs = self::colors(py, &colors)?;
        self.lock()
            .gradient_circle(x, y, radius, &cs)
            .map_err(|e| to_py(py, &e))
    }

    /// Text, properly shaped (Arabic letters join, Hindi conjuncts form).
    /// `align` is "left", "center" or "right"; `(x, y)` is the anchor's top.
    #[pyo3(signature = (text, x, y, *, size = 16.0, color = "white", bold = false, align = "left", max_width = None))]
    #[allow(clippy::too_many_arguments)]
    fn text(
        &self,
        py: Python<'_>,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        color: &str,
        bold: bool,
        align: &str,
        max_width: Option<f32>,
    ) -> PyResult<()> {
        let c = parse_color(color).map_err(|e| to_py(py, &e))?;
        let align = match align {
            "left" => HAlign::Left,
            "center" => HAlign::Center,
            "right" => HAlign::Right,
            other => {
                return Err(to_py(
                    py,
                    &Error::friendly(
                        format!("Unknown align \"{other}\""),
                        "align says which side of (x, y) the text grows from",
                        "use \"left\", \"center\" or \"right\"",
                    ),
                ));
            }
        };
        self.lock()
            .text(text, x, y, size, c, bold, align, max_width);
        Ok(())
    }

    /// Draw an image file (or another Canvas) into the box `x, y, width, height`.
    #[pyo3(signature = (source, x, y, width, height, *, fit = "contain"))]
    #[allow(clippy::too_many_arguments)]
    fn image(
        &self,
        py: Python<'_>,
        source: &Bound<'_, PyAny>,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        fit: &str,
    ) -> PyResult<()> {
        let fit = Fit::parse(fit).map_err(|e| to_py(py, &e))?;
        let img = if let Ok(other) = source.cast::<PyCanvas>() {
            other.get().snapshot("canvas")
        } else {
            let path: String = source.extract()?;
            load::from_path(std::path::Path::new(&path)).map_err(|e| to_py(py, &e))?
        };
        self.lock().image(&img, x, y, width, height, fit);
        Ok(())
    }

    /// The picture as PNG bytes.
    fn png<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = self.lock().png().map_err(|e| to_py(py, &e))?;
        Ok(PyBytes::new(py, &bytes))
    }

    /// Save the picture as a PNG file.
    fn save(&self, py: Python<'_>, path: std::path::PathBuf) -> PyResult<()> {
        let bytes = self.lock().png().map_err(|e| to_py(py, &e))?;
        std::fs::write(&path, bytes).map_err(|e| {
            to_py(
                py,
                &Error::friendly(
                    format!("Could not save \"{}\"", path.display()),
                    e.to_string(),
                    "check the folder exists and you can write to it",
                ),
            )
        })
    }

    /// Register a snapshot of this canvas for `StageImage` (internal).
    fn _register(&self) -> String {
        gfx::register(self.snapshot("canvas"))
    }

    fn __repr__(&self) -> String {
        let c = self.lock();
        format!("Canvas(width={}, height={})", c.width(), c.height())
    }
}
