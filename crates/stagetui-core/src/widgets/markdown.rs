//! `StageMarkdown`: Markdown that can stream in chunk by chunk (an AI answer).
//!
//! The renderer turns Markdown into styled terminal lines: headings, paragraphs
//! with **bold**, *italic*, `code` and links, lists, task lists, quotes, rules,
//! tables and syntax-highlighted code blocks. Right-to-left paragraphs are
//! shaped and reordered like all stagetui text.
//!
//! Streaming: everything before the last finished block is cached, so
//! `append()` only re-renders the block being written.

use std::cell::RefCell;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui_core::layout::Rect;
use ratatui_core::style::{Modifier, Style};
use serde_json::{Map, Value, json};

use super::{Cx, PaintCx, Reaction, Widget, clamp_scroll, object, scrollbar};
use crate::frame::Frame;
use crate::input::{Code, Key};
use crate::style::{Rgb, Theme};
use crate::text::{self, Direction};

/// What a piece of text is, so colors follow the theme (and theme switches).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Role {
    Text,
    Muted,
    Accent,
    Heading,
    Code,
    Link,
    /// A fixed color (syntax highlighting).
    Raw(Rgb),
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Look {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Seg {
    pub text: String,
    pub role: Role,
    pub look: Look,
}

/// One rendered terminal line.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MdLine {
    pub indent: u16,
    /// List bullet / number / quote bar drawn in the indent.
    pub marker: Option<(String, Role)>,
    pub segs: Vec<Seg>,
    /// Background for the whole line (code blocks).
    pub block: bool,
    /// A right-to-left paragraph line, already laid out.
    pub rtl: Option<text::Line>,
}

impl MdLine {
    fn blank() -> Self {
        Self::default()
    }

    /// Plain text of the line (for snapshots and copying).
    pub fn plain(&self) -> String {
        match &self.rtl {
            Some(l) => l.logical.clone(),
            None => self.segs.iter().map(|s| s.text.as_str()).collect(),
        }
    }
}

fn cells(s: &str) -> usize {
    if s.is_ascii() {
        s.len()
    } else {
        text::width(&text::arabic::shape(s))
    }
}

/// Greedy word wrap of styled segments into lines of at most `width` cells.
fn wrap(segs: &[Seg], width: usize) -> Vec<Vec<Seg>> {
    let width = width.max(1);
    let mut lines: Vec<Vec<Seg>> = vec![Vec::new()];
    let mut used = 0usize;
    let push = |lines: &mut Vec<Vec<Seg>>, text: &str, seg: &Seg| {
        let line = lines.last_mut().expect("at least one line");
        match line.last_mut() {
            Some(last) if last.role == seg.role && last.look == seg.look => {
                last.text.push_str(text)
            }
            _ => line.push(Seg {
                text: text.to_string(),
                role: seg.role,
                look: seg.look,
            }),
        }
    };
    for seg in segs {
        for (i, part) in seg.text.split('\n').enumerate() {
            if i > 0 {
                lines.push(Vec::new());
                used = 0;
            }
            for word in part.split_inclusive(' ') {
                let w = cells(word.trim_end());
                let full = cells(word);
                if used > 0 && used + w > width {
                    lines.push(Vec::new());
                    used = 0;
                }
                if used == 0 && word.trim().is_empty() {
                    continue; // no leading spaces on a wrapped line
                }
                if w > width {
                    // A very long word: cut it.
                    for piece in text::split_graphemes(word.trim_end(), width) {
                        if used > 0 {
                            lines.push(Vec::new());
                        }
                        used = cells(&piece);
                        push(&mut lines, &piece, seg);
                    }
                    if word.ends_with(' ') && used < width {
                        push(&mut lines, " ", seg);
                        used += 1;
                    }
                    continue;
                }
                push(&mut lines, word, seg);
                used += full;
            }
        }
    }
    lines
}

