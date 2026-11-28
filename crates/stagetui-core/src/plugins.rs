//! Plugins: pane types that live in their own program (video, web, 3D…).
//!
//! A plugin is an executable named `stagetui-plugin-<name>`. It is found next
//! to the app's Python (where `pip install` puts it), in `STAGETUI_PLUGIN_PATH`
//! or on the `PATH`, and asked `--describe` for its title and icon. Plugins
//! stagetui knows about but that aren't installed are offered anyway: picking
//! one asks whether to install it. A file that doesn't answer `--describe`
//! (a script whose `#!` Python lacks stagetui, a crash, bad JSON) is kept as
//! [`Broken`], so people are told why it isn't there.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use stagetui_plugin_api::Describe;

/// A plugin found on this computer.
#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub describe: Describe,
    pub path: PathBuf,
}

/// A `stagetui-plugin-<name>` file that didn't answer `--describe`.
#[derive(Debug, Clone, PartialEq)]
pub struct Broken {
    pub name: String,
    pub path: PathBuf,
    /// Short reason: the error it printed, its exit code, or what was wrong
    /// with its answer.
    pub why: String,
}

/// A plugin stagetui knows about (it can offer to install it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Known {
    pub name: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    /// The Python package to install.
    pub package: &'static str,
    pub about: &'static str,
    /// Published on PyPI by us. Until then it is never installed by name
    /// (someone else could own that name).
    pub available: bool,
}

/// The official add-ons.
pub const KNOWN: &[Known] = &[
    Known {
        name: "video",
        title: "Video",
        icon: "play",
        package: "stagetui-video",
        about: "play video files and links, with sound",
        available: true,
    },
    Known {
        name: "web",
        title: "Web",
        icon: "globe",
        package: "stagetui-web",
        about: "show a real web page (uses your Chrome or Chromium)",
        available: true,
    },
    Known {
        name: "3d",
        title: "3D",
        icon: "cube",
        package: "stagetui-3d",
        about: "look at 3D models and turn them with the mouse",
        available: true,
    },
];

const PREFIX: &str = "stagetui-plugin-";

static FOUND: Mutex<Option<Vec<Installed>>> = Mutex::new(None);
/// Found with [`FOUND`]: files that didn't answer.
static BROKEN: Mutex<Vec<Broken>> = Mutex::new(Vec::new());
static PYTHON: Mutex<Option<String>> = Mutex::new(None);
/// Tests: add-ons stagetui pretends to know (see [`pretend_known`]).
#[cfg(test)]
static PRETEND_KNOWN: Mutex<Vec<Known>> = Mutex::new(Vec::new());

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The Python this app runs on (its scripts folder is searched first, and
/// add-ons are installed into it).
pub fn set_python(python: &str) {
    *lock(&PYTHON) = Some(python.to_string());
    forget();
}

pub fn python() -> Option<String> {
    lock(&PYTHON).clone()
}

/// Where plugins are looked for, in order.
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(extra) = std::env::var_os("STAGETUI_PLUGIN_PATH") {
        dirs.extend(std::env::split_paths(&extra));
    }
    if let Some(py) = python()
        && let Some(dir) = Path::new(&py).parent()
    {
        dirs.push(dir.to_path_buf());
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

/// The plugin name in a file name (`stagetui-plugin-video[.exe]` → `video`).
fn plugin_name(file: &str) -> Option<&str> {
    let rest = file.strip_prefix(PREFIX)?;
    let name = if cfg!(windows) {
        rest.strip_suffix(".exe")
            .or_else(|| rest.strip_suffix(".cmd"))?
    } else {
        rest
    };
    (!name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .then_some(name)
}

/// Ask a plugin what it is (`--describe`), giving up after a few seconds.
pub fn describe(path: &Path) -> Option<Describe> {
    ask(path).ok()
}

/// Read a pipe to the end on a thread of its own (so a chatty program never
/// blocks on a full pipe); the text arrives on the returned channel.
fn drain(pipe: Option<impl std::io::Read + Send + 'static>) -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    if let Some(mut pipe) = pipe {
        let _ = std::thread::Builder::new()
            .name("stagetui-plugin-describe".into())
            .spawn(move || {
                let mut bytes = Vec::new();
                let _ = std::io::Read::read_to_end(&mut pipe, &mut bytes);
                let _ = tx.send(String::from_utf8_lossy(&bytes).into_owned());
            });
    }
    rx
}

/// `--describe`, or why it didn't answer (in words a person can act on).
fn ask(path: &Path) -> Result<Describe, String> {
    let mut child = Command::new(path)
        .arg("--describe")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| couldnt_start(path, &e))?;
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("it didn't answer within 5 seconds".into());
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    let grace = Duration::from_secs(1);
    let out = out.recv_timeout(grace).unwrap_or_default();
    let err = err.recv_timeout(grace).unwrap_or_default();
    let Some(line) = out.lines().find(|l| l.trim_start().starts_with('{')) else {
        return Err(if status.success() {
            "its answer to --describe has no JSON line".into()
        } else {
            stopped(status, &err)
        });
    };
    serde_json::from_str(line).map_err(|e| {
        if status.success() {
            format!("its answer to --describe isn't valid JSON: {e}")
        } else {
            stopped(status, &err)
        }
    })
}

/// The program ended with an error: the line that says what went wrong.
fn stopped(status: std::process::ExitStatus, stderr: &str) -> String {
    let mut lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    // A Rust panic ends with a hint about backtraces; the reason is above it.
    while lines.len() > 1 && lines.last().is_some_and(|l| l.starts_with("note:")) {
        lines.pop();
    }
    // A Python traceback ends with the error itself; others say it first or last.
    match lines.last() {
        Some(line) => short(line),
        None => match status.code() {
            Some(code) => format!("it stopped with exit code {code} and said nothing"),
            None => "it was stopped by a signal".into(),
        },
    }
}

fn couldnt_start(path: &Path, e: &std::io::Error) -> String {
    let first = std::fs::read(path).ok().and_then(|bytes| {
        let text = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).into_owned();
        text.lines().next().map(str::to_string)
    });
    match first {
        Some(line) if line.starts_with("#!") && e.kind() == std::io::ErrorKind::NotFound => {
            format!(
                "couldn't start it: its first line `{}` names a program that isn't here",
                short(&line)
            )
        }
        _ => format!("couldn't start it: {e}"),
    }
}

