//! Sessions: a running app that terminal windows attach to.
//!
//! Every interactive app listens on a private local socket (a Unix socket in a
//! user-only folder, or a Windows named pipe). `stagetui attach` connects to it,
//! proves it knows the session's secret token, sends its terminal's
//! capabilities and its keys, and receives ready-to-print terminal output —
//! each window drawn with its own best graphics.
//!
//! Wire format: `[u32 little-endian length][u8 kind][payload]`; kind 1 is a JSON
//! control message, kind 2 is raw terminal output.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{Listener, ListenerOptions, Name, Stream};
use serde::{Deserialize, Serialize};

use crate::caps::{self, Capabilities, DetectOptions};
use crate::input::{self, Input};
use crate::term::{Modes, TerminalGuard};
use crate::{Error, Result};

/// Bumped when the messages change.
pub const PROTOCOL: u32 = 2;
/// Largest message accepted (pictures can be big, but not this big).
const MAX_FRAME: usize = 64 * 1024 * 1024;
pub const KIND_JSON: u8 = 1;
pub const KIND_OUTPUT: u8 = 2;
/// Frames waiting for a slow window before it is let go.
const BACKLOG: usize = 256;

/// What `stagetui sessions` shows about a running app.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionInfo {
    pub name: String,
    pub pid: u32,
    /// The program, e.g. `examples/07_harness.py`.
    pub app: String,
    pub cwd: String,
    /// Unix time in seconds.
    pub started: u64,
    pub protocol: u32,
    /// Where the app's output goes when it runs in the background.
    #[serde(default)]
    pub log: Option<String>,
}

/// From a terminal window to the app.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello {
        token: String,
        protocol: u32,
        caps: Box<Capabilities>,
        cols: u16,
        rows: u16,
        /// Show just this pane (its id): the pane pops out into this window.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane: Option<String>,
    },
    Input {
        input: Input,
    },
    /// The window is leaving; the app keeps running.
    Detach,
    /// Stop the app (`stagetui stop`).
    Quit {
        token: String,
    },
}

/// From the app to a terminal window.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMsg {
    Welcome {
        session: String,
    },
    /// The window should restore itself and exit. `reason`: `detached`, `quit`,
    /// `reattached` (a pane went back), or `error` (after an `Error`).
    Bye {
        reason: String,
    },
    Error {
        message: String,
    },
}

// ----- the private folder ----------------------------------------------------

/// The user-only folder where session sockets and info files live.
pub fn runtime_dir() -> Result<PathBuf> {
    #[cfg(unix)]
    let dir = match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(base) => PathBuf::from(base).join("stagetui"),
        None => {
            std::env::temp_dir().join(format!("stagetui-{}", rustix::process::getuid().as_raw()))
        }
    };
    #[cfg(not(unix))]
    let dir = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("stagetui")
        .join("sessions");
    private_dir(&dir)?;
    Ok(dir)
}

/// Create `dir` readable only by this user (and refuse one owned by someone else).
pub fn private_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        if !dir.exists() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
        }
        // Never trust a folder someone else created for us (e.g. in /tmp).
        let meta = fs::symlink_metadata(dir)?;
        if !meta.is_dir() || meta.uid() != rustix::process::getuid().as_raw() {
            return Err(Error::friendly(
                format!("The session folder {} is not yours", dir.display()),
                "stagetui keeps session sockets in a folder only you can open",
                format!(
                    "remove {} (or set XDG_RUNTIME_DIR) and try again",
                    dir.display()
                ),
            ));
        }
        if meta.mode() & 0o077 != 0 {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)?;
        Ok(())
    }
}

fn socket_name(dir: &Path, name: &str) -> io::Result<Name<'static>> {
    #[cfg(unix)]
    {
        dir.join(format!("{name}.sock"))
            .to_fs_name::<interprocess::local_socket::GenericFilePath>()
    }
    #[cfg(not(unix))]
    {
        // Named pipes live in one global namespace: include the folder's hash so
        // test registries and real ones never meet.
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        dir.hash(&mut h);
        format!("stagetui-{:08x}-{name}", h.finish() as u32)
            .to_ns_name::<interprocess::local_socket::GenericNamespaced>()
    }
}

/// Connect to a session, waiting a few seconds at most when it is busy
/// (a Windows pipe that is between two windows).
fn dial(dir: &Path, name: &str) -> io::Result<Stream> {
    interprocess::local_socket::ConnectOptions::new()
        .name(socket_name(dir, name)?)
        .wait_mode(interprocess::ConnectWaitMode::Timeout(Duration::from_secs(
            3,
        )))
        .connect_sync()
}