/// Builds lines from pulldown-cmark events.
struct Renderer<'a> {
    width: usize,
    dark: bool,
    out: Vec<MdLine>,
    /// Inline text of the block being collected.
    segs: Vec<Seg>,
    look: Look,
    role: Role,
    heading: Option<HeadingLevel>,
    /// `(ordered_next_number, depth)` for each open list.
    lists: Vec<Option<u64>>,
    quote: usize,
    /// The marker for the next line of the current list item.
    item_marker: Option<String>,
    code: Option<(Option<String>, String)>,
    table: Option<Table>,
    _src: &'a str,
}

#[derive(Default)]
struct Table {
    align: Vec<Alignment>,
    rows: Vec<Vec<String>>,
    cell: String,
    row: Vec<String>,
}

impl<'a> Renderer<'a> {
    fn new(src: &'a str, width: u16, dark: bool) -> Self {
        Self {
            width: usize::from(width.max(4)),
            dark,
            out: Vec::new(),
            segs: Vec::new(),
            look: Look::default(),
            role: Role::Text,
            heading: None,
            lists: Vec::new(),
            quote: 0,
            item_marker: None,
            code: None,
            table: None,
            _src: src,
        }
    }

    fn indent(&self) -> u16 {
        (self.lists.len() * 2 + self.quote * 2) as u16
    }

    fn gap(&mut self) {
        if self.out.last().is_some_and(|l| *l != MdLine::blank()) {
            self.out.push(MdLine::blank());
        }
    }

    fn text(&mut self, t: &str) {
        if let Some(table) = &mut self.table {
            table.cell.push_str(t);
            return;
        }
        if let Some((_, code)) = &mut self.code {
            code.push_str(t);
            return;
        }
        self.segs.push(Seg {
            text: t.to_string(),
            role: self.role,
            look: self.look,
        });
    }

    /// Finish a paragraph / heading / list item text.
    fn flush(&mut self) {
        if self.segs.is_empty() {
            return;
        }
        let segs = std::mem::take(&mut self.segs);
        let indent = self.indent();
        let room = self.width.saturating_sub(usize::from(indent)).max(4);
        let plain: String = segs.iter().map(|s| s.text.as_str()).collect();
        let quote_marker = (self.quote > 0).then(|| ("▎".to_string(), Role::Muted));
        let mut first_marker = self.item_marker.take().map(|m| (m, Role::Accent));
        if text::first_strong(&plain) == Direction::Rtl {
            // Right-to-left: shaped and reordered as a whole (inline styles are dropped).
            let role = segs[0].role;
            let look = segs[0].look;
            for line in text::layout(&plain, room, Direction::Rtl) {
                self.out.push(MdLine {
                    indent,
                    marker: first_marker.take().or_else(|| quote_marker.clone()),
                    segs: vec![Seg {
                        text: line.logical.clone(),
                        role,
                        look,
                    }],
                    block: false,
                    rtl: Some(line),
                });
            }
            return;
        }
        for line in wrap(&segs, room) {
            self.out.push(MdLine {
                indent,
                marker: first_marker.take().or_else(|| quote_marker.clone()),
                segs: line,
                block: false,
                rtl: None,
            });
        }
    }

    fn code_block(&mut self, lang: Option<String>, code: &str) {
        let indent = self.indent();
        let lines =
            text::highlight::highlight(code.trim_end_matches('\n'), lang.as_deref(), self.dark);
        let label = lang.filter(|l| !l.is_empty());
        if let Some(l) = &label {
            self.out.push(MdLine {
                indent,
                segs: vec![Seg {
                    text: format!(" {l}"),
                    role: Role::Muted,
                    look: Look::default(),
                }],
                block: true,
                ..MdLine::default()
            });
        }
        for tokens in lines {
            let mut segs = vec![Seg {
                text: " ".into(),
                role: Role::Text,
                look: Look::default(),
            }];
            segs.extend(tokens.into_iter().map(|t| Seg {
                text: t.text,
                role: Role::Raw(t.color),
                look: Look {
                    bold: t.bold,
                    ..Look::default()
                },
            }));
            self.out.push(MdLine {
                indent,
                segs,
                block: true,
                ..MdLine::default()
            });
        }
    }

