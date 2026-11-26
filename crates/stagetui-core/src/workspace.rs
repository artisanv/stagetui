//! Workspaces: a layout of splits and panes you can save and bring back later,
//! like Blender's Layout / Modeling screens.
//!
//! A layout is plain JSON:
//!
//! ```json
//! {"split": "horizontal", "ratio": [0.3, 0.7], "children": [
//!     {"pane": "files", "id": "left"},
//!     {"pane": "terminal", "id": "shell", "args": {"command": "htop"}}
//! ]}
//! ```
//!
//! Restoring re-uses what is still there, by id: a shell that is still running
//! keeps running, wherever its pane moves. Other widgets are kept with
//! `{"keep": "<id>"}`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::tree::{Kind, NodeSpec, Props, Tree, normalize_ratios};
use crate::{Error, Result};

/// One saved arrangement of splits and panes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Layout {
    Split {
        /// `"horizontal"` (side by side) or `"vertical"` (top / bottom).
        split: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ratio: Vec<f32>,
        children: Vec<Layout>,
    },
    Pane {
        /// The pane type: `"chat"`, `"terminal"`, …
        pane: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        id: String,
        /// Options for the type (`command`, `root`, …).
        #[serde(default, skip_serializing_if = "Value::is_null")]
        args: Value,
    },
    /// Any other widget, kept by id.
    Keep { keep: String },
}

/// What [`Tree::apply_layout`] changed.
#[derive(Debug, Default)]
pub struct Applied {
    /// The node now standing where the old layout was.
    pub root: usize,
    /// New panes that still need content: `(pane, type, args)`.
    pub fresh: Vec<(usize, String, Value)>,
    /// Kept panes that must show another type: `(pane, type)`.
    pub switch: Vec<(usize, String)>,
    /// The first pane of the layout (where focus goes).
    pub first: Option<usize>,
}

fn bad_layout(why: impl Into<String>) -> Error {
    Error::friendly(
        "This workspace can't be used",
        why,
        "save it again with app.save_workspace(...), or fix the JSON by hand",
    )
}

impl Tree {
    /// The layout of the splits and panes under node `i`.
    pub fn layout(&self, i: usize) -> Layout {
        let n = &self.nodes[i];
        match &n.kind {
            Kind::Split(s) => Layout::Split {
                split: if s.horizontal {
                    "horizontal"
                } else {
                    "vertical"
                }
                .into(),
                ratio: s
                    .ratios
                    .iter()
                    .map(|r| (r * 1000.0).round() / 1000.0)
                    .collect(),
                children: n
                    .children
                    .iter()
                    .filter(|c| self.nodes[**c].alive)
                    .map(|c| self.layout(*c))
                    .collect(),
            },
            Kind::Pane(p) => {
                // Options belong to the type the pane was made with.
                let original = n.spec.content.as_str().unwrap_or_default().trim();
                let args = if original == p.ptype {
                    n.spec.props.args.clone().unwrap_or(Value::Null)
                } else {
                    Value::Null
                };
                Layout::Pane {
                    pane: p.ptype.clone(),
                    id: n.id.clone(),
                    args: if args.as_object().is_some_and(|o| o.is_empty()) {
                        Value::Null
                    } else {
                        args
                    },
                }
            }
            _ => Layout::Keep { keep: n.id.clone() },
        }
    }

    fn subtree(&self, i: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack = vec![i];
        while let Some(j) = stack.pop() {
            out.push(j);
            stack.extend(self.nodes[j].children.iter().copied());
        }
        out
    }

