//! Keyboard and mouse input in one terminal-independent form.
//!
//! Real terminals (via `crossterm`) and scripted headless runs (`press("ctrl+s")`)
//! both produce [`Input`] values, so an app behaves the same in both.

use crossterm::event::{
    Event as CtEvent, KeyCode as CtKey, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEventKind,
};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// A key without modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Code {
    Char(char),
    Enter,
    Esc,
    Tab,
    Backspace,
    Delete,
    Insert,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

/// Modifier keys held with a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub sup: bool,
}

/// One key press, normalized so that bindings compare reliably:
/// letters are lowercase (with `shift` set for capitals), and symbols such as
/// `?` never carry `shift` (the symbol already says it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Key {
    pub code: Code,
    pub mods: Mods,
}

const NAMES: [(&str, Code); 30] = [
    ("enter", Code::Enter),
    ("return", Code::Enter),
    ("escape", Code::Esc),
    ("esc", Code::Esc),
    ("tab", Code::Tab),
    ("backspace", Code::Backspace),
    ("delete", Code::Delete),
    ("del", Code::Delete),
    ("insert", Code::Insert),
    ("up", Code::Up),
    ("down", Code::Down),
    ("left", Code::Left),
    ("right", Code::Right),
    ("home", Code::Home),
    ("end", Code::End),
    ("pageup", Code::PageUp),
    ("page_up", Code::PageUp),
    ("pagedown", Code::PageDown),
    ("page_down", Code::PageDown),
    ("space", Code::Char(' ')),
    ("backslash", Code::Char('\\')),
    ("slash", Code::Char('/')),
    ("comma", Code::Char(',')),
    ("period", Code::Char('.')),
    ("minus", Code::Char('-')),
    ("plus", Code::Char('+')),
    ("equals", Code::Char('=')),
    ("semicolon", Code::Char(';')),
    ("left_bracket", Code::Char('[')),
    ("right_bracket", Code::Char(']')),
];

impl Key {
    pub fn new(code: Code, mods: Mods) -> Self {
        Self { code, mods }.normalized()
    }

    /// A plain key with no modifiers.
    pub fn plain(code: Code) -> Self {
        Self::new(code, Mods::default())
    }

    fn normalized(mut self) -> Self {
        if let Code::Char(c) = self.code {
            if c.is_uppercase() {
                self.code = Code::Char(c.to_lowercase().next().unwrap_or(c));
                self.mods.shift = true;
            } else if !c.is_alphabetic() && c != ' ' {
                self.mods.shift = false;
            }
        }
        self
    }