    fn table_done(&mut self, t: Table) {
        let cols = t.rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![0usize; cols];
        for row in &t.rows {
            for (i, c) in row.iter().enumerate() {
                widths[i] = widths[i].max(cells(c));
            }
        }
        let indent = self.indent();
        let room = self.width.saturating_sub(usize::from(indent));
        while widths.iter().sum::<usize>() + 3 * cols.saturating_sub(1) > room {
            let Some(m) = widths.iter_mut().max_by_key(|w| **w) else {
                break;
            };
            if *m <= 3 {
                break;
            }
            *m -= 1;
        }
        for (r, row) in t.rows.iter().enumerate() {
            let mut segs = Vec::new();
            for (i, w) in widths.iter().enumerate() {
                let cell = row.get(i).map_or("", String::as_str);
                let cut: String = text::split_graphemes(cell, (*w).max(1))
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                let pad = w.saturating_sub(cells(&cut));
                let (l, rr) = match t.align.get(i) {
                    Some(Alignment::Right) => (pad, 0),
                    Some(Alignment::Center) => (pad / 2, pad - pad / 2),
                    _ => (0, pad),
                };
                segs.push(Seg {
                    text: format!("{}{}{}", " ".repeat(l), cut, " ".repeat(rr)),
                    role: if r == 0 { Role::Heading } else { Role::Text },
                    look: Look {
                        bold: r == 0,
                        ..Look::default()
                    },
                });
                if i + 1 < widths.len() {
                    segs.push(Seg {
                        text: " │ ".into(),
                        role: Role::Muted,
                        look: Look::default(),
                    });
                }
            }
            self.out.push(MdLine {
                indent,
                segs,
                ..MdLine::default()
            });
            if r == 0 {
                let rule = widths
                    .iter()
                    .map(|w| "─".repeat(*w))
                    .collect::<Vec<_>>()
                    .join("─┼─");
                self.out.push(MdLine {
                    indent,
                    segs: vec![Seg {
                        text: rule,
                        role: Role::Muted,
                        look: Look::default(),
                    }],
                    ..MdLine::default()
                });
            }
        }
    }