    /// Replace the splits and panes under node `root` by `layout`, re-using
    /// nodes with the same id (their content keeps running).
    pub fn apply_layout(&mut self, root: usize, layout: &Layout) -> Result<Applied> {
        let old = self.subtree(root);
        let existing: HashMap<String, usize> = old
            .iter()
            .filter(|i| self.nodes[**i].alive)
            .map(|i| (self.nodes[*i].id.clone(), *i))
            .collect();
        let parent = self.nodes[root].parent;
        let mut b = Builder {
            existing,
            reused: HashSet::new(),
            out: Applied::default(),
            made: Vec::new(),
        };
        let built = match b.build(self, layout, parent) {
            Ok(Some(n)) => n,
            Ok(None) => {
                b.undo(self);
                return Err(bad_layout("it has no panes that can be shown"));
            }
            Err(e) => {
                b.undo(self);
                return Err(e);
            }
        };
        // The new layout takes the old one's place, size and tab title.
        match parent {
            Some(p) => self.nodes[p]
                .children
                .iter_mut()
                .filter(|c| **c == root)
                .for_each(|c| *c = built),
            None => self
                .roots
                .iter_mut()
                .filter(|r| **r == root)
                .for_each(|r| *r = built),
        }
        if built != root {
            self.nodes[built].parent = parent;
            self.nodes[built].layout = self.nodes[root].layout;
            let (title, icon) = (
                self.nodes[root].spec.props.title.clone(),
                self.nodes[root].spec.props.icon.clone(),
            );
            let props = &mut self.nodes[built].spec.props;
            props.title = props.title.take().or(title);
            props.icon = props.icon.take().or(icon);
        }
        // Whatever wasn't re-used is gone (its content stops).
        let mut keep: HashSet<usize> = HashSet::new();
        for r in &b.reused {
            keep.extend(self.subtree(*r));
        }
        for i in old {
            if !keep.contains(&i) {
                self.nodes[i].alive = false;
                self.nodes[i].children.retain(|c| keep.contains(c));
            }
        }
        self.refresh_panes();
        b.out.root = built;
        Ok(b.out)
    }
}

struct Builder {
    existing: HashMap<String, usize>,
    reused: HashSet<usize>,
    out: Applied,
    /// New nodes, removed again if the layout turns out bad.
    made: Vec<usize>,
}

impl Builder {
    fn undo(&self, tree: &mut Tree) {
        for i in &self.made {
            tree.nodes[*i].alive = false;
        }
    }

    fn take(&mut self, tree: &mut Tree, id: &str, parent: Option<usize>) -> Option<usize> {
        let i = *self.existing.get(id)?;
        if !self.reused.insert(i) {
            return None; // the same id twice: only the first one is kept
        }
        tree.nodes[i].parent = parent;
        Some(i)
    }

