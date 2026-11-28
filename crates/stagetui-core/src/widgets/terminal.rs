//! `StageTerminal`: a real shell (or any terminal program — vim, htop, git,
//! an AI CLI) running inside a pane.
//!
//! The program runs in a pseudo-terminal (`portable-pty`: Unix PTYs, Windows
//! ConPTY); its output is interpreted by `vt100` and drawn into the pane's
//! cells. While the pane has focus it receives every key — the app's prefix
//! key (Ctrl+Space by default) still reaches stagetui. The process is tracked
//! by the reaper, so it never outlives the app.

use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use ratatui_core::layout::Rect;
use ratatui_core::style::{Color, Modifier, Style};
use serde_json::{Map, Value, json};

use super::{Cx, PaintCx, Reaction, Widget, draw_box, fill, object};
use crate::frame::Frame;
use crate::input::{Button, Code, Key, MouseKind};
use crate::style::{Rgb, Theme};

const SCROLLBACK: usize = 2000;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn nz(v: u16) -> NonZeroU16 {
    NonZeroU16::new(v.max(1)).unwrap_or(NonZeroU16::MIN)
}

/// The running program and its screen, shared by every copy of the widget.
struct Inner {
    parser: Mutex<vt100::Parser>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    child: Mutex<Option<Box<dyn Child + Send + Sync>>>,
    size: Mutex<(u16, u16)>,
    generation: AtomicU64,
    painted: AtomicU64,
    started: AtomicBool,
    exit: Mutex<Option<u32>>,
    error: Mutex<Option<String>>,
    pid: Mutex<Option<u32>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        stop(self);
    }
}

fn stop(inner: &Inner) {
    if let Some(mut child) = lock(&inner.child).take() {
        let _ = child.kill();
        if !matches!(child.try_wait(), Ok(Some(_))) {
            // Collect it once it has gone (no zombie left), without making the screen wait.
            let _ = std::thread::Builder::new()
                .name("stagetui-pty-wait".into())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
    }
    if let Some(pid) = lock(&inner.pid).take() {
        let _ = crate::reaper::global().untrack_pid(pid);
    }
    lock(&inner.writer).take();
    lock(&inner.master).take();
}

#[derive(Clone)]
pub struct Terminal {
    /// Program and arguments; empty = the user's shell.
    pub command: Vec<String>,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub height: Option<u16>,
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Terminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Terminal({:?})", self.command)
    }
}

/// The user's shell.
pub fn default_shell() -> Vec<String> {
    #[cfg(windows)]
    {
        let comspec = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
        vec![comspec]
    }
    #[cfg(not(windows))]
    {
        vec![
            std::env::var("SHELL")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "/bin/sh".into()),
        ]
    }
}

impl Terminal {
    pub fn new(command: Vec<String>, cwd: Option<String>) -> Self {
        Self {
            command: if command.is_empty() {
                default_shell()
            } else {
                command
            },
            cwd,
            title: None,
            height: None,
            inner: Arc::new(Inner {
                parser: Mutex::new(vt100::Parser::new(nz(24), nz(80), SCROLLBACK)),
                writer: Mutex::new(None),
                master: Mutex::new(None),
                child: Mutex::new(None),
                size: Mutex::new((80, 24)),
                generation: AtomicU64::new(1),
                painted: AtomicU64::new(0),
                started: AtomicBool::new(false),
                exit: Mutex::new(None),
                error: Mutex::new(None),
                pid: Mutex::new(None),
            }),
        }
    }

    /// The pane closed: end its program now, not when the app quits.
    pub fn release(&mut self) {
        stop(&self.inner);
    }

    /// Keep the same running program when the widget is updated.
    pub fn carry(&mut self, old: &Terminal) {
        if self.command == old.command && self.cwd == old.cwd {
            self.inner = Arc::clone(&old.inner);
        }
    }