    fn event(&mut self, ev: Event<'_>) {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    if self.item_marker.is_none() {
                        self.gap();
                    }
                }
                Tag::Heading { level, .. } => {
                    self.gap();
                    self.heading = Some(level);
                    self.role = Role::Heading;
                    self.look.bold = true;
                }
                Tag::BlockQuote(_) => {
                    self.gap();
                    self.quote += 1;
                    self.role = Role::Muted;
                    self.look.italic = true;
                }
                Tag::CodeBlock(kind) => {
                    self.flush();
                    self.gap();
                    let lang = match kind {
                        CodeBlockKind::Fenced(l) => {
                            Some(l.split_whitespace().next().unwrap_or("").to_string())
                        }
                        CodeBlockKind::Indented => None,
                    };
                    self.code = Some((lang, String::new()));
                }
                Tag::List(start) => {
                    self.flush();
                    if self.lists.is_empty() {
                        self.gap();
                    }
                    self.lists.push(start);
                }
                Tag::Item => {
                    self.flush();
                    let depth = self.lists.len();
                    let marker = match self.lists.last_mut() {
                        Some(Some(n)) => {
                            let m = format!("{n}.");
                            *n += 1;
                            m
                        }
                        _ => if depth > 1 { "◦" } else { "•" }.to_string(),
                    };
                    self.item_marker = Some(marker);
                }
                Tag::Emphasis => self.look.italic = true,
                Tag::Strong => self.look.bold = true,
                Tag::Strikethrough => self.look.strike = true,
                Tag::Link { .. } => {
                    self.role = Role::Link;
                    self.look.underline = true;
                }
                Tag::Table(align) => {
                    self.flush();
                    self.gap();
                    self.table = Some(Table {
                        align,
                        ..Table::default()
                    });
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => self.flush(),
                TagEnd::Heading(level) => {
                    self.flush();
                    if level == HeadingLevel::H1
                        && let Some(last) = self.out.last()
                    {
                        let w = last.plain().chars().count().min(self.width);
                        let indent = last.indent;
                        self.out.push(MdLine {
                            indent,
                            segs: vec![Seg {
                                text: "━".repeat(w.max(3)),
                                role: Role::Accent,
                                look: Look::default(),
                            }],
                            ..MdLine::default()
                        });
                    }
                    self.heading = None;
                    self.role = Role::Text;
                    self.look.bold = false;
                }
                TagEnd::BlockQuote(_) => {
                    self.flush();
                    self.quote = self.quote.saturating_sub(1);
                    if self.quote == 0 {
                        self.role = Role::Text;
                        self.look.italic = false;
                    }
                }
                TagEnd::CodeBlock => {
                    if let Some((lang, code)) = self.code.take() {
                        self.code_block(lang, &code);
                    }
                }
                TagEnd::List(_) => {
                    self.flush();
                    self.lists.pop();
                }
                TagEnd::Item => self.flush(),
                TagEnd::Emphasis => self.look.italic = false,
                TagEnd::Strong => self.look.bold = self.heading.is_some(),
                TagEnd::Strikethrough => self.look.strike = false,
                TagEnd::Link => {
                    self.role = if self.heading.is_some() {
                        Role::Heading
                    } else {
                        Role::Text
                    };
                    self.look.underline = false;
                }
                TagEnd::TableCell => {
                    if let Some(t) = &mut self.table {
                        let cell = std::mem::take(&mut t.cell);
                        t.row.push(cell.trim().to_string());
                    }
                }
                TagEnd::TableHead | TagEnd::TableRow => {
                    if let Some(t) = &mut self.table {
                        let row = std::mem::take(&mut t.row);
                        t.rows.push(row);
                    }
                }
                TagEnd::Table => {
                    if let Some(t) = self.table.take() {
                        self.table_done(t);
                    }
                }
                _ => {}
            },
            Event::Text(t) => self.text(&t),
            Event::Code(t) => {
                let saved = self.role;
                self.role = Role::Code;
                self.text(&t);
                self.role = saved;
            }
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.text("\n"),
            Event::Rule => {
                self.flush();
                self.gap();
                self.out.push(MdLine {
                    indent: self.indent(),
                    segs: vec![Seg {
                        text: "─".repeat(self.width.saturating_sub(usize::from(self.indent()))),
                        role: Role::Muted,
                        look: Look::default(),
                    }],
                    ..MdLine::default()
                });
            }
            Event::TaskListMarker(done) => {
                self.item_marker = Some(if done { "[✓]" } else { "[ ]" }.to_string());
            }
            Event::Html(h) | Event::InlineHtml(h) => {
                let saved = self.role;
                self.role = Role::Muted;
                self.text(h.trim_end());
                self.role = saved;
            }
            Event::FootnoteReference(f) => self.text(&format!("[{f}]")),
            _ => {}
        }
    }

    fn finish(mut self) -> Vec<MdLine> {
        if let Some((lang, code)) = self.code.take() {
            // An unfinished code block while streaming: show what we have.
            self.code_block(lang, &code);
        }
        self.flush();
        while self.out.first().is_some_and(|l| *l == MdLine::blank()) {
            self.out.remove(0);
        }
        while self.out.last().is_some_and(|l| *l == MdLine::blank()) {
            self.out.pop();
        }
        self.out
    }
}

/// Render Markdown into terminal lines `width` cells wide.
pub fn render(src: &str, width: u16, dark: bool) -> Vec<MdLine> {
    let mut r = Renderer::new(src, width, dark);
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for ev in Parser::new_ext(src, opts) {
        r.event(ev);
    }
    r.finish()
}

/// Where the last finished block ends: text before it can't change by appending.
fn stable_prefix(src: &str) -> usize {
    let mut fence_open = false;
    let mut last = 0;
    let mut offset = 0;
    let mut prev_blank = false;
    for line in src.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence_open = !fence_open;
        }
        let blank = line.trim().is_empty();
        offset += line.len();
        if blank && !prev_blank && !fence_open && line.ends_with('\n') {
            // Only top-level blocks (not inside a list item) are safe to cut at.
            let next_is_indented = src[offset..].starts_with([' ', '\t']);
            if !next_is_indented && offset < src.len() {
                last = offset;
            }
        }
        prev_blank = blank;
    }
    last
}