fn short(line: &str) -> String {
    const MAX: usize = 200;
    if line.chars().count() <= MAX {
        line.to_string()
    } else {
        format!("{}…", line.chars().take(MAX).collect::<String>())
    }
}

/// Every plugin on this computer (looked up once; see [`forget`]).
pub fn installed() -> Vec<Installed> {
    // Tests see only what they pretend is installed, never this computer's add-ons.
    #[cfg(test)]
    return lock(&FOUND).clone().unwrap_or_default();
    #[cfg(not(test))]
    installed_here()
}

#[cfg(not(test))]
fn installed_here() -> Vec<Installed> {
    let mut found = lock(&FOUND);
    if let Some(list) = found.as_ref() {
        return list.clone();
    }
    let (list, broken) = scan(&search_dirs());
    *lock(&BROKEN) = broken;
    *found = Some(list.clone());
    list
}

/// Look for plugins in `dirs`, in order: the first file of each name wins.
/// Files that don't answer are returned too, unless another file of that name works.
fn scan(dirs: &[PathBuf]) -> (Vec<Installed>, Vec<Broken>) {
    let mut list: Vec<Installed> = Vec::new();
    let mut broken: Vec<Broken> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        files.sort();
        for path in files {
            let Some(name) = path
                .file_name()
                .and_then(|f| f.to_str())
                .and_then(plugin_name)
            else {
                continue;
            };
            if list.iter().any(|p| p.describe.name == name) || !path.is_file() {
                continue;
            }
            match ask(&path) {
                Ok(mut d) => {
                    d.name = name.to_string(); // the file name decides the type
                    list.push(Installed { describe: d, path });
                }
                Err(why) => {
                    if !broken.iter().any(|b| b.name == name) {
                        broken.push(Broken {
                            name: name.to_string(),
                            path,
                            why,
                        });
                    }
                }
            }
        }
    }
    broken.retain(|b| !list.iter().any(|p| p.describe.name == b.name));
    (list, broken)
}

/// Plugin files that were found but didn't answer `--describe` (looked up
/// with [`installed`]).
pub fn broken() -> Vec<Broken> {
    let _ = installed();
    lock(&BROKEN).clone()
}

