# Terminals & troubleshooting

First step for any display problem:

```bash
stagetui doctor
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
| GNOME Terminal, Ptyxis, Tilix | Colored blocks | Shaped letters, joined by the terminal | stagetui turns off the terminal's own reordering |
| macOS Terminal.app | Colored blocks | Shaped letters | |
| Inside tmux | Depends on the outer terminal | — | Needs `set -g allow-passthrough on` for pictures |

## Common problems

**`ModuleNotFoundError: No module named 'stagetui'`**
: Your program runs with a different Python than the one stagetui was installed
  into. Activate your virtual environment (`source .venv/bin/activate`) and use `python`, not `python3` from the system.

**Part of the app is cut off at the bottom**
: The window is too short, so the screen scrolls: use the mouse wheel, PageUp /
  PageDown, or the arrow keys. A scrollbar on the right shows where you are.

**Clicking does nothing inside tmux**
: Turn on mouse support in tmux: add `set -g mouse on` to `~/.tmux.conf`.

**A shortcut like Ctrl+\\ or Ctrl+Shift+D doesn't work**
: Some terminals can't send every key combination. stagetui uses the modern
  "Kitty keyboard" mode where available (Kitty, Ghostty, WezTerm, foot) — elsewhere
  prefer F-keys, letters, or Ctrl+letter shortcuts.

**Typing "q" doesn't quit**
: A text box has the focus, so "q" is typed into it. Press **Esc** to leave the
  text box first, or press **Ctrl+C**.

**Animations are distracting**
: Set `STAGETUI_REDUCE_MOTION=1`, or `reduce_motion = True` on your app.

**Pictures look like blurry blocks**
: Your terminal has no picture support, so stagetui uses colored blocks.
  Switch to one of the terminals marked "Best" or "Full quality" above.

**Pictures don't appear inside tmux**
: Add `set -g allow-passthrough on` to `~/.tmux.conf`, then run `tmux source ~/.tmux.conf`.

**Arabic letters look separated or show as boxes**
: This only happens in terminals that can't show pictures (GNOME Terminal,
  Terminal.app…), where your terminal font draws the letters. Use a font with
  Arabic "presentation forms", such as **DejaVu Sans Mono**, at least as a fallback.

**Arabic is reversed**
: Please report it with the output of `stagetui inspect`. As a workaround, try
  `STAGETUI_TEXT_MODE=cells`.

**Everything is slow over SSH**
: stagetui detects SSH and lowers the frame rate automatically. If the terminal
  replies slowly, raise the wait time: `STAGETUI_PROBE_TIMEOUT_MS=1000`.

**"Can't start an interactive app here"**
: There is no terminal (for example inside a test or a pipe). Use
  `run_headless()` or `stagetui render` instead — see [Snapshots](snapshots.md).

**`stagetui attach` says "No session named …"**
: The app isn't running anymore, or has another name. `stagetui sessions` lists
  what is running. Apps started with `python app.py` end when their terminal
  closes; use `stagetui start app.py` for apps that must keep going ([Sessions](sessions.md)).

**Ctrl+Space then d says "To detach, start it in the background"**
: The app was started with `python app.py`, so it lives in this terminal.
  Quit it and start it with `stagetui start app.py` instead.

**"The session folder … is not yours"**
: Someone else (another user) created stagetui's session folder. Remove that
  folder, or set `XDG_RUNTIME_DIR` to a folder only you can open.

## Environment variables

| Variable | Effect |
|---|---|
| `STAGETUI_HEADLESS=1` | Always run headless (no terminal) |
| `STAGETUI_SIZE=120x40` | Screen size in headless mode |
| `STAGETUI_NO_PROBE=1` | Never ask the terminal questions |
| `STAGETUI_PROBE_TIMEOUT_MS=250` | How long to wait for the terminal's answers |
| `STAGETUI_TEXT_MODE=auto\|cells\|pixels` | How Arabic and other complex scripts are drawn |
| `STAGETUI_SYSTEM_FONTS=0` | Use only stagetui's bundled fonts (identical PNGs everywhere) |
| `STAGETUI_REDUCE_MOTION=1` | Turn off animations |
| `STAGETUI_NO_SESSION=1` | Don't let other terminals attach to apps run with `python app.py` |
| `STAGETUI_SESSION=name` | The session name to use ([Sessions](sessions.md)) |

## Your terminal is always restored

If your program crashes, is killed, or you close the window, stagetui puts the
terminal back the way it was — and stops any helper processes it started. If
something ever looks stuck, type `reset` and press Enter.
