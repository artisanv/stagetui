# Terminals & troubleshooting

First step for any display problem:

```bash
termstage doctor
```

## What each terminal supports

| Terminal | Pictures | Arabic | Notes |
|---|---|---|---|
| Kitty (0.28+) | Best | Pictures of text | Everything supported |
| Ghostty | Best | Pictures of text | Everything supported |
| WezTerm | Full quality | Pictures of text | Uses the iTerm2 picture format |
| iTerm2 (macOS) | Full quality | Pictures of text | |
| Windows Terminal 1.22+ | Sixel | Pictures of text | |
| foot, xterm | Sixel | Pictures of text | |
| GNOME Terminal, Ptyxis, Tilix | Colored blocks | Shaped letters, joined by the terminal | termstage turns off the terminal's own reordering |
| macOS Terminal.app | Colored blocks | Shaped letters | |
| Inside tmux | Depends on the outer terminal | — | Needs `set -g allow-passthrough on` for pictures |

## Common problems

**`ModuleNotFoundError: No module named 'termstage'`**
: Your program runs with a different Python than the one termstage was installed
  into. Activate your virtual environment (`source .venv/bin/activate`) and use `python`, not `python3` from the system.

**The app says "↓ more below"**
: The window is too short for everything. Make it taller (scrolling arrives in Milestone 3).

**Pictures look like blurry blocks**
: Your terminal has no picture support, so termstage uses colored blocks.
  Switch to one of the terminals marked "Best" or "Full quality" above.

**Pictures don't appear inside tmux**
: Add `set -g allow-passthrough on` to `~/.tmux.conf`, then run `tmux source ~/.tmux.conf`.

**Arabic letters look separated or show as boxes**
: This only happens in terminals that can't show pictures (GNOME Terminal,
  Terminal.app…), where your terminal font draws the letters. Use a font with
  Arabic "presentation forms", such as **DejaVu Sans Mono**, at least as a fallback.

**Arabic is reversed**
: Please report it with the output of `termstage inspect`. As a workaround, try
  `TERMSTAGE_TEXT_MODE=cells`.

**Everything is slow over SSH**
: termstage detects SSH and lowers the frame rate automatically. If the terminal
  replies slowly, raise the wait time: `TERMSTAGE_PROBE_TIMEOUT_MS=1000`.

**"Can't start an interactive app here"**
: There is no terminal (for example inside a test or a pipe). Use
  `run_headless()` or `termstage render` instead — see [Snapshots](snapshots.md).

## Environment variables

| Variable | Effect |
|---|---|
| `TERMSTAGE_HEADLESS=1` | Always run headless (no terminal) |
| `TERMSTAGE_SIZE=120x40` | Screen size in headless mode |
| `TERMSTAGE_NO_PROBE=1` | Never ask the terminal questions |
| `TERMSTAGE_PROBE_TIMEOUT_MS=250` | How long to wait for the terminal's answers |
| `TERMSTAGE_TEXT_MODE=auto\|cells\|pixels` | How Arabic and other complex scripts are drawn |
| `TERMSTAGE_SYSTEM_FONTS=0` | Use only termstage's bundled fonts (identical PNGs everywhere) |

## Your terminal is always restored

If your program crashes, is killed, or you close the window, termstage puts the
terminal back the way it was — and stops any helper processes it started. If
something ever looks stuck, type `reset` and press Enter.
