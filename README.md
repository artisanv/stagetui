# termstage

**Beautiful terminal programs in plain Python — with pictures, drawings and every world language.**
A fast Rust engine does the hard work. Built for humans *and* for AI coding agents.

```python
import termstage as ts

ts.show("Hello! مرحبا! नमस्ते! こんにちは!", title="termstage", icon="sparkles")
ts.show("logo.png")
```

> **Status: pre-alpha (Milestone 2 of 6).** Text cards, pictures, drawing,
> Arabic/RTL and headless snapshots work today. Layouts, buttons, mouse,
> animations and agent tools are on the way — see the [roadmap](#roadmap).

## Why termstage?

- **Friendly.** One line shows a picture or a message. Errors tell you *what* went wrong, *why*, and *how to fix it*.
- **Pictures that look right everywhere.** Kitty, Ghostty, WezTerm, iTerm2, Windows Terminal, foot… termstage picks the best method your terminal supports, and falls back gracefully.
- **Every language.** Arabic and Persian letters join and flow right-to-left, Hindi conjuncts form, Chinese and Japanese line up — in every terminal, with fonts built in.
- **Made for AI agents.** Any app can be captured as plain text, a widget tree, or a PNG — no terminal needed.
- **Fast and safe.** The engine is Rust. The Python package has zero dependencies. Nothing is left running when your program exits — even after a crash.
- **Linux, macOS and Windows.**

## Install

```bash
pip install termstage
termstage doctor          # what can my terminal do?
```

New to Python? Start with **[Getting started](docs/getting-started.md)** — it explains everything step by step.

## A full-screen app

```python
from termstage import StageApp, StageCard, StageCanvas


def chart(c):
    c.polyline([(10, 90), (60, 40), (110, 60), (160, 20)], color="mint", width=4)


class Hello(StageApp):
    theme = "aurora-dark"

    def compose(self):
        return [
            StageCard("Welcome! Press q to quit.", title="Hello", icon="sparkles"),
            StageCard("مرحبا بكم في termstage", title="عربي", accent="mint"),
            StageCanvas(chart, size=(170, 100)),
        ]


Hello().run()                                  # in your terminal
Hello().run_headless((70, 24)).save("app.png")  # no terminal: text, tree or PNG
```

## Command line

```bash
termstage show chart.svg --width 40
termstage render "مرحبا بكم" --dir rtl --out card.png
termstage render my_app.py --size 100x30 --out app.png --text app.txt --tree app.json
termstage doctor
```

## Documentation

| | |
|---|---|
| [Getting started](docs/getting-started.md) | Install and write your first program |
| [Show & draw](docs/show-and-draw.md) | Pictures and drawings in one line |
| [Apps & widgets](docs/apps.md) | Cards, text, images, canvases, themes, icons |
| [World languages](docs/world-languages.md) | Arabic, Persian, Hindi, CJK |
| [Snapshots](docs/snapshots.md) | Tests, CI and AI agents |
| [Command line](docs/command-line.md) | `show`, `render`, `doctor`, `inspect` |
| [Troubleshooting](docs/troubleshooting.md) | Terminal support and fixes |
| [Reference](docs/reference.md) | Every function and option |

## Roadmap

1. ✅ Terminal core: capability detection, renderer, headless snapshots, zero-zombie cleanup
2. ✅ Text & pictures: Arabic/RTL shaping, bundled fonts, Kitty/Sixel/iTerm2 images, Canvas, PNG export
3. Layouts & interaction: split panes, buttons, inputs, mouse, themes, animations
4. Agent harness kit: streaming Markdown, code, diffs, prompt, tool cards, embedded shell, `drive`/`mcp`
5. Sessions: pop a pane out into its own terminal window and back
6. Optional plugins: video, web, 3D

## Develop

```bash
uv venv && source .venv/bin/activate && uv pip install maturin pytest
maturin develop --release
cargo test && python -m pytest
```

See [AGENTS.md](AGENTS.md) for the project rules. Licensed under MIT or Apache-2.0, at your option.
Bundled fonts: Noto (SIL OFL 1.1) and DejaVu Sans Mono (Bitstream Vera license).