    fn build(
        &mut self,
        tree: &mut Tree,
        layout: &Layout,
        parent: Option<usize>,
    ) -> Result<Option<usize>> {
        match layout {
            Layout::Split {
                split,
                ratio,
                children,
            } => {
                let horizontal = match split.as_str() {
                    "horizontal" | "h" | "row" => true,
                    "vertical" | "v" | "column" => false,
                    other => {
                        return Err(bad_layout(format!(
                            "split \"{other}\" should be \"horizontal\" or \"vertical\""
                        )));
                    }
                };
                let spec = NodeSpec {
                    kind: "split".into(),
                    props: Props {
                        direction: Some(if horizontal { "horizontal" } else { "vertical" }.into()),
                        ..Props::default()
                    },
                    ..NodeSpec::default()
                };
                let s = tree.insert(&spec, parent)?;
                self.made.push(s);
                let mut kids = Vec::new();
                let mut weights = Vec::new();
                for (k, child) in children.iter().enumerate() {
                    if let Some(c) = self.build(tree, child, Some(s))? {
                        kids.push(c);
                        weights.push(ratio.get(k).copied().unwrap_or(1.0).max(0.01));
                    }
                }
                match kids.len() {
                    0 => {
                        tree.nodes[s].alive = false;
                        Ok(None)
                    }
                    1 => {
                        // A split of one is just that one.
                        tree.nodes[s].alive = false;
                        tree.nodes[kids[0]].parent = parent;
                        Ok(Some(kids[0]))
                    }
                    n => {
                        let ratios = normalize_ratios(Some(&weights), n)?;
                        tree.nodes[s].children = kids;
                        if let Kind::Split(sb) = &mut tree.nodes[s].kind {
                            sb.ratios = ratios;
                        }
                        Ok(Some(s))
                    }
                }
            }
            Layout::Pane { pane, id, args } => {
                if pane.trim().is_empty() {
                    return Err(bad_layout("a pane has no type"));
                }
                if let Some(i) = (!id.is_empty())
                    .then(|| self.take(tree, id, parent))
                    .flatten()
                {
                    if let Kind::Pane(p) = &tree.nodes[i].kind
                        && p.ptype != *pane
                    {
                        self.out.switch.push((i, pane.clone()));
                    }
                    self.out.first.get_or_insert(i);
                    return Ok(Some(i));
                }
                // A new pane (its id only if nobody else has it).
                let id =
                    (!id.is_empty() && tree.find(&format!("#{id}")).is_none()).then(|| id.clone());
                let spec = NodeSpec {
                    kind: "pane".into(),
                    content: json!(pane),
                    props: Props {
                        id,
                        args: (!args.is_null()).then(|| args.clone()),
                        ..Props::default()
                    },
                    ..NodeSpec::default()
                };
                let i = tree.insert(&spec, parent)?;
                self.made.push(i);
                self.out.fresh.push((i, pane.clone(), args.clone()));
                self.out.first.get_or_insert(i);
                Ok(Some(i))
            }
            Layout::Keep { keep } => Ok(self.take(tree, keep, parent)),
        }
    }
}

// ----- files -----------------------------------------------------------------

/// Where settings live: `$XDG_CONFIG_HOME` or `~/.config` (Windows: `%APPDATA%`).
pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.unwrap_or_else(std::env::temp_dir).join("stagetui")
}

/// The default workspaces file of an app: `~/.config/stagetui/workspaces/<app>.json`.
pub fn default_file(app: &str) -> PathBuf {
    config_dir()
        .join("workspaces")
        .join(format!("{}.json", crate::ipc::slug(app)))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    version: u32,
    workspaces: BTreeMap<String, Layout>,
}

fn read(path: &Path) -> Result<File> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            Error::friendly(
                format!("{} is not a workspaces file", path.display()),
                e.to_string(),
                "delete the file (stagetui will make a new one) or fix the JSON",
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(File {
            version: 1,
            ..File::default()
        }),
        Err(e) => Err(e.into()),
    }
}

