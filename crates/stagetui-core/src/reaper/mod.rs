//! Zero-zombie process & resource reaper.
//!
//! Three layers make sure nothing stagetui starts outlives it:
//! 1. **RAII** — [`TrackedChild`] kills its process group on drop.
//! 2. **OS hardening** — own process group, Linux `PDEATHSIG`, Windows kill-on-close Job
//!    Object (see [`crate::os::spawn_hardened`]).
//! 3. **Reaper sidecar** (Unix) — a tiny detached process connected by a pipe. When the
//!    pipe closes (normal exit, crash, or `SIGKILL`), it kills every tracked process
//!    group and unlinks every tracked shared-memory name.

mod protocol;

pub use protocol::Msg;

use std::io;
#[cfg(unix)]
use std::io::Write;
use std::process::{Child, Command};
#[cfg(unix)]
use std::process::{ChildStdin, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

/// How to launch the sidecar (`argv`). The Rust CLI uses `[current_exe, "__reaper"]`;
/// the Python extension sets `[sys.executable, "-c", "..."]` at import time.
fn default_launcher() -> Vec<String> {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "stagetui".into());
    vec![exe, "__reaper".into()]
}

struct SpawnReq {
    cmd: Command,
    reply: mpsc::Sender<io::Result<Child>>,
}

#[cfg(unix)]
struct Sidecar {
    child: Child,
    stdin: ChildStdin,
}

/// Windows needs no sidecar (kill-on-close Job Object), but keeps the same shape.
#[cfg(not(unix))]
struct Sidecar;

/// The process-wide reaper. Use [`global`].
pub struct Reaper {
    launcher: Mutex<Vec<String>>,
    sidecar: Mutex<Option<Sidecar>>,
    spawner: Mutex<Option<mpsc::Sender<SpawnReq>>>,
}

/// The process-wide reaper instance.
pub fn global() -> &'static Reaper {
    static R: OnceLock<Reaper> = OnceLock::new();
    R.get_or_init(|| Reaper {
        launcher: Mutex::new(default_launcher()),
        sidecar: Mutex::new(None),
        spawner: Mutex::new(None),
    })
}

impl Reaper {
    /// Override how the sidecar is launched.
    pub fn set_launcher(&self, argv: Vec<String>) {
        if !argv.is_empty() {
            *self.launcher.lock().unwrap_or_else(|e| e.into_inner()) = argv;
        }
    }

    /// Spawn `cmd` hardened and tracked. The child dies when the returned handle drops,
    /// and also if this process dies unexpectedly.
    pub fn spawn(&self, cmd: Command) -> io::Result<TrackedChild> {
        let child = self.spawn_on_spawner_thread(cmd)?;
        let pid = child.id();
        // Best effort: if the sidecar cannot start, layers 1–2 still apply.
        let _ = self.send(&Msg::Pid(pid));
        Ok(TrackedChild { child: Some(child) })
    }

    /// Track a process (and its process group) started elsewhere — e.g. a
    /// program in a pseudo-terminal — so it is killed if we die.
    pub fn track_pid(&self, pid: u32) -> io::Result<()> {
        self.send(&Msg::Pid(pid))
    }

    /// Stop tracking a process that exited normally.
    pub fn untrack_pid(&self, pid: u32) -> io::Result<()> {
        self.send(&Msg::Unpid(pid))
    }

    /// Track a POSIX shared-memory name so it is unlinked if we die.
    pub fn track_shm(&self, name: &str) -> io::Result<()> {
        self.send(&Msg::Shm(name.to_string()))
    }

    /// Stop tracking a shared-memory name (it was consumed or unlinked normally).
    pub fn untrack_shm(&self, name: &str) -> io::Result<()> {
        self.send(&Msg::Unshm(name.to_string()))
    }

    /// `PDEATHSIG` is tied to the forking *thread*, so all spawns happen on one
    /// process-lifetime thread.
    fn spawn_on_spawner_thread(&self, cmd: Command) -> io::Result<Child> {
        let tx = {
            let mut guard = self.spawner.lock().unwrap_or_else(|e| e.into_inner());
            if guard.is_none() {
                let (tx, rx) = mpsc::channel::<SpawnReq>();
                std::thread::Builder::new()
                    .name("stagetui-spawner".into())
                    .spawn(move || {
                        for req in rx {
                            let _ = req.reply.send(crate::os::spawn_hardened(req.cmd));
                        }
                    })?;
                *guard = Some(tx);
            }
            guard
                .clone()
                .ok_or_else(|| io::Error::other("spawner thread unavailable"))?
        };
        let (reply, rx) = mpsc::channel();
        tx.send(SpawnReq { cmd, reply })
            .map_err(|_| io::Error::other("spawner thread stopped"))?;
        rx.recv()
            .map_err(|_| io::Error::other("spawner thread stopped"))?
    }

