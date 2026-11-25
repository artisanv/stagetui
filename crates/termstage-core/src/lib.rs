//! # termstage-core
//!
//! The Rust engine behind `termstage`. Everything that matters lives here:
//! terminal capability probing, the RAII terminal guard, the Session/View split,
//! the cell diff renderer, headless snapshots and the zero-zombie process reaper.
//!
//! Python (`termstage._core`) and the CLI (`termstage`) are thin layers on top.
//!
//! Unsafe code is denied crate-wide and only allowed inside [`os`], where each
//! block documents why it is sound.

pub mod caps;
pub mod error;
pub mod frame;
pub mod gfx;
pub mod headless;
pub mod inline;
pub mod layout;
pub mod os;
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

pub use error::{Error, Result};

/// The termstage version (shared by the core, CLI and Python package).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