/// Tests: treat `name` as installed (it won't actually run).
#[cfg(test)]
pub fn pretend_installed(name: &str) {
    let fake = Installed {
        describe: Describe::new(name, name, "dot"),
        path: PathBuf::from(format!("/nonexistent/{PREFIX}{name}")),
    };
    let mut found = lock(&FOUND);
    let list = found.get_or_insert_with(Vec::new);
    list.retain(|p| p.describe.name != name);
    list.push(fake);
}

/// Look again next time (after installing something).
pub fn forget() {
    *lock(&FOUND) = None;
    lock(&BROKEN).clear();
}

pub fn find(name: &str) -> Option<Installed> {
    installed().into_iter().find(|p| p.describe.name == name)
}

pub fn known(name: &str) -> Option<Known> {
    catalog().into_iter().find(|k| k.name == name)
}

/// The official add-ons (in tests, also the ones in [`pretend_known`]).
fn catalog() -> Vec<Known> {
    #[allow(unused_mut)]
    let mut all = KNOWN.to_vec();
    #[cfg(test)]
    all.extend(lock(&PRETEND_KNOWN).iter().copied());
    all
}

/// Tests: treat `k` as an official add-on (e.g. one that isn't out yet).
#[cfg(test)]
pub fn pretend_known(k: Known) {
    let mut list = lock(&PRETEND_KNOWN);
    list.retain(|p| p.name != k.name);
    list.push(k);
}

/// The command that installs `package` into the app's Python: pip if it has
/// it, else `uv`. `None` when neither is there.
pub fn installer(package: &str) -> Option<Vec<String>> {
    let python = python().unwrap_or_else(|| {
        if cfg!(windows) {
            "python".into()
        } else {
            "python3".into()
        }
    });
    let has_pip = Command::new(&python)
        .args(["-m", "pip", "--version"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if has_pip {
        return Some(vec![
            python,
            "-m".into(),
            "pip".into(),
            "install".into(),
            package.into(),
        ]);
    }
    let has_uv = Command::new("uv")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    has_uv.then(|| {
        vec![
            "uv".into(),
            "pip".into(),
            "install".into(),
            "--python".into(),
            python,
            package.into(),
        ]
    })
}

/// One line of `stagetui plugins`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Report {
    pub name: String,
    pub title: String,
    pub icon: Option<String>,
    pub installed: bool,
    /// The Python package that provides it (official add-ons).
    pub package: Option<String>,
    pub about: Option<String>,
    pub path: Option<String>,
    pub version: Option<String>,
    /// It can be installed now (official add-ons, once published).
    pub available: bool,
    /// Its file was found, but it didn't answer `--describe`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broken: Option<BrokenReport>,
}

/// Where a plugin file is that didn't answer, and why.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BrokenReport {
    pub path: String,
    pub why: String,
}

impl From<&Broken> for BrokenReport {
    fn from(b: &Broken) -> Self {
        Self {
            path: b.path.display().to_string(),
            why: b.why.clone(),
        }
    }
}

/// Every add-on: installed ones, files that didn't answer, then official ones
/// that aren't installed.
pub fn report() -> Vec<Report> {
    build_report(installed(), &broken(), &catalog())
}

