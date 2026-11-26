//! `stagetui sessions | attach | start | stop`: apps that keep running in the
//! background while terminal windows come and go.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use stagetui_core::Error;
use stagetui_core::ipc::{self, Ended, Registry, SessionInfo};

use crate::{fail, print_line};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn ago(started: u64) -> String {
    let s = now().saturating_sub(started);
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86_400),
    }
}

/// `stagetui sessions`
pub fn list(json: bool) -> i32 {
    let sessions = ipc::list();
    if json {
        return match serde_json::to_string_pretty(&sessions) {
            Ok(t) => print_line(&t),
            Err(e) => fail(&e.into()),
        };
    }
    if sessions.is_empty() {
        return print_line(
            "No sessions are running.\nStart an app in the background with: stagetui start my_app.py",
        );
    }
    print_line(&table(&sessions))
}

fn table(sessions: &[SessionInfo]) -> String {
    let w = sessions
        .iter()
        .map(|s| s.name.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    let mut out = format!("{:<w$}  {:>7}  {:<12}  APP", "NAME", "PID", "STARTED");
    for s in sessions {
        out.push_str(&format!(
            "\n{:<w$}  {:>7}  {:<12}  {}",
            s.name,
            s.pid,
            ago(s.started),
            s.app
        ));
    }
    out.push_str("\n\nOpen one with: stagetui attach <name>");
    out
}

/// `stagetui attach [name]`
pub fn attach(name: Option<&str>) -> i32 {
    match ipc::attach(name) {
        Ok(ended) => {
            eprintln!("{}", ended.message());
            0
        }
        Err(e) => fail(&e),
    }
}

/// `stagetui stop [name]`
pub fn stop(name: Option<&str>) -> i32 {
    let registry = match Registry::open() {
        Ok(r) => r,
        Err(e) => return fail(&e),
    };
    let info = match registry.stop(name) {
        Ok(i) => i,
        Err(e) => return fail(&e),
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if !registry.list().iter().any(|s| s.name == info.name) {
            eprintln!("✓ stopped \"{}\"", info.name);
            return 0;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    fail(&Error::friendly(
        format!("\"{}\" did not stop", info.name),
        "it was asked to quit but is still running after 10 seconds",
        format!(
            "attach to see why (`stagetui attach {}`), or end process {}",
            info.name, info.pid
        ),
    ))
}

/// Logs of background apps older than three days are removed.
fn prune_logs(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let old = SystemTime::now() - Duration::from_secs(3 * 86_400);
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "log")
            && e.metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| t < old)
        {
            let _ = fs::remove_file(p);
        }
    }
}

fn log_file(path: &Path) -> std::io::Result<fs::File> {
    let mut o = fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

fn tail(path: &Path, lines: usize) -> String {
    let Ok(f) = fs::File::open(path) else {
        return String::new();
    };
    let all: Vec<String> = BufReader::new(f).lines().map_while(Result::ok).collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

fn background(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group: closing this terminal doesn't stop it.
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
}

/// Wait until the app has opened its session (or exited).
fn wait_for_session(
    registry: &Registry,
    child: &mut Child,
    log: &Path,
) -> Result<SessionInfo, Error> {
    let log_s = log.display().to_string();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(info) = registry
            .list()
            .into_iter()
            .find(|s| s.log.as_deref() == Some(log_s.as_str()))
        {
            return Ok(info);
        }
        if let Ok(Some(status)) = child.try_wait() {
            let out = tail(log, 30);
            return Err(Error::friendly(
                "The app ended before it started",
                if out.trim().is_empty() {
                    format!("python exited ({status}) without output")
                } else {
                    format!("its last output:\n{out}")
                },
                "check that the file runs with `python <file>` and calls `MyApp().run()`",
            ));
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err(Error::friendly(
                "The app took too long to start",
                "no session appeared within 30 seconds",
                format!("look at its output in {log_s}"),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// `stagetui start app.py [--name N] [--detached] [-- args…]`
pub fn start(app: &str, name: Option<&str>, detached: bool, args: &[String]) -> i32 {
    if !Path::new(app).is_file() {
        return fail(&Error::friendly(
            format!("App file \"{app}\" was not found"),
            "stagetui start runs a .py file that calls MyApp().run()",
            "check the path, e.g. stagetui start examples/07_harness.py",
        ));
    }
    if let Some(n) = name
        && !ipc::valid_name(n)
    {
        return fail(&Error::friendly(
            format!("\"{n}\" can't be a session name"),
            "names use letters, digits, - and _ (up to 40)",
            "pick a name like --name my-agent",
        ));
    }
    let registry = match Registry::open() {
        Ok(r) => r,
        Err(e) => return fail(&e),
    };
    prune_logs(registry.dir());
    let stem = Path::new(app)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let log: PathBuf = registry.dir().join(format!(
        "{}-{}-{}.log",
        ipc::slug(name.unwrap_or(&stem)),
        now(),
        std::process::id()
    ));
    let (out, err) = match log_file(&log).and_then(|f| Ok((f.try_clone()?, f))) {
        Ok(f) => f,
        Err(e) => return fail(&e.into()),
    };
    let mut cmd = Command::new(crate::verbs::python());
    cmd.arg(app)
        .args(args)
        .env("STAGETUI_SERVE", "1")
        .env("STAGETUI_LOG", &log)
        .env("STAGETUI_NO_PROBE", "1")
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err);
    if let Some(n) = name {
        cmd.env("STAGETUI_SESSION", n);
    }
    background(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return fail(&Error::friendly(
                "Could not start Python for stagetui start",
                e.to_string(),
                "install stagetui in a Python environment (pip install stagetui) and run `stagetui` from it",
            ));
        }
    };
    let info = match wait_for_session(&registry, &mut child, &log) {
        Ok(i) => i,
        Err(e) => return fail(&e),
    };
    if detached {
        eprintln!(
            "✓ \"{}\" is running in the background.\n  Open it with: stagetui attach {}\n  Stop it with: stagetui stop {}",
            info.name, info.name, info.name
        );
        return 0;
    }
    match ipc::attach(Some(&info.name)) {
        Ok(Ended::Detached(n)) => {
            eprintln!("{}", Ended::Detached(n).message());
            0
        }
        Ok(ended) => {
            // The app ended: if it crashed, show why.
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut status = None;
            while status.is_none() && Instant::now() < deadline {
                status = child.try_wait().ok().flatten();
                std::thread::sleep(Duration::from_millis(50));
            }
            match status {
                Some(s) if !s.success() => {
                    eprintln!("The app stopped with an error:\n{}", tail(&log, 30));
                    1
                }
                _ => {
                    eprintln!("{}", ended.message());
                    0
                }
            }
        }
        Err(e) => {
            eprintln!(
                "The app keeps running in the background as \"{}\".",
                info.name
            );
            fail(&e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_ages_and_table() {
        assert_eq!(ago(now()), "just now");
        assert_eq!(ago(now() - 300), "5 min ago");
        assert_eq!(ago(now() - 7200), "2 h ago");
        let t = table(&[SessionInfo {
            name: "harness".into(),
            pid: 42,
            app: "examples/07_harness.py".into(),
            cwd: "/tmp".into(),
            started: now() - 120,
            protocol: ipc::PROTOCOL,
            log: None,
        }]);
        assert!(t.starts_with("NAME"), "{t}");
        assert!(t.contains("harness") && t.contains("2 min ago") && t.contains("stagetui attach"));
    }
}
