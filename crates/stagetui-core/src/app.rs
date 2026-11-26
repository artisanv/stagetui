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
}

enum Window {
    Local(Box<TerminalView<io::Stdout>>),
    Remote(Box<Remote>),
}

impl Window {
    fn id(&self) -> u64 {
        match self {
            Window::Local(_) => LOCAL,
            Window::Remote(r) => r.id,
        }
    }

    fn size(&self) -> (u16, u16) {
        match self {
            Window::Local(v) => v.size(),
            Window::Remote(r) => r.view.size(),
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        match self {
            Window::Local(v) => v.resize(cols, rows),
            Window::Remote(r) => r.view.resize(cols, rows),
        }
    }

    fn cell_px(&self) -> (u32, u32) {
        match self {
            Window::Local(v) => v.cell_px(),
            Window::Remote(r) => r.view.cell_px(),
        }
    }

    fn present(&mut self, frame: &Frame) -> io::Result<()> {
        match self {
            Window::Local(v) => v.present(frame),
            Window::Remote(r) => r.view.present(frame),
        }
    }

    /// Bytes for the terminal itself (e.g. the clipboard sequence).
    fn raw(&mut self, bytes: &str) {
        match self {
            Window::Local(_) => {
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
                            conn,
                            out,
                        } = *v;
                        let view = TerminalView::new(FrameWriter::new(out.clone()), &caps, size);
                        if tx
                            .send(Msg::Attach(Box::new(Remote { id, view, out })))
                            .is_err()
                        {
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
            windows.push(Window::Local(Box::new(view)));
        }
        let _ = ready.send(Ok(()));
        let start = Instant::now();
        // The window used last: the app takes its size.
        let mut active = LOCAL;
        'run: loop {
            let size = windows
                .iter()
                .find(|w| w.id() == active)
                .or(windows.first())
                .map(Window::size);
            let (frame, wake, quit, events, clip) = {
                let mut s = self.session();
                s.set_time(start.elapsed());
                let frame = match size {
                    Some((c, r)) if s.is_dirty() => Some(s.frame(c, r)),
                    _ => None,
                };
                (
                    frame,
                    s.next_wake(),
                    s.quit_requested(),
                    s.take_events(),
                    s.take_clipboard(),
                )
            };
            self.push(events);
            if quit {
                break;
            }
            if let Some(text) = clip
                && let Some(w) = windows.iter_mut().find(|w| w.id() == active)
            {
                w.raw(&osc52(&text));
            }
            if let (Some(f), Some(size)) = (frame, size) {
                let mut lost = Vec::new();
                for w in &mut windows {
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
                        Msg::Attach(r) => {
                            active = r.id;
                            s.set_cell_px(r.view.cell_px());
                            windows.push(Window::Remote(r));
                            s.touch();
                        }
                        Msg::Gone(id) => {
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
                    if *id != active {
                        active = *id;
                        s.set_cell_px(w.cell_px());
                        s.touch();
                    }
                    s.handle(i);
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
            let (mut s, _, token) = reg.connect(Some(name)).unwrap();
            s.set_recv_timeout(Some(Duration::from_secs(10))).unwrap();
            let hello = ClientMsg::Hello {
                token,
                protocol: PROTOCOL,
                caps: Box::new(caps),
                cols: size.0,
                rows: size.1,
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

            // q in the other window quits the app for everyone.
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
    }
}