/// Rendering with a cache of the finished part (fast streaming).
#[derive(Debug, Clone, Default)]
pub struct MdCache {
    /// `(stable_len, stable_hash, width, dark, lines)`
    stable: Option<(usize, u64, u16, bool, Vec<MdLine>)>,
}

impl MdCache {
    pub fn lines(&mut self, src: &str, width: u16, dark: bool) -> Vec<MdLine> {
        let cut = stable_prefix(src);
        let (head, tail) = src.split_at(cut);
        let hash = crate::gfx::hash_bytes(head.as_bytes());
        let cached = match &self.stable {
            Some((len, h, w, d, lines))
                if *len == cut && *h == hash && *w == width && *d == dark =>
            {
                lines.clone()
            }
            _ => {
                let lines = render(head, width, dark);
                self.stable = Some((cut, hash, width, dark, lines.clone()));
                lines
            }
        };
        let mut out = cached;
        let rest = render(tail, width, dark);
        if !out.is_empty() && !rest.is_empty() {
            out.push(MdLine::blank());
        }
        out.extend(rest);
        out
    }
}

/// Resolve a role to a color.
pub fn color(role: Role, t: &Theme) -> Rgb {
    match role {
        Role::Text => t.fg,
        Role::Muted => t.fg.mix(t.muted, 0.55),
        Role::Accent | Role::Heading => t.accent,
        Role::Code => t.accent.mix(t.fg, 0.35),
        Role::Link => t.accent.mix(t.fg, 0.2),
        Role::Raw(c) => c,
    }
}

/// The background of code blocks.
pub fn code_bg(t: &Theme, base: Rgb) -> Rgb {
    if t.dark {
        base.mix(Rgb(0, 0, 0), 0.25)
    } else {
        base.mix(t.border, 0.45)
    }
}

/// Draw `lines` into `area` starting at line `first`; `bg` is the surface behind.
pub fn paint_lines(
    lines: &[MdLine],
    first: usize,
    area: Rect,
    frame: &mut Frame,
    t: &Theme,
    bg: Rgb,
) {
    for (row, line) in lines
        .iter()
        .skip(first)
        .take(usize::from(area.height))
        .enumerate()
    {
        let y = area.y + row as u16;
        let x0 = area.x + line.indent.min(area.width);
        let room = area.right().saturating_sub(x0);
        let lbg = if line.block { code_bg(t, bg) } else { bg };
        if line.block {
            let style = Style::new().bg(lbg.color());
            for x in x0..area.right() {
                frame.cells[(x, y)].set_symbol(" ").set_style(style);
            }
        }
        if let Some((m, role)) = &line.marker {
            let mx = x0.saturating_sub(cells(m) as u16 + 1).max(area.x);
            frame.put_str(
                mx,
                y,
                m,
                x0.saturating_sub(mx),
                Style::new().fg(color(*role, t).color()).bg(bg.color()),
            );
        }
        if let Some(l) = &line.rtl {
            let role = line.segs.first().map_or(Role::Text, |s| s.role);
            let look = line.segs.first().map(|s| s.look).unwrap_or_default();
            let lw = (l.width as u16).min(room);
            frame.put_line(
                area.right() - lw,
                y,
                room,
                l,
                color(role, t),
                lbg,
                look.bold,
            );
            continue;
        }
        let mut x = x0;
        for seg in &line.segs {
            if x >= area.right() {
                break;
            }
            let fg = color(seg.role, t);
            let segbg = if seg.role == Role::Code {
                code_bg(t, bg)
            } else {
                lbg
            };
            // Layout trims spaces at the ends of a piece; keep them as gaps.
            let lead = seg.text.len() - seg.text.trim_start_matches(' ').len();
            let trail = seg.text.len() - seg.text.trim_end_matches(' ').len();
            let body = seg.text.trim_matches(' ');
            let space = Style::new().bg(segbg.color());
            for _ in 0..lead.min(usize::from(area.right() - x)) {
                frame.cells[(x, y)].set_symbol(" ").set_style(space);
                x += 1;
            }
            if body.is_empty() {
                continue;
            }
            let laid = text::layout(body, usize::MAX / 2, Direction::Ltr);
            let Some(l) = laid.first() else { continue };
            if x >= area.right() {
                break;
            }
            let n = frame.put_line(x, y, area.right() - x, l, fg, segbg, seg.look.bold);
            let mut m = Modifier::empty();
            if seg.look.italic {
                m |= Modifier::ITALIC;
            }
            if seg.look.underline {
                m |= Modifier::UNDERLINED;
            }
            if seg.look.strike {
                m |= Modifier::CROSSED_OUT;
            }
            if !m.is_empty() {
                for cx in x..x + n {
                    frame.cells[(cx, y)].modifier.insert(m);
                }
            }
            x += n;
            for _ in 0..trail {
                if x >= area.right() {
                    break;
                }
                frame.cells[(x, y)].set_symbol(" ").set_style(space);
                x += 1;
            }
        }
    }
}

