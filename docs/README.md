# stagetui documentation

**stagetui** lets you build good-looking programs that run inside a terminal,
using plain Python. Cards, pictures, drawings, Arabic and other world languages
all work out of the box. The heavy lifting is done by a fast engine written in
Rust, so you never have to think about it.

It is also made for **AI coding agents**: every screen can be captured as text,
as a list of widgets, or as a PNG picture — no human needed.

> stagetui is young (Milestone 5 of 6). Everything on these pages works today.
> Features that are planned but not built yet are clearly marked *coming soon*.

## Start here

| I want to… | Read |
|---|---|
| Install stagetui and make my first program | [Getting started](getting-started.md) |
| Show a picture or a message with one line | [Show & draw](show-and-draw.md) |
| Draw my own graphics with code | [Show & draw → Drawing](show-and-draw.md#drawing-with-a-canvas) |
| Build a full-screen app | [Apps & widgets](apps.md) |
| Put things side by side, split the screen, add tabs or pop-ups | [Layouts](layout.md) |
| Add buttons, text boxes, lists and keyboard shortcuts | [Making apps interactive](events.md) |
| Animate things, or draw live graphics | [Motion](motion.md) |
| Build a chat app for an AI coding agent | [Build an AI agent harness](harness.md) |
| Let people switch what each pane shows, run a shell in a pane | [Panes](panes.md) |
| Open a pane in its own terminal window (second monitor!) | [Panes → Own window](panes.md#open-a-pane-in-its-own-window) |
| Save and restore layouts, workspace tabs like Blender | [Panes → Workspaces](panes.md#workspaces-whole-layouts-like-blenders-tabs) |
| Keep an app running in the background, attach to it later | [Sessions](sessions.md) |
| Add video, web pages, 3D — or write my own pane type | [Add-ons](plugins.md) |
| Let an AI agent (Claude Code…) see and operate my app | [AI agents](agents.md) |
| Use Arabic, Persian, Hindi, Chinese… | [World languages](world-languages.md) |
| Test my app (press keys, click) or let an AI agent see it | [Snapshots](snapshots.md) |
| Use stagetui from the command line | [Command line](command-line.md) |
| Fix something that looks wrong | [Terminals & troubleshooting](troubleshooting.md) |
| Look up every function and option | [Reference](reference.md) |

## A taste

```python
import stagetui as ts

ts.show("Hello! مرحبا! नमस्ते!", title="stagetui", icon="sparkles")
```

```python
from stagetui import StageApp, StageCard


class Hello(StageApp):
    def compose(self):
        return [StageCard("My first stagetui app.", title="Hello", icon="sparkles")]


Hello().run()  # press q to quit
```
