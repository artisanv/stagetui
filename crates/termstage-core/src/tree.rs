//! The retained widget tree and the app description that builds it.
//!
//! Python (or Rust) describes an app as an [`AppSpec`]; [`Tree::build`] validates it
//! (friendly three-part errors) and assigns stable ids that agents can query.

use std::collections::HashSet;

use ratatui_core::layout::Rect;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::gfx::{self, Fit};
use crate::style::{self, Rgb, Theme};
use crate::widgets::{Card, ImageView, Paint, TextBlock};
use crate::{Error, Result};

/// Widget types available in this milestone.
pub const WIDGET_TYPES: [&str; 3] = ["card", "text", "image"];
/// Allowed `animate=` values (Rule 3).
pub const ANIMATIONS: [&str; 6] = ["none", "fade", "slide-up", "slide-left", "pulse", "pop"];
/// Allowed `dir=` values (Rule 3).
pub const DIRECTIONS: [&str; 3] = ["auto", "ltr", "rtl"];

/// Top-level app description.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppSpec {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub accent: Option<String>,
    #[serde(default)]
    pub children: Vec<NodeSpec>,
}

fn default_theme() -> String {
    "aurora-dark".into()
}

/// One widget in the description.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSpec {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub props: Props,
}

/// `padding=` accepts a name or a cell count.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Padding {
    Named(String),
    Cells(u16),
}

/// The universal properties (Rule 3) plus `title`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Props {
    pub id: Option<String>,
    pub title: Option<String>,
    pub icon: Option<String>,
    pub theme: Option<String>,
    pub accent: Option<String>,
    pub rounded: Option<bool>,
    pub padding: Option<Padding>,
    pub dir: Option<String>,
    pub animate: Option<String>,
    /// Image size in cells (image widgets only).
    pub width: Option<u16>,
    pub height: Option<u16>,
    /// `"contain" | "cover" | "stretch"` (image widgets only).
    pub fit: Option<String>,
}

/// Concrete widget behaviour.
#[derive(Debug, Clone)]
pub enum Widget {
    Card(Card),
    Text(TextBlock),
    Image(ImageView),
}

impl Widget {
    fn as_paint(&self) -> &dyn Paint {
        match self {
            Self::Card(w) => w,
            Self::Text(w) => w,
            Self::Image(w) => w,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Card(_) => "card",
            Self::Text(_) => "text",
            Self::Image(_) => "image",
        }
    }
}

/// A node in the retained tree.
#[derive(Debug, Clone)]
pub struct Node {
    pub id: String,
    pub widget: Widget,
    pub theme: Theme,
    pub animate: String,
    pub visible: bool,
    /// Cell bounds from the last layout.
    pub bounds: Rect,
}

impl Node {
    pub fn paint(&self) -> &dyn Paint {
        self.widget.as_paint()
    }
}

/// The retained widget tree (flat in Milestone 1; nested layout arrives with `taffy` in Milestone 3).
#[derive(Debug, Clone)]
pub struct Tree {
    pub theme: Theme,
    pub nodes: Vec<Node>,
}

impl Tree {
    /// Validate a spec and build the tree.
    pub fn build(spec: &AppSpec) -> Result<Self> {
        let mut app_theme = style::theme(&spec.theme)?;
        if let Some(a) = &spec.accent {
            app_theme.accent = style::accent(a)?;
        }
        let mut nodes = Vec::with_capacity(spec.children.len());
        let mut seen = HashSet::new();
        let mut counters = std::collections::HashMap::<&str, usize>::new();
        for child in &spec.children {
            let node = build_node(child, app_theme, &mut counters)?;
            if !seen.insert(node.id.clone()) {
                return Err(Error::friendly(
                    format!("Two widgets share the id \"{}\"", node.id),
                    "ids must be unique so `app.query(\"#id\")` finds exactly one widget",
                    "give each widget a different `id=`, or leave `id` out to get an automatic one",
                ));
            }
            nodes.push(node);
        }
        Ok(Self {
            theme: app_theme,
            nodes,
        })
    }