/// The `StageMarkdown` widget.
#[derive(Debug, Clone)]
pub struct Markdown {
    pub source: String,
    pub scroll: usize,
    /// Keep the end in view as text streams in (until the user scrolls up).
    pub follow: bool,
    pub height: Option<u16>,
    cache: RefCell<MdCache>,
}

impl Markdown {
    pub fn new(source: String) -> Self {
        Self {
            source,
            scroll: 0,
            follow: true,
            height: None,
            cache: RefCell::new(MdCache::default()),
        }
    }

    pub fn lines(&self, width: u16, dark: bool) -> Vec<MdLine> {
        self.cache.borrow_mut().lines(&self.source, width, dark)
    }

    fn offset(&self, total: usize, visible: usize) -> usize {
        if self.follow {
            total.saturating_sub(visible)
        } else {
            clamp_scroll(self.scroll as i64, total, visible)
        }
    }

    /// Keep scroll position and cache when the text is replaced.
    pub fn carry(&mut self, old: &Markdown) {
        self.scroll = old.scroll;
        self.follow = old.follow;
        self.cache = old.cache.clone();
    }
}

impl Widget for Markdown {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        let lines = self.lines(max_width.saturating_sub(1).max(4), true);
        let w = lines
            .iter()
            .map(|l| usize::from(l.indent) + cells(&l.plain()))
            .max()
            .unwrap_or(1) as u16
            + 1;
        let h = self.height.unwrap_or(lines.len().max(1) as u16);
        (w.min(max_width).max(1), h)
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.is_empty() {
            return;
        }
        let t = p.theme;
        let bg = super::behind(frame, area.x, area.y, t.bg);
        let lines = self.lines(area.width.saturating_sub(1).max(4), t.dark);
        let first = self.offset(lines.len(), usize::from(area.height));
        let text = Rect {
            width: area.width.saturating_sub(1),
            ..area
        };
        paint_lines(&lines, first, text, frame, t, bg);
        if lines.len() > usize::from(area.height) {
            scrollbar(
                frame,
                area.right() - 1,
                area.y,
                area.height,
                first,
                lines.len(),
                t.border,
                t.accent,
            );
        }
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({ "text": self.source, "follow": self.follow }))
    }

    fn focusable(&self) -> bool {
        true
    }

    fn key(&mut self, key: &Key, area: Rect) -> Reaction {
        let page = i32::from(area.height.saturating_sub(1).max(1));
        let delta = match key.code {
            Code::Up => -1,
            Code::Down => 1,
            Code::PageUp => -page,
            Code::PageDown => page,
            Code::Home => i32::MIN / 2,
            Code::End => {
                self.follow = true;
                return Reaction::Redraw;
            }
            _ => return Reaction::Ignored,
        };
        if self.scroll_by(delta, area) {
            Reaction::Redraw
        } else {
            Reaction::Ignored
        }
    }

    fn scroll_by(&mut self, rows: i32, area: Rect) -> bool {
        let total = self.lines(area.width.saturating_sub(1).max(4), true).len();
        let visible = usize::from(area.height);
        let current = self.offset(total, visible);
        let next = clamp_scroll(current as i64 + i64::from(rows), total, visible);
        self.follow = next + visible >= total;
        let moved = next != current;
        self.scroll = next;
        moved
    }

    fn call(&mut self, method: &str, args: &Value) -> crate::Result<Value> {
        match method {
            "append" => {
                self.source.push_str(args.as_str().unwrap_or_default());
                Ok(Value::Null)
            }
            "clear" => {
                self.source.clear();
                self.follow = true;
                Ok(Value::Null)
            }
            _ => Err(crate::Error::friendly(
                format!("StageMarkdown has no method \"{method}\""),
                "Markdown text can be appended to or cleared",
                "use md.append(\"more text\") or md.update(\"new text\")",
            )),
        }
    }

    fn value(&self) -> Value {
        Value::String(self.source.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(src: &str, width: u16) -> Vec<String> {
        render(src, width, true)
            .iter()
            .map(|l| {
                format!(
                    "{}{}{}",
                    " ".repeat(usize::from(l.indent)),
                    l.marker
                        .as_ref()
                        .map_or(String::new(), |m| format!("{} ", m.0)),
                    l.plain()
                )
            })
            .collect()
    }

    #[test]
    fn blocks_render() {
        let src = "# Title\n\nSome **bold** and `code` text that wraps around nicely.\n\n- one\n- two\n  1. nested\n\n> quoted\n\n```python\nprint('hi')\n```\n\n| a | b |\n|---|--:|\n| 1 | 22 |\n";
        let lines = plain(src, 30);
        assert_eq!(lines[0], "Title");
        assert!(lines[1].starts_with("━━━"));
        assert!(lines.iter().any(|l| l.contains("Some bold and code text")));
        assert!(lines.iter().any(|l| l.contains("• one")));
        assert!(lines.iter().any(|l| l.contains("1. nested")));
        assert!(lines.iter().any(|l| l.contains("▎ quoted")));
        assert!(lines.iter().any(|l| l.contains("print('hi')")));
        assert!(lines.iter().any(|l| l.contains("a │  b")));
        let r = render(src, 30, true);
        assert!(r.iter().any(|l| l.block), "code blocks get a background");
        let bold = r
            .iter()
            .flat_map(|l| &l.segs)
            .find(|s| s.text.contains("bold"))
            .unwrap();
        assert!(bold.look.bold);
    }

    #[test]
    fn wraps_and_handles_every_script() {
        let src = "مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج وهذا سطر طويل جداً\n\nHello مرحبا world こんにちは世界 नमस्ते दुनिया";
        let r = render(src, 20, true);
        assert!(
            r[0].rtl.is_some(),
            "Arabic paragraphs are laid out right-to-left"
        );
        for l in &r {
            let w: usize = match &l.rtl {
                Some(x) => x.width,
                None => l.segs.iter().map(|s| cells(&s.text)).sum(),
            };
            assert!(w <= 20, "{l:?}");
        }
    }

    #[test]
    fn streaming_matches_a_full_render() {
        let full =
            "# Plan\n\nFirst paragraph.\n\n```rust\nfn main() {}\n```\n\n- a\n- b\n\nDone **now**.";
        let mut cache = MdCache::default();
        let mut acc = String::new();
        for chunk in full.as_bytes().chunks(7) {
            acc.push_str(std::str::from_utf8(chunk).unwrap());
            let _ = cache.lines(&acc, 40, true);
        }
        assert_eq!(cache.lines(&acc, 40, true), render(full, 40, true));
        // An unfinished code fence still shows its code.
        assert!(
            plain("```py\nx = 1", 40)
                .iter()
                .any(|l| l.contains("x = 1"))
        );
    }
}
