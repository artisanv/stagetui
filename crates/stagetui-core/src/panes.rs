//! Blender-style panes: every pane has a header with a "what's in here?" menu,
//! and can be split, closed or switched to another type. Switching keeps the
//! old content (a running shell keeps running) so switching back restores it.

use serde_json::{Value, json};

use crate::tree::{NodeSpec, PaneTypeSpec};

/// The pane types every app gets, unless the app sends its own list.
pub fn builtin_types() -> Vec<PaneTypeSpec> {
    [
        ("chat", "Chat", "chat"),
        ("terminal", "Terminal", "terminal"),
        ("files", "Files", "folder"),
        ("diff", "Changes", "code"),
        ("code", "Code", "file"),
        ("markdown", "Notes", "book"),
        ("log", "Log", "info"),
        ("empty", "Empty", "circle"),
    ]
    .iter()
    .map(|(n, t, i)| PaneTypeSpec {
        name: (*n).into(),
        title: Some((*t).into()),
        icon: Some((*i).into()),
        ..PaneTypeSpec::default()
    })
    .collect()
}

/// Add the plugins: installed ones, then the official add-ons that aren't
/// installed yet (picking one offers to install it).
pub fn with_plugins(mut types: Vec<PaneTypeSpec>) -> Vec<PaneTypeSpec> {
    if std::env::var("STAGETUI_NO_PLUGINS").is_ok_and(|v| v == "1") {
        return types;
    }
    for p in crate::plugins::installed() {
        if !types.iter().any(|t| t.name == p.describe.name) {
            types.push(PaneTypeSpec {
                name: p.describe.name.clone(),
                title: Some(p.describe.title.clone()),
                icon: p.describe.icon.clone(),
                plugin: true,
                ..PaneTypeSpec::default()
            });
        }
    }
    for k in crate::plugins::KNOWN {
        if !types.iter().any(|t| t.name == k.name) {
            types.push(PaneTypeSpec {
                name: k.name.into(),
                title: Some(k.title.into()),
                icon: Some(k.icon.into()),
                plugin: true,
                missing: true,
                ..PaneTypeSpec::default()
            });
        }
    }
    types
}

/// The content of a plugin pane.
pub fn plugin_content(name: &str, args: Value) -> NodeSpec {
    NodeSpec {
        kind: "plugin".into(),
        content: json!(name),
        props: crate::tree::Props {
            args: (!args.is_null()).then_some(args),
            ..crate::tree::Props::default()
        },
        ..NodeSpec::default()
    }
}

/// Content for a built-in pane type (`None` for types the app builds itself).
pub fn builtin_content(name: &str) -> Option<NodeSpec> {
    let spec = |kind: &str, content: Value| NodeSpec {
        kind: kind.into(),
        content,
        ..NodeSpec::default()
    };
    Some(match name {
        "empty" => spec(
            "text",
            json!(
                "Empty pane — click its title (or press Ctrl+Space, then Space) to choose what it shows."
            ),
        ),
        "terminal" => spec("terminal", Value::Null),
        "chat" => spec("chat", Value::Null),
        "log" => spec("log", Value::Null),
        "markdown" => spec("markdown", json!("*Nothing here yet.*")),
        "code" => spec("code", json!("")),
        "diff" => spec("diff", Value::Null),
        "files" => spec("tree", files_tree(".", 3)),
        _ => return None,
    })
}

/// A folder as `StageTree` items (folders first, hidden and build folders skipped).
pub fn files_tree(root: &str, depth: usize) -> Value {
    const SKIP: [&str; 7] = [
        "node_modules",
        "target",
        "__pycache__",
        "dist",
        "build",
        ".venv",
        "venv",
    ];
    fn walk(dir: &std::path::Path, depth: usize, budget: &mut usize) -> Vec<Value> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| (!e.file_type().is_ok_and(|t| t.is_dir()), e.file_name()));
        let mut out = Vec::new();
        for e in entries {
            if *budget == 0 {
                break;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || SKIP.contains(&name.as_str()) {
                continue;
            }
            *budget -= 1;
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                let children = if depth > 1 {
                    walk(&e.path(), depth - 1, budget)
                } else {
                    Vec::new()
                };
                out.push(json!({ "label": format!("{name}/"), "children": children }));
            } else {
                out.push(Value::String(name));
            }
        }
        out
    }
    let mut budget = 800;
    Value::Array(walk(std::path::Path::new(root), depth, &mut budget))
}

/// A new, empty pane (what splitting creates).
pub fn empty_pane() -> NodeSpec {
    NodeSpec {
        kind: "pane".into(),
        content: json!("empty"),
        children: builtin_content("empty").into_iter().collect(),
        ..NodeSpec::default()
    }
}

/// One entry of the pane menu.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    /// A pane type name, or an action (`@split_right`, `@split_down`, `@close`).
    pub key: String,
    pub label: String,
    pub current: bool,
}

/// The open pane menu.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub pane: usize,
    pub items: Vec<MenuItem>,
    pub hover: usize,
}

impl Menu {
    pub fn new(pane: usize, types: &[PaneTypeSpec], current: &str, popped: bool) -> Self {
        let mut items: Vec<MenuItem> = types
            .iter()
            .map(|t| {
                let icon = t
                    .icon
                    .as_deref()
                    .and_then(|i| crate::widgets::icon(i).ok())
                    .unwrap_or("•");
                let title = t.title.clone().unwrap_or_else(|| t.name.clone());
                MenuItem {
                    key: t.name.clone(),
                    label: if t.missing {
                        let soon = crate::plugins::known(&t.name).is_some_and(|k| !k.available);
                        let note = if soon { "coming soon" } else { "install…" };
                        format!("{icon}  {title}  ({note})")
                    } else {
                        format!("{icon}  {title}")
                    },
                    current: t.name == current,
                }
            })
            .collect();
        let pop = if popped {
            "↙  Bring back here"
        } else {
            "↗  Open in its own window"
        };
        for (key, label) in [
            ("@split_right", "⊞  Split side by side"),
            ("@split_down", "⊟  Split top / bottom"),
            ("@pop", pop),
            ("@close", "×  Close pane"),
        ] {
            items.push(MenuItem {
                key: key.into(),
                label: label.into(),
                current: false,
            });
        }
        let hover = items.iter().position(|i| i.current).unwrap_or(0);
        Self { pane, items, hover }
    }

    /// Index of the first action (a separator line is drawn above it).
    pub fn first_action(&self) -> usize {
        self.items
            .iter()
            .position(|i| i.key.starts_with('@'))
            .unwrap_or(self.items.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_and_menu() {
        let types = builtin_types();
        for t in &types {
            assert!(builtin_content(&t.name).is_some(), "{}", t.name);
            assert!(crate::widgets::icon(t.icon.as_deref().unwrap()).is_ok());
        }
        assert!(builtin_content("my-custom").is_none());
        let m = Menu::new(3, &types, "terminal", false);
        assert_eq!(m.items[m.hover].key, "terminal");
        assert_eq!(m.items[m.first_action()].key, "@split_right");
        let tree = files_tree(env!("CARGO_MANIFEST_DIR"), 2);
        assert!(
            tree.as_array()
                .unwrap()
                .iter()
                .any(|v| v["label"] == "src/")
        );
    }
}