    fn start(&self) {
        let inner = &self.inner;
        if inner.started.swap(true, Ordering::SeqCst) {
            return;
        }
        *lock(&inner.exit) = None;
        *lock(&inner.error) = None;
        if let Err(e) = self.spawn() {
            *lock(&inner.error) = Some(format!("Could not start {}: {e}", self.command.join(" ")));
        }
        inner.generation.fetch_add(1, Ordering::SeqCst);
    }

    fn spawn(&self) -> std::result::Result<(), String> {
        let (cols, rows) = *lock(&self.inner.size);
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;
        let (prog, args) = self.command.split_first().ok_or("no command")?;
        let mut cmd = CommandBuilder::new(prog);
        cmd.args(args);
        let cwd = self
            .cwd
            .clone()
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_dir().ok());
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env_remove("STAGETUI_HEADLESS");
        let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        drop(pair.slave);
        if let Some(pid) = child.process_id() {
            let _ = crate::reaper::global().track_pid(pid);
            *lock(&self.inner.pid) = Some(pid);
        }
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        *lock(&self.inner.writer) = Some(writer);
        *lock(&self.inner.master) = Some(pair.master);
        *lock(&self.inner.child) = Some(child);
        let inner = Arc::downgrade(&self.inner);
        std::thread::Builder::new()
            .name("stagetui-pty".into())
            .spawn(move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    let Some(inner) = inner.upgrade() else { break };
                    let chunk = &buf[..n];
                    answer_queries(&inner, chunk);
                    lock(&inner.parser).process(chunk);
                    inner.generation.fetch_add(1, Ordering::SeqCst);
                    crate::app::wake_all();
                }
                if let Some(inner) = inner.upgrade() {
                    let code = lock(&inner.child)
                        .as_mut()
                        .and_then(|c| c.wait().ok())
                        .map_or(0, |s| s.exit_code());
                    *lock(&inner.exit) = Some(code);
                    if let Some(pid) = lock(&inner.pid).take() {
                        let _ = crate::reaper::global().untrack_pid(pid);
                    }
                    inner.generation.fetch_add(1, Ordering::SeqCst);
                    crate::app::wake_all();
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn restart(&self) {
        stop(&self.inner);
        *lock(&self.inner.parser) = {
            let (c, r) = *lock(&self.inner.size);
            vt100::Parser::new(nz(r), nz(c), SCROLLBACK)
        };
        self.inner.started.store(false, Ordering::SeqCst);
        self.start();
    }

    /// Send bytes to the program.
    pub fn write(&self, bytes: &[u8]) {
        self.start();
        if let Some(w) = lock(&self.inner.writer).as_mut() {
            let _ = w.write_all(bytes);
            let _ = w.flush();
        }
    }

    fn resize(&self, cols: u16, rows: u16) {
        let mut size = lock(&self.inner.size);
        if *size == (cols, rows) {
            return;
        }
        *size = (cols, rows);
        lock(&self.inner.parser).set_size(nz(rows), nz(cols));
        if let Some(m) = lock(&self.inner.master).as_ref() {
            let _ = m.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }

    fn inner_area(area: Rect) -> Rect {
        Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        )
    }

    pub fn exited(&self) -> Option<u32> {
        *lock(&self.inner.exit)
    }

    /// The screen as plain text.
    pub fn contents(&self) -> String {
        lock(&self.inner.parser).screen().contents()
    }
}

/// Reply to the terminal queries programs commonly wait for.
fn answer_queries(inner: &Inner, chunk: &[u8]) {
    let find = |needle: &[u8]| chunk.windows(needle.len()).any(|w| w == needle);
    let mut reply = Vec::new();
    if find(b"\x1b[6n") {
        let (r, c) = lock(&inner.parser).screen().cursor_position();
        reply.extend(format!("\x1b[{};{}R", r + 1, c + 1).bytes());
    }
    if find(b"\x1b[c") || find(b"\x1b[0c") {
        reply.extend(b"\x1b[?62;22c");
    }
    if !reply.is_empty()
        && let Some(w) = lock(&inner.writer).as_mut()
    {
        let _ = w.write_all(&reply);
        let _ = w.flush();
    }
}

/// The 16 ANSI colors, readable on stagetui's dark or light themes.
fn ansi(i: u8, t: &Theme) -> Rgb {
    const DARK: [Rgb; 16] = [
        Rgb(0x15, 0x16, 0x1e),
        Rgb(0xf7, 0x76, 0x8e),
        Rgb(0x9e, 0xce, 0x6a),
        Rgb(0xe0, 0xaf, 0x68),
        Rgb(0x7a, 0xa2, 0xf7),
        Rgb(0xbb, 0x9a, 0xf7),
        Rgb(0x7d, 0xcf, 0xff),
        Rgb(0xa9, 0xb1, 0xd6),
        Rgb(0x41, 0x48, 0x68),
        Rgb(0xff, 0x89, 0x9d),
        Rgb(0xb9, 0xf2, 0x7c),
        Rgb(0xff, 0x9e, 0x64),
        Rgb(0x8d, 0xb0, 0xff),
        Rgb(0xc7, 0xa9, 0xff),
        Rgb(0xa4, 0xda, 0xff),
        Rgb(0xc0, 0xca, 0xf5),
    ];
    const LIGHT: [Rgb; 16] = [
        Rgb(0x34, 0x3b, 0x58),
        Rgb(0xc6, 0x40, 0x5c),
        Rgb(0x38, 0x77, 0x49),
        Rgb(0x8c, 0x6c, 0x3e),
        Rgb(0x34, 0x54, 0x8a),
        Rgb(0x5a, 0x4a, 0x78),
        Rgb(0x0f, 0x4b, 0x6e),
        Rgb(0x60, 0x66, 0x88),
        Rgb(0x6a, 0x70, 0x8a),
        Rgb(0xd2, 0x4f, 0x6b),
        Rgb(0x48, 0x87, 0x59),
        Rgb(0x9c, 0x7c, 0x4e),
        Rgb(0x44, 0x64, 0x9a),
        Rgb(0x6a, 0x5a, 0x88),
        Rgb(0x1f, 0x5b, 0x7e),
        Rgb(0x34, 0x3b, 0x58),
    ];
    if t.dark {
        DARK[usize::from(i % 16)]
    } else {
        LIGHT[usize::from(i % 16)]
    }
}

fn xterm(i: u8, t: &Theme) -> Rgb {
    match i {
        0..=15 => ansi(i, t),
        16..=231 => {
            let i = i - 16;
            let v = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
            Rgb(v(i / 36), v((i / 6) % 6), v(i % 6))
        }
        _ => {
            let g = 8 + (i - 232) * 10;
            Rgb(g, g, g)
        }
    }
}

fn color(c: vt100::Color, default: Rgb, t: &Theme) -> Rgb {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) => xterm(i, t),
        vt100::Color::Rgb(r, g, b) => Rgb(r, g, b),
    }
}

