//! `termstage show` and `termstage render`.

use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;

use termstage_core::caps::{Capabilities, parse_size};
use termstage_core::gfx::{self, Fit, load};
use termstage_core::inline::{self, ShowOptions};
use termstage_core::{Error, Result, headless};

use crate::{fail, print_line};

static PYTHON: OnceLock<String> = OnceLock::new();

/// Which Python runs `.py` apps for `termstage render app.py`.
/// The Python package sets this to `sys.executable`; the standalone binary uses `python3`.
pub fn set_python(exe: String) {
    let _ = PYTHON.set(exe);
}

fn python() -> String {
    PYTHON.get().cloned().unwrap_or_else(|| {
        if cfg!(windows) {
            "python".into()
        } else {
            "python3".into()
        }
    })
}

/// Look-and-feel flags shared by `show` and `render`.
#[derive(clap::Args, Debug, Clone, Default)]
pub struct Look {
    /// Card title.
    #[arg(long)]
    pub title: Option<String>,
    /// Icon name (e.g. sparkles, check, globe).
    #[arg(long)]
    pub icon: Option<String>,
    /// Text direction: auto, ltr or rtl.
    #[arg(long)]
    pub dir: Option<String>,
    /// Theme: aurora-dark, nordic-light, velvet-dusk, paper-soft, hacker-mint.
    #[arg(long)]
    pub theme: Option<String>,
    /// Accent color: blue, mint, amber, rose, violet or #rrggbb.
    #[arg(long)]
    pub accent: Option<String>,
}

impl Look {
    fn options(&self) -> ShowOptions {
        ShowOptions {
            title: self.title.clone(),
            icon: self.icon.clone(),
            dir: self.dir.clone(),
            theme: self.theme.clone(),
            accent: self.accent.clone(),
            ..ShowOptions::default()
        }
    }
}

pub fn show(
    content: &str,
    look: &Look,
    width: Option<u16>,
    height: Option<u16>,
    fit: Option<String>,
    caps: &Capabilities,
) -> i32 {
    let opts = ShowOptions {
        width,
        height,
        fit,
        ..look.options()
    };
    match inline::show(content, &opts, caps) {
        Ok(out) => {
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(out.as_bytes());
            let _ = stdout.flush();
            0
        }
        Err(e) => fail(&e),
    }
}

pub struct RenderArgs {
    pub target: String,
    pub out: Option<String>,
    pub text: Option<String>,
    pub tree: Option<String>,
    pub size: Option<String>,
    pub cell: String,
    pub look: Look,
}

fn size_arg(s: &str, what: &str, example: &str) -> Result<(u16, u16)> {
    parse_size(s).ok_or_else(|| {
        Error::friendly(
            format!("Can't read {what} \"{s}\""),
            "sizes are written as WIDTHxHEIGHT",
            format!("use something like {example}"),
        )
    })
}

fn write(path: &str, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).map_err(|e| {
        Error::friendly(
            format!("Could not write \"{path}\""),
            e.to_string(),
            "check the folder exists and you can write to it",
        )
    })?;
    let full = std::fs::canonicalize(path).unwrap_or_else(|_| path.into());
    eprintln!("✓ saved {}", full.display());
    Ok(())
}

pub fn render(args: &RenderArgs) -> i32 {
    match render_inner(args) {
        Ok(code) => code,
        Err(e) => fail(&e),
    }
}