    /// Parse a key description such as `"ctrl+s"`, `"ctrl+shift+d"`, `"f5"`,
    /// `"ctrl+backslash"`, `"?"` or `"enter"`.
    pub fn parse(text: &str) -> Result<Self> {
        let bad = || {
            Error::friendly(
                format!("Unknown key \"{text}\""),
                "keys are written like \"ctrl+s\", \"ctrl+shift+d\", \"f5\", \"enter\" or \"?\"",
                "use modifiers ctrl, alt, shift, super joined with + and a key name or single character",
            )
        };
        let lower = text.trim().to_lowercase();
        if lower.is_empty() {
            return Err(bad());
        }
        // A lone "+" is the plus key; otherwise "+" separates parts.
        let parts: Vec<&str> = if lower == "+" {
            vec!["+"]
        } else if let Some(head) = lower.strip_suffix("++") {
            head.split('+').chain(std::iter::once("+")).collect()
        } else {
            lower.split('+').collect()
        };
        let (last, mods_part) = parts.split_last().ok_or_else(bad)?;
        let mut mods = Mods::default();
        for m in mods_part {
            match *m {
                "ctrl" | "control" => mods.ctrl = true,
                "alt" | "option" | "meta" => mods.alt = true,
                "shift" => mods.shift = true,
                "super" | "cmd" | "win" => mods.sup = true,
                _ => return Err(bad()),
            }
        }
        let code = if let Some((_, c)) = NAMES.iter().find(|(n, _)| n == last) {
            *c
        } else if let Some(n) = last.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
            if (1..=24).contains(&n) {
                Code::F(n)
            } else {
                return Err(bad());
            }
        } else {
            // Single character: use the original case ("A" means shift+a).
            let original = text.trim();
            let ch = original
                .rsplit_once('+')
                .map_or(original, |(_, k)| k)
                .chars()
                .collect::<Vec<_>>();
            match ch.as_slice() {
                [c] => Code::Char(*c),
                _ if *last == "+" => Code::Char('+'),
                _ => return Err(bad()),
            }
        };
        Ok(Self::new(code, mods))
    }

    /// The name `Key::parse` understands, e.g. `ctrl+s`, `space`, `up`, `f5`.
    pub fn name(&self) -> String {
        let mut s = String::new();
        for (on, name) in [
            (self.mods.ctrl, "ctrl+"),
            (self.mods.alt, "alt+"),
            (self.mods.sup, "super+"),
            (self.mods.shift, "shift+"),
        ] {
            if on {
                s.push_str(name);
            }
        }
        let key = match self.code {
            Code::Char(' ') => "space".to_string(),
            Code::Char(c) => c.to_lowercase().to_string(),
            Code::Enter => "enter".into(),
            Code::Esc => "esc".into(),
            Code::Tab => "tab".into(),
            Code::Backspace => "backspace".into(),
            Code::Delete => "delete".into(),
            Code::Insert => "insert".into(),
            Code::Up => "up".into(),
            Code::Down => "down".into(),
            Code::Left => "left".into(),
            Code::Right => "right".into(),
            Code::Home => "home".into(),
            Code::End => "end".into(),
            Code::PageUp => "pageup".into(),
            Code::PageDown => "pagedown".into(),
            Code::F(n) => format!("f{n}"),
        };
        s.push_str(&key);
        s
    }

    /// A short human label, e.g. `Ctrl+S`, `Tab`, `?`.
    pub fn label(&self) -> String {
        let mut s = String::new();
        for (on, name) in [
            (self.mods.ctrl, "Ctrl+"),
            (self.mods.alt, "Alt+"),
            (self.mods.sup, "Super+"),
            (self.mods.shift, "Shift+"),
        ] {
            if on {
                s.push_str(name);
            }
        }
        let key = match self.code {
            Code::Char(' ') => "Space".to_string(),
            Code::Char(c) if c.is_alphabetic() && (self.mods.ctrl || self.mods.alt) => {
                c.to_uppercase().to_string()
            }
            Code::Char(c) => c.to_string(),
            Code::Enter => "Enter".into(),
            Code::Esc => "Esc".into(),
            Code::Tab => "Tab".into(),
            Code::Backspace => "Backspace".into(),
            Code::Delete => "Del".into(),
            Code::Insert => "Ins".into(),
            Code::Up => "↑".into(),
            Code::Down => "↓".into(),
            Code::Left => "←".into(),
            Code::Right => "→".into(),
            Code::Home => "Home".into(),
            Code::End => "End".into(),
            Code::PageUp => "PgUp".into(),
            Code::PageDown => "PgDn".into(),
            Code::F(n) => format!("F{n}"),
        };
        s.push_str(&key);
        s
    }

    /// Whether this is a printable character without ctrl/alt/super.
    pub fn text(&self) -> Option<char> {
        match self.code {
            Code::Char(c) if !self.mods.ctrl && !self.mods.alt && !self.mods.sup => {
                Some(if self.mods.shift {
                    c.to_uppercase().next().unwrap_or(c)
                } else {
                    c
                })
            }
            _ => None,
        }
    }

    pub fn is(&self, code: Code) -> bool {
        self.code == code && self.mods == Mods::default()
    }
}

/// Mouse buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// What the mouse did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseKind {
    Down(Button),
    Up(Button),
    Drag(Button),
    Move,
    ScrollUp,
    ScrollDown,
}

/// A mouse event at a cell position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mouse {
    pub kind: MouseKind,
    pub x: u16,
    pub y: u16,
}

/// Everything an app can receive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Input {
    Key(Key),
    Mouse(Mouse),
    Paste(String),
    Resize(u16, u16),
    FocusGained,
    FocusLost,
}

fn mods(m: KeyModifiers) -> Mods {
    Mods {
        ctrl: m.contains(KeyModifiers::CONTROL),
        alt: m.contains(KeyModifiers::ALT),
        shift: m.contains(KeyModifiers::SHIFT),
        sup: m.contains(KeyModifiers::SUPER),
    }
}

/// Convert a crossterm key event (press or repeat; releases are ignored).
pub fn key_from_crossterm(k: &KeyEvent) -> Option<Key> {
    if k.kind == KeyEventKind::Release {
        return None;
    }
    let mut m = mods(k.modifiers);
    let code = match k.code {
        CtKey::Char(c) if m.ctrl => match c {
            // Legacy terminals send Ctrl+\ ] ^ _ as Ctrl+4 5 6 7.
            '4' => Code::Char('\\'),
            '5' => Code::Char(']'),
            '6' => Code::Char('^'),
            '7' => Code::Char('_'),
            c => Code::Char(c),
        },
        CtKey::Char(c) => Code::Char(c),
        CtKey::Enter => Code::Enter,
        CtKey::Esc => Code::Esc,
        CtKey::Tab => Code::Tab,
        CtKey::BackTab => {
            m.shift = true;
            Code::Tab
        }
        CtKey::Backspace => Code::Backspace,
        CtKey::Delete => Code::Delete,
        CtKey::Insert => Code::Insert,
        CtKey::Up => Code::Up,
        CtKey::Down => Code::Down,
        CtKey::Left => Code::Left,
        CtKey::Right => Code::Right,
        CtKey::Home => Code::Home,
        CtKey::End => Code::End,
        CtKey::PageUp => Code::PageUp,
        CtKey::PageDown => Code::PageDown,
        CtKey::F(n) => Code::F(n),
        _ => return None,
    };
    Some(Key::new(code, m))
}

