//! Running an app in a real terminal — and in other terminal windows.
//!
//! A [`Session`] is driven by background threads: one reads keyboard and mouse
//! input, another draws frames only when something changed (dirty-driven, ~0%
//! CPU when idle). Events for the app (`click`, `change`, actions…) wait in a
//! queue that the caller — usually Python — reads with [`App::next_event`].
//! Changes from the caller go through [`App::with`], which wakes the renderer.
//!
//! Every interactive app is also a **session** other windows can attach to
//! (`stagetui attach`, see [`crate::ipc`]). Each window is drawn with its own
//! terminal's graphics. The app's size follows the window used last, like tmux.

use std::collections::VecDeque;
use std::io::{self, Write as _};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ratatui_core::layout::Rect;

use crate::caps::{self, Capabilities, DetectOptions};
use crate::frame::Frame;
use crate::input::{self, Input};
use crate::ipc::{self, ClientMsg, FrameWriter, Hello, Out, Registration, ServerMsg, SessionInfo};
use crate::session::{AppEvent, Session};
use crate::term::{Modes, TerminalGuard};
use crate::view::{TerminalView, View};
use crate::{Error, Result};

/// The window of the terminal the app was started in.
const LOCAL: u64 = 0;

enum Msg {
    Input(u64, Input),
    Attach(Box<Remote>),
    Gone(u64),
    Quit,
    Wake,
    Stop,
}

/// A window watching the app from another terminal.
struct Remote {
    id: u64,
    view: TerminalView<FrameWriter>,
    out: SyncSender<Out>,
    caps: Capabilities,
    /// It asked for one pane (by id) …
    wants: Option<String>,
    /// … which is this node.
    pane: Option<usize>,
}

enum Window {
    Local(Box<TerminalView<io::Stdout>>, Box<Capabilities>),
    Remote(Box<Remote>),
}

impl Window {
    /// The popped-out pane this window shows (`None`: the whole app).
    fn pane(&self) -> Option<usize> {
        match self {
            Window::Local(..) => None,
            Window::Remote(r) => r.pane,
        }
    }

    fn caps(&self) -> &Capabilities {
        match self {
            Window::Local(_, c) => c,
            Window::Remote(r) => &r.caps,
        }
    }

    fn id(&self) -> u64 {
        match self {
            Window::Local(..) => LOCAL,
            Window::Remote(r) => r.id,
        }
    }

    fn size(&self) -> (u16, u16) {
        match self {
            Window::Local(v, _) => v.size(),
            Window::Remote(r) => r.view.size(),
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        match self {
            Window::Local(v, _) => v.resize(cols, rows),
            Window::Remote(r) => r.view.resize(cols, rows),
        }
    }

    fn cell_px(&self) -> (u32, u32) {
        match self {
            Window::Local(v, _) => v.cell_px(),
            Window::Remote(r) => r.view.cell_px(),
        }
    }

    fn present(&mut self, frame: &Frame) -> io::Result<()> {
        match self {
            Window::Local(v, _) => v.present(frame),
            Window::Remote(r) => r.view.present(frame),
        }
    }

    /// Bytes for the terminal itself (e.g. the clipboard sequence).
    fn raw(&mut self, bytes: &str) {
        match self {
            Window::Local(..) => {
                let mut out = io::stdout();
                let _ = out.write_all(bytes.as_bytes());
                let _ = out.flush();
            }
            Window::Remote(r) => {
                let _ = r.out.try_send(Out::Frame(bytes.as_bytes().to_vec()));
            }
        }
    }

    fn bye(&self, reason: &str) {
        if let Window::Remote(r) = self {
            let _ = r.out.try_send(Out::Msg(ServerMsg::Bye {
                reason: reason.into(),
            }));
        }
    }
}

/// Show `frame` in a window of another size: top-left aligned, cut or padded.
fn fit(frame: &Frame, (cols, rows): (u16, u16)) -> Frame {
    let area = Rect::new(0, 0, cols, rows);
    let mut out = Frame::new(area, frame.bg, frame.bg);
    out.blit(frame, 0, 0, area, 1.0, frame.bg, true);
    out
}

/// Opens a terminal window running a command (tests replace it).
pub type Launcher = Arc<dyn Fn(&[String], &Capabilities) -> Result<()> + Send + Sync>;

/// How to serve a session.
#[derive(Debug, Clone, Default)]
pub struct Serve {
    /// The session name to ask for (made unique).
    pub name: String,
    /// The program, shown by `stagetui sessions`.
    pub app: String,
    /// Where the app's output goes, if it runs in the background.
    pub log: Option<String>,
    /// Another sessions folder (tests); default: the user's.
    pub dir: Option<PathBuf>,
}

/// A running (or ready-to-run) app.
pub struct App {
    session: Mutex<Session>,
    queue: Mutex<VecDeque<AppEvent>>,
    ready: Condvar,
    wake: Mutex<Option<Sender<Msg>>>,
    running: AtomicBool,
    threads: Mutex<Vec<JoinHandle<()>>>,
    failure: Mutex<Option<Error>>,
    info: Mutex<Option<SessionInfo>>,
    waker: Mutex<Option<ipc::Waker>>,
    windows: AtomicU64,
    /// How a pop-out window runs stagetui: `["stagetui"]`, or Python's `-m stagetui`.
    attach_cmd: Mutex<Vec<String>>,
    /// Overrides how pop-out windows are opened (`"foot {cmd}"`).
    launch_template: Mutex<Option<String>>,
    launcher: Mutex<Option<Launcher>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn env_on(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0")
}

/// Renderers of running apps, woken when something outside changes (shell output).
static WAKERS: Mutex<Vec<Sender<Msg>>> = Mutex::new(Vec::new());

/// Wake every running app's renderer (called by background readers such as
/// embedded terminals). Cheap: wake-ups coalesce in the render loop.
pub fn wake_all() {
    lock(&WAKERS).retain(|tx| tx.send(Msg::Wake).is_ok());
}

/// OSC 52: ask the terminal to put `text` on the system clipboard.
pub fn osc52(text: &str) -> String {
    format!(
        "\x1b]52;c;{}\x07",
        crate::gfx::base64::encode(text.as_bytes())
    )
}

impl App {
    pub fn new(session: Session) -> Arc<Self> {
        Arc::new(Self {
            session: Mutex::new(session),
            queue: Mutex::new(VecDeque::new()),
            ready: Condvar::new(),
            wake: Mutex::new(None),
            running: AtomicBool::new(false),
            threads: Mutex::new(Vec::new()),
            failure: Mutex::new(None),
            info: Mutex::new(None),
            waker: Mutex::new(None),
            windows: AtomicU64::new(1),
            attach_cmd: Mutex::new(vec!["stagetui".into()]),
            launch_template: Mutex::new(None),
            launcher: Mutex::new(None),
        })
    }

