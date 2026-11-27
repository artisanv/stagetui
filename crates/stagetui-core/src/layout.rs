//! Layout: turns the widget tree into cell rectangles using `taffy`
//! (the flexbox/grid engine used by Dioxus, Bevy and Zed).
//!
//! - The screen is a column: optional bars on top, the scrolling content area,
//!   bars at the bottom.
//! - Content is centered when it fits, and scrolls when it doesn't — widgets
//!   are never squashed.
//! - `Stage` stacks children (vertical, horizontal or a grid), `StageSplit`
//!   shares space by ratio with one-cell dividers, `StageTabs` shows its active
//!   child, `StageModal` floats centered above everything.
//! - Right-to-left apps mirror horizontal layouts.

use ratatui_core::layout::Rect;
use taffy::geometry::Point;
use taffy::prelude::*;
use taffy::style::{AlignContent, AlignItems, Overflow};
use taffy::{AvailableSpace, Direction as TDir};

use crate::text::Direction;
use crate::tree::{Align, Dim, IRect, Kind, Tree};
use crate::widgets::Cx;

/// A draggable line between two split panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divider {
    pub split: usize,
    /// The divider sits after child number `index`.
    pub index: usize,
    pub rect: Rect,
    pub horizontal: bool,
}

/// What the layout produced besides node rectangles.
#[derive(Debug, Clone, Default)]
pub struct Laid {
    /// The scrolling content area (screen minus bars).
    pub content: Rect,
    /// Total content rows (more than `content.height` means it scrolls).
    pub content_rows: u16,
    pub dividers: Vec<Divider>,
    /// Visible modals, bottom to top.
    pub modals: Vec<usize>,
}

/// Cells kept free around content (2 each side) so cards never touch the edges.
pub const MARGIN: u16 = 4;

struct Builder<'a> {
    taffy: TaffyTree<usize>,
    tree: &'a Tree,
    ids: Vec<Option<NodeId>>,
}

fn dim(d: Option<Dim>) -> Dimension {
    match d {
        Some(Dim::Cells(n)) => Dimension::length(f32::from(n)),
        Some(Dim::Percent(p)) => Dimension::percent(p),
        None => Dimension::auto(),
    }
}

