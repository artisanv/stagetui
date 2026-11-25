//! `termstage._core`: the thin PyO3 layer. No engine logic lives here.
//!
//! Every long-running call releases the GIL (`Python::detach`), so Python
//! threads and asyncio keep running while Rust renders.

mod app;
mod canvas;

use std::sync::{Mutex, OnceLock};

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use termstage_core::Error;
use termstage_core::caps::{self, Capabilities, DetectOptions};
use termstage_core::gfx::{self, load};
use termstage_core::inline::{self, ShowOptions};
use termstage_core::reaper::{self, TrackedChild};

create_exception!(
    termstage._core,
    TermstageError,
    PyException,
    "A termstage error with `what`, `why` and `fix` attributes."
);

/// Children spawned through `_spawn_tracked` stay alive until the interpreter exits.
static CHILDREN: Mutex<Vec<TrackedChild>> = Mutex::new(Vec::new());

pub(crate) fn to_py(py: Python<'_>, e: &Error) -> PyErr {
    let (what, why, fix) = e.parts();
    let err = TermstageError::new_err(e.to_string());
    let value = err.value(py);
    let _ = value.setattr("what", what);
    let _ = value.setattr("why", why);
    let _ = value.setattr("fix", fix);
    err
}

/// Capabilities of the terminal `show()` prints to (probed once per process).
fn show_caps() -> &'static Capabilities {
    static CAPS: OnceLock<Capabilities> = OnceLock::new();
    CAPS.get_or_init(|| caps::detect(&DetectOptions::default()))
}

/// Detect terminal capabilities. Returns JSON (wrapped into a dict by `termstage.inspect`).
#[pyfunction]
#[pyo3(signature = (*, headless = false, probe = true))]
fn inspect(py: Python<'_>, headless: bool, probe: bool) -> PyResult<String> {
    let opts = DetectOptions {
        force_headless: headless,
        probe,
        ..Default::default()
    };
    let caps = py.detach(|| caps::detect(&opts));
    serde_json::to_string(&caps).map_err(|e| to_py(py, &e.into()))
}

/// Font coverage report (JSON).
#[pyfunction]
fn fonts(py: Python<'_>) -> PyResult<String> {
    let c = py.detach(termstage_core::text::fonts::coverage);
    serde_json::to_string(&c).map_err(|e| to_py(py, &e.into()))
}

/// Keep image bytes in memory; returns a `mem:<id>` source for StageImage.
#[pyfunction]
#[pyo3(signature = (data, name = "image"))]
fn register_image(py: Python<'_>, data: &[u8], name: &str) -> PyResult<String> {
    let image = load::from_bytes(data, name).map_err(|e| to_py(py, &e))?;
    Ok(gfx::register(image))
}

/// Build the escape sequences that display `content` inline. Python writes them to `sys.stdout`.
#[pyfunction]
#[pyo3(signature = (content, *, title=None, icon=None, dir=None, theme=None, accent=None, width=None, height=None, fit=None))]
#[allow(clippy::too_many_arguments)]
fn show(
    py: Python<'_>,
    content: &Bound<'_, PyAny>,
    title: Option<String>,
    icon: Option<String>,
    dir: Option<String>,
    theme: Option<String>,
    accent: Option<String>,
    width: Option<u16>,
    height: Option<u16>,
    fit: Option<String>,
) -> PyResult<String> {
    let opts = ShowOptions {
        title,
        icon,
        dir,
        theme,
        accent,
        width,
        height,
        fit,
    };
    if let Ok(c) = content.cast::<canvas::PyCanvas>() {
        let image = c.get().snapshot("canvas");
        return py
            .detach(|| inline::show_image(&image, &opts, show_caps()))
            .map_err(|e| to_py(py, &e));
    }
    if let Ok(b) = content.cast::<PyBytes>() {
        let image = load::from_bytes(b.as_bytes(), "image").map_err(|e| to_py(py, &e))?;
        return py
            .detach(|| inline::show_image(&image, &opts, show_caps()))
            .map_err(|e| to_py(py, &e));
    }
    let text: String = content.str()?.extract()?;
    py.detach(|| inline::show(&text, &opts, show_caps()))
        .map_err(|e| to_py(py, &e))
}

/// Entry point of the `termstage` console script.
#[pyfunction]
fn cli_main(py: Python<'_>) -> PyResult<i32> {
    let argv: Vec<String> = py.import("sys")?.getattr("argv")?.extract()?;
    Ok(py.detach(move || termstage_cli::run(argv)))
}

/// Spawn a tracked child process (internal; used by tests and future plugins). Returns its pid.
#[pyfunction]
fn _spawn_tracked(py: Python<'_>, argv: Vec<String>) -> PyResult<u32> {
    let (prog, args) = argv.split_first().ok_or_else(|| {
        to_py(
            py,
            &Error::friendly("Nothing to spawn", "argv is empty", "pass a program name"),
        )
    })?;
    let mut cmd = std::process::Command::new(prog);
    cmd.args(args);
    let child = py
        .detach(|| reaper::global().spawn(cmd))
        .map_err(|e| to_py(py, &e.into()))?;
    let pid = child.id();
    CHILDREN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(child);
    Ok(pid)
}

/// Run the reaper sidecar loop (internal).
#[pyfunction]
fn _reaper_main(py: Python<'_>) -> i32 {
    py.detach(reaper::sidecar_main)
}

#[pymodule(gil_used = false)]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    // Inside Python, helper processes are launched through the interpreter itself.
    if let Ok(exe) = py
        .import("sys")
        .and_then(|s| s.getattr("executable"))
        .and_then(|e| e.extract::<String>())
        && !exe.is_empty()
    {
        termstage_cli::set_python(exe.clone());
        reaper::global().set_launcher(vec![
            exe,
            "-I".into(),
            "-c".into(),
            "import sys, termstage._core as c; sys.exit(c._reaper_main())".into(),
        ]);
    }
    m.add("__version__", termstage_core::VERSION)?;
    m.add("TermstageError", py.get_type::<TermstageError>())?;
    m.add_class::<canvas::PyCanvas>()?;
    m.add_class::<app::PyApp>()?;
    m.add_function(wrap_pyfunction!(inspect, m)?)?;
    m.add_function(wrap_pyfunction!(fonts, m)?)?;
    m.add_function(wrap_pyfunction!(register_image, m)?)?;
    m.add_function(wrap_pyfunction!(show, m)?)?;
    m.add_function(wrap_pyfunction!(cli_main, m)?)?;
    m.add_function(wrap_pyfunction!(_spawn_tracked, m)?)?;
    m.add_function(wrap_pyfunction!(_reaper_main, m)?)?;
    Ok(())
}
