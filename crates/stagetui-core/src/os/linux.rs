//! Linux-only hardening.
#![allow(unsafe_code)]

use std::io;
use std::os::unix::process::CommandExt;
use std::process::Command;

use rustix::process::{Signal, getpid, getppid, set_parent_process_death_signal};

/// Ask the kernel to SIGKILL the child if its parent dies.
///
/// Note: `PDEATHSIG` fires when the *thread* that forked exits, so stagetui
/// spawns from a dedicated, process-lifetime spawner thread (see `reaper`).
pub fn set_parent_death_signal(cmd: &mut Command) {
    let parent = getpid();
    // SAFETY: the closure runs between fork and exec. It only issues the
    // `prctl` and `getppid` syscalls through rustix (async-signal-safe, no
    // allocation, no locks), so it cannot deadlock or corrupt state.
    unsafe {
        cmd.pre_exec(move || {
            set_parent_process_death_signal(Some(Signal::KILL)).map_err(io::Error::from)?;
            // The parent may have died before prctl took effect.
            if getppid() != Some(parent) {
                return Err(io::Error::other("parent exited before the child started"));
            }
            Ok(())
        });
    }
}