fn len(n: u16) -> LengthPercentage {
    LengthPercentage::length(f32::from(n))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Parent {
    /// The root content area.
    Root,
    Stage,
    /// A pane of a split or tabs: fills its share, never grows the split.
    Pane,
    Modal,
}

impl<'a> Builder<'a> {
    fn new(tree: &'a Tree) -> Self {
        let mut taffy = TaffyTree::new();
        taffy.enable_rounding();
        Self {
            taffy,
            tree,
            ids: vec![None; tree.nodes.len()],
        }
    }

    fn rtl(&self, i: usize) -> bool {
        // Explicit dir on a container wins; otherwise inherit, up to the app.
        let mut cur = Some(i);
        while let Some(c) = cur {
            let n = &self.tree.nodes[c];
            if n.kind.is_container() {
                match n.dir.as_str() {
                    "rtl" => return true,
                    "ltr" => return false,
                    _ => {}
                }
            }
            cur = n.parent;
        }
        self.tree.dir == Direction::Rtl
    }

    fn base_style(&self, i: usize, parent: Parent, max_w: Option<u16>) -> Style {
        let n = &self.tree.nodes[i];
        let mut s = Style {
            direction: if self.rtl(i) { TDir::Rtl } else { TDir::Ltr },
            size: Size {
                width: dim(n.layout.width),
                height: dim(n.layout.height),
            },
            flex_shrink: 0.0,
            ..Style::default()
        };
        if let Some(g) = n.layout.grow {
            s.flex_grow = g;
            s.flex_shrink = 1.0;
        }
        let fills = matches!(n.kind, Kind::Split(_) | Kind::Pane(_))
            || matches!(n.kind, Kind::Tabs(_) if n.layout.grow.is_some());
        if matches!(n.kind, Kind::Split(_) | Kind::Pane(_)) && n.layout.grow.is_none() {
            s.flex_grow = 1.0;
            s.flex_shrink = 1.0;
        }
        // Grown tabs fill the space they get; their pages scroll instead of
        // stretching the tabs to the page's full height.
        if matches!(n.kind, Kind::Tabs(_)) && n.layout.grow.is_some() {
            s.min_size.height = LengthPercentageAuto::length(3.0);
            s.overflow.y = Overflow::Hidden;
        }
        match parent {
            Parent::Root => {
                if fills || matches!(n.kind, Kind::Bar(_) | Kind::Prompt(_)) {
                    s.align_self = Some(AlignItems::STRETCH);
                } else if let Some(w) = max_w {
                    s.max_size.width = LengthPercentageAuto::length(f32::from(w));
                }
            }
            Parent::Pane => {
                s.flex_basis = Dimension::length(0.0);
                s.flex_shrink = 1.0;
                s.min_size = Size {
                    width: LengthPercentageAuto::length(0.0),
                    height: LengthPercentageAuto::length(0.0),
                };
                s.overflow = Point {
                    x: Overflow::Hidden,
                    y: Overflow::Hidden,
                };
            }
            Parent::Stage | Parent::Modal => {
                if fills {
                    s.align_self = Some(AlignItems::STRETCH);
                }
            }
        }
        s
    }

    fn build(&mut self, i: usize, parent: Parent, max_w: Option<u16>) -> Option<NodeId> {
        let n = &self.tree.nodes[i];
        if !n.alive || !n.visible || matches!(n.kind, Kind::Modal(_)) && parent != Parent::Modal {
            return None;
        }
        let mut style = self.base_style(i, parent, max_w);
        let id = match &n.kind {
            Kind::Stage(b) => {
                if let Some(cols) = b.columns {
                    style.display = Display::Grid;
                    style.grid_template_columns =
                        vec![fr(1.0); usize::from(cols)].into_iter().collect();
                    style.gap = Size {
                        width: len(b.gap.max(1)),
                        height: len(1),
                    };
                } else {
                    style.flex_direction = if b.horizontal {
                        FlexDirection::Row
                    } else {
                        FlexDirection::Column
                    };
                    style.gap = if b.horizontal {
                        Size {
                            width: len(b.gap),
                            height: len(0),
                        }
                    } else {
                        Size {
                            width: len(0),
                            height: len(b.gap),
                        }
                    };
                }
                style.align_items = match b.align {
                    Align::Start => AlignItems::START,
                    Align::Center => AlignItems::SAFE_CENTER,
                    Align::End => AlignItems::END,
                    Align::Stretch => AlignItems::STRETCH,
                };
                if b.scroll || parent == Parent::Pane {
                    style.overflow.y = Overflow::Scroll;
                    style.min_size.height = LengthPercentageAuto::length(0.0);
                }
                let kids: Vec<NodeId> = n
                    .children
                    .clone()
                    .into_iter()
                    .filter_map(|c| self.build(c, Parent::Stage, None))
                    .collect();
                self.taffy.new_with_children(style, &kids).ok()?
            }
            Kind::Split(b) => {
                style.flex_direction = if b.horizontal {
                    FlexDirection::Row
                } else {
                    FlexDirection::Column
                };
                style.gap = if b.horizontal {
                    Size {
                        width: len(1),
                        height: len(0),
                    }
                } else {
                    Size {
                        width: len(0),
                        height: len(1),
                    }
                };
                style.align_items = AlignItems::STRETCH;
                if style.min_size.height == LengthPercentageAuto::auto() {
                    style.min_size.height = LengthPercentageAuto::length(3.0);
                }
                let mut kids = Vec::new();
                for (k, c) in n.children.clone().into_iter().enumerate() {
                    if let Some(id) = self.build(c, Parent::Pane, None) {
                        let ratio = b.ratios.get(k).copied().unwrap_or(1.0).max(0.01);
                        if let Ok(mut st) = self.taffy.style(id).cloned() {
                            st.flex_grow = ratio;
                            let _ = self.taffy.set_style(id, st);
                        }
                        kids.push(id);
                    }
                }
                self.taffy.new_with_children(style, &kids).ok()?
            }
            Kind::Tabs(t) => {
                style.flex_direction = FlexDirection::Column;
                style.align_items = AlignItems::STRETCH;
                let visible: Vec<usize> = n
                    .children
                    .iter()
                    .copied()
                    .filter(|c| self.tree.nodes[*c].alive && self.tree.nodes[*c].visible)
                    .collect();
                let active = visible
                    .get(t.active.min(visible.len().saturating_sub(1)))
                    .copied();
                // Below the tab row; cards fill the page, other pages get breathing room.
                let card = active.is_some_and(|c| matches!(self.tree.nodes[c].kind, Kind::Card(_)));
                // Workspace tabs: one slim strip, the page fills the rest.
                let side = if card || t.workspaces { 0 } else { 2 };
                style.padding = taffy::Rect {
                    left: len(side),
                    right: len(side),
                    top: len(if t.workspaces {
                        1
                    } else if card {
                        2
                    } else {
                        3
                    }),
                    bottom: len(0),
                };
                let kids: Vec<NodeId> = active
                    .and_then(|c| {
                        let id = self.build(c, Parent::Pane, None)?;
                        if let Ok(mut st) = self.taffy.style(id).cloned() {
                            // Sized by its content, but may shrink (and scroll)
                            // when the tabs get a fixed or grown height.
                            st.flex_grow = 1.0;
                            st.flex_basis = Dimension::auto();
                            let _ = self.taffy.set_style(id, st);
                        }
                        Some(id)
                    })
                    .into_iter()
                    .collect();
                self.taffy.new_with_children(style, &kids).ok()?
            }
            Kind::Pane(pb) => {
                // A header row, then the content of the current type.
                style.flex_direction = FlexDirection::Column;
                style.align_items = AlignItems::STRETCH;
                style.padding = taffy::Rect {
                    left: len(1),
                    right: len(1),
                    top: len(1),
                    bottom: len(0),
                };
                if style.min_size.height == LengthPercentageAuto::auto() {
                    style.min_size.height = LengthPercentageAuto::length(3.0);
                }
                // A popped-out pane keeps its spot; its content is in another window.
                let current = pane_child(self.tree, i, pb).filter(|_| !pb.detached);
                let kids: Vec<NodeId> = current
                    .and_then(|c| {
                        let id = self.build(c, Parent::Pane, None)?;
                        if let Ok(mut st) = self.taffy.style(id).cloned() {
                            st.flex_grow = 1.0;
                            let _ = self.taffy.set_style(id, st);
                        }
                        Some(id)
                    })
                    .into_iter()
                    .collect();
                self.taffy.new_with_children(style, &kids).ok()?
            }
            Kind::Modal(_) => {
                style.flex_direction = FlexDirection::Column;
                style.gap = Size {
                    width: len(0),
                    height: len(1),
                };
                style.align_items = AlignItems::STRETCH;
                style.padding = taffy::Rect {
                    left: len(3),
                    right: len(3),
                    top: len(2),
                    bottom: len(2),
                };
                let kids: Vec<NodeId> = n
                    .children
                    .clone()
                    .into_iter()
                    .filter_map(|c| self.build(c, Parent::Stage, None))
                    .collect();
                self.taffy.new_with_children(style, &kids).ok()?
            }
            _ => self.taffy.new_leaf_with_context(style, i).ok()?,
        };
        self.ids[i] = Some(id);
        Some(id)
    }

    fn compute(&mut self, root: NodeId, w: Option<u16>, h: Option<u16>, cx: &Cx) {
        let tree = self.tree;
        let space = |v: Option<u16>| match v {
            Some(v) => AvailableSpace::Definite(f32::from(v)),
            None => AvailableSpace::MaxContent,
        };
        let _ = self.taffy.compute_layout_with_measure(
            root,
            Size {
                width: space(w),
                height: space(h),
            },
            |inputs, _id, ctx, style| {
                taffy::compute_leaf_layout(
                    inputs,
                    style,
                    |_, _| 0.0,
                    |known, avail| {
                        let Some(i) = ctx.map(|c| *c) else {
                            return Size::ZERO;
                        };
                        measure(tree, i, known, avail, cx)
                    },
                )
            },
        );
    }
}

fn measure(
    tree: &Tree,
    i: usize,
    known: Size<Option<f32>>,
    avail: Size<AvailableSpace>,
    cx: &Cx,
) -> Size<f32> {
    let n = &tree.nodes[i];
    let Some(w) = n.widget() else {
        return Size::ZERO;
    };
    let natural = w.measure(u16::MAX / 2, cx);
    let max_w = match known.width {
        Some(v) => v.round().max(1.0) as u16,
        None => match avail.width {
            AvailableSpace::Definite(v) => (v.round().max(1.0) as u16).min(natural.0.max(1)),
            AvailableSpace::MaxContent => natural.0,
            AvailableSpace::MinContent => match n.kind {
                Kind::Card(_) | Kind::Text(_) | Kind::Table(_) | Kind::Tree(_) => natural.0.min(20),
                Kind::Input(_) | Kind::Select(_) | Kind::Progress(_) => natural.0.min(12),
                Kind::Bar(_) => 1,
                _ => natural.0,
            },
        },
    };
    let (mw, mh) = w.measure(max_w.max(1), cx);
    Size {
        width: known.width.unwrap_or(f32::from(mw)),
        height: known.height.unwrap_or(f32::from(mh)),
    }
}

/// The child a pane shows: the one whose type is current (else the first).
pub fn pane_child(tree: &Tree, pane: usize, pb: &crate::tree::PaneBox) -> Option<usize> {
    let kids = &tree.nodes[pane].children;
    let live = |c: &usize| tree.nodes[*c].alive && tree.nodes[*c].visible;
    kids.iter()
        .zip(&pb.types)
        .find(|(c, t)| **t == pb.ptype && live(c))
        .map(|(c, _)| *c)
        .or_else(|| kids.iter().copied().find(live))
}

/// Lay out what a popped-out pane shows, filling a window of `cols × rows`
/// below its one-row header.
pub fn layout_popout(tree: &mut Tree, content: usize, cols: u16, rows: u16, cx: &Cx) -> Laid {
    let area = Rect::new(0, 1, cols, rows.saturating_sub(1));
    let mut laid = Laid {
        content: area,
        content_rows: area.height,
        ..Laid::default()
    };
    let mut b = Builder::new(tree);
    let Some(id) = b.build(content, Parent::Pane, None) else {
        return laid;
    };
    if let Ok(mut st) = b.taffy.style(id).cloned() {
        st.size = Size {
            width: Dimension::length(f32::from(area.width)),
            height: Dimension::length(f32::from(area.height)),
        };
        st.max_size = Size::auto();
        let _ = b.taffy.set_style(id, st);
    }
    b.compute(id, Some(area.width), Some(area.height), cx);
    let boxes = b.boxes();
    let rtls: Vec<bool> = (0..tree.nodes.len()).map(|i| b.rtl(i)).collect();
    drop(b);
    let mut stack = vec![content];
    while let Some(i) = stack.pop() {
        tree.nodes[i].rect = IRect::default();
        tree.nodes[i].content_h = 0;
        tree.nodes[i].rtl = rtls[i];
        stack.extend(tree.nodes[i].children.iter().copied());
    }
    place(tree, &boxes, content, (0, 1), &mut laid, false);
    laid
}

/// Lay out `tree` on a `cols × rows` screen. Writes every node's `rect`, `rtl`
/// and `content_h`; `root_scroll` is the content area's scroll offset (clamped
/// and written back).
pub fn layout(
    tree: &mut Tree,
    cols: u16,
    rows: u16,
    margin: u16,
    cx: &Cx,
    root_scroll: &mut u16,
) -> Laid {
    let screen = Rect::new(0, 0, cols, rows);
    let mut b = Builder::new(tree);
    let max_w = cols.saturating_sub(margin).max(1);

    // Bars before the first other widget go on top; the rest at the bottom.
    let live: Vec<usize> = tree
        .roots
        .iter()
        .copied()
        .filter(|r| tree.nodes[*r].alive && tree.nodes[*r].visible)
        .collect();
    let first_body = live
        .iter()
        .position(|r| !matches!(tree.nodes[*r].kind, Kind::Bar(_)));
    let (mut top, mut body, mut bottom) = (Vec::new(), Vec::new(), Vec::new());
    for (k, r) in live.iter().copied().enumerate() {
        match (&tree.nodes[r].kind, first_body) {
            (Kind::Modal(_), _) => {}
            (Kind::Bar(_), Some(f)) if k < f => top.push(r),
            (Kind::Bar(_), None) => top.push(r),
            (Kind::Bar(_), _) => bottom.push(r),
            _ => body.push(r),
        }
    }
    let bar_style = |b: &Builder, r: usize| {
        let mut s = b.base_style(r, Parent::Root, None);
        s.size.height = Dimension::length(1.0);
        s.flex_shrink = 0.0;
        s
    };
    let top_ids: Vec<NodeId> = top
        .iter()
        .filter_map(|r| {
            let s = bar_style(&b, *r);
            let id = b.taffy.new_leaf_with_context(s, *r).ok()?;
            b.ids[*r] = Some(id);
            Some(id)
        })
        .collect();
    let body_ids: Vec<NodeId> = body
        .iter()
        .filter_map(|r| b.build(*r, Parent::Root, Some(max_w)))
        .collect();
    let bottom_ids: Vec<NodeId> = bottom
        .iter()
        .filter_map(|r| {
            let s = bar_style(&b, *r);
            let id = b.taffy.new_leaf_with_context(s, *r).ok()?;
            b.ids[*r] = Some(id);
            Some(id)
        })
        .collect();
    let rtl = tree.dir == Direction::Rtl;
    let content = b
        .taffy
        .new_with_children(
            Style {
                direction: if rtl { TDir::Rtl } else { TDir::Ltr },
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                flex_shrink: 1.0,
                min_size: Size {
                    width: LengthPercentageAuto::auto(),
                    height: LengthPercentageAuto::length(0.0),
                },
                overflow: Point {
                    x: Overflow::Hidden,
                    y: Overflow::Scroll,
                },
                justify_content: AlignContent::SAFE_CENTER,
                align_items: AlignItems::SAFE_CENTER,
                gap: Size {
                    width: len(0),
                    height: len(1),
                },
                ..Style::default()
            },
            &body_ids,
        )
        .expect("content node");
    let mut all = top_ids.clone();
    all.push(content);
    all.extend(&bottom_ids);
    let root = b
        .taffy
        .new_with_children(
            Style {
                flex_direction: FlexDirection::Column,
                size: Size {
                    width: Dimension::length(f32::from(cols)),
                    height: Dimension::length(f32::from(rows)),
                },
                ..Style::default()
            },
            &all,
        )
        .expect("root node");
    b.compute(root, Some(cols), Some(rows), cx);

    let at = |b: &Builder, id: NodeId| {
        b.taffy
            .layout(id)
            .map(|l| (l.location, l.size))
            .unwrap_or_default()
    };
    let (cloc, csize) = at(&b, content);
    let content_rect = Rect::new(
        cloc.x.round() as u16,
        cloc.y.round() as u16,
        csize.width.round() as u16,
        csize.height.round() as u16,
    )
    .intersection(screen);
    // Content height: from the first child's top to the last child's bottom
    // (when everything fits it is centered, so the top isn't 0).
    let spans: Vec<(f32, f32)> = b
        .taffy
        .children(content)
        .unwrap_or_default()
        .iter()
        .map(|c| {
            let (l, s) = at(&b, *c);
            (l.y, l.y + s.height)
        })
        .collect();
    let top_y = spans.iter().map(|s| s.0).fold(f32::INFINITY, f32::min);
    let bottom_y = spans.iter().map(|s| s.1).fold(0.0, f32::max);
    let content_rows = if spans.is_empty() {
        0
    } else {
        (bottom_y - top_y.max(0.0)).round().max(0.0) as u16
    };
    *root_scroll = (*root_scroll).min(content_rows.saturating_sub(content_rect.height));
    let boxes = b.boxes();
    let rtls: Vec<bool> = (0..tree.nodes.len()).map(|i| b.rtl(i)).collect();
    drop(b);

    let mut laid = Laid {
        content: content_rect,
        content_rows,
        ..Laid::default()
    };
    // Reset everything (but what pop-out windows show), then place what was laid out.
    let popped = tree.popped_nodes();
    for (k, (n, rtl)) in tree.nodes.iter_mut().zip(rtls).enumerate() {
        if popped.contains(&k) {
            continue;
        }
        n.rect = IRect::default();
        n.content_h = 0;
        n.rtl = rtl;
    }
    let origin = (
        i32::from(content_rect.x),
        i32::from(content_rect.y) - i32::from(*root_scroll),
    );
    for r in &body {
        place(tree, &boxes, *r, origin, &mut laid, false);
    }
    for r in top.iter().chain(&bottom) {
        place(tree, &boxes, *r, (0, 0), &mut laid, false);
    }

    // Modals: each laid out on its own, centered.
    let modal_ids: Vec<usize> = tree
        .preorder()
        .into_iter()
        .filter(|i| {
            matches!(tree.nodes[*i].kind, Kind::Modal(_))
                && tree.nodes[*i].visible
                && ancestors_visible(tree, *i)
        })
        .collect();
    for m in modal_ids {
        let mut mb = Builder::new(tree);
        let Some(id) = mb.build(m, Parent::Modal, None) else {
            continue;
        };
        let cap_w = cols.saturating_sub(6).max(10);
        mb.compute(id, None, None, cx);
        let natural = mb
            .taffy
            .layout(id)
            .map(|l| l.size.width)
            .unwrap_or(40.0)
            .round() as u16;
        let width = match tree.nodes[m].layout.width {
            Some(Dim::Cells(w)) => w,
            Some(Dim::Percent(p)) => (f32::from(cols) * p) as u16,
            None => natural.max(36),
        }
        .min(cap_w);
        if let Ok(mut st) = mb.taffy.style(id).cloned() {
            st.size.width = Dimension::length(f32::from(width));
            st.max_size.height =
                LengthPercentageAuto::length(f32::from(rows.saturating_sub(2).max(3)));
            st.overflow.y = Overflow::Scroll;
            let _ = mb.taffy.set_style(id, st);
        }
        mb.compute(id, Some(width), Some(rows.saturating_sub(2)), cx);
        let size = mb.taffy.layout(id).map(|l| l.size).unwrap_or_default();
        let (w, h) = (size.width.round() as u16, size.height.round() as u16);
        let origin = (
            i32::from(cols.saturating_sub(w) / 2),
            i32::from(rows.saturating_sub(h) / 2),
        );
        let boxes = mb.boxes();
        let rtls: Vec<(usize, bool)> = (0..tree.nodes.len())
            .filter(|i| mb.ids[*i].is_some())
            .map(|i| (i, mb.rtl(i)))
            .collect();
        drop(mb);
        for (i, rtl) in rtls {
            tree.nodes[i].rtl = rtl;
        }
        place(tree, &boxes, m, origin, &mut laid, true);
        laid.modals.push(m);
    }
    laid
}

/// One node's computed box (relative to its parent).
#[derive(Debug, Clone, Copy)]
struct Boxed {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    pad_bottom: f32,
}

impl Builder<'_> {
    fn boxes(&self) -> Vec<Option<Boxed>> {
        self.ids
            .iter()
            .map(|id| {
                let l = self.taffy.layout((*id)?).ok()?;
                Some(Boxed {
                    x: l.location.x,
                    y: l.location.y,
                    w: l.size.width,
                    h: l.size.height,
                    pad_bottom: l.padding.bottom,
                })
            })
            .collect()
    }
}

