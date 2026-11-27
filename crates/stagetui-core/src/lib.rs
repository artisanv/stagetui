//! # stagetui-core
//!
//! The Rust engine behind `stagetui`. Everything that matters lives here:
//! terminal capability probing, the RAII terminal guard, the Session/View split,
//! the cell diff renderer, headless snapshots and the zero-zombie process reaper.
//!
//! Python (`stagetui._core`) and the CLI (`stagetui`) are thin layers on top.
//!
//! Unsafe code is denied crate-wide and only allowed inside [`os`], where each
//! block documents why it is sound.

pub mod app;
pub mod caps;
pub mod error;
pub mod frame;
pub mod gfx;
pub mod headless;
pub mod inline;
pub mod input;
pub mod ipc;
pub mod launch;
pub mod layout;
pub mod motion;
pub mod os;
pub mod panes;
pub mod plugins;
pub mod reaper;
pub mod render;
pub mod runtime;
pub mod session;
pub mod style;
pub mod term;
pub mod text;
pub mod tree;
pub mod view;
pub mod widgets;
pub mod workspace;

pub use error::{Error, Result};

/// The stagetui version (shared by the core, CLI and Python package).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The version as Python writes it (`0.1.0-alpha.1` → `0.1.0a1`, what pip shows).
pub fn version() -> String {
    VERSION
        .replace("-alpha.", "a")
        .replace("-beta.", "b")
        .replace("-rc.", "rc")
}
