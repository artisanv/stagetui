//! Opening a new terminal window — for a pane popped out of a running app.
//!
//! The window is opened with the same kind of terminal the person is using
//! (Kitty opens Kitty, foot opens foot…). `STAGETUI_LAUNCHER` (or an app's
//! launcher setting) overrides it, e.g. `"foot {cmd}"` or `"alacritty -e {cmd}"`.

use std::process::{Command, Stdio};

use crate::caps::{Capabilities, Family};
use crate::{Error, Result};

/// The program and arguments that run `cmd` in a new window of the terminal
/// described by `caps`. `None` when there's no way to open one from here:
/// over SSH (it would open on the other computer) or in an unknown terminal.
pub fn window_command(
    caps: &Capabilities,
    cmd: &[String],
    template: Option<&str>,
) -> Option<Vec<String>> {
    if let Some(t) = template.filter(|t| !t.trim().is_empty()) {
        return Some(fill(t, cmd));
    }
    if caps.remote.ssh {
        return None;
    }
    let with = |prefix: &[&str]| -> Vec<String> {
        prefix
            .iter()
            .map(|s| (*s).to_string())
            .chain(cmd.iter().cloned())
            .collect()
    };
    if caps.remote.tmux || caps.terminal.family == Family::Tmux {
        return Some(with(&["tmux", "new-window"]));
    }
    let mac = caps.platform.os == "macos";
    Some(match caps.terminal.family {
        Family::Kitty => with(&["kitty"]),
        Family::Wezterm => with(&["wezterm", "start", "--"]),
        Family::Ghostty if mac => with(&["open", "-na", "Ghostty", "--args", "-e"]),
        Family::Ghostty => with(&["ghostty", "-e"]),
        Family::Foot => with(&["foot"]),
        Family::Alacritty => with(&["alacritty", "-e"]),
        Family::Konsole => with(&["konsole", "-e"]),
        Family::Vte => with(&["gnome-terminal", "--"]),
        Family::Xterm => with(&["xterm", "-e"]),
        Family::Mintty => with(&["mintty", "-e"]),
        Family::WindowsTerminal => with(&["wt.exe", "-w", "new"]),
        Family::AppleTerminal => apple_script(&format!(
            "tell application \"Terminal\"\ndo script \"{}\"\nactivate\nend tell",
            script_string(&shell_line(cmd))
        )),
        Family::Iterm2 => apple_script(&format!(
            "tell application \"iTerm\"\ncreate window with default profile command \"{}\"\nend tell",
            script_string(&shell_line(cmd))
        )),
        _ => {
            let term = std::env::var("TERMINAL").ok().filter(|t| !t.is_empty())?;
            let mut v = vec![term, "-e".to_string()];
            v.extend(cmd.iter().cloned());
            v
        }
    })
}

/// `"foot -e {cmd}"` → `["foot", "-e", cmd…]` (`cmd` goes last without `{cmd}`).
fn fill(template: &str, cmd: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut placed = false;
    for word in template.split_whitespace() {
        if word == "{cmd}" {
            out.extend(cmd.iter().cloned());
            placed = true;
        } else {
            out.push(word.to_string());
        }
    }
    if !placed {
        out.extend(cmd.iter().cloned());
    }
    out
}

fn apple_script(script: &str) -> Vec<String> {
    vec!["osascript".into(), "-e".into(), script.into()]
}

