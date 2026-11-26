# Getting started

This page takes you from nothing to your first stagetui program. No previous
terminal experience needed.

## What you need

- **Python 3.11 or newer.** Check with `python --version` (on some systems `python3 --version`).
- **A terminal.** That's the window where you type commands. Any terminal works.
  For the sharpest pictures use one of these:
  - Linux: **Kitty**, **Ghostty**, **WezTerm**, **foot**, or GNOME Terminal
  - macOS: **iTerm2**, **Kitty**, **Ghostty**, **WezTerm**
  - Windows: **Windows Terminal** (version 1.22 or newer)

## Install

It's best to install Python packages into a *virtual environment*: a private
folder of packages for one project, so projects don't disturb each other.

```bash
python -m venv .venv
source .venv/bin/activate        # Windows: .venv\Scripts\activate
pip install stagetui
```

> **"ModuleNotFoundError: No module named 'stagetui'"?** You are probably using
> a different Python than the one you installed into. Activate the virtual
> environment again (the `source` line above), then run your program with `python`.

### From the source code (before the first PyPI release)

You need [Rust](https://rustup.rs) and [uv](https://docs.astral.sh/uv/):

```bash
git clone <the stagetui repository> && cd stagetui
uv venv && source .venv/bin/activate
uv pip install maturin
maturin develop --release
```

## Check that it works

```bash
stagetui doctor
```

`doctor` looks at your terminal and explains, in plain English, what it can do:
pictures, colors, keyboard shortcuts, Arabic, and so on. If something could be
better, it tells you how to fix it.

## Your first picture

Make a file called `hello.py`:

```python
import stagetui as ts

ts.show("Hello from stagetui!", title="My first card", icon="sparkles")
```

Run it:

```bash
python hello.py
```

A neat card with a rounded border appears. `ts.show` is like `print`, but prettier.
It also shows pictures: `ts.show("photo.png")`.

## Your first app

An *app* takes over the whole terminal window until you quit — like a game or
a text editor. Here is the smallest one:

```python
from stagetui import StageApp, StageCard


class Hello(StageApp):
    theme = "aurora-dark"

    def compose(self):
        return [
            StageCard("Welcome! Press q to quit.", title="Hello", icon="sparkles"),
            StageCard("مرحبا بكم", title="عربي", accent="mint"),
        ]


Hello().run()
```

What's going on:

1. `class Hello(StageApp)` makes your own app, based on stagetui's `StageApp`.
2. `theme = "aurora-dark"` picks a color scheme (there are 5).
3. `compose` returns a **list of widgets** — the things to show on screen.
4. `StageCard(...)` is a widget: a box with a title and some text.
5. `Hello().run()` starts the app. Press **q**, **Esc** or **Ctrl+C** to quit.

## Words you'll see

| Word | Meaning |
|---|---|
| **Terminal** | The window where you type commands and see text output. |
| **Cell** | One character box in the terminal grid. Sizes like `width=20` are in cells. |
| **Pixel** | One dot of a picture. Canvas sizes like `size=(640, 360)` are in pixels. |
| **Widget** | One thing on screen: a card, some text, an image. |
| **Headless** | Running without a terminal (in tests, CI or AI agents). stagetui then draws offscreen. |
| **RTL** | Right-to-left: languages like Arabic, Persian and Hebrew. |
| **Theme / accent** | The color scheme / the highlight color. |

## Next steps

- [Show & draw](show-and-draw.md) — pictures and drawings in one line.
- [Apps & widgets](apps.md) — everything you can put in an app.
- [Snapshots](snapshots.md) — test your app, or let an AI agent look at it.