    /// Lock the session (for reading or headless driving).
    pub fn session(&self) -> MutexGuard<'_, Session> {
        lock(&self.session)
    }

    /// Change the session and wake the renderer.
    pub fn with<R>(&self, f: impl FnOnce(&mut Session) -> R) -> R {
        let r = f(&mut self.session());
        self.wake();
        r
    }

    /// Ask the renderer to look at the session again.
    pub fn wake(&self) {
        if let Some(tx) = lock(&self.wake).as_ref() {
            let _ = tx.send(Msg::Wake);
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// How a new window runs stagetui, e.g. `["python", "-m", "stagetui"]`.
    pub fn set_attach_command(&self, cmd: Vec<String>) {
        if !cmd.is_empty() {
            *lock(&self.attach_cmd) = cmd;
        }
    }

    /// How pop-out windows are opened, e.g. `"foot {cmd}"` (`None`: the
    /// terminal in use, or `STAGETUI_LAUNCHER`).
    pub fn set_launch_template(&self, template: Option<String>) {
        *lock(&self.launch_template) = template;
    }

    /// Replace how windows are opened (tests).
    pub fn set_launcher(&self, launcher: Launcher) {
        *lock(&self.launcher) = Some(launcher);
    }

    /// Open a window showing pane `pane` (an id), like the window the person used.
    fn open_window(&self, pane: &str, caps: &Capabilities) -> Result<()> {
        let session = lock(&self.info)
            .as_ref()
            .map(|i| i.name.clone())
            .ok_or_else(|| {
                Error::friendly(
                    "This app can't open other windows",
                    "it was started without sessions (STAGETUI_NO_SESSION)",
                    "start it without STAGETUI_NO_SESSION, or with `stagetui start`",
                )
            })?;
        let mut cmd = lock(&self.attach_cmd).clone();
        cmd.push("attach".into());
        cmd.push(format!("{session}:{pane}"));
        if let Some(f) = lock(&self.launcher).clone() {
            return f(&cmd, caps);
        }
        let template = lock(&self.launch_template)
            .clone()
            .or_else(|| std::env::var("STAGETUI_LAUNCHER").ok());
        let argv =
            crate::launch::window_command(caps, &cmd, template.as_deref()).ok_or_else(|| {
                Error::friendly(
                    "Can't open a window from here",
                    "over SSH, or in a terminal stagetui doesn't know how to open",
                    format!(
                        "open another terminal and run: {}",
                        crate::launch::shell_line(&cmd)
                    ),
                )
            })?;
        crate::launch::spawn(&argv)
    }

    /// The session other windows can attach to, while the app runs.
    pub fn session_info(&self) -> Option<SessionInfo> {
        lock(&self.info).clone()
    }

    /// Run the way the environment asks: in this terminal (and attachable), or in
    /// the background with `STAGETUI_SERVE=1` (what `stagetui start` does).
    /// `name` suggests the session name; `STAGETUI_SESSION` overrides it.
    pub fn launch(self: &Arc<Self>, name: &str, app: &str) -> Result<()> {
        let name = std::env::var("STAGETUI_SESSION")
            .ok()
            .filter(|n| ipc::valid_name(n))
            .unwrap_or_else(|| name.to_string());
        let serve = Serve {
            name,
            app: app.to_string(),
            log: std::env::var("STAGETUI_LOG").ok().filter(|l| !l.is_empty()),
            dir: None,
        };
        if env_on("STAGETUI_SERVE") {
            return self.serve(serve);
        }
        let caps = caps::detect(&DetectOptions::default());
        let serve = (!env_on("STAGETUI_NO_SESSION")).then_some(serve);
        self.start_with(Some(caps), serve)
    }

    /// Take over the terminal and start drawing. Returns once the screen is ready.
    pub fn start(self: &Arc<Self>, caps: Capabilities) -> Result<()> {
        self.start_with(Some(caps), None)
    }

    /// Run without a terminal of its own: only attached windows show it.
    pub fn serve(self: &Arc<Self>, serve: Serve) -> Result<()> {
        self.start_with(None, Some(serve))
    }

    /// Start drawing in this terminal (`local`), serving a session, or both.
    pub fn start_with(
        self: &Arc<Self>,
        local: Option<Capabilities>,
        serve: Option<Serve>,
    ) -> Result<()> {
        if let Some(caps) = &local
            && caps.headless
        {
            return Err(Error::friendly(
                "Can't start an interactive app here",
                format!(
                    "stagetui is in headless mode: {}",
                    caps.headless_reason.as_deref().unwrap_or("no terminal")
                ),
                "run it inside a terminal, or capture it with `app.run_headless()` / `stagetui render`",
            ));
        }
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(Error::friendly(
                "This app is already running",
                "an app runs once at a time",
                "wait for it to finish before calling run() again",
            ));
        }
        // Without a terminal of its own the session is required; with one it's a bonus.
        let registration = match serve.map(|s| {
            let registry = match &s.dir {
                Some(d) => ipc::Registry::at(d),
                None => ipc::Registry::open(),
            };
            registry.and_then(|r| r.register(&s.name, &s.app, s.log.clone()))
        }) {
            Some(Ok(r)) => Some(r),
            Some(Err(e)) if local.is_none() => {
                self.running.store(false, Ordering::SeqCst);
                return Err(e);
            }
            _ => None,
        };
        let has_local = local.is_some();
        lock(&self.queue).clear();
        let (tx, rx) = mpsc::channel();
        *lock(&self.wake) = Some(tx.clone());
        lock(&WAKERS).push(tx.clone());
        *lock(&self.info) = registration.as_ref().map(|r| r.info.clone());
        self.session()
            .set_session_name(registration.as_ref().map(|r| r.info.name.clone()));
        *lock(&self.waker) = registration.as_ref().map(Registration::waker);
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<()>>(1);
        let me = Arc::clone(self);
        let render = std::thread::Builder::new()
            .name("stagetui-render".into())
            .spawn(move || me.render_loop(local, rx, ready_tx))?;
        let failed = match ready_rx.recv() {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e),
            Err(_) => Some(Error::friendly(
                "The app stopped while starting",
                "the drawing thread ended unexpectedly",
                "run `stagetui doctor` and report the output",
            )),
        };
        if let Some(e) = failed {
            let _ = render.join();
            self.running.store(false, Ordering::SeqCst);
            *lock(&self.info) = None;
            *lock(&self.waker) = None;
            return Err(e);
        }
        let mut threads = vec![render];
        if has_local {
            let me = Arc::clone(self);
            let tx = tx.clone();
            threads.push(
                std::thread::Builder::new()
                    .name("stagetui-input".into())
                    .spawn(move || {
                        while me.is_running() {
                            match crossterm::event::poll(Duration::from_millis(50)) {
                                Ok(true) => {
                                    if let Ok(ev) = crossterm::event::read()
                                        && let Some(i) = input::from_crossterm(ev)
                                        && tx.send(Msg::Input(LOCAL, i)).is_err()
                                    {
                                        break;
                                    }
                                }
                                Ok(false) => {}
                                Err(_) => break,
                            }
                        }
                    })?,
            );
        }
        if let Some(reg) = registration {
            let me = Arc::clone(self);
            threads.push(
                std::thread::Builder::new()
                    .name("stagetui-accept".into())
                    .spawn(move || me.accept_loop(reg, tx))?,
            );
        }
        lock(&self.threads).extend(threads);
        Ok(())
    }

