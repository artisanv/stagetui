//! Running an app in a real terminal.
//!
//! A [`Session`] is driven by two background threads: one reads keyboard and
//! mouse input, the other draws frames only when something changed (dirty-driven,
//! ~0% CPU when idle). Events for the app (`click`, `change`, actions…) wait in
//! a queue that the caller — usually Python — reads with [`App::next_event`].
//! Changes from the caller go through [`App::with`], which wakes the renderer.

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::caps::Capabilities;
use crate::input::{self, Input};
use crate::session::{AppEvent, Session};
use crate::term::{Modes, TerminalGuard};
use crate::view::{TerminalView, View};
use crate::{Error, Result};

enum Msg {
    Input(Input),
    Wake,
    Stop,
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
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
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

    /// Take over the terminal and start drawing. Returns once the screen is ready.
    pub fn start(self: &Arc<Self>, caps: Capabilities) -> Result<()> {
        if caps.headless {
            return Err(Error::friendly(
                "Can't start an interactive app here",
                format!(
                    "termstage is in headless mode: {}",
                    caps.headless_reason.as_deref().unwrap_or("no terminal")
                ),
                "run it inside a terminal, or capture it with `app.run_headless()` / `termstage render`",
            ));
        }
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(Error::friendly(
                "This app is already running",
                "an app can be shown in one terminal at a time",
                "wait for it to finish before calling run() again",
            ));
        }
        lock(&self.queue).clear();
        let (tx, rx) = mpsc::channel();
        *lock(&self.wake) = Some(tx.clone());
        lock(&WAKERS).push(tx.clone());
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<()>>(1);
        let me = Arc::clone(self);
        let render = std::thread::Builder::new()
            .name("termstage-render".into())
            .spawn(move || me.render_loop(caps, rx, ready_tx))?;
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = render.join();
                self.running.store(false, Ordering::SeqCst);
                return Err(e);
            }
            Err(_) => {
                let _ = render.join();
                self.running.store(false, Ordering::SeqCst);
                return Err(Error::friendly(
                    "The app stopped while starting",
                    "the drawing thread ended unexpectedly",
                    "run `termstage doctor` and report the output",
                ));
            }
        }
        let me = Arc::clone(self);
        let reader = std::thread::Builder::new()
            .name("termstage-input".into())
            .spawn(move || {
                while me.is_running() {
                    match crossterm::event::poll(Duration::from_millis(50)) {
                        Ok(true) => {
                            if let Ok(ev) = crossterm::event::read()
                                && let Some(i) = input::from_crossterm(ev)
                                && tx.send(Msg::Input(i)).is_err()
                            {
                                break;
                            }
                        }
                        Ok(false) => {}
                        Err(_) => break,
                    }
                }
            })?;
        let mut threads = lock(&self.threads);
        threads.push(render);
        threads.push(reader);
        Ok(())
    }

    fn render_loop(
        self: Arc<Self>,
        caps: Capabilities,
        rx: mpsc::Receiver<Msg>,
        ready: mpsc::SyncSender<Result<()>>,
    ) {
        let mut guard = match TerminalGuard::enter(Modes::app(&caps)) {
            Ok(g) => g,
            Err(e) => {
                let _ = ready.send(Err(e.into()));
                return;
            }
        };
        let size = crossterm::terminal::size().unwrap_or((caps.size.cols, caps.size.rows));
        let mut view = TerminalView::new(io::stdout(), &caps, size);
        self.session().set_cell_px(view.cell_px());
        let _ = ready.send(Ok(()));
        let start = Instant::now();
        loop {
            let (frame, wake, quit, events, clip) = {
                let mut s = self.session();
                s.set_time(start.elapsed());
                let frame = if s.is_dirty() {
                    let (c, r) = view.size();
                    Some(s.frame(c, r))
                } else {
                    None
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
            if let Some(text) = clip {
                use std::io::Write as _;
                let mut out = io::stdout();
                let _ = out.write_all(osc52(&text).as_bytes());
                let _ = out.flush();
            }
            if let Some(f) = frame
                && let Err(e) = view.present(&f)
            {
                *lock(&self.failure) = Some(e.into());
                break;
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
            for m in first.into_iter().chain(rx.try_iter()) {
                match m {
                    Msg::Input(i) => inputs.push(i),
                    Msg::Wake => {}
                    Msg::Stop => stop = true,
                }
            }
            if !inputs.is_empty() {
                let mut s = self.session();
                for i in &inputs {
                    if let Input::Resize(c, r) = i {
                        view.resize(*c, *r);
                    }
                    s.handle(i);
                }
            }
            if stop {
                break;
            }
        }
        let _ = guard.restore();
        self.running.store(false, Ordering::SeqCst);
        let mut s = self.session();
        s.request_quit();
        let events = s.take_events();
        drop(s);
        self.push(events);
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
}
