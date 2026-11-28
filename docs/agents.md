# Let AI agents see and use your app

stagetui apps can be operated by programs, not only by people. An AI coding
agent can start your app, read the screen, click, type, and check the result —
without a terminal window. This makes "build it, look at it, fix it" loops work.

There are three ways in, from simplest to most flexible:

| Way | For |
|---|---|
| `stagetui mcp my_app.py` | AI agents that speak **MCP** (Claude Code and many others) |
| `stagetui drive my_app.py` | Any program: JSON messages on stdin/stdout |
| `app.drive()` in Python | Tests and scripts |

## MCP: plug your app into a coding agent

MCP (Model Context Protocol) is how AI agents get new tools. One command turns
your app into tools the agent can call:

```bash
stagetui mcp my_app.py              # serves the app's tools on stdin/stdout
```

To add it to **Claude Code**:

```bash
claude mcp add my-app -- stagetui mcp /full/path/to/my_app.py
```

Other agents have a similar "add MCP server" setting: the command is
`stagetui`, the arguments are `mcp /full/path/to/my_app.py`.

The agent then has these tools:

| Tool | What it does |
|---|---|
| `snapshot` | The screen as text plus every widget (ids, positions, focus); `png: true` adds a picture |
| `press` | Press a key (`"enter"`, `"tab"`, `"ctrl+s"`…) or several (`keys: [...]`) |
| `type` | Type text into the focused widget |
| `click` | Click a widget (`target: "#send"`) or a cell (`x`, `y`) |
| `drag` | Drag from a widget (`target`) or a cell (`x`, `y`) to a cell (`to_x`, `to_y`): split lines, 3D models |
| `scroll` | Mouse-wheel scrolling |
| `resize` | Change the screen size |
| `wait` | Let time pass (animations, live drawings) |
| `query` | One widget's details by id |
| `list_panes` / `set_pane_type` | See and switch [panes](panes.md) |
| `keys` | Every key the app answers to — its built-in keys by name (an app or its user may have [moved them](keys.md)), its own shortcuts, and whether quitting asks first |
| `workspace` / `load_workspace` / `save_workspace` | Read the layout of splits and panes, rearrange it (`layout: {...}` or `name`), save it ([workspaces](panes.md#save-a-layout-bring-it-back)) |

Every action answers with the new screen, so the agent always sees the effect.

When the app quits — the agent pressed `q`, Ctrl+Space then `q`, or clicked the
last pane's × — that answer starts with **"The app has quit"** and carries
`"quit": true`. After that only `snapshot` still answers (with the last screen);
every other tool returns an error saying the app has quit. Start
`stagetui mcp` again to go on.

The app runs as if you had typed `python my_app.py`: `sys.argv` is
`["my_app.py"]`, so an app that reads its own arguments gets none.

## drive: JSON messages for any program

`stagetui drive my_app.py` reads one JSON-RPC 2.0 message per line and answers
each one on its own line:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"snapshot"}' | stagetui drive my_app.py
```

Methods: `snapshot`, `press`, `type`, `click`, `drag`, `scroll`, `resize`, `wait`,
`wait_idle`, `query`, `panes`, `keys`, `set_pane_type`, `shutdown`. Their parameters are
the same as the MCP tools above. Errors come back with `what`, `why` and `fix`.
Once the app has quit, answers carry `"quit": true`, and every method but
`snapshot` and `shutdown` returns the error "The app has quit".

Anything your app prints goes to stderr, so it never mixes with the answers.

## In Python: `app.drive()`

```python
from stagetui import StageApp, StageButton, StageCard, StageInput


class Greeter(StageApp):
    def compose(self):
        return [
            StageInput(placeholder="Name", id="name"),
            StageButton("Greet", id="greet", on_click=self.greet),
            StageCard("…", id="out"),
        ]

    def greet(self):
        self.query("#out").update(f"Hello, {self.query('#name').value}!")


with Greeter().drive((60, 16)) as d:
    d.type("Sara")
    d.click("#greet")
    print(d.snapshot().text)  # the screen, as text
    print(d.query("out")["text"])  # "Hello, Sara!"
```

A driver has `press`, `type`, `click`, `drag`, `scroll`, `resize`, `wait`, `wait_idle`,
`snapshot(png=False)`, `query(id)`, `panes()` and `keys()`. Time is virtual, so every run
gives the same result.

When the `with` block ends, every shell and add-on the app started stops right
away (so does `run_headless()` when it returns); the snapshots you took still
work. If the app quits on the way — say a step pressed `q` — `d.has_quit` turns
`True`:

```python
from stagetui import StageApp, StageCard


class Hello(StageApp):
    def compose(self):
        return [StageCard("مَرْحَبًا Hello 日本語 नमस्ते")]


with Hello().drive((60, 10)) as d:
    d.press("q")
    print(d.has_quit)  # True: you can still look (d.snapshot()), not press
```

## Look at a running app

While your app runs in a real terminal, your own code can capture what it shows —
handy for bug reports or for an agent inside your harness:

```python
from stagetui import StageApp, StageCard


class Report(StageApp):
    def compose(self):
        return [StageCard("All good", id="status")]

    def on_mount(self):
        text = self.capture_text()  # the screen as text
        tree = self.capture_tree()  # every widget as a dict
        picture = self.capture_png()  # PNG bytes


Report().run()
```
