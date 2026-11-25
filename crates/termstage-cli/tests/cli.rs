//! End-to-end tests of the `termstage` binary.

use std::process::{Command, Stdio};

fn termstage() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_termstage"));
    c.env_remove("TERMSTAGE_HEADLESS").stdin(Stdio::null());
    c
}

#[test]
fn inspect_headless_is_json() {
    let out = termstage()
        .args(["inspect", "--json", "--headless"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["headless"], true);
    assert_eq!(v["graphics"]["best"], "halfblocks");
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["probe"]["performed"], false);
}

#[test]
fn no_tty_means_headless() {
    // stdin is /dev/null and stdout is a pipe: no TTY at all.
    let out = termstage().args(["inspect", "--compact"]).output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(text.trim().lines().count(), 1);
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["headless"], true);
    assert!(v["headless_reason"].as_str().unwrap().contains("TTY"));
}

#[test]
fn headless_size_from_env() {
    let out = termstage()
        .args(["inspect", "--headless"])
        .env("TERMSTAGE_SIZE", "120x40")
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        (v["size"]["cols"].as_u64(), v["size"]["rows"].as_u64()),
        (Some(120), Some(40))
    );
}

#[test]
fn doctor_headless() {
    let out = termstage().args(["doctor", "--headless"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("termstage doctor") && text.contains("headless"),
        "{text}"
    );
}

#[test]
fn version_and_bad_usage() {
    let out = termstage().arg("--version").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains(env!("CARGO_PKG_VERSION")));
    let bad = termstage().arg("frobnicate").output().unwrap();
    assert_eq!(bad.status.code(), Some(2));
}

#[cfg(unix)]
#[test]
fn reaper_kills_children_and_unlinks_shm_after_sigkill() {
    use std::io::{BufRead, BufReader};
    use std::time::{Duration, Instant};

    let shm = format!("/tsg-test-{}", std::process::id());
    let _ = termstage_core::reaper::unlink_shm(&shm);
    let mut parent = termstage()
        .args(["__reaper-selftest", "--shm", &shm])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(parent.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let child_pid: u32 = line
        .trim()
        .strip_prefix("pid ")
        .expect(&line)
        .parse()
        .unwrap();
    assert!(alive(child_pid), "child should be running");

    parent.kill().unwrap(); // SIGKILL: no destructors, no cleanup code runs in the parent.
    parent.wait().unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    while alive(child_pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !alive(child_pid),
        "child {child_pid} survived its parent's SIGKILL"
    );

    // The shm name must have been unlinked: creating it exclusively succeeds again.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut unlinked = false;
    while Instant::now() < deadline {
        if termstage_core::reaper::create_shm(&shm).is_ok() {
            unlinked = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = termstage_core::reaper::unlink_shm(&shm);
    assert!(unlinked, "shared memory {shm} was not unlinked");
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    // `kill -0` fails for both missing processes and reaped zombies' ghosts.
    let status = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .unwrap();
    if !status.success() {
        return false;
    }
    // A zombie still answers `kill -0`; treat it as dead.
    #[cfg(target_os = "linux")]
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        && let Some(state) = stat
            .rsplit(')')
            .next()
            .and_then(|s| s.split_whitespace().next())
    {
        return state != "Z";
    }
    true
}