/// A secret for one session: 256 random bits as hex.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    #[cfg(unix)]
    let from_os = fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .is_ok();
    #[cfg(not(unix))]
    let from_os = false;
    if !from_os {
        // Rust seeds every `RandomState` from the operating system's randomness.
        use std::hash::{BuildHasher, Hasher};
        for (i, chunk) in bytes.chunks_mut(8).enumerate() {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_usize(i);
            h.write_u128(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos()),
            );
            chunk.copy_from_slice(&h.finish().to_le_bytes());
        }
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Compare tokens without leaking how many characters matched.
pub fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// Session names are short slugs: letters, digits, `-` and `_`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Turn any text (an app's class name) into a session name.
pub fn slug(text: &str) -> String {
    let mut s = String::new();
    let mut prev_lower = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            // "HarnessApp" → "harness-app"
            if c.is_ascii_uppercase() && prev_lower {
                s.push('-');
            }
            prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
            s.push(c.to_ascii_lowercase());
        } else {
            prev_lower = false;
            s.push('-');
        }
    }
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s: String = s.trim_matches('-').chars().take(32).collect();
    let s = s.trim_end_matches('-');
    if s.is_empty() { "app".into() } else { s.into() }
}

fn private_file(path: &Path, contents: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let _ = fs::remove_file(path);
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(contents.as_bytes())
    }
    #[cfg(not(unix))]
    {
        fs::write(path, contents)
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

// ----- the registry ----------------------------------------------------------

/// The folder of running sessions.
#[derive(Debug, Clone)]
pub struct Registry {
    dir: PathBuf,
}

impl Registry {
    /// The current user's sessions.
    pub fn open() -> Result<Self> {
        Ok(Self {
            dir: runtime_dir()?,
        })
    }

    /// Sessions in another private folder (tests).
    pub fn at(dir: &Path) -> Result<Self> {
        private_dir(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn answering(&self, name: &str) -> bool {
        match dial(&self.dir, name) {
            Ok(_) => true,
            // Busy is alive; "not found" / "refused" means the app is gone.
            Err(e) => matches!(
                e.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ),
        }
    }

    fn remove_files(&self, name: &str) {
        for ext in ["json", "token", "sock"] {
            let _ = fs::remove_file(self.dir.join(format!("{name}.{ext}")));
        }
    }

    /// Running sessions, oldest first (entries left by crashed apps are cleaned up).
    pub fn list(&self) -> Vec<SessionInfo> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut out: Vec<SessionInfo> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| serde_json::from_slice::<SessionInfo>(&fs::read(e.path()).ok()?).ok())
            .filter(|info| {
                let alive = self.answering(&info.name);
                if !alive {
                    self.remove_files(&info.name);
                }
                alive
            })
            .collect();
        out.sort_by(|a, b| a.started.cmp(&b.started).then(a.name.cmp(&b.name)));
        out
    }

    /// Start serving a session named after `hint` (made unique: `hint`, `hint-2`, …).
    pub fn register(&self, hint: &str, app: &str, log: Option<String>) -> Result<Registration> {
        let base = slug(hint);
        let mut n = 1;
        let (name, listener) = loop {
            let candidate = if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            };
            if !self.answering(&candidate) {
                self.remove_files(&candidate);
                let opened = socket_name(&self.dir, &candidate)
                    .and_then(|n| ListenerOptions::new().name(n).create_sync());
                match opened {
                    Ok(l) => break (candidate, l),
                    Err(e) if e.kind() == io::ErrorKind::AddrInUse => {}
                    Err(e) => {
                        return Err(Error::friendly(
                            "Could not open a session for this app",
                            format!("{e} (in {})", self.dir.display()),
                            "set XDG_RUNTIME_DIR to a short folder only you can open, \
                             or STAGETUI_NO_SESSION=1 to run without sessions",
                        ));
                    }
                }
            }
            n += 1;
            if n > 99 {
                return Err(Error::friendly(
                    "Too many stagetui sessions are running",
                    format!("99 sessions are already called \"{base}\""),
                    "stop some with `stagetui stop <name>` (see `stagetui sessions`)",
                ));
            }
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                self.dir.join(format!("{name}.sock")),
                fs::Permissions::from_mode(0o600),
            )?;
        }
        let token = new_token();
        let info = SessionInfo {
            name: name.clone(),
            pid: std::process::id(),
            app: app.to_string(),
            cwd: std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            started: now_secs(),
            protocol: PROTOCOL,
            log,
        };
        let reg = Registration {
            info,
            token,
            listener: Some(listener),
            registry: self.clone(),
        };
        private_file(&self.dir.join(format!("{name}.token")), &reg.token)?;
        private_file(
            &self.dir.join(format!("{name}.json")),
            &serde_json::to_string(&reg.info)?,
        )?;
        Ok(reg)
    }

    /// Find a session: by name, or the only one running.
    pub fn find(&self, name: Option<&str>) -> Result<SessionInfo> {
        let sessions = self.list();
        let names = |v: &[SessionInfo]| {
            v.iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        match name {
            Some(n) => sessions
                .iter()
                .find(|s| s.name == n)
                .cloned()
                .ok_or_else(|| {
                    Error::friendly(
                        format!("No session named \"{n}\""),
                        if sessions.is_empty() {
                            "no stagetui app is running right now".to_string()
                        } else {
                            format!("running: {}", names(&sessions))
                        },
                        "check the name with `stagetui sessions`",
                    )
                }),
            None => match sessions.as_slice() {
                [one] => Ok(one.clone()),
                [] => Err(Error::friendly(
                    "No stagetui session is running",
                    "attach connects to an app that is already running",
                    "start one in the background with `stagetui start my_app.py`",
                )),
                many => Err(Error::friendly(
                    "Several sessions are running",
                    format!("running: {}", names(many)),
                    "pick one: `stagetui attach <name>`",
                )),
            },
        }
    }

    /// Connect to a session; returns the stream, its info and its token.
    pub fn connect(&self, name: Option<&str>) -> Result<(Stream, SessionInfo, String)> {
        let info = self.find(name)?;
        let token = fs::read_to_string(self.dir.join(format!("{}.token", info.name)))?;
        let stream = dial(&self.dir, &info.name)?;
        Ok((stream, info, token.trim().to_string()))
    }

    /// Ask a session to quit (without the "Leave?" question).
    pub fn stop(&self, name: Option<&str>) -> Result<SessionInfo> {
        let (mut stream, info, token) = self.connect(name)?;
        send(&mut stream, &ClientMsg::Quit { token })?;
        Ok(info)
    }
}

