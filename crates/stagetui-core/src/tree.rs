//! The retained widget tree and the app description that builds it.
//!
//! Python (or Rust) describes an app as an [`AppSpec`]: plain nested data.
//! [`Tree::build`] validates it (friendly three-part errors) and assigns stable
//! ids that agents can query. The tree is an arena: nodes refer to each other
//! by index, and can be updated, split and removed while the app runs.

use std::collections::{HashMap, HashSet};

use ratatui_core::layout::Rect;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::gfx::{self, Fit};
use crate::input::Key;
use crate::motion::Anim;
use crate::style::{self, Rgb, Theme};
use crate::text::{self, Direction};
use crate::widgets::{
    Bar, Button, Card, Chat, Checkbox, Code, Diff, ImageView, Input, Log, Markdown, Permission,
    Progress, Prompt, Select, Spinner, Table, Terminal, TextBlock, ToolCall, TreeItem, TreeView,
    Variant, Widget, tool_from,
};
use crate::{Error, Result};

/// Widget types, as written in app descriptions (`"type": "card"`).
pub const WIDGET_TYPES: [&str; 26] = [
    "stage",
    "split",
    "tabs",
    "modal",
    "pane",
    "bar",
    "card",
    "text",
    "image",
    "button",
    "input",
    "checkbox",
    "select",
    "progress",
    "spinner",
    "table",
    "tree",
    "markdown",
    "code",
    "diff",
    "log",
    "toolcall",
    "permission",
    "chat",
    "prompt",
    "terminal",
];
pub use crate::motion::ANIMATIONS;
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
    pub dir: Option<String>,
    #[serde(default)]
    pub bindings: Vec<BindingSpec>,
    #[serde(default)]
    pub reduce_motion: Option<bool>,
    /// The key that reaches the app from inside a shell pane (default `ctrl+space`).
    #[serde(default)]
    pub prefix: Option<String>,
    /// Pane types offered in the pane menu: `[{"name", "title", "icon"}]`.
    #[serde(default)]
    pub pane_types: Vec<PaneTypeSpec>,
    #[serde(default)]
    pub children: Vec<NodeSpec>,
}

/// A pane type offered in the pane switcher menu.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PaneTypeSpec {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    /// Built by the app (Python), not by stagetui's built-ins.
    #[serde(default)]
    pub custom: bool,
}

fn default_theme() -> String {
    "aurora-dark".into()
}

/// A keyboard shortcut: `("ctrl+s", "save", "Save")`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingSpec {
    pub key: String,
    pub action: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// One widget in the description.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSpec {
    #[serde(rename = "type")]
    pub kind: String,
    /// The primary content: text, a path, a number, a list…
    #[serde(default)]
    pub content: Value,
    #[serde(default)]
    pub props: Props,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<NodeSpec>,
    /// Opaque handle chosen by the caller (Python) to route events back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Events the caller listens to (`"click"`, `"change"`, `"frame"`…).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
}

impl NodeSpec {
    /// A node of `kind` with text content.
    pub fn text(kind: &str, content: &str) -> Self {
        Self {
            kind: kind.into(),
            content: Value::String(content.into()),
            ..Self::default()
        }
    }
}

/// `padding=` accepts a name or a cell count.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Padding {
    Named(String),
    Cells(u16),
}

/// A size: whole cells (`20`) or a share of the parent (`"50%"`).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum SizeSpec {
    Cells(u16),
    Text(String),
}

/// A resolved size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dim {
    Cells(u16),
    Percent(f32),
}

impl SizeSpec {
    fn resolve(&self, what: &str) -> Result<Dim> {
        match self {
            Self::Cells(0) => Err(Error::friendly(
                format!("{what}=0 is too small"),
                "sizes are measured in terminal cells",
                format!("use {what}=1 or more, a share like \"50%\", or leave it out"),
            )),
            Self::Cells(n) => Ok(Dim::Cells(*n)),
            Self::Text(t) => t
                .trim()
                .strip_suffix('%')
                .and_then(|n| n.trim().parse::<f32>().ok())
                .filter(|p| *p > 0.0 && *p <= 100.0)
                .map(|p| Dim::Percent(p / 100.0))
                .ok_or_else(|| {
                    Error::friendly(
                        format!("Can't read {what}=\"{t}\""),
                        "a size is a number of cells or a percentage of the space around it",
                        format!("use {what}=20 or {what}=\"50%\""),
                    )
                }),
        }
    }
}

/// Every property a widget can take. Unknown names are rejected with a clear error.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Props {
    // Universal (Rule 3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rounded: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<Padding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<bool>,
    // Layout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<SizeSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<SizeSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grow: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<usize>,
    // Widget-specific.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hints: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dismissable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<u16>,
    // Harness widgets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_numbers: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_lines: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<(String, String)>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mentions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<bool>,
}

/// How children line up across a stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    Stretch,
}

impl Align {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "start" => Self::Start,
            "center" => Self::Center,
            "end" => Self::End,
            "stretch" => Self::Stretch,
            other => {
                return Err(Error::friendly(
                    format!("Unknown align=\"{other}\""),
                    "align says how children line up across the stack",
                    "use \"start\", \"center\", \"end\" or \"stretch\"",
                ));
            }
        })
    }
}

/// A container of other widgets.
#[derive(Debug, Clone)]
pub struct StageBox {
    pub horizontal: bool,
    pub gap: u16,
    pub align: Align,
    /// Lay children out in a grid with this many columns.
    pub columns: Option<u16>,
    /// Scroll when the content is taller than the box.
    pub scroll: bool,
}

/// Panes side by side (`horizontal`) or stacked, with draggable dividers.
#[derive(Debug, Clone)]
pub struct SplitBox {
    pub horizontal: bool,
    /// One share per child.
    pub ratios: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct TabsBox {
    pub active: usize,
}

/// A Blender-style pane: shows one of several contents, chosen by type.
#[derive(Debug, Clone)]
pub struct PaneBox {
    /// The type on show (`"chat"`, `"terminal"`, …).
    pub ptype: String,
    /// The type of each child; only the child of the current type is shown,
    /// the others are kept (stashed) so switching back restores them.
    pub types: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ModalBox {
    pub title: Option<String>,
    pub icon: Option<&'static str>,
    pub dismissable: bool,
}

/// Concrete widget behaviour.
#[derive(Debug, Clone)]
pub enum Kind {
    Stage(StageBox),
    Split(SplitBox),
    Tabs(TabsBox),
    Modal(ModalBox),
    Pane(PaneBox),
    Bar(Bar),
    Card(Card),
    Text(TextBlock),
    Image(ImageView),
    Button(Button),
    Input(Input),
    Checkbox(Checkbox),
    Select(Select),
    Progress(Progress),
    Spinner(Spinner),
    Table(Table),
    Tree(TreeView),
    Markdown(Markdown),
    Code(Code),
    Diff(Diff),
    Log(Log),
    ToolCall(ToolCall),
    Permission(Permission),
    Chat(Chat),
    Prompt(Prompt),
    Terminal(Terminal),
}

impl Kind {
    pub fn widget(&self) -> Option<&dyn Widget> {
        Some(match self {
            Self::Bar(w) => w,
            Self::Card(w) => w,
            Self::Text(w) => w,
            Self::Image(w) => w,
            Self::Button(w) => w,
            Self::Input(w) => w,
            Self::Checkbox(w) => w,
            Self::Select(w) => w,
            Self::Progress(w) => w,
            Self::Spinner(w) => w,
            Self::Table(w) => w,
            Self::Tree(w) => w,
            Self::Markdown(w) => w,
            Self::Code(w) => w,
            Self::Diff(w) => w,
            Self::Log(w) => w,
            Self::ToolCall(w) => w,
            Self::Permission(w) => w,
            Self::Chat(w) => w,
            Self::Prompt(w) => w,
            Self::Terminal(w) => w,
            Self::Stage(_) | Self::Split(_) | Self::Tabs(_) | Self::Modal(_) | Self::Pane(_) => {
                return None;
            }
        })
    }

