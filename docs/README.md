# termstage documentation

**termstage** lets you build good-looking programs that run inside a terminal,
using plain Python. Cards, pictures, drawings, Arabic and other world languages
all work out of the box. The heavy lifting is done by a fast engine written in
Rust, so you never have to think about it.

It is also made for **AI coding agents**: every screen can be captured as text,
as a list of widgets, or as a PNG picture — no human needed.

> termstage is young (Milestone 2 of 6). Everything on these pages works today.
> Features that are planned but not built yet are clearly marked *coming soon*.

## Start here

| I want to… | Read |
|---|---|
| Install termstage and make my first program | [Getting started](getting-started.md) |
| Show a picture or a message with one line | [Show & draw](show-and-draw.md) |
| Draw my own graphics with code | [Show & draw → Drawing](show-and-draw.md#drawing-with-a-canvas) |
| Build a full-screen app | [Apps & widgets](apps.md) |
| Use Arabic, Persian, Hindi, Chinese… | [World languages](world-languages.md) |
| Test my app or let an AI agent see it | [Snapshots](snapshots.md) |
| Use termstage from the command line | [Command line](command-line.md) |
| Fix something that looks wrong | [Terminals & troubleshooting](troubleshooting.md) |
| Look up every function and option | [Reference](reference.md) |

## A taste

```python
import termstage as ts

ts.show("Hello! مرحبا! नमस्ते!", title="termstage", icon="sparkles")
```

```python
from termstage import StageApp, StageCard


class Hello(StageApp):
    def compose(self):
        return [StageCard("My first termstage app.", title="Hello", icon="sparkles")]


Hello().run()  # press q to quit
```