/// A command line for a POSIX shell, every word quoted.
pub fn shell_line(cmd: &[String]) -> String {
    cmd.iter()
        .map(|w| {
            if !w.is_empty()
                && w.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c))
            {
                w.clone()
            } else {
                format!("'{}'", w.replace('\'', r"'\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn script_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Let a child process outlive this terminal (its own process group).
pub fn background(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
}

/// Start `argv` (a terminal program) without waiting for it.
pub fn spawn(argv: &[String]) -> Result<()> {
    let Some((program, args)) = argv.split_first() else {
        return Err(Error::friendly(
            "No terminal program to start",
            "the launcher command is empty",
            "set STAGETUI_LAUNCHER, e.g. STAGETUI_LAUNCHER=\"foot {cmd}\"",
        ));
    };
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // The new window is a fresh terminal: let it find out what it can do.
    for var in [
        "STAGETUI_SERVE",
        "STAGETUI_SESSION",
        "STAGETUI_LOG",
        "STAGETUI_NO_PROBE",
        "STAGETUI_HEADLESS",
    ] {
        cmd.env_remove(var);
    }
    background(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| {
        Error::friendly(
            format!("Could not open a new {program} window"),
            e.to_string(),
            "set STAGETUI_LAUNCHER to your terminal, e.g. STAGETUI_LAUNCHER=\"foot {cmd}\"",
        )
    })?;
    // Reap it when it ends, so no zombie is left behind.
    let _ = std::thread::Builder::new()
        .name("stagetui-launch".into())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::{DetectOptions, detect};

    fn caps(family: Family, os: &str) -> Capabilities {
        let mut c = detect(&DetectOptions {
            force_headless: true,
            ..Default::default()
        });
        c.terminal.family = family;
        c.platform.os = os.into();
        c.remote.ssh = false;
        c.remote.tmux = false;
        c
    }

    fn cmd() -> Vec<String> {
        ["stagetui", "attach", "night-shift:chat"]
            .map(String::from)
            .to_vec()
    }

    #[test]
    fn each_terminal_opens_its_own_kind_of_window() {
        let run = |f, os| {
            window_command(&caps(f, os), &cmd(), None)
                .unwrap()
                .join(" ")
        };
        assert_eq!(
            run(Family::Foot, "linux"),
            "foot stagetui attach night-shift:chat"
        );
        assert_eq!(
            run(Family::Kitty, "linux"),
            "kitty stagetui attach night-shift:chat"
        );
        assert_eq!(
            run(Family::Wezterm, "linux"),
            "wezterm start -- stagetui attach night-shift:chat"
        );
        assert_eq!(
            run(Family::Ghostty, "macos"),
            "open -na Ghostty --args -e stagetui attach night-shift:chat"
        );
        assert_eq!(
            run(Family::Vte, "linux"),
            "gnome-terminal -- stagetui attach night-shift:chat"
        );
        assert_eq!(
            run(Family::WindowsTerminal, "windows"),
            "wt.exe -w new stagetui attach night-shift:chat"
        );
        let apple = window_command(&caps(Family::AppleTerminal, "macos"), &cmd(), None).unwrap();
        assert_eq!(apple[0], "osascript");
        assert!(apple[2].contains("do script \"stagetui attach night-shift:chat\""));
    }

    #[test]
    fn tmux_ssh_and_overrides() {
        let mut c = caps(Family::Foot, "linux");
        c.remote.tmux = true;
        assert_eq!(
            window_command(&c, &cmd(), None).unwrap()[..2],
            ["tmux", "new-window"]
        );
        c.remote.ssh = true;
        assert!(
            window_command(&c, &cmd(), None).is_none(),
            "no windows over SSH"
        );
        let own = window_command(&c, &cmd(), Some("alacritty --class pop -e {cmd}")).unwrap();
        assert_eq!(
            own.join(" "),
            "alacritty --class pop -e stagetui attach night-shift:chat"
        );
        let plain = window_command(&c, &cmd(), Some("foot")).unwrap();
        assert_eq!(plain.join(" "), "foot stagetui attach night-shift:chat");
    }

    #[test]
    fn shell_words_are_quoted() {
        let words = ["python", "-m", "stagetui", "it's مرحبا 日本語 नमस्ते"].map(String::from);
        assert_eq!(
            shell_line(&words),
            r"python -m stagetui 'it'\''s مرحبا 日本語 नमस्ते'"
        );
        assert!(
            spawn(&[])
                .unwrap_err()
                .to_string()
                .contains("STAGETUI_LAUNCHER")
        );
    }
}
