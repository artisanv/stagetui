//! Plugins: pane types that live in their own program (video, web, 3D…).
//!
//! A plugin is an executable named `stagetui-plugin-<name>`. It is found next
//! to the app's Python (where `pip install` puts it), in `STAGETUI_PLUGIN_PATH`
//! or on the `PATH`, and asked `--describe` for its title and icon. Plugins
//! stagetui knows about but that aren't installed are offered anyway: picking
//! one asks whether to install it.

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
    let mut child = Command::new(path)
        .arg("--describe")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    serde_json::from_str(out.lines().find(|l| l.trim_start().starts_with('{'))?).ok()
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
    let mut list: Vec<Installed> = Vec::new();
    for dir in search_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
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
            if let Some(mut d) = describe(&path) {
                d.name = name.to_string(); // the file name decides the type
                list.push(Installed { describe: d, path });
            }
        }
    }
    *found = Some(list.clone());
    list
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
}

/// Every add-on: installed ones, then official ones that aren't installed.
pub fn report() -> Vec<Report> {
    let mut out: Vec<Report> = installed()
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
            }
        })
        .collect();
    for k in catalog() {
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
