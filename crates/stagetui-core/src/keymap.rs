//! The keys stagetui itself answers to, by name.
//!
//! Every built-in key has a name (`quit`, `prefix`, `close_pane`…) and default
//! keys. An app renames them with `KEYS = {"quit": "ctrl+q"}`; a person can
//! rename them for every app in `keys.json` (see [`user_file`]). `None` turns a
//! key off. Two rules keep this safe: there is always a way to quit and a
//! prefix key, and one key never means two things at once.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::input::{Code, Key};
use crate::{Error, Result};

/// When a key works: pressed on its own, or right after the prefix key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    Direct,
    AfterPrefix,
}

/// One built-in key action.
#[derive(Debug, Clone, Copy)]
pub struct Action {
    pub name: &'static str,
    pub keys: &'static [&'static str],
    pub when: When,
    pub about: &'static str,
}

/// Every built-in key action, with its default keys.
pub const ACTIONS: &[Action] = &[
    Action {
        name: "quit",
        keys: &["q", "esc", "ctrl+c"],
        when: When::Direct,
        about: "Quit the app (Esc first closes a dialog or leaves the focused widget). \
                After the prefix the same keys quit too, all but Esc",
    },
    Action {
        name: "prefix",
        keys: &["ctrl+space"],
        when: When::Direct,
        about: "Press it, then a pane key. It reaches the app even from inside a shell pane",
    },
    Action {
        name: "focus_next",
        keys: &["tab"],
        when: When::Direct,
        about: "Move to the next widget or pane (after the prefix: leave a shell pane)",
    },
    Action {
        name: "focus_previous",
        keys: &["shift+tab"],
        when: When::Direct,
        about: "Move to the previous widget or pane",
    },
    Action {
        name: "next_tab",
        keys: &["ctrl+pagedown"],
        when: When::Direct,
        about: "Show the next tab or workspace",
    },
    Action {
        name: "previous_tab",
        keys: &["ctrl+pageup"],
        when: When::Direct,
        about: "Show the previous tab or workspace",
    },
    Action {
        name: "pane_menu",
        keys: &["space", "enter"],
        when: When::AfterPrefix,
        about: "Choose what the pane shows",
    },
    Action {
        name: "split_right",
        keys: &["|", "\\"],
        when: When::AfterPrefix,
        about: "Split the pane: a new one on the right",
    },
    Action {
        name: "split_down",
        keys: &["-"],
        when: When::AfterPrefix,
        about: "Split the pane: a new one below",
    },
    Action {
        name: "close_pane",
        keys: &["x"],
        when: When::AfterPrefix,
        about: "Close the pane (closing the last one closes the app)",
    },
    Action {
        name: "pop_out",
        keys: &["o"],
        when: When::AfterPrefix,
        about: "Pop the pane out into its own window, or back",
    },
    Action {
        name: "detach",
        keys: &["d"],
        when: When::AfterPrefix,
        about: "Leave the app running in the background (stagetui sessions)",
    },
];

/// Keys after the prefix that always mean the same thing.
const FIXED_AFTER_PREFIX: &[(&str, &str)] = &[
    ("up", "moving between panes"),
    ("down", "moving between panes"),
    ("left", "moving between panes"),
    ("right", "moving between panes"),
    ("1", "workspace 1"),
    ("2", "workspace 2"),
    ("3", "workspace 3"),
    ("4", "workspace 4"),
    ("5", "workspace 5"),
    ("6", "workspace 6"),
    ("7", "workspace 7"),
    ("8", "workspace 8"),
    ("9", "workspace 9"),
];

fn action(name: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|a| a.name == name)
}