    /// Welcome windows that attach, until the app stops.
    fn accept_loop(self: Arc<Self>, reg: Registration, tx: Sender<Msg>) {
        while self.is_running() {
            let Ok(stream) = reg.accept() else {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            };
            if !self.is_running() {
                break;
            }
            let id = self.windows.fetch_add(1, Ordering::SeqCst);
            let (tx, token, name) = (tx.clone(), reg.token.clone(), reg.info.name.clone());
            let _ = std::thread::Builder::new()
                .name("stagetui-ipc-in".into())
                .spawn(move || match ipc::handshake(stream, &token, &name) {
                    Hello::Viewer(v) => {
                        let ipc::Viewer {
                            caps,
                            size,
                            pane,
                            conn,
                            out,
                        } = *v;
                        let view = TerminalView::new(FrameWriter::new(out.clone()), &caps, size);
                        let remote = Remote {
                            id,
                            view,
                            out,
                            caps,
                            wants: pane,
                            pane: None,
                        };
                        if tx.send(Msg::Attach(Box::new(remote))).is_err() {
                            return;
                        }
                        let mut r = &*conn;
                        loop {
                            match ipc::recv::<ClientMsg>(&mut r) {
                                Ok(ClientMsg::Input { input }) => {
                                    if tx.send(Msg::Input(id, input)).is_err() {
                                        return;
                                    }
                                }
                                Ok(ClientMsg::Detach) | Err(_) => break,
                                Ok(_) => {}
                            }
                        }
                        let _ = tx.send(Msg::Gone(id));
                    }
                    Hello::Quit => {
                        let _ = tx.send(Msg::Quit);
                    }
                    Hello::Nothing => {}
                });
        }
        drop(reg);
    }