/// Bytes a terminal program expects for a key.
pub fn encode_key(key: &Key, app_cursor: bool) -> Vec<u8> {
    let m = key.mods;
    let modifier = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.ctrl);
    let csi = |end: char| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{end}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{end}").into_bytes()
        } else {
            format!("\x1b[{end}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[{n};{modifier}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    let mut out = match key.code {
        Code::Char(c) if m.ctrl => {
            let b = match c {
                'a'..='z' => (c as u8) & 0x1f,
                '@' | ' ' | '2' => 0,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '-' | '7' => 0x1f,
                '?' | '8' => 0x7f,
                _ => return Vec::new(),
            };
            vec![b]
        }
        Code::Char(c) => {
            let c = if m.shift {
                c.to_uppercase().next().unwrap_or(c)
            } else {
                c
            };
            c.to_string().into_bytes()
        }
        Code::Enter => b"\r".to_vec(),
        Code::Tab if m.shift => b"\x1b[Z".to_vec(),
        Code::Tab => b"\t".to_vec(),
        Code::Backspace => vec![0x7f],
        Code::Esc => vec![0x1b],
        Code::Up => csi('A'),
        Code::Down => csi('B'),
        Code::Right => csi('C'),
        Code::Left => csi('D'),
        Code::Home => csi('H'),
        Code::End => csi('F'),
        Code::Insert => tilde(2),
        Code::Delete => tilde(3),
        Code::PageUp => tilde(5),
        Code::PageDown => tilde(6),
        Code::F(n @ 1..=4) => format!("\x1bO{}", (b'P' + n - 1) as char).into_bytes(),
        Code::F(n) => {
            let code = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => return Vec::new(),
            };
            tilde(code)
        }
    };
    if m.alt && matches!(key.code, Code::Char(_)) {
        out.insert(0, 0x1b);
    }
    out
}