/// The user's running sessions.
pub fn list() -> Vec<SessionInfo> {
    Registry::open().map(|r| r.list()).unwrap_or_default()
}

/// A session this process serves. Dropping it removes the socket and files.
pub struct Registration {
    pub info: SessionInfo,
    pub token: String,
    listener: Option<Listener>,
    registry: Registry,
}

impl Registration {
    /// Wait for the next window (blocks).
    pub fn accept(&self) -> io::Result<Stream> {
        match &self.listener {
            Some(l) => l.accept(),
            None => Err(io::Error::other("the session was closed")),
        }
    }

    /// Something that unblocks [`Registration::accept`] from another thread.
    pub fn waker(&self) -> Waker {
        Waker {
            dir: self.registry.dir.clone(),
            name: self.info.name.clone(),
        }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.listener.take();
        self.registry.remove_files(&self.info.name);
    }
}

/// Unblocks a session's accept loop (by connecting to it).
#[derive(Debug, Clone)]
pub struct Waker {
    dir: PathBuf,
    name: String,
}

impl Waker {
    pub fn wake(&self) {
        let _ = dial(&self.dir, &self.name);
    }
}

// ----- framing ---------------------------------------------------------------

pub fn write_frame(w: &mut impl Write, kind: u8, payload: &[u8]) -> io::Result<()> {
    let len =
        u32::try_from(payload.len() + 1).map_err(|_| io::Error::other("message too large"))?;
    let mut buf = Vec::with_capacity(payload.len() + 5);
    buf.extend_from_slice(&len.to_le_bytes());
    buf.push(kind);
    buf.extend_from_slice(payload);
    w.write_all(&buf)?;
    w.flush()
}

pub fn read_frame(r: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad message size",
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    let kind = buf.remove(0);
    Ok((kind, buf))
}

pub fn send<T: Serialize>(w: &mut impl Write, msg: &T) -> io::Result<()> {
    let json = serde_json::to_vec(msg).map_err(io::Error::other)?;
    write_frame(w, KIND_JSON, &json)
}