    #[cfg(unix)]
    fn send(&self, msg: &Msg) -> io::Result<()> {
        let mut guard = self.sidecar.lock().unwrap_or_else(|e| e.into_inner());
        for _attempt in 0..2 {
            if guard
                .as_mut()
                .is_some_and(|s| matches!(s.child.try_wait(), Ok(None)))
            {
                let sidecar = guard
                    .as_mut()
                    .ok_or_else(|| io::Error::other("no sidecar"))?;
                let line = msg.encode();
                if sidecar
                    .stdin
                    .write_all(line.as_bytes())
                    .and_then(|()| sidecar.stdin.flush())
                    .is_ok()
                {
                    return Ok(());
                }
            }
            *guard = Some(self.start_sidecar()?);
        }
        Err(io::Error::other("reaper sidecar is not accepting messages"))
    }

    #[cfg(not(unix))]
    fn send(&self, _msg: &Msg) -> io::Result<()> {
        // Windows relies on the kill-on-close Job Object; named mappings vanish with their handles.
        let _ = &self.sidecar;
        Ok(())
    }

    #[cfg(unix)]
    fn start_sidecar(&self) -> io::Result<Sidecar> {
        let argv = self
            .launcher
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let (prog, args) = argv
            .split_first()
            .ok_or_else(|| io::Error::other("empty reaper launcher"))?;
        let mut child = Command::new(prog)
            .args(args)
            .env("STAGETUI_REAPER", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("reaper sidecar has no stdin"))?;
        Ok(Sidecar { child, stdin })
    }
}

/// A child process that is killed (with its whole process group) when dropped.
#[derive(Debug)]
pub struct TrackedChild {
    child: Option<Child>,
}

impl TrackedChild {
    /// OS process id.
    pub fn id(&self) -> u32 {
        self.child.as_ref().map_or(0, Child::id)
    }

    /// Access the underlying child (e.g. for its pipes).
    pub fn child_mut(&mut self) -> Option<&mut Child> {
        self.child.as_mut()
    }

    /// Kill the process (group) now and wait for it.
    pub fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let pid = child.id();
            #[cfg(unix)]
            if let Some(p) = rustix::process::Pid::from_raw(pid as i32) {
                let _ = rustix::process::kill_process_group(p, rustix::process::Signal::KILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            let _ = global().send(&Msg::Unpid(pid));
        }
    }
}

impl Drop for TrackedChild {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Create an empty POSIX shared-memory object (`0600`). Used by the graphics
/// transport in Milestone 2 and by the reaper self-test today.
pub fn create_shm(name: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        use rustix::fs::Mode;
        use rustix::shm::OFlags;
        rustix::shm::open(
            name,
            OFlags::CREATE | OFlags::EXCL | OFlags::RDWR,
            Mode::RUSR | Mode::WUSR,
        )?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = name;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "POSIX shared memory is Unix-only",
        ))
    }
}

/// Unlink a POSIX shared-memory object.
pub fn unlink_shm(name: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        rustix::shm::unlink(name)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = name;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "POSIX shared memory is Unix-only",
        ))
    }
}

/// Entry point of the sidecar process: read messages until EOF, then clean up.
/// Returns the process exit code.
pub fn sidecar_main() -> i32 {
    #[cfg(unix)]
    {
        use std::collections::BTreeSet;
        use std::io::BufRead;

        // Leave the terminal's session so Ctrl+C / SIGHUP aimed at the app do not hit us.
        let _ = rustix::process::setsid();
        let mut pids = BTreeSet::new();
        let mut shms = BTreeSet::new();
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            match Msg::decode(&line) {
                Some(Msg::Pid(p)) => {
                    pids.insert(p);
                }
                Some(Msg::Unpid(p)) => {
                    pids.remove(&p);
                }
                Some(Msg::Shm(n)) => {
                    shms.insert(n);
                }
                Some(Msg::Unshm(n)) => {
                    shms.remove(&n);
                }
                None => {}
            }
        }
        cleanup(&pids, &shms);
        0
    }
    #[cfg(not(unix))]
    {
        0
    }
}

#[cfg(unix)]
fn cleanup(pids: &std::collections::BTreeSet<u32>, shms: &std::collections::BTreeSet<String>) {
    use rustix::process::{Pid, Signal, kill_process, kill_process_group, test_kill_process};
    use std::time::{Duration, Instant};

    let targets: Vec<Pid> = pids
        .iter()
        .filter_map(|&p| Pid::from_raw(p as i32))
        .collect();
    for &p in &targets {
        let _ = kill_process_group(p, Signal::TERM);
        let _ = kill_process(p, Signal::TERM);
    }
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline && targets.iter().any(|&p| test_kill_process(p).is_ok()) {
        std::thread::sleep(Duration::from_millis(20));
    }
    for &p in &targets {
        let _ = kill_process_group(p, Signal::KILL);
        let _ = kill_process(p, Signal::KILL);
    }
    for name in shms {
        let _ = rustix::shm::unlink(name.as_str());
    }
}
