//! Platform-specific code. This is the **only** place `unsafe` is allowed;
//! every block carries a `SAFETY:` comment.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod windows;

use std::process::{Child, Command};

/// Spawn `cmd` with the platform's "die with the parent" hardening applied:
/// - Unix: own process group (so the whole tree can be killed with `killpg`).
/// - Linux: `PR_SET_PDEATHSIG = SIGKILL`.
/// - Windows: the current process is placed in a kill-on-close Job Object first,
///   so every child (and grandchild) dies when we do.
pub fn spawn_hardened(mut cmd: Command) -> std::io::Result<Child> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(target_os = "linux")]
    linux::set_parent_death_signal(&mut cmd);
    #[cfg(windows)]
    windows::ensure_kill_on_close_job()?;
    cmd.spawn()
}

/// Prepare the console for termstage output (Windows: VT processing + UTF-8 code pages).
/// Returns a guard that restores the previous state. No-op elsewhere.
pub fn console_setup() -> ConsoleGuard {
    #[cfg(windows)]
    {
        ConsoleGuard(windows::ConsoleState::enter())
    }
    #[cfg(not(windows))]
    {
        ConsoleGuard(())
    }
}

/// Restores console state on drop (see [`console_setup`]).
#[allow(dead_code)] // the field is held only for its `Drop`
pub struct ConsoleGuard(
    #[cfg(windows)] Option<windows::ConsoleState>,
    #[cfg(not(windows))] (),
);