    pub fn widget_mut(&mut self) -> Option<&mut dyn Widget> {
        Some(match self {
            Self::Bar(w) => w,
            Self::Card(w) => w,
            Self::Text(w) => w,
            Self::Image(w) => w,
            Self::Button(w) => w,
            Self::Input(w) => w,
            Self::Checkbox(w) => w,
            Self::Select(w) => w,
            Self::Progress(w) => w,
            Self::Spinner(w) => w,
            Self::Table(w) => w,
            Self::Tree(w) => w,
            Self::Markdown(w) => w,
            Self::Code(w) => w,
            Self::Diff(w) => w,
            Self::Log(w) => w,
            Self::ToolCall(w) => w,
            Self::Permission(w) => w,
            Self::Chat(w) => w,
            Self::Prompt(w) => w,
            Self::Terminal(w) => w,
            Self::Stage(_) | Self::Split(_) | Self::Tabs(_) | Self::Modal(_) | Self::Pane(_) => {
                return None;
            }
        })
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Stage(_) => "stage",
            Self::Split(_) => "split",
            Self::Tabs(_) => "tabs",
            Self::Modal(_) => "modal",
            Self::Pane(_) => "pane",
            Self::Bar(_) => "bar",
            Self::Card(_) => "card",
            Self::Text(_) => "text",
            Self::Image(_) => "image",
            Self::Button(_) => "button",
            Self::Input(_) => "input",
            Self::Checkbox(_) => "checkbox",
            Self::Select(_) => "select",
            Self::Progress(_) => "progress",
            Self::Spinner(_) => "spinner",
            Self::Table(_) => "table",
            Self::Tree(_) => "tree",
            Self::Markdown(_) => "markdown",
            Self::Code(_) => "code",
            Self::Diff(_) => "diff",
            Self::Log(_) => "log",
            Self::ToolCall(_) => "toolcall",
            Self::Permission(_) => "permission",
            Self::Chat(_) => "chat",
            Self::Prompt(_) => "prompt",
            Self::Terminal(_) => "terminal",
        }
    }

    pub fn is_container(&self) -> bool {
        self.widget().is_none()
    }
}

/// Layout requests of one node.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LayoutProps {
    pub width: Option<Dim>,
    pub height: Option<Dim>,
    pub grow: Option<f32>,
}

/// A node in the retained tree.
#[derive(Debug, Clone)]
pub struct Node {
    pub id: String,
    pub key: Option<String>,
    pub kind: Kind,
    /// The description it was built from (children excluded), for updates.
    pub spec: NodeSpec,
    pub theme: Theme,
    pub animate: Anim,
    pub visible: bool,
    pub layout: LayoutProps,
    /// `auto`, `ltr` or `rtl` as given.
    pub dir: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub alive: bool,
    pub events: HashSet<String>,
    /// Position from the last layout (may lie partly off screen).
    pub rect: IRect,
    /// The visible part of `rect` from the last frame.
    pub bounds: Rect,
    /// Resolved layout direction.
    pub rtl: bool,
    /// Scroll offset (rows) for scrolling containers.
    pub scroll: u16,
    /// Content height (rows) from the last layout, for scrolling containers.
    pub content_h: u16,
    /// When the node was first shown (entrance animations start here).
    pub born: Option<std::time::Duration>,
}

impl Node {
    pub fn widget(&self) -> Option<&dyn Widget> {
        self.kind.widget()
    }

    /// The scrolling-related geometry (a cheap copy).
    pub fn clone_geometry(&self) -> Geometry {
        Geometry {
            rect: self.rect,
            scroll: self.scroll,
            content_h: self.content_h,
            parent: self.parent,
            scrolls: matches!(self.kind, Kind::Stage(_) | Kind::Modal(_))
                && self.content_h > self.rect.h,
        }
    }
}

/// See [`Node::clone_geometry`].
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub rect: IRect,
    pub scroll: u16,
    pub content_h: u16,
    pub parent: Option<usize>,
    pub scrolls: bool,
}

/// A rectangle that may start off screen (negative) while scrolling or sliding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub w: u16,
    pub h: u16,
}

impl IRect {
    pub fn right(&self) -> i32 {
        self.x + i32::from(self.w)
    }

    pub fn bottom(&self) -> i32 {
        self.y + i32::from(self.h)
    }

    /// The part inside `clip`.
    pub fn clip(&self, clip: Rect) -> Rect {
        let x0 = self.x.max(i32::from(clip.x));
        let y0 = self.y.max(i32::from(clip.y));
        let x1 = self.right().min(i32::from(clip.right()));
        let y1 = self.bottom().min(i32::from(clip.bottom()));
        if x1 <= x0 || y1 <= y0 {
            return Rect::new(clip.x, clip.y, 0, 0);
        }
        Rect::new(x0 as u16, y0 as u16, (x1 - x0) as u16, (y1 - y0) as u16)
    }

