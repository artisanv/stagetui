//! Terminal capability detection.
//!
//! Capabilities come from **asking the terminal** (escape-sequence queries),
//! with environment variables used only as hints and fallbacks.
//! [`resolve`] is pure so every terminal can be unit-tested from recorded replies.

pub mod parse;
pub mod probe;

use std::collections::HashMap;
use std::io::IsTerminal;
use std::time::Duration;

use serde::Serialize;

pub use parse::Replies;

/// Image protocol used to draw pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GraphicsProtocol {
    Kitty,
    Sixel,
    Iterm2,
    Halfblocks,
}

/// Terminal families we recognise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Family {
    Kitty,
    Ghostty,
    Wezterm,
    Foot,
    Xterm,
    Iterm2,
    WindowsTerminal,
    Vte,
    Konsole,
    Alacritty,
    AppleTerminal,
    Mintty,
    Vscode,
    Tmux,
    Screen,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct Platform {
    pub os: &'static str,
    pub arch: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct TerminalId {
    pub family: Family,
    pub name: Option<String>,
    pub version: Option<String>,
    /// `$TERM`
    pub term: Option<String>,
    /// `$TERM_PROGRAM`
    pub program: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct TtyInfo {
    pub stdin: bool,
    pub stdout: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CellSizeSource {
    Ioctl,
    Csi16t,
    Csi14t,
    Default,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct SizeInfo {
    pub cols: u16,
    pub rows: u16,
    pub cell_width_px: u32,
    pub cell_height_px: u32,
    pub cell_size_source: CellSizeSource,
}

#[derive(Debug, Clone, Serialize)]
pub struct Graphics {
    pub best: GraphicsProtocol,
    pub kitty: bool,
    /// Kitty Unicode placeholders (`U=1`): images live in cells (Kitty ≥ 0.28, Ghostty).
    pub kitty_placeholders: bool,
    pub sixel: bool,
    pub iterm2: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Keyboard {
    pub kitty_protocol: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Bidi {
    /// The terminal itself reorders RTL text (VTE, mlterm, mintty). stagetui switches it
    /// to explicit mode (BDSM, `CSI 8 l`) before emitting pre-shaped visual-order text.
    pub terminal_reorders: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Remote {
    pub ssh: bool,
    pub tmux: bool,
    pub screen: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Link {
    pub slow: bool,
    pub max_fps: u32,
    pub compress: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ProbeInfo {
    pub performed: bool,
    pub rtt_ms: Option<f64>,
    pub timed_out: bool,
    pub error: Option<String>,
}

/// Everything stagetui knows about where it is running.
#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    pub version: String,
    pub platform: Platform,
    pub terminal: TerminalId,
    pub tty: TtyInfo,
    pub headless: bool,
    pub headless_reason: Option<String>,
    pub size: SizeInfo,
    pub graphics: Graphics,
    pub keyboard: Keyboard,
    pub sync_output: bool,
    /// The terminal's default background color (`#rrggbb`), if it told us.
    pub background: Option<String>,
    pub truecolor: bool,
    pub bidi: Bidi,
    pub remote: Remote,
    pub link: Link,
    pub probe: ProbeInfo,
}

/// A snapshot of environment variables (injectable for tests).
#[derive(Debug, Clone, Default)]
pub struct Env(HashMap<String, String>);

impl Env {
    /// The current process environment.
    pub fn current() -> Self {
        Self(
            std::env::vars_os()
                .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
                .collect(),
        )
    }

    /// Build from pairs (tests).
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        Self(
            pairs
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
}

/// Grid size as reported by the OS.
#[derive(Debug, Clone, Copy, Default)]
pub struct Grid {
    pub cols: u16,
    pub rows: u16,
    pub width_px: u32,
    pub height_px: u32,
}

/// Options for [`detect`].
#[derive(Debug, Clone)]
pub struct DetectOptions {
    pub force_headless: bool,
    pub probe: bool,
    pub timeout: Duration,
}

impl Default for DetectOptions {
    fn default() -> Self {
        let ms = std::env::var("STAGETUI_PROBE_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(250);
        Self {
            force_headless: false,
            probe: true,
            timeout: Duration::from_millis(ms),
        }
    }
}

impl Capabilities {
    /// The terminal background as RGB, if known.
    pub fn background_rgb(&self) -> Option<crate::style::Rgb> {
        self.background
            .as_deref()
            .and_then(crate::style::Rgb::parse_hex)
    }
}

/// Default headless grid when nothing else is known.
pub const DEFAULT_HEADLESS_GRID: (u16, u16) = (80, 24);
/// Default cell size in pixels when the terminal will not tell us.
pub const DEFAULT_CELL_PX: (u32, u32) = (10, 20);

/// Detect capabilities of the current terminal (or headless environment).
pub fn detect(opts: &DetectOptions) -> Capabilities {
    let env = Env::current();
    let tty = TtyInfo {
        stdin: std::io::stdin().is_terminal(),
        stdout: std::io::stdout().is_terminal(),
    };
    let headless_reason = headless_reason(&env, tty, opts.force_headless);

    let mut probe_info = ProbeInfo::default();
    let mut replies = Replies::default();
    if headless_reason.is_none() && opts.probe && env.get("STAGETUI_NO_PROBE") != Some("1") {
        match probe::run(&queries(&env), opts.timeout) {
            Ok(out) => {
                replies = parse::parse(&out.bytes);
                probe_info = ProbeInfo {
                    performed: true,
                    rtt_ms: out
                        .rtt
                        .map(|d| (d.as_secs_f64() * 1000.0 * 100.0).round() / 100.0),
                    timed_out: out.timed_out,
                    error: None,
                };
            }
            Err(e) => probe_info.error = Some(e.to_string()),
        }
    }

    let grid = if headless_reason.is_some() {
        headless_grid(&env)
    } else {
        os_grid()
    };
    resolve(&env, tty, headless_reason, &replies, grid, probe_info)
}

/// The batch of queries sent to the terminal. DA1 goes last as the end marker.
pub fn queries(env: &Env) -> Vec<u8> {
    let kitty_query = format!(
        "\x1b_Gi={},s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\",
        parse::KITTY_PROBE_ID
    );
    let mut q = String::new();
    q.push_str("\x1b[>0q"); // XTVERSION
    if env.has("TMUX") {
        // tmux answers most queries itself; graphics must pass through to the outer terminal.
        q.push_str(&tmux_passthrough(&kitty_query));
    } else {
        q.push_str(&kitty_query);
    }
    q.push_str("\x1b[?u"); // Kitty keyboard flags
    q.push_str("\x1b[?2026$p"); // DECRQM synchronized output
    q.push_str("\x1b[16t"); // cell size in px
    q.push_str("\x1b[14t"); // text area size in px
    q.push_str("\x1b]11;?\x1b\\"); // default background color (OSC 11)
    q.push_str("\x1b[c"); // DA1 (end marker)
    q.into_bytes()
}

/// Wrap a sequence for tmux passthrough (`allow-passthrough on`).
pub fn tmux_passthrough(seq: &str) -> String {
    format!("\x1bPtmux;{}\x1b\\", seq.replace('\x1b', "\x1b\x1b"))
}

fn headless_reason(env: &Env, tty: TtyInfo, force: bool) -> Option<String> {
    if force {
        Some("requested (--headless)".into())
    } else if env.get("STAGETUI_HEADLESS") == Some("1") {
        Some("STAGETUI_HEADLESS=1".into())
    } else if !tty.stdin && !tty.stdout {
        Some("no TTY (neither stdin nor stdout is a terminal)".into())
    } else {
        None
    }
}

fn headless_grid(env: &Env) -> Grid {
    let (cols, rows) = env
        .get("STAGETUI_SIZE")
        .and_then(parse_size)
        .unwrap_or(DEFAULT_HEADLESS_GRID);
    Grid {
        cols,
        rows,
        width_px: 0,
        height_px: 0,
    }
}

fn os_grid() -> Grid {
    if let Ok(ws) = crossterm::terminal::window_size()
        && ws.columns > 0
        && ws.rows > 0
    {
        return Grid {
            cols: ws.columns,
            rows: ws.rows,
            width_px: ws.width.into(),
            height_px: ws.height.into(),
        };
    }
    let (cols, rows) = crossterm::terminal::size().unwrap_or(DEFAULT_HEADLESS_GRID);
    Grid {
        cols,
        rows,
        width_px: 0,
        height_px: 0,
    }
}

/// Parse `"120x40"` into `(cols, rows)`.
pub fn parse_size(s: &str) -> Option<(u16, u16)> {
    let (c, r) = s.trim().split_once(['x', 'X', '×'])?;
    let (c, r) = (c.trim().parse().ok()?, r.trim().parse().ok()?);
    (c > 0 && r > 0).then_some((c, r))
}

/// Combine environment, probe replies and OS info into [`Capabilities`]. Pure.
pub fn resolve(
    env: &Env,
    tty: TtyInfo,
    headless_reason: Option<String>,
    replies: &Replies,
    grid: Grid,
    probe: ProbeInfo,
) -> Capabilities {
    let (name, version) = match replies.xtversion.as_deref() {
        Some(x) => {
            let (n, v) = parse::split_xtversion(x);
            (Some(n), v)
        }
        None => (
            env.get("TERM_PROGRAM").map(str::to_string),
            env.get("TERM_PROGRAM_VERSION").map(str::to_string),
        ),
    };
    let family = family(env, name.as_deref());
    let version_tuple = version.as_deref().map(version_tuple);

    let kitty = replies.kitty_graphics == Some(true);
    let sixel = replies.da1.as_ref().is_some_and(|a| a.contains(&4));
    let iterm2 = matches!(family, Family::Iterm2 | Family::Wezterm)
        || env.get("TERM_PROGRAM") == Some("iTerm.app")
        || env.get("LC_TERMINAL") == Some("iTerm2");
    let kitty_placeholders = kitty
        && match family {
            Family::Kitty => version_tuple.is_some_and(|v| v >= (0, 28, 0)),
            Family::Ghostty => true,
            _ => false,
        };
    // WezTerm's Kitty implementation is partial; its iTerm2 protocol is solid.
    let best = if kitty && family != Family::Wezterm {
        GraphicsProtocol::Kitty
    } else if iterm2 {
        GraphicsProtocol::Iterm2
    } else if sixel {
        GraphicsProtocol::Sixel
    } else if kitty {
        GraphicsProtocol::Kitty
    } else {
        GraphicsProtocol::Halfblocks
    };

    let ssh = env.has("SSH_TTY") || env.has("SSH_CONNECTION") || env.has("SSH_CLIENT");
    let slow = ssh || probe.rtt_ms.is_some_and(|ms| ms > 40.0);

    let truecolor = matches!(env.get("COLORTERM"), Some("truecolor" | "24bit"))
        || env
            .get("TERM")
            .is_some_and(|t| t.contains("direct") || t.contains("truecolor"))
        || cfg!(windows)
        || matches!(
            family,
            Family::Kitty
                | Family::Ghostty
                | Family::Wezterm
                | Family::Foot
                | Family::Iterm2
                | Family::WindowsTerminal
                | Family::Vte
                | Family::Konsole
                | Family::Alacritty
                | Family::Vscode
        );

    let terminal_reorders = env
        .get("VTE_VERSION")
        .and_then(|v| v.parse::<u32>().ok())
        .is_some_and(|v| v >= 5800)
        || env.has("MLTERM")
        || family == Family::Mintty;

    let (cell_width_px, cell_height_px, cell_size_source) =
        if grid.width_px > 0 && grid.cols > 0 && grid.rows > 0 {
            (
                grid.width_px / u32::from(grid.cols),
                grid.height_px / u32::from(grid.rows),
                CellSizeSource::Ioctl,
            )
        } else if let Some((w, h)) = replies.cell_px {
            (w, h, CellSizeSource::Csi16t)
        } else if let (Some((w, h)), true) = (replies.window_px, grid.cols > 0 && grid.rows > 0) {
            (
                w / u32::from(grid.cols),
                h / u32::from(grid.rows),
                CellSizeSource::Csi14t,
            )
        } else {
            (
                DEFAULT_CELL_PX.0,
                DEFAULT_CELL_PX.1,
                CellSizeSource::Default,
            )
        };

    Capabilities {
        version: crate::version(),
        platform: Platform {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        },
        terminal: TerminalId {
            family,
            name,
            version,
            term: env.get("TERM").map(str::to_string),
            program: env.get("TERM_PROGRAM").map(str::to_string),
        },
        tty,
        headless: headless_reason.is_some(),
        headless_reason,
        size: SizeInfo {
            cols: grid.cols,
            rows: grid.rows,
            cell_width_px,
            cell_height_px,
            cell_size_source,
        },
        graphics: Graphics {
            best,
            kitty,
            kitty_placeholders,
            sixel,
            iterm2,
        },
        keyboard: Keyboard {
            kitty_protocol: replies.kitty_keyboard.is_some(),
        },
        sync_output: replies.mode_supported(2026),
        background: replies
            .background
            .map(|(r, g, b)| format!("#{r:02x}{g:02x}{b:02x}")),
        truecolor,
        bidi: Bidi { terminal_reorders },
        remote: Remote {
            ssh,
            tmux: env.has("TMUX"),
            screen: env.has("STY"),
        },
        link: Link {
            slow,
            max_fps: if slow { 24 } else { 60 },
            compress: slow,
        },
        probe,
    }
}

fn family(env: &Env, name: Option<&str>) -> Family {
    if let Some(name) = name {
        let n = name.to_ascii_lowercase();
        let by_name = match n.as_str() {
            "kitty" | "xterm-kitty" => Some(Family::Kitty),
            "ghostty" => Some(Family::Ghostty),
            "wezterm" => Some(Family::Wezterm),
            "foot" => Some(Family::Foot),
            "xterm" => Some(Family::Xterm),
            "iterm2" | "iterm.app" => Some(Family::Iterm2),
            "vte" => Some(Family::Vte),
            "konsole" => Some(Family::Konsole),
            "alacritty" => Some(Family::Alacritty),
            "apple_terminal" => Some(Family::AppleTerminal),
            "mintty" => Some(Family::Mintty),
            "vscode" => Some(Family::Vscode),
            "tmux" => Some(Family::Tmux),
            "screen" => Some(Family::Screen),
            _ => None,
        };
        if let Some(f) = by_name {
            return f;
        }
    }
    if env.has("KITTY_WINDOW_ID") {
        Family::Kitty
    } else if env.has("GHOSTTY_RESOURCES_DIR") {
        Family::Ghostty
    } else if env.has("WEZTERM_EXECUTABLE") || env.has("WEZTERM_PANE") {
        Family::Wezterm
    } else if env.has("WT_SESSION") {
        Family::WindowsTerminal
    } else if env.has("VTE_VERSION") {
        Family::Vte
    } else if env.has("KONSOLE_VERSION") {
        Family::Konsole
    } else if env.has("ALACRITTY_WINDOW_ID") || env.has("ALACRITTY_LOG") {
        Family::Alacritty
    } else {
        Family::Unknown
    }
}

fn version_tuple(v: &str) -> (u32, u32, u32) {
    let mut it = v
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap_or(0));
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTY: TtyInfo = TtyInfo {
        stdin: true,
        stdout: true,
    };
    const GRID: Grid = Grid {
        cols: 120,
        rows: 40,
        width_px: 0,
        height_px: 0,
    };

    fn caps(env: &[(&str, &str)], replies: &[u8]) -> Capabilities {
        resolve(
            &Env::from_pairs(env.iter().copied()),
            TTY,
            None,
            &parse::parse(replies),
            GRID,
            ProbeInfo::default(),
        )
    }

    #[test]
    fn kitty_prefers_placeholders() {
        let c = caps(&[("TERM", "xterm-kitty")], b"\x1bP>|kitty(0.39.1)\x1b\\\x1b_Gi=31;OK\x1b\\\x1b[?0u\x1b[?2026;2$y\x1b[6;21;10t\x1b[?62c");
        assert_eq!(c.terminal.family, Family::Kitty);
        assert_eq!(c.graphics.best, GraphicsProtocol::Kitty);
        assert!(c.graphics.kitty_placeholders);
        assert!(c.keyboard.kitty_protocol && c.sync_output && c.truecolor);
        assert_eq!((c.size.cell_width_px, c.size.cell_height_px), (10, 21));
        assert_eq!(c.size.cell_size_source, CellSizeSource::Csi16t);
    }

    #[test]
    fn old_kitty_has_no_placeholders() {
        let c = caps(
            &[],
            b"\x1bP>|kitty(0.26.5)\x1b\\\x1b_Gi=31;OK\x1b\\\x1b[?62c",
        );
        assert!(c.graphics.kitty && !c.graphics.kitty_placeholders);
    }

    #[test]
    fn wezterm_uses_iterm2_protocol() {
        let c = caps(
            &[],
            b"\x1bP>|WezTerm 20240203\x1b\\\x1b_Gi=31;OK\x1b\\\x1b[?65;4c",
        );
        assert_eq!(c.graphics.best, GraphicsProtocol::Iterm2);
        assert!(c.graphics.sixel && c.graphics.kitty);
    }

    #[test]
    fn windows_terminal_uses_sixel() {
        let c = caps(
            &[("WT_SESSION", "abc")],
            b"\x1b[?2026;2$y\x1b[?61;4;6;7;22c",
        );
        assert_eq!(c.terminal.family, Family::WindowsTerminal);
        assert_eq!(c.graphics.best, GraphicsProtocol::Sixel);
    }

    #[test]
    fn vte_reorders_bidi_and_falls_back_to_blocks() {
        let c = caps(&[("VTE_VERSION", "8002")], b"\x1b[?65;1;9c");
        assert_eq!(c.terminal.family, Family::Vte);
        assert!(c.bidi.terminal_reorders);
        assert_eq!(c.graphics.best, GraphicsProtocol::Halfblocks);
    }

    #[test]
    fn apple_terminal_from_env() {
        let c = caps(&[("TERM_PROGRAM", "Apple_Terminal")], b"\x1b[?1;2c");
        assert_eq!(c.terminal.family, Family::AppleTerminal);
        assert_eq!(c.graphics.best, GraphicsProtocol::Halfblocks);
    }

    #[test]
    fn ssh_throttles() {
        let c = caps(&[("SSH_TTY", "/dev/pts/3")], b"\x1b[?62c");
        assert!(c.remote.ssh && c.link.slow && c.link.compress);
        assert_eq!(c.link.max_fps, 24);
    }

    #[test]
    fn ioctl_cell_size_wins() {
        let grid = Grid {
            cols: 100,
            rows: 50,
            width_px: 1000,
            height_px: 1050,
        };
        let c = resolve(
            &Env::default(),
            TTY,
            None,
            &Replies::default(),
            grid,
            ProbeInfo::default(),
        );
        assert_eq!((c.size.cell_width_px, c.size.cell_height_px), (10, 21));
        assert_eq!(c.size.cell_size_source, CellSizeSource::Ioctl);
    }

    #[test]
    fn headless_rules() {
        let env = Env::from_pairs([("STAGETUI_HEADLESS", "1")]);
        assert!(headless_reason(&env, TTY, false).is_some());
        assert!(
            headless_reason(
                &Env::default(),
                TtyInfo {
                    stdin: false,
                    stdout: false
                },
                false
            )
            .is_some()
        );
        assert!(
            headless_reason(
                &Env::default(),
                TtyInfo {
                    stdin: true,
                    stdout: false
                },
                false
            )
            .is_none()
        );
        assert!(headless_reason(&Env::default(), TTY, true).is_some());
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("120x40"), Some((120, 40)));
        assert_eq!(parse_size(" 80 X 24 "), Some((80, 24)));
        assert_eq!(parse_size("0x10"), None);
        assert_eq!(parse_size("wide"), None);
    }

    #[test]
    fn tmux_wraps_graphics_query() {
        let q = String::from_utf8(queries(&Env::from_pairs([(
            "TMUX",
            "/tmp/tmux-1000/default,1,0",
        )])))
        .unwrap();
        assert!(q.contains("\x1bPtmux;\x1b\x1b_G"));
        assert!(q.ends_with("\x1b[c"));
    }

    #[test]
    fn serializes_to_json() {
        let c = caps(&[], b"\x1b[?62c");
        let v = serde_json::to_value(&c).unwrap();
        for key in [
            "platform", "terminal", "graphics", "keyboard", "size", "bidi", "remote", "link",
            "probe",
        ] {
            assert!(v.get(key).is_some(), "missing {key}");
        }
        assert_eq!(v["graphics"]["best"], "halfblocks");
    }
}