fn build_report(installed: Vec<Installed>, broken: &[Broken], catalog: &[Known]) -> Vec<Report> {
    let mut out: Vec<Report> = installed
        .into_iter()
        .map(|p| {
            let k = known(&p.describe.name);
            Report {
                name: p.describe.name.clone(),
                title: p.describe.title.clone(),
                icon: p.describe.icon.clone(),
                installed: true,
                package: k.map(|k| k.package.to_string()),
                about: k.map(|k| k.about.to_string()),
                path: Some(p.path.display().to_string()),
                version: Some(p.describe.version.clone()).filter(|v| !v.is_empty()),
                available: true,
                broken: None,
            }
        })
        .collect();
    let broken_named = |name: &str| broken.iter().find(|b| b.name == name);
    for b in broken {
        if !out.iter().any(|r| r.name == b.name) && !catalog.iter().any(|k| k.name == b.name) {
            out.push(Report {
                name: b.name.clone(),
                title: b.name.clone(),
                icon: None,
                installed: false,
                package: None,
                about: None,
                path: Some(b.path.display().to_string()),
                version: None,
                available: false,
                broken: Some(b.into()),
            });
        }
    }
    for k in catalog {
        if !out.iter().any(|r| r.name == k.name) {
            out.push(Report {
                name: k.name.into(),
                title: k.title.into(),
                icon: Some(k.icon.into()),
                installed: false,
                package: Some(k.package.into()),
                about: Some(k.about.into()),
                path: None,
                version: None,
                available: k.available,
                broken: broken_named(k.name).map(Into::into),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_file_names() {
        let exe = if cfg!(windows) { ".exe" } else { "" };
        assert_eq!(
            plugin_name(&format!("stagetui-plugin-video{exe}")),
            Some("video")
        );
        assert_eq!(plugin_name(&format!("stagetui-plugin-3d{exe}")), Some("3d"));
        assert_eq!(plugin_name("stagetui-plugin-"), None);
        assert_eq!(plugin_name("stagetui-plugin-a b"), None);
        assert_eq!(plugin_name("stagetui"), None);
        assert!(known("video").is_some_and(|k| k.package == "stagetui-video"));
        assert!(known("مرحبا").is_none());
    }

    #[test]
    fn the_official_add_ons_are_published() {
        // 3D joined video and web on PyPI: each can be installed by name.
        for name in ["video", "web", "3d"] {
            let k = known(name).unwrap();
            assert!(k.available, "{name}");
            assert_eq!(k.package, format!("stagetui-{name}"));
        }
        let three = report().into_iter().find(|r| r.name == "3d").unwrap();
        assert!(three.available && !three.installed, "{three:?}");
    }

    /// Run `scan` until no file is "busy": a file just written can't be run
    /// while another test thread is starting a program.
    fn scan_settled(dirs: &[PathBuf]) -> (Vec<Installed>, Vec<Broken>) {
        for _ in 0..20 {
            let (found, broken) = scan(dirs);
            if !broken.iter().any(|b| b.why.contains("busy")) {
                return (found, broken);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        scan(dirs)
    }

    fn fresh_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("stagetui-plugins-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const SAYS: &str = "ModuleNotFoundError: No module named 'stagetui' مَرْحَبًا Hello 日本語 नमस्ते";

    #[cfg(unix)]
    #[test]
    fn a_plugin_file_that_does_not_answer_is_reported_not_lost() {
        use std::os::unix::fs::PermissionsExt;
        let dir = fresh_dir("broken");
        let later = fresh_dir("later");
        let write = |dir: &Path, name: &str, body: &str| {
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        // A Python script run by a Python without stagetui: a traceback, exit 1.
        write(
            &dir,
            "stagetui-plugin-crash",
            &format!(
                "#!/bin/sh\necho 'Traceback (most recent call last):' >&2\necho \"{SAYS}\" >&2\nexit 1\n"
            ),
        );
        write(
            &dir,
            "stagetui-plugin-garbled",
            "#!/bin/sh\necho '{not json'\n",
        );
        write(&dir, "stagetui-plugin-nopython", "#!/nonexistent/python3\n");
        write(&dir, "stagetui-plugin-silent", "#!/bin/sh\nexit 3\n");
        write(&dir, "stagetui-plugin-video", "#!/bin/sh\nexit 1\n");
        let good = r#"{"name":"x","title":"Good","icon":"star"}"#;
        write(
            &dir,
            "stagetui-plugin-good",
            &format!("#!/bin/sh\necho '{good}'\n"),
        );
        // A working file of the same name further down the search path wins.
        write(
            &later,
            "stagetui-plugin-silent",
            &format!("#!/bin/sh\necho '{good}'\n"),
        );

        let (found, broken) = scan_settled(&[dir.clone(), later.clone()]);
        let names: Vec<&str> = found.iter().map(|p| p.describe.name.as_str()).collect();
        assert_eq!(names, ["good", "silent"], "the file name decides the type");
        let why = |name: &str| {
            let b = broken.iter().find(|b| b.name == name).unwrap();
            assert_eq!(b.path, dir.join(format!("stagetui-plugin-{name}")));
            b.why.clone()
        };
        assert_eq!(
            why("crash"),
            SAYS,
            "the error it printed, not the traceback header"
        );
        assert!(
            why("garbled").contains("isn't valid JSON"),
            "{}",
            why("garbled")
        );
        assert!(
            why("nopython").contains("#!/nonexistent/python3"),
            "{}",
            why("nopython")
        );
        assert_eq!(why("video"), "it stopped with exit code 1 and said nothing");
        assert!(!broken.iter().any(|b| b.name == "silent"), "{broken:?}");

        // `stagetui plugins` / ts.plugins(): broken files are listed, the
        // official add-on stays installable and says what was found.
        let report = build_report(found, &broken, KNOWN);
        let entry = |name: &str| report.iter().find(|r| r.name == name).unwrap();
        let crash = entry("crash");
        assert!(!crash.installed && !crash.available && crash.package.is_none());
        assert_eq!(crash.broken.as_ref().unwrap().why, SAYS);
        let video = entry("video");
        assert!(!video.installed && video.available);
        assert_eq!(video.package.as_deref(), Some("stagetui-video"));
        assert!(
            video
                .broken
                .as_ref()
                .unwrap()
                .path
                .ends_with("stagetui-plugin-video")
        );
        assert!(entry("good").installed && entry("good").broken.is_none());
        let json = serde_json::to_value(&report).unwrap();
        assert!(json[0].get("broken").is_none(), "only broken ones carry it");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&later);
    }

    #[cfg(windows)]
    #[test]
    fn a_plugin_file_that_does_not_answer_is_reported_not_lost() {
        let dir = fresh_dir("broken");
        std::fs::write(
            dir.join("stagetui-plugin-crash.cmd"),
            format!("@echo off\r\necho {SAYS} 1>&2\r\nexit /b 1\r\n"),
        )
        .unwrap();
        let (found, broken) = scan_settled(std::slice::from_ref(&dir));
        assert!(found.is_empty());
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].name, "crash");
        assert!(!broken[0].why.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An add-on that isn't out yet (only these tests know it).
    const SOON: Known = Known {
        name: "soon-test",
        title: "Soon",
        icon: "star",
        package: "stagetui-soon-test",
        about: "an add-on that isn't on PyPI yet",
        available: false,
    };

    const PANES: &str = r##"{"children":[{"type":"split","children":[
        {"type":"pane","content":"soon-test","props":{"id":"s"},"children":[{"type":"plugin","content":"soon-test","props":{"id":"sp"}}]},
        {"type":"pane","content":"3d","props":{"id":"t"},"children":[{"type":"plugin","content":"3d","props":{"id":"tp"}}]},
        {"type":"pane","content":"markdown","props":{"id":"m"},"children":[{"type":"markdown","content":"# مَرْحَبًا Hello 日本語 नमस्ते"}]}]}]}"##;

    #[test]
    fn an_add_on_that_is_not_out_yet_is_never_installed() {
        pretend_known(SOON);
        let soon = report()
            .into_iter()
            .find(|r| r.name == "soon-test")
            .unwrap();
        assert!(!soon.available && !soon.installed, "{soon:?}");

        // The pane menu says which ones are coming and which can be installed.
        let missing = |name: &str| crate::tree::PaneTypeSpec {
            name: name.into(),
            title: known(name).map(|k| k.title.into()),
            icon: known(name).map(|k| k.icon.into()),
            plugin: true,
            missing: true,
            ..Default::default()
        };
        let menu = crate::panes::Menu::new(0, &[missing("soon-test"), missing("3d")], "", false);
        let label = |key: &str| {
            let item = menu.items.iter().find(|i| i.key == key).unwrap();
            item.label.clone()
        };
        assert!(label("soon-test").ends_with("Soon  (coming soon)"));
        assert!(label("3d").ends_with("3D  (install…)"));

        // In a pane: it says so, and Enter or a click only says it again.
        let mut s = crate::session::Session::from_json(PANES).unwrap();
        let text = s.frame(120, 30).text();
        assert!(text.contains("Soon is coming soon"), "{text}");
        assert!(text.contains("3D isn't installed yet"), "{text}");
        s.focus("#sp").unwrap();
        s.press("enter").unwrap();
        s.click("#sp").unwrap();
        assert_eq!(s.dialog_open(), None, "never offers to install it");
        let tree = s.tree_json(120, 30);
        assert!(
            tree["toasts"][0]
                .as_str()
                .is_some_and(|t| t.contains("The Soon add-on is coming soon")),
            "{}",
            tree["toasts"]
        );

        // A published one asks first (nothing runs until you say yes).
        s.focus("#tp").unwrap();
        s.press("enter").unwrap();
        assert_eq!(s.dialog_open(), Some("install"));
        assert!(s.frame(120, 30).text().contains("Install 3D?"));
        s.press("esc").unwrap();
        assert_eq!(s.dialog_open(), None);
        s.click("#t").unwrap(); // anywhere in the pane
        assert_eq!(s.dialog_open(), Some("install"));
        s.press("esc").unwrap();
        assert_eq!(s.dialog_open(), None);
    }
}