    pub fn contains(&self, x: u16, y: u16) -> bool {
        let (x, y) = (i32::from(x), i32::from(y));
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// A resolved keyboard shortcut.
#[derive(Debug, Clone)]
pub struct Binding {
    pub key: Key,
    pub action: String,
    pub description: Option<String>,
}

/// The retained widget tree.
#[derive(Debug, Clone)]
pub struct Tree {
    pub theme: Theme,
    /// App accent override (kept across theme switches).
    pub accent: Option<Rgb>,
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
    /// The app's resolved layout direction.
    pub dir: Direction,
    pub bindings: Vec<Binding>,
    pub reduce_motion: bool,
    /// Pane types offered in the pane menu.
    pub pane_types: Vec<PaneTypeSpec>,
    counters: HashMap<&'static str, usize>,
}

impl Tree {
    /// Validate a spec and build the tree.
    pub fn build(spec: &AppSpec) -> Result<Self> {
        let mut theme = style::theme(&spec.theme)?;
        let accent = spec.accent.as_deref().map(style::accent).transpose()?;
        if let Some(a) = accent {
            theme.accent = a;
        }
        let bindings = spec
            .bindings
            .iter()
            .map(|b| {
                if b.action.trim().is_empty() {
                    return Err(Error::friendly(
                        format!("The shortcut \"{}\" has no action", b.key),
                        "each binding is (key, action) or (key, action, description)",
                        "write it like (\"ctrl+s\", \"save\", \"Save\")",
                    ));
                }
                Ok(Binding {
                    key: Key::parse(&b.key)?,
                    action: b.action.clone(),
                    description: b.description.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut tree = Self {
            theme,
            accent,
            nodes: Vec::new(),
            roots: Vec::new(),
            dir: Direction::Ltr,
            bindings,
            reduce_motion: spec.reduce_motion.unwrap_or(false)
                || std::env::var("STAGETUI_REDUCE_MOTION").is_ok_and(|v| v == "1"),
            pane_types: if spec.pane_types.is_empty() {
                crate::panes::builtin_types()
            } else {
                spec.pane_types.clone()
            },
            counters: HashMap::new(),
        };
        for child in &spec.children {
            let idx = tree.insert(child, None)?;
            tree.roots.push(idx);
        }
        tree.dir = match spec.dir.as_deref().unwrap_or("auto") {
            "rtl" => Direction::Rtl,
            "ltr" => Direction::Ltr,
            "auto" => text::first_strong(&tree.all_text()),
            other => return Err(bad_dir(other)),
        };
        tree.refresh_panes();
        Ok(tree)
    }

    /// All text in reading order (used to guess the app's direction).
    fn all_text(&self) -> String {
        let mut out = String::new();
        let mut stack: Vec<usize> = self.roots.iter().rev().copied().collect();
        while let Some(i) = stack.pop() {
            let n = &self.nodes[i];
            if let Some(t) = &n.spec.props.title {
                out.push_str(t);
                out.push('\n');
            }
            if let Value::String(s) = &n.spec.content
                && n.kind.name() != "image"
            {
                out.push_str(s);
                out.push('\n');
            }
            stack.extend(n.children.iter().rev());
        }
        out
    }

    /// Add `spec` (and its children) under `parent`. Returns the new index.
    pub fn insert(&mut self, spec: &NodeSpec, parent: Option<usize>) -> Result<usize> {
        let mut own = spec.clone();
        own.children.clear();
        let node = self.build_node(&own, parent)?;
        if self.find_id(&node.id).is_some() {
            return Err(Error::friendly(
                format!("Two widgets share the id \"{}\"", node.id),
                "ids must be unique so `app.query(\"#id\")` finds exactly one widget",
                "give each widget a different `id=`, or leave `id` out to get an automatic one",
            ));
        }
        let idx = self.nodes.len();
        self.nodes.push(node);
        if !spec.children.is_empty() && !self.nodes[idx].kind.is_container() {
            return Err(Error::friendly(
                format!("A {} can't hold other widgets", self.nodes[idx].kind.name()),
                "only Stage, StageSplit, StageTabs and StageModal contain children",
                "wrap the widgets in a Stage([...]) instead",
            ));
        }
        for child in &spec.children {
            let c = self.insert(child, Some(idx))?;
            self.nodes[idx].children.push(c);
        }
        if let Kind::Split(s) = &mut self.nodes[idx].kind {
            s.ratios = normalize_ratios(spec.props.ratio.as_deref(), spec.children.len())?;
        }
        if let Kind::Pane(p) = &mut self.nodes[idx].kind {
            // Children described in the app are the pane's current content.
            p.types = vec![p.ptype.clone(); spec.children.len()];
        }
        Ok(idx)
    }

    /// Whether node `i` is a Blender-style pane.
    pub fn is_typed_pane(&self, i: usize) -> bool {
        matches!(self.nodes[i].kind, Kind::Pane(_))
    }

    fn build_node(&mut self, spec: &NodeSpec, parent: Option<usize>) -> Result<Node> {
        let kind_name = WIDGET_TYPES
            .iter()
            .copied()
            .find(|k| *k == spec.kind)
            .ok_or_else(|| {
                Error::friendly(
                    format!("Unknown widget type \"{}\"", spec.kind),
                    "this version of stagetui does not have that widget",
                    format!("use one of: {}", WIDGET_TYPES.join(", ")),
                )
            })?;
        let p = &spec.props;
        let base = match parent {
            Some(i) => self.nodes[i].theme,
            None => self.theme,
        };
        let theme = self.resolve_theme(p, base)?;
        let dir = p.dir.clone().unwrap_or_else(|| "auto".into());
        if !DIRECTIONS.contains(&dir.as_str()) {
            return Err(bad_dir(&dir));
        }
        let animate = Anim::parse(p.animate.as_deref().unwrap_or("none"))?;
        let kind = build_kind(kind_name, spec)?;
        let layout = LayoutProps {
            width: match (&kind, &p.width) {
                (Kind::Image(_), _) | (_, None) => None,
                (_, Some(w)) => Some(w.resolve("width")?),
            },
            height: match (&kind, &p.height) {
                (
                    Kind::Image(_)
                    | Kind::Table(_)
                    | Kind::Tree(_)
                    | Kind::Markdown(_)
                    | Kind::Code(_)
                    | Kind::Diff(_)
                    | Kind::Log(_)
                    | Kind::Chat(_)
                    | Kind::Terminal(_),
                    _,
                )
                | (_, None) => None,
                (_, Some(h)) => Some(h.resolve("height")?),
            },
            grow: p.grow.filter(|g| *g > 0.0),
        };
        let id = match &p.id {
            Some(id)
                if !id.trim().is_empty()
                    && !id.contains(char::is_whitespace)
                    && !id.starts_with('#') =>
            {
                id.clone()
            }
            Some(id) => {
                return Err(Error::friendly(
                    format!("Invalid id \"{id}\""),
                    "ids are used in queries like app.query(\"#id\") and cannot be empty or contain spaces",
                    "use a short slug such as \"chat\" or \"status-card\"",
                ));
            }
            None => loop {
                let n = self.counters.entry(kind_name).or_insert(0);
                *n += 1;
                let candidate = format!("{kind_name}-{n}");
                if self.find_id(&candidate).is_none() {
                    break candidate;
                }
            },
        };
        let visible = p.visible.unwrap_or(!matches!(kind, Kind::Modal(_)));
        Ok(Node {
            id,
            key: spec.key.clone(),
            kind,
            spec: spec.clone(),
            theme,
            animate,
            visible,
            layout,
            dir,
            parent,
            children: Vec::new(),
            alive: true,
            events: spec.events.iter().cloned().collect(),
            rect: IRect::default(),
            bounds: Rect::default(),
            rtl: false,
            scroll: 0,
            content_h: 0,
            born: None,
        })
    }

    fn resolve_theme(&self, p: &Props, base: Theme) -> Result<Theme> {
        let mut theme = match &p.theme {
            Some(name) => {
                let mut t = style::theme(name)?;
                t.accent = self.accent.unwrap_or(t.accent);
                t
            }
            None => base,
        };
        if let Some(a) = &p.accent {
            theme.accent = style::accent(a)?;
        }
        Ok(theme)
    }

    /// Switch the app theme while running (widgets with their own theme keep it).
    pub fn set_theme(&mut self, name: &str) -> Result<()> {
        let mut theme = style::theme(name)?;
        if let Some(a) = self.accent {
            theme.accent = a;
        }
        self.theme = theme;
        let order = self.preorder();
        for i in order {
            let base = match self.nodes[i].parent {
                Some(p) => self.nodes[p].theme,
                None => self.theme,
            };
            let props = self.nodes[i].spec.props.clone();
            self.nodes[i].theme = self.resolve_theme(&props, base)?;
        }
        Ok(())
    }

    /// Live nodes in tree order.
    pub fn preorder(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack: Vec<usize> = self.roots.iter().rev().copied().collect();
        while let Some(i) = stack.pop() {
            if !self.nodes[i].alive {
                continue;
            }
            out.push(i);
            stack.extend(self.nodes[i].children.iter().rev());
        }
        out
    }

    fn find_id(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.alive && n.id == id)
    }

    /// Find a node by `"#id"`, caller key, or plain id.
    pub fn find(&self, target: &str) -> Option<usize> {
        if let Some(id) = target.strip_prefix('#') {
            return self.find_id(id);
        }
        self.nodes
            .iter()
            .position(|n| n.alive && n.key.as_deref() == Some(target))
            .or_else(|| self.find_id(target))
    }

    /// Like [`Tree::find`], with a friendly error.
    pub fn require(&self, target: &str) -> Result<usize> {
        self.find(target).ok_or_else(|| {
            Error::friendly(
                format!("No widget matches \"{target}\""),
                "queries look widgets up by id, like \"#chat\"",
                "check the id= you gave the widget (ids are case-sensitive)",
            )
        })
    }

    /// Find a node by id (convenience for tests and tools).
    pub fn get(&self, id: &str) -> Option<&Node> {
        self.find_id(id).map(|i| &self.nodes[i])
    }

    /// Apply `{"content": ..., "props": {...}}` to a node, keeping its state.
    pub fn update(&mut self, idx: usize, patch: &Value) -> Result<()> {
        let Value::Object(patch) = patch else {
            return Err(bad_update());
        };
        let mut spec = self.nodes[idx].spec.clone();
        for (k, v) in patch {
            match k.as_str() {
                "content" => spec.content = v.clone(),
                "props" => {
                    let mut current = serde_json::to_value(&spec.props)?;
                    if let (Value::Object(cur), Value::Object(new)) = (&mut current, v) {
                        for (pk, pv) in new {
                            if pv.is_null() {
                                cur.remove(pk);
                            } else {
                                cur.insert(pk.clone(), pv.clone());
                            }
                        }
                    }
                    spec.props = serde_json::from_value(current).map_err(|e| {
                        Error::friendly(
                            "That widget property doesn't exist",
                            e.to_string(),
                            "check the spelling; see docs/reference.md for every property",
                        )
                    })?;
                }
                _ => return Err(bad_update()),
            }
        }
        if spec.props.id != self.nodes[idx].spec.props.id {
            return Err(Error::friendly(
                "A widget's id can't change",
                "ids are how the app and agents find widgets",
                "create a new widget with the new id instead",
            ));
        }
        let parent = self.nodes[idx].parent;
        let mut fresh = self.build_node(&spec, parent)?;
        let old = &self.nodes[idx];
        carry_state(&mut fresh.kind, &old.kind, &spec);
        if let (Kind::Split(new), Kind::Split(old_split)) = (&mut fresh.kind, &old.kind) {
            new.ratios = if spec.props.ratio.is_some() {
                normalize_ratios(spec.props.ratio.as_deref(), old.children.len())?
            } else {
                old_split.ratios.clone()
            };
        }
        let n = &mut self.nodes[idx];
        n.kind = fresh.kind;
        n.spec = spec;
        n.theme = fresh.theme;
        n.layout = fresh.layout;
        n.dir = fresh.dir;
        if n.animate != fresh.animate {
            n.born = None;
        }
        n.animate = fresh.animate;
        if let Some(v) = n.spec.props.visible {
            n.visible = v;
        }
        // Children follow a theme change.
        let order = self.preorder();
        for i in order {
            if let Some(p) = self.nodes[i].parent
                && self.is_descendant(i, idx)
            {
                let props = self.nodes[i].spec.props.clone();
                self.nodes[i].theme = self.resolve_theme(&props, self.nodes[p].theme)?;
            }
        }
        self.refresh_panes();
        Ok(())
    }

    fn is_descendant(&self, mut i: usize, ancestor: usize) -> bool {
        while let Some(p) = self.nodes[i].parent {
            if p == ancestor {
                return true;
            }
            i = p;
        }
        false
    }

    /// Split `idx` with a new widget: side by side (`horizontal`) or stacked.
    pub fn split(&mut self, idx: usize, spec: &NodeSpec, horizontal: bool) -> Result<usize> {
        let parent = self.nodes[idx].parent;
        // Already inside a split going the same way: add a sibling pane.
        if let Some(p) = parent
            && let Kind::Split(s) = &self.nodes[p].kind
            && s.horizontal == horizontal
        {
            let pos = self.nodes[p]
                .children
                .iter()
                .position(|c| *c == idx)
                .unwrap_or(0);
            let new = self.insert(spec, Some(p))?;
            self.nodes[p].children.insert(pos + 1, new);
            if let Kind::Split(s) = &mut self.nodes[p].kind {
                let share = s.ratios.get(pos).copied().unwrap_or(1.0) / 2.0;
                if let Some(r) = s.ratios.get_mut(pos) {
                    *r = share;
                }
                s.ratios.insert(pos + 1, share);
            }
            self.refresh_panes();
            return Ok(new);
        }
        // Otherwise wrap `idx` in a new split.
        let split_spec = NodeSpec {
            kind: "split".into(),
            props: Props {
                direction: Some(if horizontal { "horizontal" } else { "vertical" }.into()),
                ..Props::default()
            },
            ..NodeSpec::default()
        };
        let split = self.insert(&split_spec, parent)?;
        // The split takes idx's place.
        match parent {
            Some(p) => {
                for c in &mut self.nodes[p].children {
                    if *c == idx {
                        *c = split;
                    }
                }
            }
            None => {
                for r in &mut self.roots {
                    if *r == idx {
                        *r = split;
                    }
                }
            }
        }
        // Size requests move to the split so the layout around it doesn't change.
        let layout = std::mem::take(&mut self.nodes[idx].layout);
        self.nodes[split].layout = LayoutProps {
            grow: layout.grow.or(Some(1.0)),
            ..layout
        };
        self.nodes[idx].parent = Some(split);
        let new = self.insert(spec, Some(split))?;
        self.nodes[split].children = vec![idx, new];
        if let Kind::Split(s) = &mut self.nodes[split].kind {
            s.ratios = vec![0.5, 0.5];
        }
        self.refresh_panes();
        Ok(new)
    }

    /// Remove a widget. A split left with one pane is replaced by that pane.
    pub fn remove(&mut self, idx: usize) {
        let parent = self.nodes[idx].parent;
        let mut stack = vec![idx];
        while let Some(i) = stack.pop() {
            self.nodes[i].alive = false;
            stack.extend(self.nodes[i].children.iter().copied());
        }
        match parent {
            None => self.roots.retain(|r| *r != idx),
            Some(p) => {
                let pos = self.nodes[p].children.iter().position(|c| *c == idx);
                self.nodes[p].children.retain(|c| *c != idx);
                if let (Kind::Pane(pb), Some(pos)) = (&mut self.nodes[p].kind, pos)
                    && pos < pb.types.len()
                {
                    pb.types.remove(pos);
                }
                if let (Kind::Split(s), Some(pos)) = (&mut self.nodes[p].kind, pos) {
                    if pos < s.ratios.len() {
                        s.ratios.remove(pos);
                    }
                    let sum: f32 = s.ratios.iter().sum();
                    if sum > 0.0 {
                        s.ratios.iter_mut().for_each(|r| *r /= sum);
                    }
                }
                if matches!(self.nodes[p].kind, Kind::Split(_)) && self.nodes[p].children.len() == 1
                {
                    self.collapse(p);
                }
            }
        }
        self.refresh_panes();
    }

    /// Replace a one-pane split by its only child.
    fn collapse(&mut self, split: usize) {
        let only = self.nodes[split].children[0];
        let grand = self.nodes[split].parent;
        self.nodes[only].parent = grand;
        if self.nodes[only].layout == LayoutProps::default() {
            self.nodes[only].layout = self.nodes[split].layout;
        }
        match grand {
            None => {
                for r in &mut self.roots {
                    if *r == split {
                        *r = only;
                    }
                }
            }
            Some(g) => {
                for c in &mut self.nodes[g].children {
                    if *c == split {
                        *c = only;
                    }
                }
            }
        }
        self.nodes[split].alive = false;
        self.nodes[split].children.clear();
    }

    /// Cards directly inside a split or tabs are panes: they take focus.
    fn refresh_panes(&mut self) {
        for i in 0..self.nodes.len() {
            let pane = self.is_pane(i);
            if let Kind::Card(c) = &mut self.nodes[i].kind {
                c.selectable = pane;
            }
        }
    }

    /// Whether node `i` is a direct child of a split or tabs.
    pub fn is_pane(&self, i: usize) -> bool {
        self.nodes[i]
            .parent
            .is_some_and(|p| matches!(self.nodes[p].kind, Kind::Split(_) | Kind::Tabs(_)))
    }

    /// JSON description of the tree for agents (`capture_tree`).
    pub fn to_json(&self, cols: u16, rows: u16, focus: Option<usize>) -> Value {
        json!({
            "type": "app",
            "size": { "cols": cols, "rows": rows },
            "theme": self.theme.name,
            "accent": self.theme.accent.hex(),
            "dir": if self.dir == Direction::Rtl { "rtl" } else { "ltr" },
            "focus": focus.map(|f| self.nodes[f].id.clone()),
            "children": self.roots.iter().filter(|r| self.nodes[**r].alive).map(|r| self.node_json(*r, focus)).collect::<Vec<_>>(),
        })
    }

    fn node_json(&self, i: usize, focus: Option<usize>) -> Value {
        let n = &self.nodes[i];
        let b = n.bounds;
        let mut v = json!({
            "id": n.id,
            "type": n.kind.name(),
            "bounds": { "x": b.x, "y": b.y, "w": b.width, "h": b.height },
            "visible": n.visible,
            "on_screen": n.visible && b.width > 0 && b.height > 0,
            "focused": focus == Some(i),
            "theme": n.theme.name,
            "accent": n.theme.accent.hex(),
            "animate": n.animate.name(),
        });
        if let Value::Object(map) = &mut v {
            if let Some(w) = n.widget() {
                map.extend(w.describe());
            }
            match &n.kind {
                Kind::Stage(s) => {
                    map.insert(
                        "direction".into(),
                        json!(if s.horizontal {
                            "horizontal"
                        } else {
                            "vertical"
                        }),
                    );
                    if n.content_h > n.rect.h {
                        map.insert(
                            "scroll".into(),
                            json!({ "offset": n.scroll, "content_rows": n.content_h }),
                        );
                    }
                }
                Kind::Split(s) => {
                    map.insert(
                        "direction".into(),
                        json!(if s.horizontal {
                            "horizontal"
                        } else {
                            "vertical"
                        }),
                    );
                    map.insert(
                        "ratio".into(),
                        json!(
                            s.ratios
                                .iter()
                                .map(|r| (r * 1000.0).round() / 1000.0)
                                .collect::<Vec<_>>()
                        ),
                    );
                }
                Kind::Tabs(t) => {
                    map.insert("active".into(), json!(t.active));
                }
                Kind::Modal(m) => {
                    map.insert("title".into(), json!(m.title));
                }
                Kind::Pane(p) => {
                    map.insert("pane_type".into(), json!(p.ptype));
                    let stashed: Vec<&String> = p.types.iter().filter(|t| **t != p.ptype).collect();
                    map.insert("stashed".into(), json!(stashed));
                }
                _ => {}
            }
            if let Some(t) = &n.spec.props.title
                && !map.contains_key("title")
            {
                map.insert("title".into(), json!(t));
            }
            let kids: Vec<Value> = n
                .children
                .iter()
                .filter(|c| self.nodes[**c].alive)
                .map(|c| self.node_json(*c, focus))
                .collect();
            if !kids.is_empty() {
                map.insert("children".into(), Value::Array(kids));
            }
        }
        v
    }
}

fn bad_dir(dir: &str) -> Error {
    Error::friendly(
        format!("Unknown direction dir=\"{dir}\""),
        "dir controls text direction and layout mirroring",
        "use \"auto\" (detect from the text), \"ltr\" or \"rtl\"",
    )
}

fn bad_update() -> Error {
    Error::friendly(
        "Can't apply that update",
        "an update changes a widget's content or its properties",
        "call widget.update(content=..., title=..., ...) with known property names",
    )
}

fn normalize_ratios(given: Option<&[f32]>, n: usize) -> Result<Vec<f32>> {
    if n == 0 {
        return Ok(Vec::new());
    }
    let r: Vec<f32> = match given {
        None => vec![1.0; n],
        Some(r) if r.len() == n && r.iter().all(|v| *v > 0.0 && v.is_finite()) => r.to_vec(),
        Some(r) => {
            return Err(Error::friendly(
                format!("ratio has {} numbers for {n} panes", r.len()),
                "a split needs one positive share per pane",
                format!("give {n} numbers, e.g. ratio=[{}]", vec!["1"; n].join(", ")),
            ));
        }
    };
    let sum: f32 = r.iter().sum();
    Ok(r.into_iter().map(|v| v / sum).collect())
}

fn cells_prop(s: &Option<SizeSpec>) -> Option<u16> {
    match s {
        Some(SizeSpec::Cells(n)) => Some(*n),
        _ => None,
    }
}

fn text_content(spec: &NodeSpec) -> String {
    match &spec.content {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn bad_content(widget: &str, want: &str, example: &str) -> Error {
    Error::friendly(
        format!("{widget} got content it can't use"),
        format!("{widget} expects {want}"),
        format!("write it like {example}"),
    )
}

fn strings(v: &Value) -> Option<Vec<String>> {
    v.as_array().map(|a| {
        a.iter()
            .map(|x| match x {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            })
            .collect()
    })
}

/// `(horizontal, vertical)` padding in cells.
fn padding_cells(p: Option<&Padding>) -> Result<(u16, u16)> {
    Ok(match p {
        None => (2, 1),
        Some(Padding::Cells(n)) => (*n, (*n).min(2)),
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

fn direction(p: &Props, default_horizontal: bool) -> Result<bool> {
    match p.direction.as_deref() {
        None => Ok(default_horizontal),
        Some("horizontal" | "row") => Ok(true),
        Some("vertical" | "column") => Ok(false),
        Some(other) => Err(Error::friendly(
            format!("Unknown direction=\"{other}\""),
            "direction says which way children are placed",
            "use \"horizontal\" (side by side) or \"vertical\" (stacked)",
        )),
    }
}

fn build_kind(kind: &str, spec: &NodeSpec) -> Result<Kind> {
    let p = &spec.props;
    let icon = p.icon.as_deref().map(crate::widgets::icon).transpose()?;
    let dir = p.dir.clone().unwrap_or_else(|| "auto".into());
    Ok(match kind {
        "stage" => {
            let horizontal = direction(p, false)?;
            Kind::Stage(StageBox {
                horizontal,
                gap: p.gap.unwrap_or(if horizontal { 2 } else { 1 }),
                // Stacks keep natural widths; rows and grids line heights up.
                align: p.align.as_deref().map(Align::parse).transpose()?.unwrap_or(
                    if horizontal || p.columns.is_some() {
                        Align::Stretch
                    } else {
                        Align::Start
                    },
                ),
                columns: p.columns.filter(|c| *c > 0),
                scroll: p.height.is_some(),
            })
        }
        "split" => Kind::Split(SplitBox {
            horizontal: direction(p, true)?,
            ratios: Vec::new(),
        }),
        "tabs" => Kind::Tabs(TabsBox {
            active: p.active.unwrap_or(0),
        }),
        "modal" => Kind::Modal(ModalBox {
            title: p.title.clone(),
            icon,
            dismissable: p.dismissable.unwrap_or(true),
        }),
        "bar" => Kind::Bar(Bar {
            text: text_content(spec),
            hints: p.hints.unwrap_or(true),
        }),
        "card" => {
            let mut c = Card::new(text_content(spec));
            c.title = p.title.clone();
            c.icon = icon;
            c.rounded = p.rounded.unwrap_or(true);
            c.padding = padding_cells(p.padding.as_ref())?;
            c.dir = dir;
            c.shadow = p.shadow.unwrap_or(false);
            Kind::Card(c)
        }
        "text" => Kind::Text(TextBlock {
            content: text_content(spec),
            dir,
        }),
        "image" => {
            let content = text_content(spec);
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
            let cells = |s: &Option<SizeSpec>, what: &str| -> Result<Option<u16>> {
                match s {
                    None => Ok(None),
                    Some(SizeSpec::Cells(0)) | Some(SizeSpec::Text(_)) => Err(Error::friendly(
                        format!("StageImage {what} must be a number of cells"),
                        "sizes are measured in terminal cells",
                        format!("use {what}=1 or more, or leave it out for the natural size"),
                    )),
                    Some(SizeSpec::Cells(n)) => Ok(Some(*n)),
                }
            };
            Kind::Image(ImageView {
                image: gfx::resolve(&content)?,
                source: content,
                width: cells(&p.width, "width")?,
                height: cells(&p.height, "height")?,
                fit,
            })
        }
        "button" => Kind::Button(Button {
            label: text_content(spec),
            icon,
            variant: p
                .variant
                .as_deref()
                .map(Variant::parse)
                .transpose()?
                .unwrap_or(Variant::Secondary),
        }),
        "input" => {
            let mut i = Input::new(text_content(spec));
            i.placeholder = p.placeholder.clone().unwrap_or_default();
            i.title = p.title.clone();
            i.password = p.password.unwrap_or(false);
            i.dir = dir;
            i.width = match &p.width {
                Some(SizeSpec::Cells(n)) => Some(*n),
                _ => None,
            };
            Kind::Input(i)
        }
        "checkbox" => Kind::Checkbox(Checkbox {
            label: text_content(spec),
            checked: p.checked.unwrap_or(false),
        }),
        "select" => {
            let options = strings(&spec.content).ok_or_else(|| {
                bad_content(
                    "StageSelect",
                    "a list of options",
                    "StageSelect([\"Small\", \"Large\"])",
                )
            })?;
            let selected = match &p.value {
                Some(Value::String(v)) => options.iter().position(|o| o == v).unwrap_or(0),
                Some(Value::Number(n)) => n.as_u64().unwrap_or(0) as usize,
                _ => 0,
            };
            let mut s = Select::new(options, selected);
            s.title = p.title.clone();
            s.width = match &p.width {
                Some(SizeSpec::Cells(n)) => Some(*n),
                _ => None,
            };
            Kind::Select(s)
        }
        "progress" => {
            let value = match &spec.content {
                Value::Null => None,
                Value::Number(n) => Some(n.as_f64().unwrap_or(0.0) as f32),
                _ => {
                    return Err(bad_content(
                        "StageProgress",
                        "a number from 0 to 1, or None while unknown",
                        "StageProgress(0.4, label=\"Downloading\")",
                    ));
                }
            };
            Kind::Progress(Progress {
                value,
                label: p.label.clone().or_else(|| p.title.clone()),
                width: match &p.width {
                    Some(SizeSpec::Cells(n)) => Some(*n),
                    _ => None,
                },
            })
        }
        "spinner" => Kind::Spinner(Spinner {
            label: text_content(spec),
        }),
        "table" => {
            let rows = match &spec.content {
                Value::Null => Vec::new(),
                Value::Array(rows) => rows
                    .iter()
                    .map(|r| strings(r).ok_or(()))
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|_| {
                        bad_content(
                            "StageTable",
                            "a list of rows, each a list of cells",
                            "StageTable([[\"Ada\", 36], [\"Linus\", 54]], headers=[\"Name\", \"Age\"])",
                        )
                    })?,
                _ => {
                    return Err(bad_content(
                        "StageTable",
                        "a list of rows, each a list of cells",
                        "StageTable([[\"Ada\", 36]], headers=[\"Name\", \"Age\"])",
                    ));
                }
            };
            Kind::Table(Table {
                headers: p.headers.clone().unwrap_or_default(),
                rows,
                selected: None,
                scroll: 0,
                height: match &p.height {
                    Some(SizeSpec::Cells(n)) => Some(*n),
                    _ => None,
                },
            })
        }
        "tree" => {
            let items = match &spec.content {
                Value::Null => Vec::new(),
                Value::Array(a) => a.iter().map(TreeItem::parse).collect::<Result<_>>()?,
                other => vec![TreeItem::parse(other)?],
            };
            Kind::Tree(TreeView {
                items,
                selected: 0,
                scroll: 0,
                height: match &p.height {
                    Some(SizeSpec::Cells(n)) => Some(*n),
                    _ => None,
                },
            })
        }
        "pane" => Kind::Pane(PaneBox {
            ptype: text_content(spec)
                .trim()
                .to_string()
                .chars()
                .take(64)
                .collect::<String>(),
            types: Vec::new(),
        }),
        "markdown" => {
            let mut m = Markdown::new(text_content(spec));
            m.height = cells_prop(&p.height);
            Kind::Markdown(m)
        }
        "code" => {
            let mut c = Code::new(text_content(spec));
            c.language = p.language.clone();
            c.title = p.title.clone().or_else(|| p.path.clone());
            c.line_numbers = p.line_numbers.unwrap_or(true);
            c.height = cells_prop(&p.height);
            Kind::Code(c)
        }
        "diff" => {
            let (old, new) = match &spec.content {
                Value::Null => (String::new(), String::new()),
                Value::Array(a) if a.len() == 2 => (
                    a[0].as_str().unwrap_or_default().to_string(),
                    a[1].as_str().unwrap_or_default().to_string(),
                ),
                Value::Object(o) => (
                    o.get("old")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    o.get("new")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                ),
                _ => {
                    return Err(bad_content(
                        "StageDiff",
                        "the old and new text, as a pair",
                        "StageDiff((old_text, new_text), path=\"app.py\")",
                    ));
                }
            };
            let mut d = Diff::new(old, new, p.path.clone().or_else(|| p.title.clone()));
            d.height = cells_prop(&p.height);
            Kind::Diff(d)
        }
        "log" => {
            let mut l = Log::new(&text_content(spec), p.max_lines.unwrap_or(5000) as usize);
            l.filter = p.filter.clone().unwrap_or_default();
            l.height = cells_prop(&p.height);
            Kind::Log(l)
        }
        "toolcall" => {
            let mut t = tool_from(&serde_json::json!({ "name": text_content(spec) }));
            t.args = p.args.clone().unwrap_or(Value::Null);
            t.status = p.status.clone().unwrap_or_else(|| "running".into());
            t.output = p.output.clone().unwrap_or_default();
            t.expanded = p.expanded.unwrap_or(false);
            Kind::ToolCall(t)
        }
        "permission" => Kind::Permission(Permission {
            boxed: p.border.unwrap_or(true),
            question: text_content(spec),
            tool: p.tool.clone(),
            args: p.args.clone().unwrap_or(Value::Null),
            choice: 0,
            decided: None,
        }),
        "chat" => {
            let mut c = Chat::new(&spec.content)?;
            c.height = cells_prop(&p.height);
            Kind::Chat(c)
        }
        "prompt" => {
            let mut pr = Prompt::new(&text_content(spec));
            pr.placeholder = p.placeholder.clone().unwrap_or_default();
            pr.title = p.title.clone();
            pr.commands = p.commands.clone().unwrap_or_default();
            pr.mentions = p.mentions.clone();
            pr.max_lines = p.max_lines.map_or(8, |m| m.clamp(1, 50) as u16);
            Kind::Prompt(pr)
        }
        "terminal" => {
            let command = match &spec.content {
                Value::Array(a) => a
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect(),
                Value::String(s) if s.trim().is_empty() => Vec::new(),
                Value::String(s) if !s.trim().contains(' ') => vec![s.trim().to_string()],
                Value::String(s) => {
                    if cfg!(windows) {
                        vec!["cmd.exe".into(), "/C".into(), s.clone()]
                    } else {
                        vec!["/bin/sh".into(), "-c".into(), s.clone()]
                    }
                }
                _ => Vec::new(),
            };
            let mut t = Terminal::new(command, p.cwd.clone());
            t.title = p.title.clone();
            t.height = cells_prop(&p.height);
            Kind::Terminal(t)
        }
        _ => unreachable!("checked against WIDGET_TYPES"),
    })
}

/// Keep what the user did (typed text, selections, scroll) across an update.
fn carry_state(new: &mut Kind, old: &Kind, spec: &NodeSpec) {
    let content_given = !spec.content.is_null();
    match (new, old) {
        (Kind::Input(n), Kind::Input(o)) => {
            if n.value == o.value {
                n.cursor = o.cursor;
            }
        }
        (Kind::Card(n), Kind::Card(o)) => n.scroll = o.scroll,
        (Kind::Table(n), Kind::Table(o)) => {
            n.selected = o.selected.filter(|s| *s < n.rows.len());
            n.scroll = o.scroll;
        }
        (Kind::Tree(n), Kind::Tree(o)) => n.carry_state(o),
        (Kind::Select(n), Kind::Select(o)) => {
            if spec.props.value.is_none() {
                let cur = o.options.get(o.selected);
                n.selected = cur
                    .and_then(|c| n.options.iter().position(|x| x == c))
                    .unwrap_or(0);
                n.hover = n.selected;
            }
        }
        (Kind::Tabs(n), Kind::Tabs(o)) if spec.props.active.is_none() => n.active = o.active,
        (Kind::Pane(n), Kind::Pane(o)) => n.types = o.types.clone(),
        (Kind::Markdown(n), Kind::Markdown(o)) => n.carry(o),
        (Kind::Code(n), Kind::Code(o)) => n.carry(o),
        (Kind::Diff(n), Kind::Diff(o)) => n.carry(o),
        (Kind::Log(n), Kind::Log(o)) => {
            n.carry(o);
            if !content_given {
                n.lines = o.lines.clone();
            }
        }
        (Kind::Chat(n), Kind::Chat(o)) => n.carry(o),
        (Kind::Prompt(n), Kind::Prompt(o)) => n.carry(o),
        (Kind::Terminal(n), Kind::Terminal(o)) => n.carry(o),
        (Kind::ToolCall(n), Kind::ToolCall(o)) if spec.props.expanded.is_none() => {
            n.expanded = o.expanded;
        }
        (Kind::Checkbox(n), Kind::Checkbox(o)) if spec.props.checked.is_none() => {
            n.checked = o.checked;
        }
        _ => {}
    }
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
        let ids: Vec<_> = t
            .preorder()
            .iter()
            .map(|i| t.nodes[*i].id.clone())
            .collect();
        assert_eq!(ids, ["card-1", "card-2", "text-1"]);
        assert_eq!(t.theme.name, "aurora-dark");
        assert!(t.get("card-2").is_some());
    }

    #[test]
    fn friendly_errors() {
        let cases = [
            (
                r#"{"children":[{"type":"spaceship"}]}"#,
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
            (
                r#"{"children":[{"type":"card","children":[{"type":"text"}]}]}"#,
                "can't hold",
            ),
            (
                r#"{"children":[{"type":"split","props":{"ratio":[1]},"children":[{"type":"card"},{"type":"card"}]}]}"#,
                "ratio has 1",
            ),
            (
                r#"{"children":[{"type":"stage","props":{"width":"lots"}}]}"#,
                "Can't read width",
            ),
            (
                r#"{"children":[{"type":"select","content":"nope"}]}"#,
                "StageSelect",
            ),
            (
                r#"{"children":[{"type":"button","props":{"variant":"loud"}}]}"#,
                "variant",
            ),
            (
                r#"{"bindings":[{"key":"hyper+x","action":"go"}]}"#,
                "Unknown key",
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
    fn per_widget_theme_and_accent_and_inheritance() {
        let t = Tree::build(&spec(r##"{"theme":"paper-soft","accent":"mint","children":[{"type":"card","props":{"theme":"hacker-mint","accent":"#ff0000"}},{"type":"stage","props":{"theme":"velvet-dusk"},"children":[{"type":"card"}]}]}"##)).unwrap();
        assert_eq!(t.nodes[0].theme.name, "hacker-mint");
        assert_eq!(t.nodes[0].theme.accent, Rgb(255, 0, 0));
        assert_eq!(t.nodes[1].theme.name, "velvet-dusk");
        assert_eq!(t.nodes[2].theme.name, "velvet-dusk", "children inherit");
        assert_eq!(
            t.nodes[2].theme.accent,
            style::accent("mint").unwrap(),
            "app accent wins"
        );
    }

    #[test]
    fn find_by_id_key_and_hash() {
        let t = Tree::build(&spec(
            r#"{"children":[{"type":"button","content":"Go","key":"w7","props":{"id":"go"}}]}"#,
        ))
        .unwrap();
        assert_eq!(t.find("#go"), Some(0));
        assert_eq!(t.find("w7"), Some(0));
        assert_eq!(t.find("go"), Some(0));
        assert!(t.require("#nope").unwrap_err().to_string().contains("fix:"));
    }

    #[test]
    fn split_unsplit_keeps_the_tree_consistent() {
        let mut t = Tree::build(&spec(
            r#"{"children":[{"type":"card","content":"a","props":{"id":"a"}}]}"#,
        ))
        .unwrap();
        let a = t.find("#a").unwrap();
        let b = t.split(a, &NodeSpec::text("card", "b"), true).unwrap();
        let root = t.roots[0];
        assert_eq!(t.nodes[root].kind.name(), "split");
        assert_eq!(t.nodes[root].children, vec![a, b]);
        let Kind::Card(c) = &t.nodes[a].kind else {
            panic!()
        };
        assert!(c.selectable, "panes take focus");
        // Same direction: a sibling, not a nested split.
        let c2 = t.split(b, &NodeSpec::text("card", "c"), true).unwrap();
        assert_eq!(t.nodes[root].children, vec![a, b, c2]);
        let Kind::Split(s) = &t.nodes[root].kind else {
            panic!()
        };
        assert_eq!(s.ratios, vec![0.5, 0.25, 0.25]);
        // Other direction: nested.
        let d = t.split(a, &NodeSpec::text("card", "d"), false).unwrap();
        let inner = t.nodes[d].parent.unwrap();
        assert_ne!(inner, root);
        // Unsplit back down to one card.
        t.remove(d);
        assert_eq!(t.nodes[a].parent, Some(root), "one-pane split collapses");
        t.remove(b);
        t.remove(c2);
        assert_eq!(t.roots, vec![a]);
        assert_eq!(t.nodes[a].parent, None);
        assert_eq!(t.preorder(), vec![a]);
    }

    #[test]
    fn updates_validate_and_keep_state() {
        let mut t = Tree::build(&spec(r#"{"children":[{"type":"input","content":"hi","props":{"id":"name"}},{"type":"table","content":[["a"],["b"]],"props":{"id":"t"}}]}"#)).unwrap();
        let i = t.find("#name").unwrap();
        t.update(i, &json!({"props": {"placeholder": "Your name"}}))
            .unwrap();
        let Kind::Input(inp) = &t.nodes[i].kind else {
            panic!()
        };
        assert_eq!((inp.value.as_str(), inp.cursor), ("hi", 2));
        assert!(
            t.update(i, &json!({"props": {"colour": "red"}}))
                .unwrap_err()
                .to_string()
                .contains("fix:")
        );
        assert!(t.update(i, &json!({"props": {"id": "other"}})).is_err());
        let ti = t.find("#t").unwrap();
        if let Kind::Table(tb) = &mut t.nodes[ti].kind {
            tb.selected = Some(1);
        }
        t.update(ti, &json!({"content": [["a"], ["b"], ["c"]]}))
            .unwrap();
        let Kind::Table(tb) = &t.nodes[ti].kind else {
            panic!()
        };
        assert_eq!((tb.rows.len(), tb.selected), (3, Some(1)));
    }

    #[test]
    fn app_direction_follows_the_first_text() {
        let t = Tree::build(&spec(
            r#"{"children":[{"type":"card","content":"مرحبا"},{"type":"card","content":"Hello"}]}"#,
        ))
        .unwrap();
        assert_eq!(t.dir, Direction::Rtl);
        let t = Tree::build(&spec(
            r#"{"dir":"ltr","children":[{"type":"card","content":"مرحبا"}]}"#,
        ))
        .unwrap();
        assert_eq!(t.dir, Direction::Ltr);
    }

    #[test]
    fn theme_switch_reaches_every_node() {
        let mut t = Tree::build(&spec(r#"{"children":[{"type":"stage","children":[{"type":"card"}]},{"type":"card","props":{"theme":"paper-soft"}}]}"#)).unwrap();
        t.set_theme("hacker-mint").unwrap();
        assert_eq!(t.nodes[1].theme.name, "hacker-mint");
        assert_eq!(t.nodes[2].theme.name, "paper-soft", "own theme kept");
        assert!(t.set_theme("disco").is_err());
    }
}