    fn render_loop(
        self: Arc<Self>,
        local: Option<Capabilities>,
        rx: mpsc::Receiver<Msg>,
        ready: mpsc::SyncSender<Result<()>>,
    ) {
        let mut guard = None;
        let mut windows: Vec<Window> = Vec::new();
        if let Some(caps) = &local {
            match TerminalGuard::enter(Modes::app(caps)) {
                Ok(g) => guard = Some(g),
                Err(e) => {
                    let _ = ready.send(Err(e.into()));
                    return;
                }
            }
            let size = crossterm::terminal::size().unwrap_or((caps.size.cols, caps.size.rows));
            let view = TerminalView::new(io::stdout(), caps, size);
            self.session().set_cell_px(view.cell_px());
            windows.push(Window::Local(Box::new(view), Box::new(caps.clone())));
        }
        let _ = ready.send(Ok(()));
        let start = Instant::now();
        // The window used last: the app takes its size.
        let mut active = LOCAL;
        'run: loop {
            let mains = || windows.iter().filter(|w| w.pane().is_none());
            let size = mains()
                .find(|w| w.id() == active)
                .or(mains().next())
                .map(Window::size);
            let (frame, pane_frames, wake, quit, events, clip, popouts, popins) = {
                let mut s = self.session();
                s.set_time(start.elapsed());
                let dirty = s.is_dirty() && !windows.is_empty();
                // Pop-out windows only: keep the app's own screen up to date anyway.
                let fallback = match s.size() {
                    (0, _) | (_, 0) => (100, 30),
                    other => other,
                };
                let frame =
                    dirty.then(|| s.frame(size.unwrap_or(fallback).0, size.unwrap_or(fallback).1));
                let pane_frames: Vec<(u64, Option<Frame>)> = if dirty {
                    windows
                        .iter()
                        .filter_map(|w| {
                            let p = w.pane()?;
                            let (c, r) = w.size();
                            Some((w.id(), s.pane_frame(p, c, r)))
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                (
                    frame,
                    pane_frames,
                    s.next_wake(),
                    s.quit_requested(),
                    s.take_events(),
                    s.take_clipboard(),
                    s.take_popouts(),
                    s.take_popins(),
                )
            };
            self.push(events);
            if quit {
                break;
            }
            // Panes that went back: their windows close.
            for id in &popins {
                let pane = self.session().tree().find(&format!("#{id}"));
                for w in windows
                    .iter()
                    .filter(|w| w.pane().is_some() && w.pane() == pane)
                {
                    w.bye("reattached");
                }
                windows.retain(|w| w.pane().is_none() || w.pane() != pane);
            }
            // Panes that want their own window: open one like the window used last.
            for id in popouts {
                let caps = windows
                    .iter()
                    .find(|w| w.id() == active)
                    .or(windows.first())
                    .map(|w| w.caps().clone());
                let opened = match caps {
                    Some(c) => self.open_window(&id, &c),
                    None => Err(Error::friendly(
                        "No window to open from",
                        "nobody is looking at the app right now",
                        "attach first: stagetui attach",
                    )),
                };
                if let Err(e) = opened {
                    let mut s = self.session();
                    let _ = s.pop_in(&format!("#{id}"));
                    let _ = s.notify(&e.to_string(), Some("info"), None, Duration::from_secs(8));
                }
            }
            if let Some(text) = clip
                && let Some(w) = windows.iter_mut().find(|w| w.id() == active)
            {
                w.raw(&osc52(&text));
            }
            let mut lost = Vec::new();
            for (id, f) in pane_frames {
                let Some(w) = windows.iter_mut().find(|w| w.id() == id) else {
                    continue;
                };
                match f {
                    Some(f) => {
                        if w.present(&f).is_err() {
                            lost.push(id);
                        }
                    }
                    // The pane is gone (closed, or back): so is its window.
                    None => {
                        w.bye("reattached");
                        lost.push(id);
                    }
                }
            }
            if let (Some(f), Some(size)) = (frame, size) {
                for w in windows.iter_mut().filter(|w| w.pane().is_none()) {
                    let shown = if w.size() == size {
                        w.present(&f)
                    } else {
                        w.present(&fit(&f, w.size()))
                    };
                    if let Err(e) = shown {
                        if w.id() == LOCAL {
                            *lock(&self.failure) = Some(e.into());
                            break 'run;
                        }
                        lost.push(w.id());
                    }
                }
            }
            if !lost.is_empty() {
                let mut s = self.session();
                for w in windows.iter().filter(|w| lost.contains(&w.id())) {
                    if let Some(p) = w.pane() {
                        s.pane_window_closed(p);
                    }
                }
                windows.retain(|w| !lost.contains(&w.id()));
            }
            let timeout = wake
                .unwrap_or(Duration::from_secs(1))
                .max(Duration::from_millis(1));
            let first = match rx.recv_timeout(timeout) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            let mut stop = false;
            let mut inputs = Vec::new();
            {
                let mut s = self.session();
                for m in first.into_iter().chain(rx.try_iter()) {
                    match m {
                        Msg::Input(id, i) => inputs.push((id, i)),
                        Msg::Attach(mut r) => {
                            if let Some(want) = r.wants.clone() {
                                // A window for one pane: it pops out into it.
                                let pane = s
                                    .tree()
                                    .find(&format!("#{want}"))
                                    .filter(|p| s.tree().is_typed_pane(*p));
                                match pane {
                                    Some(p) => {
                                        s.pane_window_opened(p);
                                        r.pane = Some(p);
                                        windows.push(Window::Remote(r));
                                    }
                                    None => {
                                        let message = format!("there is no pane \"{want}\"");
                                        let _ =
                                            r.out.try_send(Out::Msg(ServerMsg::Error { message }));
                                        Window::Remote(r).bye("error");
                                    }
                                }
                                s.touch();
                                continue;
                            }
                            active = r.id;
                            s.set_cell_px(r.view.cell_px());
                            windows.push(Window::Remote(r));
                            s.touch();
                        }
                        Msg::Gone(id) => {
                            if let Some(p) =
                                windows.iter().find(|w| w.id() == id).and_then(Window::pane)
                            {
                                s.pane_window_closed(p);
                            }
                            windows.retain(|w| w.id() != id);
                            if active == id {
                                active = windows.last().map_or(LOCAL, Window::id);
                            }
                            s.touch();
                        }
                        Msg::Quit => s.request_quit(),
                        Msg::Wake => {}
                        Msg::Stop => stop = true,
                    }
                }
                for (id, i) in &inputs {
                    let Some(w) = windows.iter_mut().find(|w| w.id() == *id) else {
                        continue; // a window that already left
                    };
                    if let Input::Resize(c, r) = i {
                        w.resize(*c, *r);
                    }
                    if let Some(p) = w.pane() {
                        // A pop-out window: its pane gets the keys; leaving puts it back.
                        s.set_detachable(true);
                        s.handle_in(Some(p), i);
                        if s.take_detach() {
                            // Ctrl+Space d in a pop-out window: the pane goes back.
                            let pid = s.tree().nodes[p].id.clone();
                            let _ = s.pop_in(&format!("#{pid}"));
                        }
                        continue;
                    }
                    if *id != active {
                        active = *id;
                        s.set_cell_px(w.cell_px());
                        s.touch();
                    }
                    // q in an attached window asks: detach or quit the app?
                    s.set_detachable(*id != LOCAL);
                    s.handle_in(None, i);
                    if s.take_detach() {
                        self.detach(&mut s, &mut windows, *id);
                        if active == *id {
                            active = windows.last().map_or(LOCAL, Window::id);
                        }
                    }
                }
            }
            if stop {
                break;
            }
        }
        for w in &windows {
            w.bye("quit");
        }
        drop(windows);
        if let Some(mut g) = guard {
            let _ = g.restore();
        }
        self.running.store(false, Ordering::SeqCst);
        if let Some(w) = lock(&self.waker).take() {
            w.wake();
        }
        *lock(&self.info) = None;
        let mut s = self.session();
        s.request_quit();
        let events = s.take_events();
        drop(s);
        self.push(events);
    }

    /// A window asked to leave (prefix + d).
    fn detach(&self, s: &mut Session, windows: &mut Vec<Window>, id: u64) {
        if id != LOCAL {
            if let Some(w) = windows.iter().find(|w| w.id() == id) {
                w.bye("detached");
            }
            windows.retain(|w| w.id() != id);
            s.touch();
            return;
        }
        // The terminal the app runs in can't let go of it: say how to get there.
        let app = lock(&self.info)
            .as_ref()
            .map_or_else(|| "my_app.py".to_string(), |i| i.app.clone());
        let _ = s.notify(
            &format!("To detach, start it in the background: stagetui start {app}"),
            Some("info"),
            None,
            Duration::from_secs(6),
        );
    }

    fn push(&self, events: Vec<AppEvent>) {
        if events.is_empty() {
            return;
        }
        lock(&self.queue).extend(events);
        self.ready.notify_all();
    }

    /// Wait up to `timeout` for the next event. `None` on timeout, or when the
    /// app has stopped and every event was read.
    pub fn next_event(&self, timeout: Duration) -> Option<AppEvent> {
        let deadline = Instant::now() + timeout;
        let mut q = lock(&self.queue);
        loop {
            if let Some(e) = q.pop_front() {
                return Some(e);
            }
            if !self.is_running() {
                return None;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            q = self
                .ready
                .wait_timeout(q, left)
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }

    /// Stop drawing, give the terminal back, and wait for the threads.
    pub fn stop(&self) -> Result<()> {
        if let Some(tx) = lock(&self.wake).take() {
            let _ = tx.send(Msg::Stop);
        }
        let threads: Vec<_> = lock(&self.threads).drain(..).collect();
        for t in threads {
            let _ = t.join();
        }
        self.running.store(false, Ordering::SeqCst);
        match lock(&self.failure).take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_to_start_headless() {
        let caps = crate::caps::detect(&crate::caps::DetectOptions {
            force_headless: true,
            ..Default::default()
        });
        let app = App::new(Session::from_json(r#"{"children":[]}"#).unwrap());
        let err = app.start(caps).unwrap_err().to_string();
        assert!(err.contains("headless"), "{err}");
        assert!(app.next_event(Duration::from_millis(1)).is_none());
    }

    #[test]
    fn with_changes_the_session() {
        let app = App::new(
            Session::from_json(
                r#"{"children":[{"type":"card","content":"a","props":{"id":"c"}}]}"#,
            )
            .unwrap(),
        );
        app.with(|s| s.update("#c", &serde_json::json!({"content": "b"})))
            .unwrap();
        assert_eq!(app.session().value("#c").unwrap(), serde_json::Value::Null);
        let snap = crate::headless::snapshot(&mut app.session(), 30, 8);
        assert!(snap.text.contains('b'));
    }

    mod windows {
        use super::*;
        use crate::caps::{Env, Grid, ProbeInfo, TtyInfo, parse, resolve};
        use crate::input::Key;
        use crate::ipc::{PROTOCOL, Registry};
        use interprocess::local_socket::Stream;
        use interprocess::local_socket::traits::Stream as _;

        fn term(replies: &[u8]) -> Capabilities {
            resolve(
                &Env::default(),
                TtyInfo {
                    stdin: true,
                    stdout: true,
                },
                None,
                &parse::parse(replies),
                Grid {
                    cols: 50,
                    rows: 14,
                    width_px: 500,
                    height_px: 280,
                },
                ProbeInfo::default(),
            )
        }

        fn open(reg: &Registry, name: &str, caps: Capabilities, size: (u16, u16)) -> Stream {
            open_pane(reg, name, caps, size, None)
        }

        fn open_pane(
            reg: &Registry,
            name: &str,
            caps: Capabilities,
            size: (u16, u16),
            pane: Option<&str>,
        ) -> Stream {
            let (mut s, _, token) = reg.connect(Some(name)).unwrap();
            // Not on Windows pipes; there a stuck test is ended by the CI timeout.
            let _ = s.set_recv_timeout(Some(Duration::from_secs(10)));
            let hello = ClientMsg::Hello {
                token,
                protocol: PROTOCOL,
                caps: Box::new(caps),
                cols: size.0,
                rows: size.1,
                pane: pane.map(String::from),
            };
            ipc::send(&mut s, &hello).unwrap();
            let reply = ipc::recv::<ServerMsg>(&mut s).unwrap();
            assert!(matches!(reply, ServerMsg::Welcome { .. }), "{reply:?}");
            s
        }

        /// Read output until `done` says so; returns it (and a goodbye, if any).
        fn read_until(s: &mut Stream, done: impl Fn(&str) -> bool) -> (String, Option<ServerMsg>) {
            let mut seen = String::new();
            loop {
                let (kind, bytes) = ipc::read_frame(s)
                    .unwrap_or_else(|e| panic!("the app stopped talking ({e}); got {seen:?}"));
                if kind == ipc::KIND_JSON {
                    return (seen, serde_json::from_slice(&bytes).ok());
                }
                seen.push_str(&String::from_utf8_lossy(&bytes));
                if done(&seen) {
                    return (seen, None);
                }
            }
        }

        fn press(s: &mut Stream, key: &str) {
            let input = Input::Key(Key::parse(key).unwrap());
            ipc::send(s, &ClientMsg::Input { input }).unwrap();
        }

        #[test]
        fn two_windows_with_different_graphics_detach_and_quit() {
            let dir =
                std::env::temp_dir().join(format!("stagetui-test-app-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let reg = Registry::at(&dir).unwrap();
            let mut px = tiny_skia::Pixmap::new(40, 40).unwrap();
            px.fill(tiny_skia::Color::from_rgba8(220, 40, 40, 255));
            let img = crate::gfx::register(crate::gfx::Image::from_pixmap("red", px));
            let text = "مَرْحَبًا Hello 日本語 नमस्ते";
            let app = App::new(
                Session::from_json(&format!(
                    r#"{{"children":[{{"type":"card","content":"{text}","props":{{"title":"Hello"}}}},{{"type":"image","content":"{img}","props":{{"width":8,"height":4}}}}]}}"#
                ))
                .unwrap(),
            );
            app.serve(Serve {
                name: "Two Windows".into(),
                app: "test.py".into(),
                log: None,
                dir: Some(dir.clone()),
            })
            .unwrap();
            let info = app.session_info().unwrap();
            assert_eq!(info.name, "two-windows");
            assert_eq!(reg.list()[0].app, "test.py");

            // A Kitty window gets Kitty pictures; a plain one gets half blocks.
            let kitty = b"\x1bP>|kitty(0.39.1)\x1b\\\x1b_Gi=31;OK\x1b\\\x1b[?62c";
            let mut a = open(&reg, "two-windows", term(kitty), (50, 14));
            let (out, _) = read_until(&mut a, |o| o.contains("Hello") && o.contains("a=t"));
            assert!(out.contains("a=p"), "Kitty places the picture");
            let mut b = open(&reg, "two-windows", term(b"\x1b[?62c"), (50, 14));
            let (out, _) = read_until(&mut b, |o| o.contains("Hello") && o.contains('▀'));
            assert!(
                out.contains("日本語"),
                "plain terminals get the text as text"
            );
            assert!(
                !out.contains("\x1b_G"),
                "no Kitty codes for a plain terminal"
            );

            // Ctrl+Space then d: only that window leaves; the app keeps running.
            press(&mut b, "ctrl+space");
            press(&mut b, "d");
            let (_, bye) = read_until(&mut b, |_| false);
            assert!(
                matches!(bye, Some(ServerMsg::Bye { ref reason }) if reason == "detached"),
                "{bye:?}"
            );
            assert!(app.is_running());

            // q in an attached window asks first; q again quits the app for everyone.
            press(&mut a, "q");
            let (out, _) = read_until(&mut a, |o| o.contains("Quit app"));
            assert!(out.contains("Leave \"two-windows\"?"));
            press(&mut a, "q");
            let (_, bye) = read_until(&mut a, |_| false);
            assert!(
                matches!(bye, Some(ServerMsg::Bye { ref reason }) if reason == "quit"),
                "{bye:?}"
            );
            app.stop().unwrap();
            assert!(!app.is_running() && app.session_info().is_none());
            assert!(reg.list().is_empty(), "the session's files are gone");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn stop_from_outside_and_wrong_tokens() {
            let dir =
                std::env::temp_dir().join(format!("stagetui-test-stop-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let reg = Registry::at(&dir).unwrap();
            let app = App::new(Session::from_json(r#"{"children":[]}"#).unwrap());
            let serve = Serve {
                name: "stopper".into(),
                dir: Some(dir.clone()),
                ..Serve::default()
            };
            app.serve(serve).unwrap();
            let (mut s, _, _) = reg.connect(Some("stopper")).unwrap();
            ipc::send(
                &mut s,
                &ClientMsg::Quit {
                    token: "guess".into(),
                },
            )
            .unwrap();
            assert!(matches!(
                ipc::recv::<ServerMsg>(&mut s).unwrap(),
                ServerMsg::Error { .. }
            ));
            assert!(app.is_running(), "a wrong token changes nothing");
            reg.stop(Some("stopper")).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.is_running() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(!app.is_running());
            app.stop().unwrap();
            let quit = std::iter::from_fn(|| app.next_event(Duration::ZERO)).any(|e| e.id == "app");
            assert!(quit, "the app hears that it was asked to quit");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_pane_pops_out_into_a_window_and_snaps_back_when_it_closes() {
            let dir =
                std::env::temp_dir().join(format!("stagetui-test-pop-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let reg = Registry::at(&dir).unwrap();
            let app = App::new(
                Session::from_json(
                    r##"{"children":[{"type":"split","children":[
                        {"type":"pane","content":"markdown","props":{"id":"doc"},"children":[{"type":"markdown","content":"# مَرْحَبًا Hello 日本語 नमस्ते"}]},
                        {"type":"pane","content":"notes","props":{"id":"notes"},"children":[{"type":"input","props":{"id":"box"}}]}]}]}"##,
                )
                .unwrap(),
            );
            let asked: Arc<Mutex<Vec<Vec<String>>>> = Arc::default();
            let log = Arc::clone(&asked);
            app.set_attach_command(vec!["python".into(), "-m".into(), "stagetui".into()]);
            app.set_launcher(Arc::new(move |cmd: &[String], _caps: &Capabilities| {
                lock(&log).push(cmd.to_vec());
                Ok(())
            }));
            app.serve(Serve {
                name: "popper".into(),
                dir: Some(dir.clone()),
                ..Serve::default()
            })
            .unwrap();
            let plain = || term(b"\x1b[?62c");
            let mut main = open(&reg, "popper", plain(), (80, 20));
            read_until(&mut main, |o| o.contains("Hello"));

            // Ctrl+Space then o on the focused pane asks for a new window …
            let mut s = app.session();
            s.focus("#box").unwrap();
            drop(s);
            app.wake();
            press(&mut main, "ctrl+space");
            press(&mut main, "o");
            let (out, _) = read_until(&mut main, |o| o.contains("open in its own window"));
            assert!(out.contains("Bring it back"));
            let deadline = Instant::now() + Duration::from_secs(10);
            while lock(&asked).is_empty() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                lock(&asked)[0],
                ["python", "-m", "stagetui", "attach", "popper:notes"],
                "the window runs `stagetui attach app:pane`"
            );

            // … which shows just that pane, and types into it.
            let mut win = open_pane(&reg, "popper", plain(), (40, 8), Some("notes"));
            read_until(&mut win, |o| o.contains("Put back"));
            for c in ["h", "i"] {
                press(&mut win, c);
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.session().value("#box").unwrap() != serde_json::json!("hi")
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                app.session().value("#box").unwrap(),
                serde_json::json!("hi")
            );

            // Closing the window snaps the pane back, with everything in it.
            drop(win);
            let (out, _) = read_until(&mut main, |o| o.contains("hi") && !o.ends_with("window"));
            assert!(out.contains("hi"));
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.session().query("#notes").unwrap()["detached"] == true
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(app.session().query("#notes").unwrap()["detached"], false);

            // A pane that doesn't exist is refused.
            let (mut s, _, token) = reg.connect(Some("popper")).unwrap();
            let hello = ClientMsg::Hello {
                token,
                protocol: PROTOCOL,
                caps: Box::new(plain()),
                cols: 40,
                rows: 8,
                pane: Some("nope".into()),
            };
            ipc::send(&mut s, &hello).unwrap();
            assert!(matches!(
                ipc::recv::<ServerMsg>(&mut s).unwrap(),
                ServerMsg::Welcome { .. }
            ));
            let (_, msg) = read_until(&mut s, |_| false);
            assert!(
                matches!(msg, Some(ServerMsg::Error { ref message }) if message.contains("nope")),
                "{msg:?}"
            );
            app.stop().unwrap();
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