fn ancestors_visible(tree: &Tree, mut i: usize) -> bool {
    while let Some(p) = tree.nodes[i].parent {
        if !tree.nodes[p].visible {
            return false;
        }
        i = p;
    }
    true
}

fn place(
    tree: &mut Tree,
    boxes: &[Option<Boxed>],
    i: usize,
    origin: (i32, i32),
    laid: &mut Laid,
    is_root: bool,
) {
    let Some(l) = boxes[i] else { return };
    let (x, y) = if is_root {
        origin
    } else {
        (origin.0 + l.x.round() as i32, origin.1 + l.y.round() as i32)
    };
    let rect = IRect {
        x,
        y,
        w: l.w.round() as u16,
        h: l.h.round() as u16,
    };
    tree.nodes[i].rect = rect;
    let children = tree.nodes[i].children.clone();
    // Scrolling containers: content height and offset.
    let content_h = children
        .iter()
        .filter_map(|c| boxes[*c])
        .map(|c| (c.y + c.h).round() as u16)
        .max()
        .unwrap_or(0)
        .saturating_add(l.pad_bottom.round() as u16);
    let scrolls =
        matches!(tree.nodes[i].kind, Kind::Stage(_) | Kind::Modal(_)) && content_h > rect.h;
    if scrolls {
        tree.nodes[i].content_h = content_h;
        tree.nodes[i].scroll = tree.nodes[i].scroll.min(content_h - rect.h);
    } else {
        tree.nodes[i].scroll = 0;
    }
    let offset = i32::from(tree.nodes[i].scroll);
    for c in &children {
        place(tree, boxes, *c, (x, y - offset), laid, false);
    }
    // Dividers between laid-out panes.
    if let Kind::Split(s) = &tree.nodes[i].kind {
        let horizontal = s.horizontal;
        let panes: Vec<(usize, IRect)> = children
            .iter()
            .enumerate()
            .filter(|(_, c)| boxes[**c].is_some())
            .map(|(k, c)| (k, tree.nodes[*c].rect))
            .collect();
        for pair in panes.windows(2) {
            let ((ka, a), (_, bb)) = (pair[0], pair[1]);
            let r = if horizontal {
                let gx = if a.x < bb.x { a.right() } else { bb.right() };
                IRect {
                    x: gx,
                    y: rect.y,
                    w: 1,
                    h: rect.h,
                }
            } else {
                IRect {
                    x: rect.x,
                    y: a.bottom(),
                    w: rect.w,
                    h: 1,
                }
            };
            laid.dividers.push(Divider {
                split: i,
                index: ka,
                rect: r.clip(Rect::new(0, 0, u16::MAX, u16::MAX)),
                horizontal,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::AppSpec;

    fn tree(json: &str) -> Tree {
        Tree::build(&serde_json::from_str::<AppSpec>(json).unwrap()).unwrap()
    }

    fn run(t: &mut Tree, cols: u16, rows: u16) -> Laid {
        let mut scroll = 0;
        layout(t, cols, rows, MARGIN, &Cx::default(), &mut scroll)
    }

    fn overlap(a: IRect, b: IRect) -> bool {
        a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
    }

    #[test]
    fn centers_when_everything_fits() {
        let mut t = tree(
            r#"{"children":[{"type":"card","content":"hello"},{"type":"text","content":"x"}]}"#,
        );
        let laid = run(&mut t, 80, 24);
        let (a, b) = (t.nodes[0].rect, t.nodes[1].rect);
        assert!(a.y > 0 && a.x > 0, "centered: {a:?}");
        assert_eq!(b.y, a.bottom() + 1, "one-row gap");
        assert_eq!(laid.content_rows, a.h + 1 + b.h);
    }

    #[test]
    fn overflow_scrolls_instead_of_squashing() {
        let mut t = tree(
            r#"{"children":[{"type":"card","content":"a\nb\nc"},{"type":"card","content":"d\ne\nf"},{"type":"card","content":"g\nh\ni"}]}"#,
        );
        let laid = run(&mut t, 60, 12);
        for n in &t.nodes {
            assert_eq!(n.rect.h, 7, "cards keep their height");
        }
        assert_eq!(t.nodes[0].rect.y, 0, "starts at the top when it overflows");
        assert!(laid.content_rows > 12);
    }

    #[test]
    fn split_reflow_never_overlaps() {
        let json = r#"{"children":[{"type":"split","props":{"ratio":[2,1]},"children":[
            {"type":"card","content":"left"},
            {"type":"split","props":{"direction":"vertical"},"children":[{"type":"card","content":"top"},{"type":"card","content":"bottom"}]}]}]}"#;
        for (cols, rows) in [(80, 24), (120, 40), (40, 12), (13, 7), (200, 60)] {
            let mut t = tree(json);
            let laid = run(&mut t, cols, rows);
            let leaves: Vec<IRect> = [1, 3, 4].iter().map(|i| t.nodes[*i].rect).collect();
            for (k, a) in leaves.iter().enumerate() {
                assert!(
                    a.right() <= i32::from(cols) && a.bottom() <= i32::from(rows),
                    "{cols}x{rows} {a:?}"
                );
                for b in &leaves[k + 1..] {
                    assert!(!overlap(*a, *b), "{cols}x{rows}: {a:?} overlaps {b:?}");
                }
                for d in &laid.dividers {
                    let d = IRect {
                        x: i32::from(d.rect.x),
                        y: i32::from(d.rect.y),
                        w: d.rect.width,
                        h: d.rect.height,
                    };
                    assert!(!overlap(*a, d), "pane under a divider");
                }
            }
            assert_eq!(laid.dividers.len(), 2);
            let split = t.nodes[0].rect;
            assert_eq!(
                (split.w, split.h),
                (cols, rows),
                "a root split fills the screen"
            );
            if cols >= 40 {
                let (l, r) = (t.nodes[1].rect.w as f32, t.nodes[2].rect.w as f32);
                assert!((l / r - 2.0).abs() < 0.3, "ratio 2:1 → {l}:{r}");
            }
        }
    }

    #[test]
    fn rtl_mirrors_rows_and_splits() {
        let mut t = tree(
            r#"{"dir":"rtl","children":[{"type":"split","children":[{"type":"card","content":"first"},{"type":"card","content":"second"}]}]}"#,
        );
        run(&mut t, 80, 20);
        assert!(
            t.nodes[1].rect.x > t.nodes[2].rect.x,
            "first pane on the right"
        );
        assert!(t.nodes[1].rtl);
    }

    #[test]
    fn bars_pin_to_the_edges() {
        let mut t = tree(
            r#"{"children":[{"type":"bar","content":"top"},{"type":"card","content":"x"},{"type":"bar","content":"bottom"}]}"#,
        );
        let laid = run(&mut t, 50, 20);
        assert_eq!((t.nodes[0].rect.y, t.nodes[0].rect.w), (0, 50));
        assert_eq!(t.nodes[2].rect.y, 19);
        assert_eq!((laid.content.y, laid.content.height), (1, 18));
    }

    #[test]
    fn grid_and_rows() {
        let mut t = tree(
            r#"{"children":[{"type":"stage","props":{"columns":2,"width":"100%"},"children":[{"type":"card","content":"a"},{"type":"card","content":"b"},{"type":"card","content":"c"}]}]}"#,
        );
        run(&mut t, 60, 30);
        let (a, b, c) = (t.nodes[1].rect, t.nodes[2].rect, t.nodes[3].rect);
        assert_eq!(a.y, b.y);
        assert!(b.x > a.x);
        assert_eq!(c.x, a.x);
        assert!(c.y > a.y);
        let mut t = tree(
            r#"{"children":[{"type":"stage","props":{"direction":"horizontal"},"children":[{"type":"button","content":"One"},{"type":"button","content":"Two"}]}]}"#,
        );
        run(&mut t, 60, 10);
        assert_eq!(t.nodes[1].rect.y, t.nodes[2].rect.y);
        assert_eq!(t.nodes[2].rect.x, t.nodes[1].rect.right() + 2);
    }

    #[test]
    fn modals_float_centered() {
        let mut t = tree(
            r#"{"children":[{"type":"card","content":"under"},{"type":"modal","props":{"visible":true,"title":"Sure?"},"children":[{"type":"text","content":"Delete it?"},{"type":"button","content":"Yes"}]}]}"#,
        );
        let laid = run(&mut t, 80, 24);
        assert_eq!(laid.modals, vec![1]);
        let m = t.nodes[1].rect;
        assert!((i32::from(80 - m.w) / 2 - m.x).abs() <= 1, "{m:?}");
        assert!(m.w >= 36);
        let txt = t.nodes[2].rect;
        assert!(txt.x > m.x && txt.y > m.y && txt.bottom() < m.bottom());
    }

    #[test]
    fn hidden_and_inactive_tabs_take_no_space() {
        let mut t = tree(
            r#"{"children":[{"type":"tabs","props":{"active":1},"children":[{"type":"card","content":"one","props":{"title":"One"}},{"type":"card","content":"two","props":{"title":"Two"}}]},{"type":"card","content":"gone","props":{"visible":false}}]}"#,
        );
        run(&mut t, 60, 20);
        assert_eq!(t.nodes[1].rect, IRect::default(), "inactive tab");
        assert!(t.nodes[2].rect.h > 0);
        assert_eq!(
            t.nodes[2].rect.y,
            t.nodes[0].rect.y + 2,
            "below the tab row"
        );
        assert_eq!(t.nodes[3].rect, IRect::default());
    }
}