/// Read one JSON message (output frames are an error here).
pub fn recv<T: for<'de> Deserialize<'de>>(r: &mut impl Read) -> io::Result<T> {
    let (kind, payload) = read_frame(r)?;
    if kind != KIND_JSON {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected a control message",
        ));
    }
    serde_json::from_slice(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

// ----- the app's side --------------------------------------------------------

/// Something for a window's writer thread.
pub enum Out {
    Frame(Vec<u8>),
    Msg(ServerMsg),
}

/// Where a remote window's `TerminalView` writes: each flush ships one frame.
pub struct FrameWriter {
    buf: Vec<u8>,
    tx: SyncSender<Out>,
}

impl FrameWriter {
    pub fn new(tx: SyncSender<Out>) -> Self {
        Self {
            buf: Vec::new(),
            tx,
        }
    }
}

impl Write for FrameWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        match self.tx.try_send(Out::Frame(std::mem::take(&mut self.buf))) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the window stopped reading",
            )),
            Err(TrySendError::Disconnected(_)) => {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "the window left"))
            }
        }
    }
}

/// A window that proved it may watch this session.
pub struct Viewer {
    pub caps: Capabilities,
    pub size: (u16, u16),
    /// The pane this window shows, if just one.
    pub pane: Option<String>,
    /// Read the window's messages with `ipc::recv(&mut &*viewer.conn)`.
    pub conn: Arc<Stream>,
    pub out: SyncSender<Out>,
}

/// What a new connection wants.
pub enum Hello {
    Viewer(Box<Viewer>),
    Quit,
    /// Wrong token, wrong version, or just `stagetui sessions` checking we're alive.
    Nothing,
}

/// Check a new connection's first message (it must carry the session token).
pub fn handshake(mut stream: Stream, token: &str, session: &str) -> Hello {
    let _ = stream.set_recv_timeout(Some(Duration::from_secs(5)));
    let Ok(msg) = recv::<ClientMsg>(&mut stream) else {
        return Hello::Nothing;
    };
    let refuse = |stream: &mut Stream, message: &str| {
        let _ = send(
            stream,
            &ServerMsg::Error {
                message: message.into(),
            },
        );
        Hello::Nothing
    };
    match msg {
        ClientMsg::Quit { token: t } if same_token(&t, token) => Hello::Quit,
        ClientMsg::Hello {
            token: t,
            protocol,
            caps,
            cols,
            rows,
            pane,
        } if same_token(&t, token) => {
            if protocol != PROTOCOL {
                return refuse(
                    &mut stream,
                    "this session runs a different stagetui version",
                );
            }
            let _ = stream.set_recv_timeout(None);
            let welcome = ServerMsg::Welcome {
                session: session.into(),
            };
            if send(&mut stream, &welcome).is_err() {
                return Hello::Nothing;
            }
            let conn = Arc::new(stream);
            Hello::Viewer(Box::new(Viewer {
                caps: *caps,
                size: (cols.max(1), rows.max(1)),
                pane,
                out: spawn_writer(Arc::clone(&conn)),
                conn,
            }))
        }
        _ => refuse(&mut stream, "wrong session token"),
    }
}

/// A thread that writes a window's frames in order; it ends after a `Bye`.
fn spawn_writer(conn: Arc<Stream>) -> SyncSender<Out> {
    let (tx, rx) = mpsc::sync_channel::<Out>(BACKLOG);
    let _ = std::thread::Builder::new()
        .name("stagetui-ipc-out".into())
        .spawn(move || {
            let mut send_half = &*conn;
            for out in rx {
                let r = match &out {
                    Out::Frame(bytes) => write_frame(&mut send_half, KIND_OUTPUT, bytes),
                    Out::Msg(m) => send(&mut send_half, m),
                };
                if r.is_err() || matches!(out, Out::Msg(ServerMsg::Bye { .. })) {
                    break;
                }
            }
        });
    tx
}

// ----- the window's side (`stagetui attach`) ---------------------------------

/// How an attached window ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// The window detached; the app keeps running.
    Detached(String),
    /// The app quit.
    Quit(String),
    /// The connection broke (the app crashed or was killed).
    Lost(String),
    /// This window showed one pane, and the pane went back to the app's window.
    Back(String),
}

impl Ended {
    /// A friendly line to print once the terminal is restored.
    pub fn message(&self) -> String {
        match self {
            Ended::Detached(n) => format!(
                "Detached from \"{n}\" — it keeps running. Come back with: stagetui attach {n}"
            ),
            Ended::Quit(n) => format!("\"{n}\" has quit."),
            Ended::Lost(n) => format!("Lost the connection to \"{n}\" (the app stopped)."),
            Ended::Back(n) => format!("The pane is back in \"{n}\"."),
        }
    }
}

