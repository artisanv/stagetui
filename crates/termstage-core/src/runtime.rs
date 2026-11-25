//! Run a session in the terminal from Rust (no Python): draw, react to input,
//! quit on q / Esc / Ctrl+C.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::Result;
use crate::app::App;
use crate::caps::Capabilities;
use crate::session::Session;

/// Keys that quit an app with nothing focused.
pub fn is_quit(k: &KeyEvent) -> bool {
    matches!(k.code, KeyCode::Char('q') | KeyCode::Esc)
        || (k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL))
}

/// Run `session` interactively until the user quits.
pub fn run_terminal(session: Session, caps: &Capabilities) -> Result<()> {
    let app = App::new(session);
    app.start(caps.clone())?;
    loop {
        match app.next_event(Duration::from_millis(250)) {
            Some(e) if e.name == "quit" => break,
            Some(_) => {}
            None if !app.is_running() => break,
            None => {}
        }
    }
    app.stop()
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
