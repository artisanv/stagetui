# stagetui

**Beautiful terminal programs in plain Python — with pictures, drawings and every world language.**
A fast Rust engine does the hard work. Built for humans *and* for AI coding agents.

```python
import stagetui as ts

ts.show("Hello! مرحبا! नमस्ते! こんにちは!", title="stagetui", icon="sparkles")
ts.show("logo.png")
```

> **Status: first alpha (Milestone 4 of 6).** The agent harness kit (chat, streaming
> Markdown, prompt, diff, tool calls, permission prompts, a real shell in a pane),
> Blender-style panes, agent access (`stagetui mcp` / `drive`), layouts, the mouse,
> animations, Arabic/RTL and headless snapshots work today — see the [roadmap](#roadmap).

## Why stagetui?

- **Friendly.** One line shows a picture or a message. Errors tell you *what* went wrong, *why*, and *how to fix it*.
- **Pictures that look right everywhere.** Kitty, Ghostty, WezTerm, iTerm2, Windows Terminal, foot… stagetui picks the best method your terminal supports, and falls back gracefully.
- **Every language.** Arabic and Persian letters join and flow right-to-left, Hindi conjuncts form, Chinese and Japanese line up — in every terminal, with fonts built in.
- **Made for AI agents.** Any app can be captured as plain text, a widget tree, or a PNG — no terminal needed.
- **Fast and safe.** The engine is Rust. The Python package has zero dependencies. Nothing is left running when your program exits — even after a crash.
- **Linux, macOS and Windows.**

## Install

```bash
pip install stagetui
stagetui doctor          # what can my terminal do?
```

New to Python? Start with **[Getting started](docs/getting-started.md)** — it explains everything step by step.

## A full-screen app

```python
from stagetui import StageApp, StageBar, StageButton, StageCard, StageInput, StageSplit


class Chat(StageApp):
    BINDINGS = [("ctrl+t", "next_theme", "Theme")]

    def compose(self):
        return [
            StageSplit(
                [
                    StageCard("Welcome! Type below and press Enter.", title="Chat", id="log"),
                    StageCard("مرحبا بكم في stagetui", title="عربي", accent="mint"),
                ],
                ratio=[2, 1],  # drag the line between the panes with the mouse
            ),
            StageInput(placeholder="Say something…", on_submit=self.send),
            StageButton("Celebrate", icon="sparkles", variant="primary", on_click=self.party),
            StageBar("Ready"),
        ]

    def send(self, text):
        self.query("#log").update(f"You said: {text}")

    def party(self):
        self.notify("Well done!", icon="check")


Chat().run()  # in your terminal
Chat().run_headless((80, 24)).save("app.png")  # no terminal: text, tree or PNG
```

Try the tours: `python examples/05_ui_gallery.py`, `06_splits.py`, `07_harness.py` (a coding-agent
harness with a pretend agent) and `08_workspace.py` (Blender-style panes).

## Command line

```bash
stagetui show chart.svg --width 40
stagetui render "مرحبا بكم" --dir rtl --out card.png
stagetui render my_app.py --size 100x30 --out app.png --text app.txt --tree app.json
stagetui doctor
stagetui mcp my_app.py      # let a coding agent (Claude Code…) see and operate your app
```

## Documentation

| | |
|---|---|
| [Getting started](docs/getting-started.md) | Install and write your first program |
| [Show & draw](docs/show-and-draw.md) | Pictures and drawings in one line |
| [Apps & widgets](docs/apps.md) | Cards, text, images, canvases, themes, icons |
| [Layouts](docs/layout.md) | Rows, grids, split panes, tabs, pop-ups, scrolling |
| [Making apps interactive](docs/events.md) | Buttons, inputs, lists, shortcuts, mouse, `async` |
| [Motion](docs/motion.md) | Animations and live drawings |
| [Agent harness](docs/harness.md) | Chat, prompt, diffs, tool calls, permissions, shell |
| [Panes](docs/panes.md) | Blender-style panes: switch, split, close, your own types |
| [AI agents](docs/agents.md) | `stagetui mcp`, `stagetui drive`, `app.drive()`, golden snapshot tests |
| [World languages](docs/world-languages.md) | Arabic, Persian, Hindi, CJK |
| [Snapshots](docs/snapshots.md) | Tests, CI and AI agents |
| [Command line](docs/command-line.md) | `show`, `render`, `doctor`, `inspect` |
| [Troubleshooting](docs/troubleshooting.md) | Terminal support and fixes |
| [Reference](docs/reference.md) | Every function and option |

## Roadmap

1. ✅ Terminal core: capability detection, renderer, headless snapshots, zero-zombie cleanup
2. ✅ Text & pictures: Arabic/RTL shaping, bundled fonts, Kitty/Sixel/iTerm2 images, Canvas, PNG export
3. ✅ Layouts & interaction: taffy layout, split panes, tabs, modals, buttons, inputs, tables, trees, mouse, themes, animations, live canvas, async handlers
4. ✅ Agent harness kit: streaming Markdown, code, diffs, prompt, chat, tool cards, permissions, embedded shell, Blender-style panes, `drive`/`mcp`, pytest plugin
5. Sessions & workspaces: pop a pane out into its own terminal window and back, save/restore layouts
6. Optional plugins: video, web, 3D

## Develop

```bash
uv venv && source .venv/bin/activate && uv pip install maturin pytest
maturin develop --release
cargo test && python -m pytest
```

See [AGENTS.md](AGENTS.md) for the project rules. Licensed under MIT or Apache-2.0, at your option.
Bundled fonts: Noto (SIL OFL 1.1) and DejaVu Sans Mono (Bitstream Vera license).