fn names() -> String {
    ACTIONS
        .iter()
        .map(|a| a.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The keys in effect for one app.
#[derive(Debug, Clone)]
pub struct Keymap {
    keys: BTreeMap<&'static str, Vec<Key>>,
}

impl Default for Keymap {
    fn default() -> Self {
        let keys = ACTIONS
            .iter()
            .map(|a| {
                let keys = a.keys.iter().filter_map(|k| Key::parse(k).ok()).collect();
                (a.name, keys)
            })
            .collect();
        Self { keys }
    }
}

impl Keymap {
    /// Rename keys: `{"quit": "ctrl+q", "pane_menu": null, "split_right": ["|", "v"]}`.
    /// `source` names where they came from in messages (`KEYS`, a file path).
    pub fn apply(&mut self, renames: &Value, source: &str) -> Result<()> {
        let Some(map) = renames.as_object() else {
            if renames.is_null() {
                return Ok(());
            }
            return Err(Error::friendly(
                format!("{source} should be a dict of key names"),
                format!("it is {renames}"),
                "write it like {\"quit\": \"ctrl+q\", \"close_pane\": \"w\"}",
            ));
        };
        for (name, value) in map {
            let Some(a) = action(name) else {
                return Err(Error::friendly(
                    format!("{source} has no key called \"{name}\""),
                    format!("the built-in keys are: {}", names()),
                    "use one of those names (your own shortcuts go in BINDINGS)",
                ));
            };
            let texts: Vec<&str> = match value {
                Value::Null => Vec::new(),
                Value::String(s) => vec![s.as_str()],
                Value::Array(list) => {
                    let mut out = Vec::new();
                    for v in list {
                        let Some(s) = v.as_str() else {
                            return Err(Error::friendly(
                                format!("{source}: \"{name}\" has {v}, which isn't a key"),
                                "keys are written as text, like \"ctrl+q\"",
                                format!("write \"{name}\": [\"ctrl+q\", \"f10\"]"),
                            ));
                        };
                        out.push(s);
                    }
                    out
                }
                other => {
                    return Err(Error::friendly(
                        format!("{source}: \"{name}\" has {other}, which isn't a key"),
                        "give one key, a list of keys, or None to turn it off",
                        format!("write \"{name}\": \"ctrl+q\""),
                    ));
                }
            };
            let mut keys = Vec::new();
            for t in texts {
                let k = Key::parse(t)?;
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
            self.keys.insert(a.name, keys);
        }
        Ok(())
    }

    /// The safety rules: a way to quit, a prefix, and no key with two meanings.
    pub fn check(&self) -> Result<()> {
        if self.keys("prefix").is_empty() {
            return Err(Error::friendly(
                "The prefix key can't be turned off",
                "it's how people reach the pane keys and leave a shell pane",
                "give \"prefix\" another key instead, like \"ctrl+a\"",
            ));
        }
        if self.keys("quit").is_empty() {
            return Err(Error::friendly(
                "Every app needs a way to quit",
                "\"quit\" has no keys left",
                "give \"quit\" at least one key, like \"ctrl+q\" (CONFIRM_QUIT = True asks first)",
            ));
        }
        let mut direct: Vec<(Key, String)> = Vec::new();
        let mut after: Vec<(Key, String)> = FIXED_AFTER_PREFIX
            .iter()
            .filter_map(|(k, what)| Some((Key::parse(k).ok()?, (*what).to_string())))
            .collect();
        for a in ACTIONS {
            for k in self.keys(a.name) {
                match a.when {
                    When::Direct => direct.push((*k, format!("\"{}\"", a.name))),
                    When::AfterPrefix => after.push((*k, format!("\"{}\"", a.name))),
                }
            }
        }
        // These work after the prefix too.
        for name in ["quit", "focus_next", "focus_previous"] {
            for k in self.keys(name) {
                if !(name == "quit" && k.is(Code::Esc)) {
                    after.push((*k, format!("\"{name}\"")));
                }
            }
        }
        for (list, when) in [(&direct, ""), (&after, " after the prefix")] {
            for (i, (k, a)) in list.iter().enumerate() {
                if let Some((_, b)) = list[..i].iter().find(|(other, _)| other == k) {
                    return Err(Error::friendly(
                        format!("{} has two jobs{when}", k.label()),
                        format!("{b} and {a} both use it"),
                        "give one of them another key in KEYS",
                    ));
                }
            }
        }
        Ok(())
    }

    /// The keys for an action (empty when it's off).
    pub fn keys(&self, name: &str) -> &[Key] {
        self.keys.get(name).map_or(&[], Vec::as_slice)
    }

    /// Whether `k` is one of the action's keys.
    pub fn is(&self, name: &str, k: &Key) -> bool {
        self.keys(name).contains(k)
    }

    /// The first key, as people read it (`Ctrl+Space`, `q`), for hints.
    pub fn label(&self, name: &str) -> Option<String> {
        self.keys(name).first().map(Key::label)
    }

    /// Every action with its keys, for `stagetui keys`, agents and docs.
    pub fn to_json(&self) -> Value {
        Value::Array(
            ACTIONS
                .iter()
                .map(|a| {
                    let keys = self.keys(a.name);
                    json!({
                        "name": a.name,
                        "keys": keys.iter().map(Key::name).collect::<Vec<_>>(),
                        "labels": keys.iter().map(Key::label).collect::<Vec<_>>(),
                        "after_prefix": a.when == When::AfterPrefix,
                        "about": a.about,
                    })
                })
                .collect(),
        )
    }
}

/// A person's own keys for every app: `STAGETUI_KEYS`, else `keys.json` in the
/// stagetui settings folder. Headless runs (tests, agents) only use
/// `STAGETUI_KEYS`, so they behave the same on every computer.
pub fn user_file() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("STAGETUI_KEYS").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    if cfg!(test) || std::env::var_os("STAGETUI_HEADLESS").is_some() {
        return None;
    }
    Some(crate::workspace::config_dir().join("keys.json"))
}

/// Read a person's keys file onto `map`, one entry at a time. An entry with a
/// problem is skipped and explained, so a typo never stops an app. `taken`
/// are the app's own shortcuts: they stay the app's.
pub fn apply_user_file(map: &mut Keymap, path: &std::path::Path, taken: &[Key]) -> Vec<String> {
    let shown = path.display();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => return vec![format!("{shown}: couldn't read it ({e})")],
    };
    let value: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return vec![format!("{shown} isn't valid JSON: {e}")],
    };
    let Some(entries) = value.as_object() else {
        return vec![format!(
            "{shown} should hold {{\"quit\": \"ctrl+q\", …}}, not {value}"
        )];
    };
    let mut problems = Vec::new();
    for (name, keys) in entries {
        let mut next = map.clone();
        let one = json!({ name: keys });
        let result = next
            .apply(&one, &shown.to_string())
            .and_then(|()| next.check());
        match result {
            Err(e) => {
                let (what, why, _) = e.parts();
                problems.push(format!("{what} ({why}) — skipped"));
            }
            Ok(()) => {
                if let Some(k) = next.keys(name).iter().find(|k| taken.contains(k)) {
                    problems.push(format!(
                        "{shown}: {} for \"{name}\" is this app's own shortcut — skipped",
                        k.label()
                    ));
                } else {
                    *map = next;
                }
            }
        }
    }
    problems
}

