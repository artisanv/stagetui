//! Drives a Session on a real terminal: input → state → dirty-driven render.
//! Idle CPU is ~0%: we only draw when something changed.

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::caps::Capabilities;
use crate::session::Session;
use crate::term::{Modes, TerminalGuard};
use crate::view::{TerminalView, View};
use crate::{Error, Result};

/// Keys that quit a Milestone 1 app.
pub fn is_quit(k: &KeyEvent) -> bool {
    matches!(k.code, KeyCode::Char('q') | KeyCode::Esc)
        || (k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL))
}

/// Run `session` interactively until the user quits.
pub fn run_terminal(mut session: Session, caps: &Capabilities) -> Result<()> {
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
    let mut guard = TerminalGuard::enter(Modes::app(caps))?;
    let (cols, rows) = crossterm::terminal::size()?;
    let mut view = TerminalView::new(io::stdout(), caps, (cols, rows));
    session.set_cell_px(view.cell_px());

    let mut dirty = true;
    loop {
        if dirty {
            let (c, r) = view.size();
            let frame = session.frame(c, r);
            view.present(&frame)?;
            dirty = false;
        }
        if !event::poll(Duration::from_millis(500))? {
            continue;
        }
        match event::read()? {
            Event::Key(k) if k.kind != KeyEventKind::Release && is_quit(&k) => break,
            Event::Resize(c, r) => {
                view.resize(c, r);
                dirty = true;
            }
            Event::FocusGained => dirty = true,
            _ => {}
        }
    }
    guard.restore()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_keys() {
        assert!(is_quit(&KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE
        )));
        assert!(is_quit(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(is_quit(&KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_quit(&KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::NONE
        )));
    }

    #[test]
    fn refuses_to_run_headless() {
        let caps = crate::caps::detect(&crate::caps::DetectOptions {
            force_headless: true,
            ..Default::default()
        });
        let s = Session::from_json(r#"{"children":[]}"#).unwrap();
        let err = run_terminal(s, &caps).unwrap_err().to_string();
        assert!(err.contains("headless"), "{err}");
    }
}