/// Convert any crossterm event.
pub fn from_crossterm(ev: CtEvent) -> Option<Input> {
    let button = |b: MouseButton| match b {
        MouseButton::Left => Button::Left,
        MouseButton::Right => Button::Right,
        MouseButton::Middle => Button::Middle,
    };
    Some(match ev {
        CtEvent::Key(k) => Input::Key(key_from_crossterm(&k)?),
        CtEvent::Mouse(m) => Input::Mouse(Mouse {
            kind: match m.kind {
                MouseEventKind::Down(b) => MouseKind::Down(button(b)),
                MouseEventKind::Up(b) => MouseKind::Up(button(b)),
                MouseEventKind::Drag(b) => MouseKind::Drag(button(b)),
                MouseEventKind::Moved => MouseKind::Move,
                MouseEventKind::ScrollUp => MouseKind::ScrollUp,
                MouseEventKind::ScrollDown => MouseKind::ScrollDown,
                _ => return None,
            },
            x: m.column,
            y: m.row,
        }),
        CtEvent::Paste(s) => Input::Paste(s),
        CtEvent::Resize(c, r) => Input::Resize(c, r),
        CtEvent::FocusGained => Input::FocusGained,
        CtEvent::FocusLost => Input::FocusLost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl(c: char) -> Key {
        Key::new(
            Code::Char(c),
            Mods {
                ctrl: true,
                ..Mods::default()
            },
        )
    }

    #[test]
    fn parsing() {
        assert_eq!(Key::parse("ctrl+s").unwrap(), ctrl('s'));
        assert_eq!(Key::parse("Ctrl+Backslash").unwrap(), ctrl('\\'));
        let k = Key::parse("ctrl+shift+d").unwrap();
        assert!(k.mods.ctrl && k.mods.shift && k.code == Code::Char('d'));
        assert_eq!(Key::parse("A").unwrap(), Key::parse("shift+a").unwrap());
        assert_eq!(Key::parse("?").unwrap(), Key::plain(Code::Char('?')));
        assert_eq!(Key::parse("shift+?").unwrap(), Key::plain(Code::Char('?')));
        assert_eq!(Key::parse("f5").unwrap(), Key::plain(Code::F(5)));
        assert_eq!(Key::parse("enter").unwrap(), Key::plain(Code::Enter));
        assert_eq!(Key::parse("+").unwrap(), Key::plain(Code::Char('+')));
        assert_eq!(Key::parse("ctrl++").unwrap(), ctrl('+'));
        for bad in ["", "hyper+x", "f99", "ctrl+abc"] {
            let e = Key::parse(bad).unwrap_err().to_string();
            assert!(e.contains("fix:"), "{bad}: {e}");
        }
    }

    #[test]
    fn crossterm_events_match_bindings() {
        let legacy = KeyEvent::new(CtKey::Char('4'), KeyModifiers::CONTROL);
        assert_eq!(
            key_from_crossterm(&legacy),
            Some(Key::parse("ctrl+backslash").unwrap())
        );
        let upper = KeyEvent::new(
            CtKey::Char('D'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(
            key_from_crossterm(&upper),
            Some(Key::parse("ctrl+shift+d").unwrap())
        );
        let back = KeyEvent::new(CtKey::BackTab, KeyModifiers::SHIFT);
        assert_eq!(
            key_from_crossterm(&back),
            Some(Key::parse("shift+tab").unwrap())
        );
    }

    #[test]
    fn labels_and_text() {
        assert_eq!(Key::parse("ctrl+backslash").unwrap().label(), "Ctrl+\\");
        assert_eq!(Key::parse("ctrl+t").unwrap().label(), "Ctrl+T");
        assert_eq!(Key::parse("shift+tab").unwrap().label(), "Shift+Tab");
        assert_eq!(Key::parse("A").unwrap().text(), Some('A'));
        assert_eq!(Key::parse("ctrl+a").unwrap().text(), None);
        assert_eq!(Key::plain(Code::Char('م')).text(), Some('م'));
    }
}