/// The keys as a table people can read (`stagetui keys`): built-in keys,
/// pane keys, the app's own shortcuts `(key label, what)`, and the person's file.
pub fn text(
    map: &Keymap,
    shortcuts: &[(String, String)],
    file: Option<&std::path::Path>,
    problems: &[String],
) -> String {
    let mut out = String::new();
    let row = |out: &mut String, name: &str, keys: &str, about: &str| {
        out.push_str(&format!("  {name:<15} {keys:<20} {about}\n"));
    };
    let keys_of = |name: &str| {
        let labels: Vec<String> = map.keys(name).iter().map(Key::label).collect();
        if labels.is_empty() {
            "(off)".to_string()
        } else {
            labels.join(", ")
        }
    };
    out.push_str("Keys\n");
    for a in ACTIONS.iter().filter(|a| a.when == When::Direct) {
        row(&mut out, a.name, &keys_of(a.name), a.about);
    }
    let prefix = map.label("prefix").unwrap_or_default();
    out.push_str(&format!("\nAfter the prefix ({prefix}, then…)\n"));
    for a in ACTIONS.iter().filter(|a| a.when == When::AfterPrefix) {
        row(&mut out, a.name, &keys_of(a.name), a.about);
    }
    row(&mut out, "", "← ↑ ↓ →", "Move between panes (always)");
    row(&mut out, "", "1-9", "Show a workspace (always)");
    if !shortcuts.is_empty() {
        out.push_str("\nThe app's own shortcuts (they come first)\n");
        for (key, what) in shortcuts {
            row(&mut out, "", key, what);
        }
    }
    match file {
        Some(path) => {
            let state = if path.is_file() { "" } else { " (none yet)" };
            out.push_str(&format!(
                "\nYour keys for every app: {}{state}\n",
                path.display()
            ));
        }
        None => out.push_str(
            "\nYour keys for every app: not read here (headless) — set STAGETUI_KEYS to a file\n",
        ),
    }
    for p in problems {
        out.push_str(&format!("  ! {p}\n"));
    }
    out
}