/// Show a running session in this terminal until it detaches or quits.
/// `name:pane` shows just one pane (it pops out of the app's window into this one).
pub fn attach(name: Option<&str>) -> Result<Ended> {
    let (name, pane) = match name.and_then(|n| n.split_once(':')) {
        Some((n, p)) => ((!n.is_empty()).then_some(n), Some(p.to_string())),
        None => (name, None),
    };
    let registry = Registry::open()?;
    let (mut stream, info, token) = registry.connect(name)?;
    let caps = caps::detect(&DetectOptions::default());
    if caps.headless {
        return Err(Error::friendly(
            "attach needs a terminal",
            format!(
                "stagetui is in headless mode: {}",
                caps.headless_reason.as_deref().unwrap_or("no terminal")
            ),
            "run `stagetui attach` in a terminal window",
        ));
    }
    let mut guard = TerminalGuard::enter(Modes::app(&caps))?;
    let (cols, rows) = crossterm::terminal::size().unwrap_or((caps.size.cols, caps.size.rows));
    let hello = ClientMsg::Hello {
        token,
        protocol: PROTOCOL,
        caps: Box::new(caps),
        cols,
        rows,
        pane,
    };
    let welcome = send(&mut stream, &hello).and_then(|()| recv::<ServerMsg>(&mut stream));
    match welcome {
        Ok(ServerMsg::Welcome { .. }) => {}
        Ok(ServerMsg::Error { message }) => {
            let _ = guard.restore();
            return Err(Error::friendly(
                format!("\"{}\" refused this window", info.name),
                message,
                "use the same stagetui version on both sides (pip install -U stagetui)",
            ));
        }
        Ok(ServerMsg::Bye { .. }) | Err(_) => {
            let _ = guard.restore();
            return Ok(Ended::Lost(info.name));
        }
    }
    let conn = Arc::new(stream);
    let running = Arc::new(AtomicBool::new(true));
    let keys = {
        let running = Arc::clone(&running);
        let conn = Arc::clone(&conn);
        std::thread::Builder::new()
            .name("stagetui-attach-input".into())
            .spawn(move || {
                let mut tx = &*conn;
                while running.load(Ordering::SeqCst) {
                    match crossterm::event::poll(Duration::from_millis(50)) {
                        Ok(true) => {
                            if let Ok(ev) = crossterm::event::read()
                                && let Some(input) = input::from_crossterm(ev)
                                && send(&mut tx, &ClientMsg::Input { input }).is_err()
                            {
                                return;
                            }
                        }
                        Ok(false) => {}
                        Err(_) => break,
                    }
                }
                // The keyboard went away (window closed): let the app know.
                if running.load(Ordering::SeqCst) {
                    let _ = send(&mut tx, &ClientMsg::Detach);
                }
            })?
    };
    let mut out = io::stdout();
    let mut rx = &*conn;
    let mut refused = None;
    let ended = loop {
        match read_frame(&mut rx) {
            Ok((KIND_OUTPUT, bytes)) => {
                if out.write_all(&bytes).and_then(|()| out.flush()).is_err() {
                    break Ended::Lost(info.name.clone());
                }
            }
            Ok((KIND_JSON, json)) => match serde_json::from_slice::<ServerMsg>(&json) {
                Ok(ServerMsg::Bye { reason }) if reason == "detached" => {
                    break Ended::Detached(info.name.clone());
                }
                Ok(ServerMsg::Bye { reason }) if reason == "reattached" => {
                    break Ended::Back(info.name.clone());
                }
                Ok(ServerMsg::Bye { .. }) if refused.is_some() => {
                    break Ended::Lost(info.name.clone());
                }
                Ok(ServerMsg::Bye { .. }) => break Ended::Quit(info.name.clone()),
                Ok(ServerMsg::Error { message }) => refused = Some(message),
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break Ended::Lost(info.name.clone()),
        }
    };
    running.store(false, Ordering::SeqCst);
    let _ = keys.join();
    guard.restore()?;
    if let Some(why) = refused {
        return Err(Error::friendly(
            format!("\"{}\" can't show that", info.name),
            why,
            "list the panes with `app.panes()`, or attach without `:pane`",
        ));
    }
    Ok(ended)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn temp_registry(tag: &str) -> (Registry, PathBuf) {
        let dir = std::env::temp_dir().join(format!("stagetui-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        (Registry::at(&dir).unwrap(), dir)
    }

    #[test]
    fn names_tokens_and_frames() {
        assert_eq!(slug("HarnessApp"), "harness-app");
        assert_eq!(slug("Harness App!"), "harness-app");
        assert_eq!(slug("مرحبا"), "app");
        assert_eq!(slug("日本語 Demo"), "demo");
        assert!(valid_name("my-app_2") && !valid_name("a b") && !valid_name(""));
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(same_token(&a, &a.clone()) && !same_token(&a, &b));
        let mut buf = Vec::new();
        let input = Input::Paste("مَرْحَبًا Hello 日本語 नमस्ते".into());
        send(
            &mut buf,
            &ClientMsg::Input {
                input: input.clone(),
            },
        )
        .unwrap();
        write_frame(&mut buf, KIND_OUTPUT, b"\x1b[2J").unwrap();
        let mut r = io::Cursor::new(buf);
        match recv::<ClientMsg>(&mut r).unwrap() {
            ClientMsg::Input { input: got } => assert_eq!(got, input),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            read_frame(&mut r).unwrap(),
            (KIND_OUTPUT, b"\x1b[2J".to_vec())
        );
        assert!(read_frame(&mut io::Cursor::new(vec![0, 0, 0, 0])).is_err());
    }

    /// Answers connections like a running app does, reporting what each wanted.
    struct Server {
        done: Arc<AtomicBool>,
        thread: std::thread::JoinHandle<()>,
        waker: Waker,
        seen: mpsc::Receiver<&'static str>,
    }

    fn serve(r: Arc<Registration>) -> Server {
        let done = Arc::new(AtomicBool::new(false));
        let (tx, seen) = mpsc::channel();
        let waker = r.waker();
        let flag = Arc::clone(&done);
        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                if let Ok(s) = r.accept() {
                    let what = match handshake(s, &r.token, &r.info.name) {
                        Hello::Quit => "quit",
                        Hello::Viewer(_) => "viewer",
                        Hello::Nothing => continue,
                    };
                    let _ = tx.send(what);
                }
            }
        });
        Server {
            done,
            thread,
            waker,
            seen,
        }
    }

    impl Server {
        fn stop(self) {
            self.done.store(true, Ordering::SeqCst);
            self.waker.wake();
            self.thread.join().unwrap();
        }
    }

    #[test]
    fn register_list_connect_stop_and_clean_up() {
        let (reg, dir) = temp_registry("reg");
        let first = Arc::new(reg.register("Test App", "test.py", None).unwrap());
        assert_eq!(first.info.name, "test-app");
        let second = Arc::new(reg.register("Test App", "test.py", None).unwrap());
        assert_eq!(second.info.name, "test-app-2");
        let (one, two) = (serve(Arc::clone(&first)), serve(Arc::clone(&second)));
        let names: Vec<String> = reg.list().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["test-app", "test-app-2"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &str| fs::metadata(dir.join(p)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(""), 0o700, "only the user can open the folder");
            assert_eq!(mode("test-app.token"), 0o600);
            assert_eq!(mode("test-app.sock"), 0o600);
        }
        // The right token may stop the session; a wrong one is refused.
        let info = reg.stop(Some("test-app")).unwrap();
        assert_eq!(info.pid, std::process::id());
        assert_eq!(one.seen.recv_timeout(Duration::from_secs(10)), Ok("quit"));
        let (mut s, _, _) = reg.connect(Some("test-app")).unwrap();
        let wrong = ClientMsg::Quit {
            token: "nope".into(),
        };
        send(&mut s, &wrong).unwrap();
        let reply = recv::<ServerMsg>(&mut s).unwrap();
        assert!(matches!(reply, ServerMsg::Error { .. }), "{reply:?}");
        assert!(one.seen.try_recv().is_err(), "nothing happened");
        // When a session ends its files go away.
        one.stop();
        drop(first);
        let names: Vec<String> = reg.list().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["test-app-2"]);
        let err = reg.connect(Some("nope")).unwrap_err().to_string();
        assert!(err.contains("test-app-2") && err.contains("fix:"), "{err}");
        assert_eq!(reg.find(None).unwrap().name, "test-app-2");
        two.stop();
        drop(second);
        assert!(reg.list().is_empty());
        let err = reg.find(None).unwrap_err().to_string();
        assert!(err.contains("stagetui start"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn tightens_a_folder_others_could_open() {
        use std::os::unix::fs::PermissionsExt;
        let (_, dir) = temp_registry("perm");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).unwrap();
        Registry::at(&dir).unwrap();
        let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "loose permissions are tightened");
        let _ = fs::remove_dir_all(&dir);
    }
}
