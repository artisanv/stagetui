//! `stagetui._core.App`: one running (or headless) app. Glue only — every
//! behaviour lives in `stagetui_core::session` / `stagetui_core::app`.

use std::sync::Arc;
use std::time::Duration;

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use stagetui_core::app::App;
use stagetui_core::caps::{self, DetectOptions};
use stagetui_core::session::{AppEvent, Session};
use stagetui_core::tree::NodeSpec;
use stagetui_core::{Error, headless};

use crate::canvas::PyCanvas;
use crate::to_py;

/// `(key, id, name, value_json)`
type PyEvent = (Option<String>, String, String, String);

fn event(e: AppEvent) -> PyEvent {
    (e.key, e.id, e.name, e.value.to_string())
}

fn spec(py: Python<'_>, json: &str) -> PyResult<NodeSpec> {
    serde_json::from_str(json).map_err(|e| {
        to_py(
            py,
            &Error::friendly(
                "That widget description has a mistake",
                e.to_string(),
                "check widget names and property spellings (see docs/reference.md)",
            ),
        )
    })
}

fn json(py: Python<'_>, s: &str) -> PyResult<serde_json::Value> {
    serde_json::from_str(s).map_err(|e| to_py(py, &e.into()))
}

#[pyclass(name = "App", module = "stagetui._core", frozen)]
pub struct PyApp {
    app: Arc<App>,
}

impl PyApp {
    fn run<R>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&mut Session) -> stagetui_core::Result<R>,
    ) -> PyResult<R> {
        self.app.with(f).map_err(|e| to_py(py, &e))
    }
}

#[pymethods]
impl PyApp {
    #[new]
    #[pyo3(signature = (spec_json, cell = (16, 32)))]
    fn new(py: Python<'_>, spec_json: &str, cell: (u32, u32)) -> PyResult<Self> {
        let mut session = Session::from_json(spec_json).map_err(|e| to_py(py, &e))?;
        session.set_cell_px(cell);
        Ok(Self {
            app: App::new(session),
        })
    }

    /// Take over the terminal and start drawing (returns immediately).
    fn start(&self, py: Python<'_>) -> PyResult<()> {
        let app = Arc::clone(&self.app);
        py.detach(move || {
            let caps = caps::detect(&DetectOptions::default());
            app.start(caps)
        })
        .map_err(|e| to_py(py, &e))
    }

    /// Give the terminal back and wait for the drawing threads.
    fn stop(&self, py: Python<'_>) -> PyResult<()> {
        let app = Arc::clone(&self.app);
        py.detach(move || app.stop()).map_err(|e| to_py(py, &e))
    }

    fn running(&self) -> bool {
        self.app.is_running()
    }

    /// Wait for the next event (GIL released). `None` on timeout.
    #[pyo3(signature = (timeout = 0.25))]
    fn next_event(&self, py: Python<'_>, timeout: f64) -> Option<PyEvent> {
        let app = Arc::clone(&self.app);
        let t = Duration::from_secs_f64(timeout.clamp(0.0, 60.0));
        py.detach(move || app.next_event(t)).map(event)
    }

    /// Events produced by headless input since the last call.
    fn poll_events(&self) -> Vec<PyEvent> {
        self.app
            .session()
            .take_events()
            .into_iter()
            .map(event)
            .collect()
    }

    fn update(&self, py: Python<'_>, target: &str, patch_json: &str) -> PyResult<()> {
        let patch = json(py, patch_json)?;
        self.run(py, |s| s.update(target, &patch))
    }

    fn show(&self, py: Python<'_>, target: &str) -> PyResult<()> {
        self.run(py, |s| s.set_shown(target, true))
    }

    fn hide(&self, py: Python<'_>, target: &str) -> PyResult<()> {
        self.run(py, |s| s.set_shown(target, false))
    }

    fn focus(&self, py: Python<'_>, target: &str) -> PyResult<()> {
        self.run(py, |s| s.focus(target))
    }

    fn remove(&self, py: Python<'_>, target: &str) -> PyResult<()> {
        self.run(py, |s| s.remove(target))
    }

    /// Split `target` with a new widget; returns the new widget's id.
    fn split(
        &self,
        py: Python<'_>,
        target: &str,
        spec_json: &str,
        direction: &str,
    ) -> PyResult<String> {
        let spec = spec(py, spec_json)?;
        self.run(py, |s| s.split(target, &spec, direction))
    }

    /// Add a widget inside `parent` (or to the app); returns its id.
    #[pyo3(signature = (parent, spec_json))]
    fn mount(&self, py: Python<'_>, parent: Option<&str>, spec_json: &str) -> PyResult<String> {
        let spec = spec(py, spec_json)?;
        self.run(py, |s| s.mount(parent, &spec))
    }

    /// Run a widget method (`chat.add`, `log.append`, …). Args and result are JSON.
    fn call(
        &self,
        py: Python<'_>,
        target: &str,
        method: &str,
        args_json: &str,
    ) -> PyResult<String> {
        let args = json(py, args_json)?;
        self.run(py, |s| s.call(target, method, &args))
            .map(|v| v.to_string())
    }

    /// Set what a pane shows; returns the content widget's id.
    #[pyo3(signature = (target, pane_type, spec_json, replace = true))]
    fn pane_set(
        &self,
        py: Python<'_>,
        target: &str,
        pane_type: &str,
        spec_json: &str,
        replace: bool,
    ) -> PyResult<String> {
        let spec = spec(py, spec_json)?;
        self.run(py, |s| s.pane_set(target, pane_type, &spec, replace))
    }