    /// Find a node by id.
    pub fn get(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// JSON description of the tree for agents (`capture_tree`).
    pub fn to_json(&self, cols: u16, rows: u16) -> Value {
        json!({
            "type": "app",
            "size": { "cols": cols, "rows": rows },
            "theme": self.theme.name,
            "accent": self.theme.accent.hex(),
            "focus": Value::Null,
            "children": self.nodes.iter().map(|n| {
                let mut v = json!({
                    "id": n.id,
                    "type": n.widget.kind(),
                    "bounds": { "x": n.bounds.x, "y": n.bounds.y, "w": n.bounds.width, "h": n.bounds.height },
                    "visible": n.visible,
                    "on_screen": n.visible && n.bounds.width > 0 && n.bounds.height > 0,
                    "theme": n.theme.name,
                    "accent": n.theme.accent.hex(),
                    "animate": n.animate,
                });
                if let Value::Object(map) = &mut v {
                    map.extend(n.paint().describe());
                }
                v
            }).collect::<Vec<_>>(),
        })
    }
}

fn build_node(
    spec: &NodeSpec,
    app_theme: Theme,
    counters: &mut std::collections::HashMap<&str, usize>,
) -> Result<Node> {
    let kind = WIDGET_TYPES
        .iter()
        .copied()
        .find(|k| *k == spec.kind)
        .ok_or_else(|| {
            Error::friendly(
                format!("Unknown widget type \"{}\"", spec.kind),
                "this version of termstage does not have that widget",
                format!("use one of: {}", WIDGET_TYPES.join(", ")),
            )
        })?;
    let p = &spec.props;

    let mut theme = match &p.theme {
        Some(name) => style::theme(name)?,
        None => app_theme,
    };
    let accent: Rgb = match &p.accent {
        Some(a) => style::accent(a)?,
        None => app_theme.accent,
    };
    theme.accent = accent;

    let dir = p.dir.clone().unwrap_or_else(|| "auto".into());
    if !DIRECTIONS.contains(&dir.as_str()) {
        return Err(Error::friendly(
            format!("Unknown direction dir=\"{dir}\""),
            "dir controls text direction and layout mirroring",
            "use \"auto\" (detect from the text), \"ltr\" or \"rtl\"",
        ));
    }
    let animate = p.animate.clone().unwrap_or_else(|| "none".into());
    if !ANIMATIONS.contains(&animate.as_str()) {
        return Err(Error::friendly(
            format!("Unknown animation animate=\"{animate}\""),
            "animations are a fixed, curated set",
            format!("use one of: {}", ANIMATIONS.join(", ")),
        ));
    }
    let padding = padding_cells(p.padding.as_ref())?;
    let icon = match &p.icon {
        Some(name) => Some(crate::widgets::icon(name)?),
        None => None,
    };

    let content = spec.content.clone().unwrap_or_default();
    let widget = match kind {
        "card" => Widget::Card(Card {
            content,
            title: p.title.clone(),
            icon,
            rounded: p.rounded.unwrap_or(true),
            padding,
            dir,
        }),
        "image" => {
            if content.trim().is_empty() {
                return Err(Error::friendly(
                    "StageImage needs something to show",
                    "no file path, bytes or canvas was given",
                    "pass a path like StageImage(\"logo.png\")",
                ));
            }
            let fit = match &p.fit {
                Some(f) => Fit::parse(f)?,
                None => Fit::Contain,
            };
            for (name, v) in [("width", p.width), ("height", p.height)] {
                if v == Some(0) {
                    return Err(Error::friendly(
                        format!("StageImage {name}=0 is too small"),
                        "sizes are measured in terminal cells",
                        format!("use {name}=1 or more, or leave it out for the natural size"),
                    ));
                }
            }
            Widget::Image(ImageView {
                image: gfx::resolve(&content)?,
                source: content,
                width: p.width,
                height: p.height,
                fit,
            })
        }
        _ => Widget::Text(TextBlock { content, dir }),
    };

    let id = match &p.id {
        Some(id) if !id.trim().is_empty() && !id.contains(char::is_whitespace) => id.clone(),
        Some(id) => {
            return Err(Error::friendly(
                format!("Invalid id \"{id}\""),
                "ids are used in queries like app.query(\"#id\") and cannot be empty or contain spaces",
                "use a short slug such as \"chat\" or \"status-card\"",
            ));
        }
        None => {
            let n = counters.entry(kind).or_insert(0);
            *n += 1;
            format!("{kind}-{n}")
        }
    };

    Ok(Node {
        id,
        widget,
        theme,
        animate,
        visible: true,
        bounds: Rect::default(),
    })
}

/// `(horizontal, vertical)` padding in cells.
fn padding_cells(p: Option<&Padding>) -> Result<(u16, u16)> {
    Ok(match p {
        None => (2, 1),
        Some(Padding::Cells(n)) => (*n, *n),
        Some(Padding::Named(name)) => match name.as_str() {
            "compact" => (1, 0),
            "normal" => (2, 1),
            "spacious" => (4, 2),
            other => {
                return Err(Error::friendly(
                    format!("Unknown padding \"{other}\""),
                    "padding is a name or a number of cells",
                    "use \"compact\", \"normal\", \"spacious\", or an integer like 2",
                ));
            }
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(json: &str) -> AppSpec {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn auto_ids_and_defaults() {
        let t = Tree::build(&spec(r#"{"children":[{"type":"card","content":"a"},{"type":"card","content":"b"},{"type":"text","content":"c"}]}"#)).unwrap();
        let ids: Vec<_> = t.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["card-1", "card-2", "text-1"]);
        assert_eq!(t.theme.name, "aurora-dark");
        assert!(t.get("card-2").is_some());
    }

    #[test]
    fn friendly_errors() {
        let cases = [
            (
                r#"{"children":[{"type":"spinner"}]}"#,
                "Unknown widget type",
            ),
            (r#"{"theme":"neon"}"#, "Unknown theme"),
            (
                r#"{"children":[{"type":"card","props":{"accent":"plaid"}}]}"#,
                "Unknown accent",
            ),
            (
                r#"{"children":[{"type":"card","props":{"dir":"up"}}]}"#,
                "Unknown direction",
            ),
            (
                r#"{"children":[{"type":"card","props":{"animate":"spin"}}]}"#,
                "Unknown animation",
            ),
            (
                r#"{"children":[{"type":"card","props":{"padding":"huge"}}]}"#,
                "Unknown padding",
            ),
            (
                r#"{"children":[{"type":"card","props":{"icon":"unicorn"}}]}"#,
                "Unknown icon",
            ),
            (
                r#"{"children":[{"type":"card","props":{"id":"a"}},{"type":"card","props":{"id":"a"}}]}"#,
                "share the id",
            ),
            (
                r#"{"children":[{"type":"card","props":{"id":"a b"}}]}"#,
                "Invalid id",
            ),
        ];
        for (json, needle) in cases {
            let err = Tree::build(&spec(json)).unwrap_err().to_string();
            assert!(err.contains(needle), "{json} → {err}");
            assert!(err.contains("fix:"), "{err}");
        }
    }

    #[test]
    fn unknown_props_are_rejected_by_serde() {
        assert!(
            serde_json::from_str::<AppSpec>(
                r#"{"children":[{"type":"card","props":{"colour":"red"}}]}"#
            )
            .is_err()
        );
    }

    #[test]
    fn per_widget_theme_and_accent() {
        let t = Tree::build(&spec(r##"{"theme":"paper-soft","accent":"mint","children":[{"type":"card","props":{"theme":"hacker-mint","accent":"#ff0000"}},{"type":"card"}]}"##)).unwrap();
        assert_eq!(t.nodes[0].theme.name, "hacker-mint");
        assert_eq!(t.nodes[0].theme.accent, Rgb(255, 0, 0));
        assert_eq!(t.nodes[1].theme.name, "paper-soft");
        assert_eq!(t.nodes[1].theme.accent, style::accent("mint").unwrap());
    }
}
