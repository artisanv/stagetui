//! RAII terminal guard: whatever happens (normal exit, error, panic, SIGTERM,
//! SIGHUP), the terminal is put back exactly as we found it.

use std::io::{self, Write};
use std::sync::{Mutex, Once};

use crate::caps::Capabilities;

/// BiDi Support Mode: stop the terminal reordering text (we already did).
pub const BIDI_EXPLICIT: &str = "\x1b[8l";
/// BiDi Support Mode: back to the terminal's default implicit reordering.
pub const BIDI_IMPLICIT: &str = "\x1b[8h";

/// Terminal modes stagetui turns on for an interactive app.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modes {
    pub alt_screen: bool,
    pub hide_cursor: bool,
    pub mouse: bool,
    pub bracketed_paste: bool,
    pub focus_events: bool,
    pub kitty_keyboard: bool,
    /// Switch BiDi-aware terminals (VTE, mlterm) to explicit mode: stagetui
    /// already sends Arabic in visual order, so the terminal must not reorder it again.
    pub bidi_explicit: bool,
}

impl Modes {
    /// Everything a full-screen app wants, gated on what the terminal supports.
    pub fn app(caps: &Capabilities) -> Self {
        Self {
            alt_screen: true,
            hide_cursor: true,
            mouse: true,
            bracketed_paste: true,
            focus_events: true,
            kitty_keyboard: caps.keyboard.kitty_protocol,
            bidi_explicit: caps.bidi.terminal_reorders,
        }
    }

    fn table(&self) -> [(bool, &'static str, &'static str); 7] {
        [
            (self.alt_screen, "\x1b[?1049h", "\x1b[?1049l"),
            (self.hide_cursor, "\x1b[?25l", "\x1b[?25h"),
            // Button events + drag + any-motion (hover), SGR encoding.
            (
                self.mouse,
                "\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h",
                "\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l",
            ),
            (self.bracketed_paste, "\x1b[?2004h", "\x1b[?2004l"),
            (self.focus_events, "\x1b[?1004h", "\x1b[?1004l"),
            // Kitty keyboard: push "disambiguate escape codes" (flag 1); pop on exit.
            (self.kitty_keyboard, "\x1b[>1u", "\x1b[<u"),
            // BDSM: CSI 8 l = explicit (no implicit reordering), CSI 8 h = implicit (default).
            (self.bidi_explicit, BIDI_EXPLICIT, BIDI_IMPLICIT),
        ]
    }

    /// Escape sequence that enables these modes.
    pub fn enter_sequence(&self) -> String {
        self.table()
            .iter()
            .filter(|(on, _, _)| *on)
            .map(|(_, e, _)| *e)
            .collect()
    }

    /// Escape sequence that disables them, in reverse order. Always resets SGR.
    pub fn exit_sequence(&self) -> String {
        let mut s = String::from("\x1b[0m");
        s.extend(
            self.table()
                .iter()
                .rev()
                .filter(|(on, _, _)| *on)
                .map(|(_, _, x)| *x),
        );
        s
    }
}

/// What to undo if the process dies unexpectedly.
struct Restore {
    exit_sequence: String,
    raw: bool,
}

static RESTORE: Mutex<Option<Restore>> = Mutex::new(None);

/// Undo terminal changes. Safe to call many times; only the first call does work.
pub fn emergency_restore() {
    let state = match RESTORE.try_lock() {
        Ok(mut guard) => guard.take(),
        Err(_) => None,
    };
    if let Some(state) = state {
        let mut out = io::stdout();
        let _ = out.write_all(state.exit_sequence.as_bytes());
        let _ = out.flush();
        if state.raw {
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }
}

fn install_hooks() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            emergency_restore();
            previous(info);
        }));
        #[cfg(unix)]
        install_signal_thread();
    });
}

#[cfg(unix)]
fn install_signal_thread() {
    use signal_hook::consts::{SIGHUP, SIGTERM};
    use signal_hook::iterator::Signals;
    if let Ok(mut signals) = Signals::new([SIGTERM, SIGHUP]) {
        let _ = std::thread::Builder::new()
            .name("stagetui-signals".into())
            .spawn(move || {
                if let Some(sig) = signals.forever().next() {
                    emergency_restore();
                    let _ = signal_hook::low_level::emulate_default_handler(sig);
                }
            });
    }
}

/// Holds the terminal in app mode; restores it on drop.
pub struct TerminalGuard {
    modes: Modes,
    raw: bool,
    active: bool,
    _console: crate::os::ConsoleGuard,
}

impl TerminalGuard {
    /// Enter raw mode and enable `modes`.
    pub fn enter(modes: Modes) -> io::Result<Self> {
        let console = crate::os::console_setup();
        install_hooks();
        crossterm::terminal::enable_raw_mode()?;
        *RESTORE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Restore {
            exit_sequence: modes.exit_sequence(),
            raw: true,
        });
        let mut out = io::stdout();
        out.write_all(modes.enter_sequence().as_bytes())?;
        out.flush()?;
        Ok(Self {
            modes,
            raw: true,
            active: true,
            _console: console,
        })
    }

    /// Restore the terminal now (idempotent).
    pub fn restore(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        RESTORE.lock().unwrap_or_else(|e| e.into_inner()).take();
        let mut out = io::stdout();
        out.write_all(self.modes.exit_sequence().as_bytes())?;
        out.flush()?;
        if self.raw {
            crossterm::terminal::disable_raw_mode()?;
        }
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Modes {
        Modes {
            alt_screen: true,
            hide_cursor: true,
            mouse: true,
            bracketed_paste: true,
            focus_events: true,
            kitty_keyboard: true,
            bidi_explicit: true,
        }
    }

    #[test]
    fn exit_mirrors_enter() {
        let m = all();
        let enter = m.enter_sequence();
        let exit = m.exit_sequence();
        // Every private mode set with 'h' is reset with 'l'.
        for set in enter.split("\x1b[?").filter_map(|s| s.strip_suffix('h')) {
            assert!(
                exit.contains(&format!("\x1b[?{set}l")),
                "missing reset for {set}"
            );
        }
        assert!(enter.contains("\x1b[>1u") && exit.contains("\x1b[<u"));
        // Alternate screen is left last.
        assert!(exit.ends_with("\x1b[?1049l"));
        assert!(exit.starts_with("\x1b[0m"));
    }

    #[test]
    fn kitty_keyboard_only_when_supported() {
        let m = Modes {
            kitty_keyboard: false,
            ..all()
        };
        assert!(!m.enter_sequence().contains("\x1b[>1u"));
        assert!(!m.exit_sequence().contains("\x1b[<u"));
    }

    #[test]
    fn emergency_restore_is_idempotent_without_state() {
        emergency_restore();
        emergency_restore();
    }
}