    /// Switch a pane to another type (restoring stashed content if any).
    fn switch_pane(&self, py: Python<'_>, target: &str, pane_type: &str) -> PyResult<()> {
        self.run(py, |s| {
            let i = s.tree().require(target)?;
            s.switch_pane(i, pane_type);
            Ok(())
        })
    }

    /// The whole widget tree as JSON (bounds from the last frame).
    fn tree(&self) -> String {
        let s = self.app.session();
        let (c, r) = s.size();
        s.tree_json(c, r).to_string()
    }

    fn set_theme(&self, py: Python<'_>, name: &str) -> PyResult<()> {
        self.run(py, |s| s.set_theme(name))
    }

    #[pyo3(signature = (text, icon = None, accent = None, timeout = 3.0))]
    fn notify(
        &self,
        py: Python<'_>,
        text: &str,
        icon: Option<&str>,
        accent: Option<&str>,
        timeout: f64,
    ) -> PyResult<()> {
        let t = Duration::from_secs_f64(timeout.clamp(0.5, 3600.0));
        self.run(py, |s| s.notify(text, icon, accent, t))
    }

    /// A live canvas finished drawing its next picture.
    fn set_canvas(
        &self,
        py: Python<'_>,
        target: &str,
        canvas: PyRef<'_, PyCanvas>,
    ) -> PyResult<()> {
        let image = canvas.snapshot("canvas");
        self.run(py, |s| s.set_image(target, image))
    }

    fn value(&self, py: Python<'_>, target: &str) -> PyResult<String> {
        self.run(py, |s| s.value(target)).map(|v| v.to_string())
    }

    fn query(&self, target: &str) -> Option<String> {
        self.app.session().query(target).map(|v| v.to_string())
    }

    /// `(key, id)` of the widget matching `target` (`"#id"` or a key).
    fn find(&self, target: &str) -> Option<(Option<String>, String)> {
        let s = self.app.session();
        let i = s.tree().find(target)?;
        let n = &s.tree().nodes[i];
        Some((n.key.clone(), n.id.clone()))
    }

    fn focused(&self) -> Option<(Option<String>, String)> {
        self.app.session().focused()
    }

    fn focused_pane(&self) -> Option<(Option<String>, String)> {
        self.app.session().focused_pane()
    }

    /// Run an action by name (`"quit"`, `"next_theme"`, …).
    fn action(&self, name: &str) {
        self.app.with(|s| s.action(name));
    }

    fn quit(&self) {
        self.app.with(Session::request_quit);
    }

    // ----- headless driving -----

    fn press(&self, py: Python<'_>, key: &str) -> PyResult<()> {
        self.run(py, |s| s.press(key))
    }

    fn type_text(&self, text: &str) {
        self.app.with(|s| s.type_text(text));
    }

    fn click(&self, py: Python<'_>, target: &str) -> PyResult<()> {
        self.run(py, |s| s.click(target))
    }

    fn click_at(&self, x: u16, y: u16) {
        self.app.with(|s| s.click_at(x, y));
    }

    /// Scroll the wheel over a widget (or the middle of the screen): `rows` < 0 is up.
    #[pyo3(signature = (rows, target = None))]
    fn scroll(&self, py: Python<'_>, rows: i32, target: Option<&str>) -> PyResult<()> {
        self.run(py, |s| {
            let (x, y) = match target {
                Some(t) => {
                    let v = s.query(t).ok_or_else(|| s.tree().require(t).unwrap_err())?;
                    let b = &v["bounds"];
                    let n = |k: &str| b[k].as_u64().unwrap_or(0) as u16;
                    (n("x") + n("w") / 2, n("y") + n("h") / 2)
                }
                None => (0, 0),
            };
            s.scroll_at(x, y, rows);
            Ok(())
        })
    }

    /// Move the virtual clock forward.
    fn advance(&self, ms: u64) {
        self.app.with(|s| s.advance(Duration::from_millis(ms)));
    }

    /// Draw a frame at `cols × rows` (needed before clicks find widgets).
    fn render(&self, cols: u16, rows: u16) {
        let _ = self.app.session().frame(cols, rows);
    }

    /// Play entrance animations to the end.
    fn settle(&self, py: Python<'_>, cols: u16, rows: u16) {
        let app = Arc::clone(&self.app);
        py.detach(move || app.session().settle(cols, rows));
    }

    /// `(text, tree_json)` of the screen right now.
    fn snapshot(&self, py: Python<'_>, cols: u16, rows: u16) -> PyResult<(String, String)> {
        let app = Arc::clone(&self.app);
        py.detach(move || {
            let snap = headless::snapshot(&mut app.session(), cols, rows);
            Ok::<_, Error>((snap.text, serde_json::to_string(&snap.tree)?))
        })
        .map_err(|e| to_py(py, &e))
    }

    fn png<'py>(&self, py: Python<'py>, cols: u16, rows: u16) -> PyResult<Bound<'py, PyBytes>> {
        let app = Arc::clone(&self.app);
        let png = py
            .detach(move || headless::png(&mut app.session(), cols, rows))
            .map_err(|e| to_py(py, &e))?;
        Ok(PyBytes::new(py, &png))
    }
}
