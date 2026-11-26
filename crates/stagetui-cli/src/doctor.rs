//! `stagetui doctor`: a plain-English capability report with concrete fixes.

use std::fmt::Write as _;

use stagetui_core::caps::{Capabilities, GraphicsProtocol};
use stagetui_core::text::fonts::Coverage;

const OK: &str = "✓";
const NO: &str = "✗";
const INFO: &str = "•";

/// Build the human-readable report. `fonts` adds a font-coverage line when given.
pub fn report(c: &Capabilities, fonts: Option<&Coverage>) -> String {
    let mut s = String::new();
    let mut fixes: Vec<String> = Vec::new();
    let _ = writeln!(s, "stagetui doctor · v{}\n", c.version);

    let row = |s: &mut String, label: &str, mark: &str, text: &str| {
        let _ = writeln!(s, "  {label:<13} {mark} {text}");
    };

    let term = match (&c.terminal.name, &c.terminal.version) {
        (Some(n), Some(v)) => format!("{n} {v}"),
        (Some(n), None) => n.clone(),
        _ => format!("{:?}", c.terminal.family).to_lowercase(),
    };
    let term_extra = c
        .terminal
        .term
        .as_deref()
        .map(|t| format!("  (TERM={t})"))
        .unwrap_or_default();
    row(&mut s, "Terminal", INFO, &format!("{term}{term_extra}"));
    row(
        &mut s,
        "Platform",
        INFO,
        &format!("{} {}", c.platform.os, c.platform.arch),
    );

    if c.headless {
        row(
            &mut s,
            "Mode",
            INFO,
            &format!("headless — {}", c.headless_reason.as_deref().unwrap_or("")),
        );
        row(
            &mut s,
            "Size",
            INFO,
            &format!("{}×{} cells (virtual)", c.size.cols, c.size.rows),
        );
        let _ = writeln!(
            s,
            "\n  Headless mode renders apps offscreen (text + JSON tree) for CI and agents."
        );
        let _ = writeln!(
            s,
            "  Run `stagetui doctor` inside a terminal to check that terminal."
        );
        return s;
    }

    row(
        &mut s,
        "Size",
        INFO,
        &format!(
            "{}×{} cells · {}×{} px per cell ({:?})",
            c.size.cols,
            c.size.rows,
            c.size.cell_width_px,
            c.size.cell_height_px,
            c.size.cell_size_source
        )
        .to_lowercase(),
    );

    let g = &c.graphics;
    match g.best {
        GraphicsProtocol::Kitty => row(
            &mut s,
            "Images",
            OK,
            &format!(
                "Kitty graphics{}",
                if g.kitty_placeholders {
                    " (Unicode placeholders ✓)"
                } else {
                    ""
                }
            ),
        ),
        GraphicsProtocol::Sixel => row(&mut s, "Images", OK, "Sixel"),
        GraphicsProtocol::Iterm2 => row(&mut s, "Images", OK, "iTerm2 inline images"),
        GraphicsProtocol::Halfblocks => {
            row(
                &mut s,
                "Images",
                NO,
                "no pixel protocol — using half-block characters",
            );
            if c.remote.tmux {
                fixes.push("Inside tmux, add `set -g allow-passthrough on` to ~/.tmux.conf so images reach your terminal.".into());
            }
            fixes.push(
                "For crisp images use Kitty, Ghostty, WezTerm, iTerm2, foot, or Windows Terminal ≥ 1.22.".into(),
            );
        }
    }

    if c.keyboard.kitty_protocol {
        row(
            &mut s,
            "Keyboard",
            OK,
            "Kitty keyboard protocol (every shortcut is unambiguous)",
        );
    } else {
        row(
            &mut s,
            "Keyboard",
            INFO,
            "legacy keys (a few Ctrl+Shift shortcuts may be indistinguishable)",
        );
    }

    if c.truecolor {
        row(&mut s, "Colors", OK, "24-bit truecolor");
    } else {
        row(&mut s, "Colors", NO, "256 colors (themes are approximated)");
        fixes.push("If your terminal supports truecolor, set COLORTERM=truecolor.".into());
    }

    if c.sync_output {
        row(
            &mut s,
            "Smooth draw",
            OK,
            "synchronized output (no tearing)",
        );
    } else {
        row(
            &mut s,
            "Smooth draw",
            INFO,
            "no synchronized output (fast redraws may flicker slightly)",
        );
    }

    let pixels = g.best != GraphicsProtocol::Halfblocks
        && std::env::var("STAGETUI_TEXT_MODE").as_deref() != Ok("cells");
    if pixels {
        row(
            &mut s,
            "Arabic/RTL",
            OK,
            "drawn as pictures with bundled fonts — joined and ordered, whatever your font",
        );
    } else if c.bidi.terminal_reorders {
        row(
            &mut s,
            "Arabic/RTL",
            OK,
            "shaped by stagetui; terminal BiDi switched to explicit mode",
        );
    } else {
        row(
            &mut s,
            "Arabic/RTL",
            INFO,
            "shaped and ordered by stagetui, drawn with your terminal font",
        );
        fixes.push(
            "For Arabic in text cells, use a monospace font with Arabic presentation forms (e.g. DejaVu Sans Mono) as a fallback.".into(),
        );
    }

    if let Some(fonts) = fonts {
        let missing: Vec<&str> = fonts
            .scripts
            .iter()
            .filter(|s| !s.covered)
            .map(|s| s.script)
            .collect();
        if missing.is_empty() {
            row(
                &mut s,
                "Fonts",
                OK,
                "bundled Noto + DejaVu; every common script covered",
            );
        } else {
            row(
                &mut s,
                "Fonts",
                INFO,
                &format!(
                    "bundled Noto + DejaVu; no system font for: {}",
                    missing.join(", ")
                ),
            );
            if missing
                .iter()
                .any(|m| matches!(*m, "chinese" | "japanese" | "korean"))
            {
                fixes.push(
                    "For Chinese/Japanese/Korean in pictures, install Noto CJK (e.g. `apt install fonts-noto-cjk`, `pacman -S noto-fonts-cjk`).".into(),
                );
            }
        }
    }

    let remote = match (c.remote.ssh, c.remote.tmux, c.remote.screen) {
        (false, false, false) => "local".to_string(),
        (ssh, tmux, screen) => {
            let mut parts = vec![];
            if ssh {
                parts.push("SSH");
            }
            if tmux {
                parts.push("tmux");
            }
            if screen {
                parts.push("screen");
            }
            parts.join(" + ")
        }
    };
    let link = if c.link.slow {
        format!(" · slow link: {} FPS, compressed", c.link.max_fps)
    } else {
        String::new()
    };
    row(&mut s, "Connection", INFO, &format!("{remote}{link}"));

    match (&c.probe.error, c.probe.rtt_ms, c.probe.timed_out) {
        (Some(e), _, _) => row(
            &mut s,
            "Probe",
            NO,
            &format!("could not query the terminal: {e}"),
        ),
        (None, Some(ms), _) => row(&mut s, "Probe", OK, &format!("{ms:.1} ms round trip")),
        (None, None, true) => {
            row(&mut s, "Probe", NO, "terminal did not answer in time");
            fixes.push(
                "Raise the probe timeout with STAGETUI_PROBE_TIMEOUT_MS=1000 (slow SSH links)."
                    .into(),
            );
        }
        _ => row(&mut s, "Probe", INFO, "skipped"),
    }

    if fixes.is_empty() {
        let _ = writeln!(s, "\n  Everything looks great.");
    } else {
        let _ = writeln!(s, "\n  Suggestions:");
        for f in fixes {
            let _ = writeln!(s, "   → {f}");
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use stagetui_core::caps::{Env, Grid, ProbeInfo, TtyInfo, parse, resolve};

    fn report_for(c: &Capabilities) -> String {
        report(c, None)
    }

    fn caps(env: &[(&str, &str)], replies: &[u8]) -> Capabilities {
        resolve(
            &Env::from_pairs(env.iter().copied()),
            TtyInfo {
                stdin: true,
                stdout: true,
            },
            None,
            &parse::parse(replies),
            Grid {
                cols: 100,
                rows: 30,
                width_px: 0,
                height_px: 0,
            },
            ProbeInfo {
                performed: true,
                rtt_ms: Some(1.5),
                ..Default::default()
            },
        )
    }

    #[test]
    fn kitty_is_happy() {
        let r = report_for(&caps(
            &[],
            b"\x1bP>|kitty(0.39.1)\x1b\\\x1b_Gi=31;OK\x1b\\\x1b[?0u\x1b[?2026;2$y\x1b[?62c",
        ));
        assert!(r.contains("Kitty graphics (Unicode placeholders ✓)"), "{r}");
        assert!(r.contains("Everything looks great"), "{r}");
    }

    #[test]
    fn tmux_without_passthrough_gets_a_fix() {
        let r = report_for(&caps(&[("TMUX", "x")], b"\x1bP>|tmux 3.5a\x1b\\\x1b[?1;2c"));
        assert!(r.contains("allow-passthrough"), "{r}");
    }

    #[test]
    fn vte_bidi_is_reported() {
        let r = report_for(&caps(&[("VTE_VERSION", "8002")], b"\x1b[?65;1;9c"));
        assert!(r.contains("terminal BiDi switched to explicit mode"), "{r}");
    }

    #[test]
    fn headless_is_explained() {
        let c = resolve(
            &Env::default(),
            TtyInfo {
                stdin: false,
                stdout: false,
            },
            Some("no TTY".into()),
            &parse::Replies::default(),
            Grid {
                cols: 80,
                rows: 24,
                width_px: 0,
                height_px: 0,
            },
            ProbeInfo::default(),
        );
        let r = report(&c, None);
        assert!(r.contains("headless") && r.contains("80×24"), "{r}");
    }
}