/// The keys every app starts with on this computer (defaults plus the
/// person's file), without an app: JSON, or the table (`stagetui keys`).
pub fn overview(as_text: bool) -> String {
    let mut map = Keymap::default();
    let file = user_file();
    let problems = file
        .as_ref()
        .map(|p| apply_user_file(&mut map, p, &[]))
        .unwrap_or_default();
    if as_text {
        text(&map, &[], file.as_deref(), &problems)
    } else {
        json!({
            "keys": map.to_json(),
            "shortcuts": [],
            "confirm_quit": false,
            "file": file.map(|p| p.display().to_string()),
            "problems": problems,
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> Key {
        Key::parse(s).unwrap()
    }

    #[test]
    fn defaults_are_safe_and_complete() {
        let m = Keymap::default();
        m.check().unwrap();
        assert!(
            m.is("quit", &key("q")) && m.is("quit", &key("ctrl+c")) && m.is("quit", &key("esc"))
        );
        assert!(m.is("prefix", &key("ctrl+space")));
        assert!(m.is("split_right", &key("|")) && m.is("split_right", &key("\\")));
        assert_eq!(m.label("prefix").as_deref(), Some("Ctrl+Space"));
        for a in ACTIONS {
            assert_eq!(m.keys(a.name).len(), a.keys.len(), "{} parses", a.name);
        }
    }

    #[test]
    fn renaming_and_turning_off() {
        let mut m = Keymap::default();
        m.apply(
            &json!({"quit": "ctrl+q", "pane_menu": null, "split_right": ["v", "|"]}),
            "KEYS",
        )
        .unwrap();
        m.check().unwrap();
        assert!(m.is("quit", &key("ctrl+q")) && !m.is("quit", &key("q")));
        assert!(m.keys("pane_menu").is_empty() && m.label("pane_menu").is_none());
        assert_eq!(m.label("split_right").as_deref(), Some("v"));
    }

    #[test]
    fn mistakes_are_explained() {
        let what = |v: Value| {
            let mut m = Keymap::default();
            match m.apply(&v, "KEYS").and_then(|()| m.check()) {
                Err(e) => e.to_string(),
                Ok(()) => String::new(),
            }
        };
        assert!(what(json!({"quitt": "x"})).contains("no key called \"quitt\""));
        assert!(what(json!({"quit": "ctrl+nope"})).contains("Unknown key"));
        assert!(what(json!({"quit": 5})).contains("isn't a key"));
        assert!(what(json!({"quit": null})).contains("way to quit"));
        assert!(what(json!({"prefix": []})).contains("prefix key can't be turned off"));
        // One key, two jobs.
        assert!(what(json!({"focus_next": "q"})).contains("two jobs"));
        assert!(what(json!({"close_pane": "q"})).contains("after the prefix"));
        assert!(what(json!({"pop_out": "1"})).contains("workspace 1"));
        assert!(what(json!({"split_down": "left"})).contains("moving between panes"));
    }

    #[test]
    fn a_persons_file_never_stops_an_app() {
        let dir = std::env::temp_dir().join(format!("stagetui-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("keys.json");
        std::fs::write(
            &path,
            r#"{"close_pane": "w", "quit": null, "pop_out": "ctrl+s", "nope": "x", "split_down": "_"}"#,
        )
        .unwrap();
        let mut m = Keymap::default();
        let problems = apply_user_file(&mut m, &path, &[key("ctrl+s")]);
        assert!(
            m.is("close_pane", &key("w")) && m.is("split_down", &key("_")),
            "good ones apply"
        );
        assert!(m.is("quit", &key("q")), "quit can't be taken away");
        assert!(
            !m.is("pop_out", &key("ctrl+s")),
            "the app's own shortcut stays the app's"
        );
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems.iter().all(|p| p.contains("skipped")));
        std::fs::write(&path, "{ not json").unwrap();
        let problems = apply_user_file(&mut m, &path, &[]);
        assert!(problems[0].contains("isn't valid JSON"));
        assert!(apply_user_file(&mut m, &dir.join("missing.json"), &[]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