fn render_inner(a: &RenderArgs) -> Result<i32> {
    let cell = size_arg(&a.cell, "--cell", "--cell 10x20")?;
    let cell = (u32::from(cell.0), u32::from(cell.1));
    let target = Path::new(&a.target);

    // 1. A Python app: hand over to Python, which owns the app's code.
    if a.target.ends_with(".py") {
        if !target.is_file() {
            return Err(Error::friendly(
                format!("App file \"{}\" was not found", a.target),
                "termstage render needs a .py file that defines a StageApp",
                "check the path, e.g. termstage render examples/01_hello.py",
            ));
        }
        let mut cmd = std::process::Command::new(python());
        cmd.args(["-m", "termstage._render", &a.target, "--cell", &a.cell]);
        for (flag, v) in [
            ("--out", &a.out),
            ("--text", &a.text),
            ("--tree", &a.tree),
            ("--size", &a.size),
        ] {
            if let Some(v) = v {
                cmd.args([flag, v]);
            }
        }
        let status = cmd.status().map_err(|e| {
            Error::friendly(
                "Could not start Python to render the app",
                e.to_string(),
                "install termstage in a Python environment (pip install termstage) and run `termstage` from it",
            )
        })?;
        return Ok(status.code().unwrap_or(1));
    }

    // 2. An image file: rasterize (handy for SVG → PNG).
    if load::is_image_path(target) && target.is_file() {
        let image = load::from_path(target)?;
        let (w, h) = match &a.size {
            Some(s) => {
                let (w, h) = size_arg(s, "--size", "--size 800x600 (pixels for images)")?;
                (u32::from(w), u32::from(h))
            }
            None => (image.width.max(1), image.height.max(1)),
        };
        if let Some(out) = &a.out {
            write(out, &gfx::png(&image.render(w, h, Fit::Contain))?)?;
        }
        let label = format!("[image: {} {}×{}]", image.name, image.width, image.height);
        if let Some(t) = &a.text {
            write(t, label.as_bytes())?;
        }
        if a.out.is_none() && a.text.is_none() {
            print_line(&label);
        }
        return Ok(0);
    }

    // 3. Text: a card.
    let opts = a.look.options();
    let frame = match &a.size {
        Some(s) => {
            let (c, r) = size_arg(s, "--size", "--size 80x24")?;
            let mut session = card_app(&a.target, &opts)?;
            session.set_cell_px(cell);
            return outputs(a, &mut session, c, r);
        }
        None => inline::card_frame(&a.target, &opts, 80, cell)?,
    };
    // Fit the canvas to the card plus a little breathing room.
    let (c, r) = (frame.area().width + 4, frame.area().height + 2);
    let mut session = card_app(&a.target, &opts)?;
    session.set_cell_px(cell);
    session.set_margin(0);
    outputs(a, &mut session, c, r)
}

fn card_app(text: &str, opts: &ShowOptions) -> Result<termstage_core::session::Session> {
    let spec = serde_json::json!({
        "theme": opts.theme.clone().unwrap_or_else(|| "aurora-dark".into()),
        "accent": opts.accent,
        "children": [{
            "type": "card",
            "content": text,
            "props": { "id": "card", "title": opts.title, "icon": opts.icon, "dir": opts.dir }
        }]
    });
    let mut v = spec;
    strip_nulls(&mut v);
    termstage_core::session::Session::from_json(&v.to_string())
}

fn strip_nulls(v: &mut serde_json::Value) {
    if let serde_json::Value::Object(map) = v {
        map.retain(|_, x| !x.is_null());
        for x in map.values_mut() {
            strip_nulls(x);
        }
    } else if let serde_json::Value::Array(items) = v {
        for x in items {
            strip_nulls(x);
        }
    }
}

fn outputs(
    a: &RenderArgs,
    session: &mut termstage_core::session::Session,
    cols: u16,
    rows: u16,
) -> Result<i32> {
    let snap = headless::snapshot(session, cols, rows);
    if let Some(out) = &a.out {
        write(out, &headless::png(session, cols, rows)?)?;
    }
    if let Some(t) = &a.text {
        write(t, snap.text.as_bytes())?;
    }
    if let Some(t) = &a.tree {
        write(t, serde_json::to_string_pretty(&snap.tree)?.as_bytes())?;
    }
    if a.out.is_none() && a.text.is_none() && a.tree.is_none() {
        print_line(&snap.text);
    }
    Ok(0)
}