impl Widget for Terminal {
    fn measure(&self, max_width: u16, _cx: &Cx) -> (u16, u16) {
        (max_width.min(100), self.height.unwrap_or(16))
    }

    fn paint(&self, area: Rect, frame: &mut Frame, p: &PaintCx) {
        let area = area.intersection(frame.area());
        if area.width < 4 || area.height < 3 {
            return;
        }
        let t = p.theme;
        let inner = Self::inner_area(area);
        self.resize(inner.width.max(1), inner.height.max(1));
        self.start();
        self.inner.painted.store(
            self.inner.generation.load(Ordering::SeqCst),
            Ordering::SeqCst,
        );
        let outside = super::behind(frame, area.x, area.y, t.bg);
        let tbg = super::markdown::code_bg(t, t.surface);
        fill(frame, area, t.fg, tbg);
        let exit = self.exited();
        let name = self.title.clone().unwrap_or_else(|| {
            let prog = self.command.first().map_or("shell", String::as_str);
            let prog = prog.rsplit(['/', '\\']).next().unwrap_or(prog);
            std::iter::once(prog.to_string())
                .chain(self.command[1..].iter().cloned())
                .collect::<Vec<_>>()
                .join(" ")
        });
        let heading = match exit {
            Some(code) => format!("❯ {name} · exited ({code})"),
            None => format!("❯ {name}"),
        };
        draw_box(
            frame,
            area,
            true,
            p.border(),
            outside,
            Some(&heading),
            t.accent,
            false,
        );
        if let Some(err) = lock(&self.inner.error).clone() {
            frame.put_str(
                inner.x,
                inner.y,
                &err,
                inner.width,
                Style::new().fg(t.accent.color()).bg(tbg.color()),
            );
            return;
        }
        let parser = lock(&self.inner.parser);
        let screen = parser.screen();
        for row in 0..inner.height {
            for col in 0..inner.width {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut fg = color(cell.fgcolor(), t.fg, t);
                let mut bg = color(cell.bgcolor(), tbg, t);
                if cell.inverse() {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let mut style = Style::new().fg(fg.color()).bg(bg.color());
                for (on, m) in [
                    (cell.bold(), Modifier::BOLD),
                    (cell.dim(), Modifier::DIM),
                    (cell.italic(), Modifier::ITALIC),
                    (cell.underline(), Modifier::UNDERLINED),
                ] {
                    if on {
                        style = style.add_modifier(m);
                    }
                }
                let sym = cell.contents();
                let (x, y) = (inner.x + col, inner.y + row);
                let wide = cell.is_wide();
                if wide && col + 1 >= inner.width {
                    frame.cells[(x, y)].set_symbol(" ").set_style(style);
                    continue;
                }
                frame.cells[(x, y)]
                    .set_symbol(if sym.is_empty() { " " } else { sym })
                    .set_style(style);
                if wide && let Some(c) = frame.cells.cell_mut((x + 1, y)) {
                    c.set_symbol("").set_style(style);
                }
            }
        }
        let (cr, cc) = screen.cursor_position();
        if p.focused
            && !screen.hide_cursor()
            && screen.scrollback() == 0
            && exit.is_none()
            && cr < inner.height
            && cc < inner.width
        {
            let c = &mut frame.cells[(inner.x + cc, inner.y + cr)];
            c.bg = t.accent.color();
            c.fg = Color::Rgb(t.bg.0, t.bg.1, t.bg.2);
        }
        if screen.scrollback() > 0 {
            let label = format!(" ↑ {} lines back · End to return ", screen.scrollback());
            let x = inner.right().saturating_sub(label.chars().count() as u16);
            frame.put_str(
                x,
                inner.y,
                &label,
                inner.width,
                Style::new().fg(t.bg.color()).bg(t.accent.color()),
            );
        }
        if exit.is_some() {
            let label = " Process ended · Enter restarts ";
            frame.put_str(
                inner.x,
                inner.bottom() - 1,
                label,
                inner.width,
                Style::new().fg(t.bg.color()).bg(t.muted.color()),
            );
        }
    }

    fn describe(&self) -> Map<String, Value> {
        object(json!({
            "command": self.command,
            "pid": *lock(&self.inner.pid),
            "running": self.inner.started.load(Ordering::SeqCst) && self.exited().is_none(),
            "exit_code": self.exited(),
            "screen": self.contents(),
        }))
    }

    fn focusable(&self) -> bool {
        true
    }

    fn takes_text(&self) -> bool {
        true
    }

    fn captures_keys(&self) -> bool {
        true
    }

    fn key(&mut self, key: &Key, _area: Rect) -> Reaction {
        if self.exited().is_some() {
            if key.is(Code::Enter) {
                self.restart();
                return Reaction::Redraw;
            }
            return Reaction::Ignored;
        }
        let back = lock(&self.inner.parser).screen().scrollback();
        if back > 0 {
            let mut p = lock(&self.inner.parser);
            match key.code {
                Code::PageUp => p.screen_mut().set_scrollback(back + 10),
                Code::PageDown => p.screen_mut().set_scrollback(back.saturating_sub(10)),
                _ => p.screen_mut().set_scrollback(0),
            }
            drop(p);
            if matches!(key.code, Code::PageUp | Code::PageDown | Code::End) {
                return Reaction::Redraw;
            }
        }
        let app_cursor = lock(&self.inner.parser).screen().application_cursor();
        let bytes = encode_key(key, app_cursor);
        if bytes.is_empty() {
            return Reaction::Ignored;
        }
        self.write(&bytes);
        Reaction::Redraw
    }

    fn paste(&mut self, text: &str) -> Reaction {
        let bracketed = lock(&self.inner.parser).screen().bracketed_paste();
        if bracketed {
            self.write(format!("\x1b[200~{text}\x1b[201~").as_bytes());
        } else {
            self.write(text.replace('\n', "\r").as_bytes());
        }
        Reaction::Redraw
    }

    fn mouse(&mut self, kind: MouseKind, x: u16, y: u16, _area: Rect) -> Reaction {
        let (mode, enc) = {
            let p = lock(&self.inner.parser);
            (
                p.screen().mouse_protocol_mode(),
                p.screen().mouse_protocol_encoding(),
            )
        };
        if mode == vt100::MouseProtocolMode::None || x == 0 || y == 0 {
            return Reaction::Ignored;
        }
        let (cx, cy) = (x - 1, y - 1);
        let button = |b: Button| match b {
            Button::Left => 0u8,
            Button::Middle => 1,
            Button::Right => 2,
        };
        let (code, release) = match kind {
            MouseKind::Down(b) => (button(b), false),
            MouseKind::Up(b) => (button(b), true),
            MouseKind::Drag(b)
                if mode != vt100::MouseProtocolMode::Press
                    && mode != vt100::MouseProtocolMode::PressRelease =>
            {
                (button(b) + 32, false)
            }
            MouseKind::Move if mode == vt100::MouseProtocolMode::AnyMotion => (35, false),
            MouseKind::ScrollUp => (64, false),
            MouseKind::ScrollDown => (65, false),
            _ => return Reaction::Ignored,
        };
        if release && mode == vt100::MouseProtocolMode::Press {
            return Reaction::Ignored;
        }
        let bytes = if enc == vt100::MouseProtocolEncoding::Sgr {
            format!(
                "\x1b[<{code};{};{}{}",
                cx + 1,
                cy + 1,
                if release { 'm' } else { 'M' }
            )
            .into_bytes()
        } else {
            let b = if release { 3 } else { code };
            vec![
                0x1b,
                b'[',
                b'M',
                32 + b,
                (33 + cx).min(255) as u8,
                (33 + cy).min(255) as u8,
            ]
        };
        self.write(&bytes);
        Reaction::Redraw
    }

    fn scroll_by(&mut self, rows: i32, _area: Rect) -> bool {
        let mut p = lock(&self.inner.parser);
        if p.screen().alternate_screen() {
            return false;
        }
        let back = p.screen().scrollback();
        let next = (back as i64 - i64::from(rows)).max(0) as usize;
        p.screen_mut().set_scrollback(next);
        p.screen().scrollback() != back
    }

    fn poll_dirty(&self) -> bool {
        self.inner.generation.load(Ordering::SeqCst) != self.inner.painted.load(Ordering::SeqCst)
    }

    fn call(&mut self, method: &str, args: &Value) -> crate::Result<Value> {
        match method {
            "write" | "send" => {
                self.write(args.as_str().unwrap_or_default().as_bytes());
                Ok(Value::Null)
            }
            "restart" => {
                self.restart();
                Ok(Value::Null)
            }
            "screen" => {
                self.start();
                Ok(Value::String(self.contents()))
            }
            _ => Err(crate::Error::friendly(
                format!("StageTerminal has no method \"{method}\""),
                "a terminal can receive text, restart, or report its screen",
                "use term.send(\"ls\\n\"), term.restart() or term.screen",
            )),
        }
    }

    fn value(&self) -> Value {
        json!({ "screen": self.contents(), "exit_code": self.exited() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Mods;

    #[test]
    fn keys_encode_like_xterm() {
        let k = |s: &str| encode_key(&Key::parse(s).unwrap(), false);
        assert_eq!(k("a"), b"a");
        assert_eq!(k("A"), b"A");
        assert_eq!(k("ctrl+c"), vec![3]);
        assert_eq!(k("alt+x"), b"\x1bx");
        assert_eq!(k("enter"), b"\r");
        assert_eq!(k("up"), b"\x1b[A");
        assert_eq!(encode_key(&Key::parse("up").unwrap(), true), b"\x1bOA");
        assert_eq!(k("ctrl+right"), b"\x1b[1;5C");
        assert_eq!(k("f5"), b"\x1b[15~");
        assert_eq!(k("shift+tab"), b"\x1b[Z");
        assert_eq!(
            encode_key(&Key::new(Code::Char('م'), Mods::default()), false),
            "م".as_bytes()
        );
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_program_and_shows_its_output() {
        let mut t = Terminal::new(
            vec![
                "sh".into(),
                "-c".into(),
                "printf 'hello مرحبا\\n'; sleep 0.2".into(),
            ],
            None,
        );
        let theme = crate::style::THEMES[0];
        let mut frame = Frame::new(Rect::new(0, 0, 40, 6), theme.bg, theme.fg);
        let hints = [];
        let p = PaintCx {
            theme: &theme,
            cell_px: (10, 20),
            focused: true,
            hovered: false,
            now: std::time::Duration::ZERO,
            glow: 0.0,
            rtl: false,
            pane: false,
            hints: &hints,
        };
        t.paint(Rect::new(0, 0, 40, 6), &mut frame, &p);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !t.contents().contains("hello") && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(t.contents().contains("hello مرحبا"), "{:?}", t.contents());
        assert!(t.poll_dirty());
        while t.exited().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(t.exited(), Some(0));
        assert_eq!(
            t.call("screen", &Value::Null)
                .unwrap()
                .as_str()
                .map(|s| s.contains("hello")),
            Some(true)
        );
        assert_eq!(
            t.key(&Key::plain(Code::Char('x')), Rect::default()),
            Reaction::Ignored,
            "ended: keys are ignored"
        );
    }
}