/// Save `layout` as `name` in a workspaces file (other names are kept).
pub fn save(path: &Path, name: &str, layout: &Layout) -> Result<()> {
    if name.trim().is_empty() {
        return Err(Error::friendly(
            "A workspace needs a name",
            "workspaces are saved by name so you can load them later",
            "pass a name, e.g. app.save_workspace(\"review\")",
        ));
    }
    let mut file = read(path)?;
    file.version = 1;
    file.workspaces.insert(name.to_string(), layout.clone());
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // Write next to it, then swap: a crash never leaves half a file.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(&file)? + "\n")?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Load the layout saved as `name`.
pub fn load(path: &Path, name: &str) -> Result<Layout> {
    let file = read(path)?;
    let names = || {
        if file.workspaces.is_empty() {
            "none are saved yet".to_string()
        } else {
            format!(
                "saved: {}",
                file.workspaces
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    };
    file.workspaces.get(name).cloned().ok_or_else(|| {
        Error::friendly(
            format!("No workspace named \"{name}\""),
            format!("{} ({})", names(), path.display()),
            "save one first with app.save_workspace(\"<name>\")",
        )
    })
}

/// The names saved in a workspaces file.
pub fn names(path: &Path) -> Result<Vec<String>> {
    Ok(read(path)?.workspaces.into_keys().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_read_and_write_plain_json() {
        let text = r#"{"split":"horizontal","ratio":[0.3,0.7],"children":[
            {"pane":"files","id":"left"},
            {"split":"vertical","children":[
                {"pane":"terminal","id":"sh","args":{"command":"htop"}},
                {"keep":"status"}]}]}"#;
        let layout: Layout = serde_json::from_str(text).unwrap();
        let Layout::Split { children, .. } = &layout else {
            panic!("{layout:?}")
        };
        assert_eq!(children.len(), 2);
        let again: Layout = serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
        assert_eq!(again, layout);
    }

    #[test]
    fn files_keep_every_name_and_explain_mistakes() {
        let dir = std::env::temp_dir().join(format!("stagetui-test-ws-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("workspaces").join("demo.json");
        let a = Layout::Pane {
            pane: "chat".into(),
            id: "c".into(),
            args: Value::Null,
        };
        let b = Layout::Keep {
            keep: "مرحبا-日本語-नमस्ते".into(),
        };
        save(&path, "مراجعة", &a).unwrap();
        save(&path, "agent", &b).unwrap();
        assert_eq!(load(&path, "مراجعة").unwrap(), a);
        assert_eq!(load(&path, "agent").unwrap(), b);
        assert_eq!(names(&path).unwrap(), ["agent", "مراجعة"]);
        let err = load(&path, "nope").unwrap_err().to_string();
        assert!(
            err.contains("saved: agent, مراجعة") && err.contains("fix:"),
            "{err}"
        );
        assert!(save(&path, " ", &a).is_err());
        fs::write(&path, "{ not json").unwrap();
        assert!(
            load(&path, "agent")
                .unwrap_err()
                .to_string()
                .contains("not a workspaces file")
        );
        assert!(default_file("HarnessApp").ends_with("workspaces/harness-app.json"));
        let _ = fs::remove_dir_all(&dir);
    }

    mod live {
        use super::*;
        use crate::session::Session;

        const APP: &str = r##"{"children":[
            {"type":"split","props":{"ratio":[1,2]},"children":[
                {"type":"pane","content":"chat","props":{"id":"a"},"children":[{"type":"chat","props":{"id":"talk"}}]},
                {"type":"pane","content":"markdown","props":{"id":"b","args":{"text":"# مَرْحَبًا Hello 日本語 नमस्ते"}},
                 "children":[{"type":"markdown","content":"# مَرْحَبًا Hello 日本語 नमस्ते","props":{"id":"doc"}}]}]},
            {"type":"bar","content":"status","props":{"id":"status"}}]}"##;

        fn pane_type(s: &Session, id: &str) -> String {
            s.query(&format!("#{id}")).unwrap()["pane_type"]
                .as_str()
                .unwrap()
                .to_string()
        }

        #[test]
        fn save_change_load_gives_the_same_tree_and_keeps_content() {
            let mut s = Session::from_json(APP).unwrap();
            s.frame(100, 30);
            let saved = s.workspace(None).unwrap();
            let text = serde_json::to_string(&saved).unwrap();
            assert!(text.contains(r#""ratio":[0.333,0.667]"#), "{text}");
            assert!(text.contains("नमस्ते"), "pane options are saved: {text}");

            // Rearrange like a person would: split, switch types, close.
            s.focus("#b").unwrap();
            s.press("ctrl+space").unwrap();
            s.press("-").unwrap();
            let b = s.tree().require("#b").unwrap();
            s.switch_pane(b, "log");
            s.focus("#a").unwrap();
            s.press("ctrl+space").unwrap();
            s.press("x").unwrap();
            s.frame(100, 30);
            assert!(s.query("#a").is_none());
            assert_ne!(s.workspace(None).unwrap(), saved);

            // Loading brings back the same tree; "b" is the same pane, back on markdown.
            let doc_before = s.tree().require("#doc").unwrap();
            s.load_workspace(None, &saved).unwrap();
            s.frame(100, 30);
            assert_eq!(s.workspace(None).unwrap(), saved);
            assert_eq!(pane_type(&s, "b"), "markdown");
            assert_eq!(
                s.tree().require("#doc").unwrap(),
                doc_before,
                "content kept, not rebuilt"
            );
            assert_eq!(pane_type(&s, "a"), "chat", "a closed pane comes back");
            assert!(
                s.query("#status").is_some(),
                "widgets outside the layout stay"
            );
            assert!(s.frame(100, 30).text().contains("Hello"));
        }

        #[test]
        fn new_panes_are_built_or_asked_for() {
            let mut s = Session::from_json(APP).unwrap();
            s.frame(100, 30);
            s.take_events();
            let layout: Layout = serde_json::from_str(
                r#"{"split":"vertical","children":[
                    {"pane":"log","id":"fresh"},
                    {"pane":"weather","id":"w","args":{"city":"الكويت"}},
                    {"pane":"terminal","id":"sh","args":{"command":"htop"}},
                    {"keep":"nothing-by-this-id"}]}"#,
            )
            .unwrap();
            s.load_workspace(None, &layout).unwrap();
            s.frame(100, 30);
            assert_eq!(pane_type(&s, "fresh"), "log");
            assert!(
                s.query("#a").is_none() && s.query("#talk").is_none(),
                "left-out panes close"
            );
            let asked: Vec<Value> = s
                .take_events()
                .into_iter()
                .filter(|e| e.name == "pane")
                .map(|e| json!([e.id, e.value["type"], e.value["built"], e.value["args"]]))
                .collect();
            assert_eq!(
                asked,
                [
                    json!(["w", "weather", false, {"city": "الكويت"}]),
                    json!(["sh", "terminal", false, {"command": "htop"}]),
                ],
                "the app builds custom types and panes with options"
            );
            let bad: Layout =
                serde_json::from_str(r#"{"split":"diagonal","children":[]}"#).unwrap();
            assert!(
                s.load_workspace(None, &bad)
                    .unwrap_err()
                    .to_string()
                    .contains("horizontal")
            );
            let empty: Layout = serde_json::from_str(r#"{"keep":"nope"}"#).unwrap();
            assert!(s.load_workspace(None, &empty).is_err());
            assert_eq!(
                pane_type(&s, "fresh"),
                "log",
                "a bad layout changes nothing"
            );
        }

        #[test]
        fn workspace_tabs_switch_with_the_prefix_and_keep_their_names() {
            let mut s = Session::from_json(
                r#"{"children":[{"type":"workspaces","props":{"grow":1},"children":[
                    {"type":"pane","content":"chat","props":{"id":"c","title":"Code"},"children":[{"type":"chat"}]},
                    {"type":"pane","content":"log","props":{"id":"r","title":"مراجعة"},"children":[{"type":"log"}]}]}]}"#,
            )
            .unwrap();
            let f = s.frame(80, 20);
            assert!(f.text().lines().next().unwrap().contains("Code"));
            assert!(matches!(s.workspace(None).unwrap(), Layout::Pane { ref id, .. } if id == "c"));
            s.press("ctrl+space").unwrap();
            s.press("2").unwrap();
            s.frame(80, 20);
            assert!(matches!(s.workspace(None).unwrap(), Layout::Pane { ref id, .. } if id == "r"));
            let split: Layout = serde_json::from_str(
                r#"{"split":"horizontal","children":[{"pane":"log","id":"r"},{"pane":"files","id":"f"}]}"#,
            )
            .unwrap();
            s.load_workspace(None, &split).unwrap();
            let f = s.frame(80, 20);
            assert!(
                f.text().lines().next().unwrap().contains("مراجعة"),
                "the tab keeps its name"
            );
            assert_eq!(
                s.query("#c").unwrap()["pane_type"],
                "chat",
                "other workspaces are untouched"
            );
        }
    }
}
