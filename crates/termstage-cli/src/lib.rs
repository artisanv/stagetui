//! The `termstage` CLI. Shared by the standalone binary and the Python entry point
//! (`termstage._core.cli_main`), so both behave identically.

mod doctor;
mod verbs;

use std::ffi::OsString;
use std::io::Write;

use clap::{Parser, Subcommand};
use termstage_core::caps::{self, DetectOptions};

pub use doctor::report as doctor_report;
pub use verbs::set_python;

#[derive(Parser)]
#[command(
    name = "termstage",
    version = termstage_core::VERSION,
    about = "Rust-first TUI engine for agent harnesses and humans.",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print terminal capabilities as JSON (for agents and bug reports).
    Inspect {
        /// Accepted for parity with the spec; output is always JSON.
        #[arg(long)]
        json: bool,
        /// Single-line JSON.
        #[arg(long)]
        compact: bool,
        #[command(flatten)]
        detect: DetectArgs,
    },
    /// Diagnose this terminal in plain English, with fixes.
    Doctor {
        #[command(flatten)]
        detect: DetectArgs,
    },
    /// Show an image or a text card right here in the terminal.
    ///
    /// Examples:
    ///   termstage show logo.png
    ///   termstage show "مرحبا بكم" --title "Arabic"
    Show {
        /// An image file (PNG, JPEG, GIF, WebP, BMP, SVG) or any text.
        content: String,
        #[command(flatten)]
        look: verbs::Look,
        /// Width in terminal cells.
        #[arg(long)]
        width: Option<u16>,
        /// Height in terminal cells (images).
        #[arg(long)]
        height: Option<u16>,
        /// How an image fills its box: contain, cover or stretch.
        #[arg(long)]
        fit: Option<String>,
        #[command(flatten)]
        detect: DetectArgs,
    },
    /// Render text, an image, or a termstage app to PNG / text / JSON — no terminal needed.
    ///
    /// Examples:
    ///   termstage render "Hello" --out hello.png
    ///   termstage render my_app.py --size 100x30 --out app.png --text app.txt --tree app.json
    ///   termstage render logo.svg --out logo.png
    Render {
        /// A `.py` file with a StageApp, an image file, or any text.
        target: String,
        /// Write a PNG here.
        #[arg(long)]
        out: Option<String>,
        /// Write the screen as plain text here.
        #[arg(long)]
        text: Option<String>,
        /// Write the JSON widget tree here.
        #[arg(long)]
        tree: Option<String>,
        /// Screen size in cells, like 100x30 (default: fit the content).
        #[arg(long)]
        size: Option<String>,
        /// Pixel size of one cell for PNGs, like 16x32 (bigger = sharper, larger file).
        #[arg(long, default_value = "16x32")]
        cell: String,
        /// Accepted for parity with the spec; render never needs a terminal.
        #[arg(long)]
        headless: bool,
        #[command(flatten)]
        look: verbs::Look,
    },
    /// Let a program (an AI agent, a test) control a termstage app: JSON-RPC 2.0,
    /// one message per line on stdin/stdout.
    ///
    /// Methods: snapshot, press, type, click, scroll, resize, wait, wait_idle, query,
    /// panes, set_pane_type, shutdown.
    ///
    /// Example:
    ///   echo '{"jsonrpc":"2.0","id":1,"method":"snapshot"}' | termstage drive my_app.py
    Drive {
        /// A `.py` file with a StageApp.
        app: String,
        /// Screen size in cells.
        #[arg(long, default_value = "100x30")]
        size: String,
    },
    /// Serve a termstage app as MCP tools (stdio), so coding agents such as Claude Code
    /// can see and operate it.
    ///
    /// Example (Claude Code):
    ///   claude mcp add my-app -- termstage mcp my_app.py
    Mcp {
        /// A `.py` file with a StageApp.
        app: String,
        /// Screen size in cells.
        #[arg(long, default_value = "100x30")]
        size: String,
    },
    /// Reaper sidecar (internal).
    #[command(name = "__reaper", hide = true)]
    Reaper,
    /// Spawn a tracked `sleep` child, print its pid, then wait forever (internal, for tests).
    #[command(name = "__reaper-selftest", hide = true)]
    ReaperSelftest {
        /// Also create and track a POSIX shared-memory object with this name.
        #[arg(long)]
        shm: Option<String>,
    },
}

#[derive(clap::Args)]
struct DetectArgs {
    /// Skip terminal probing and report headless capabilities.
    #[arg(long)]
    headless: bool,
    /// Do not send escape-sequence queries (environment hints only).
    #[arg(long)]
    no_probe: bool,
}

impl DetectArgs {
    fn options(&self) -> DetectOptions {
        DetectOptions {
            force_headless: self.headless,
            probe: !self.no_probe,
            ..Default::default()
        }
    }
}

/// Run the CLI with `args` (including argv\[0\]). Returns the process exit code.
pub fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return e.exit_code();
        }
    };
    match cli.command {
        Command::Inspect {
            json: _,
            compact,
            detect,
        } => {
            let caps = caps::detect(&detect.options());
            let text = if compact {
                serde_json::to_string(&caps)
            } else {
                serde_json::to_string_pretty(&caps)
            };
            match text {
                Ok(t) => print_line(&t),
                Err(e) => fail(&termstage_core::Error::from(e)),
            }
        }
        Command::Doctor { detect } => {
            let caps = caps::detect(&detect.options());
            let fonts = (!caps.headless).then(termstage_core::text::fonts::coverage);
            print_line(&doctor::report(&caps, fonts.as_ref()))
        }
        Command::Show {
            content,
            look,
            width,
            height,
            fit,
            detect,
        } => {
            let caps = caps::detect(&detect.options());
            verbs::show(&content, &look, width, height, fit, &caps)
        }
        Command::Render {
            target,
            out,
            text,
            tree,
            size,
            cell,
            headless: _,
            look,
        } => verbs::render(&verbs::RenderArgs {
            target,
            out,
            text,
            tree,
            size,
            cell,
            look,
        }),
        Command::Drive { app, size } => verbs::agent("drive", &app, &size),
        Command::Mcp { app, size } => verbs::agent("mcp", &app, &size),
        Command::Reaper => termstage_core::reaper::sidecar_main(),
        Command::ReaperSelftest { shm } => reaper_selftest(shm.as_deref()),
    }
}

pub(crate) fn print_line(s: &str) -> i32 {
    let mut out = std::io::stdout().lock();
    // A closed pipe (e.g. `| head`) is not an error worth reporting.
    let _ = writeln!(out, "{s}");
    let _ = out.flush();
    0
}

pub(crate) fn fail(e: &termstage_core::Error) -> i32 {
    eprintln!("termstage: {e}");
    1
}

fn reaper_selftest(shm: Option<&str>) -> i32 {
    use std::process::Command as Cmd;
    let reaper = termstage_core::reaper::global();
    #[cfg(windows)]
    let cmd = {
        let mut c = Cmd::new("ping");
        c.args(["-n", "600", "127.0.0.1"])
            .stdout(std::process::Stdio::null());
        c
    };
    #[cfg(not(windows))]
    let cmd = {
        let mut c = Cmd::new("sleep");
        c.arg("600");
        c
    };
    let child = match reaper.spawn(cmd) {
        Ok(c) => c,
        Err(e) => return fail(&e.into()),
    };
    if let Some(name) = shm
        && let Err(e) =
            termstage_core::reaper::create_shm(name).and_then(|()| reaper.track_shm(name))
    {
        return fail(&e.into());
    }
    println!("pid {}", child.id());
    let _ = std::io::stdout().flush();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
