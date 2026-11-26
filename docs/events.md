# Making apps interactive

Widgets can call your Python functions when something happens: a click, typed
text, a choice. These functions are called **handlers**. You pass them with
`on_<something>=`.

```python
from stagetui import StageApp, StageButton, StageCard, StageInput


class Greeter(StageApp):
    def compose(self):
        return [
            StageInput(placeholder="Your name", id="name", on_submit=self.greet),
            StageButton("Say hello", on_click=self.greet),
            StageCard("…", id="hello"),
        ]

    def greet(self):
        name = self.query("#name").value or "stranger"
        self.query("#hello").update(f"Hello, {name}!")


Greeter().run()
```

Three things happen here:

1. `on_submit=self.greet` — when Enter is pressed in the input, `greet` runs.
2. `self.query("#name")` finds a widget by its `id`; `.value` is its current text.
3. `.update(...)` changes what a widget shows — the screen refreshes by itself.

## The controls and their events

| Widget | Event | Your handler receives |
|---|---|---|
| `StageButton("Save", on_click=...)` | clicked, or Enter/Space while focused | nothing |
| `StageInput("", on_change=..., on_submit=...)` | every edit / Enter | the text |
| `StageCheckbox("Agree", on_change=...)` | ticked or unticked | `True` or `False` |
| `StageSelect(["S", "M", "L"], on_change=...)` | a new choice | the chosen text |
| `StageTable(rows, on_select=..., on_submit=...)` | row selected / Enter | `{"index": 2, "row": [...]}` |
| `StageTree(items, on_select=..., on_submit=...)` | item selected / leaf opened | `{"label": ..., "path": [...], "leaf": True}` |
| `StageTabs(pages, on_change=...)` | another tab chosen | the page number |
| `StageModal(children, on_close=...)` | closed with Esc or a click outside | nothing |
| `StageCanvas(on_frame=...)` | time for a new picture | `(canvas, seconds)` — see [Motion](motion.md) |

Handlers may take **fewer** arguments than offered: `on_change=lambda: print("changed")`
works too.

### More widgets you can use

```python
from stagetui import (
    Stage,
    StageApp,
    StageCheckbox,
    StageProgress,
    StageSelect,
    StageSpinner,
    StageTable,
    StageTree,
)


class Everything(StageApp):
    def compose(self):
        return [
            StageCheckbox("Send me updates", checked=True),
            StageSelect(["Small", "Medium", "Large"], value="Medium", title="Size"),
            StageProgress(0.4, label="Download"),  # 0 to 1; None = "busy" animation
            StageSpinner("Thinking…"),
            Stage(
                [
                    StageTable([["Ada", 36], ["Linus", 54]], headers=["Name", "Age"]),
                    StageTree({"src": ["main.py", "app.py"], "README.md": None}),
                ],
                direction="horizontal",
            ),
        ]


Everything().run()
```

## `async` handlers

A handler can be `async def`. Use it for slow work — downloads, AI calls — so
the screen stays responsive while you wait:

```python
import asyncio

from stagetui import StageApp, StageButton, StageProgress


class Download(StageApp):
    def compose(self):
        return [StageButton("Download", on_click=self.download), StageProgress(0, id="bar")]

    async def download(self):
        bar = self.query("#bar")
        for step in range(11):
            bar.update(step / 10)
            await asyncio.sleep(0.1)  # the app keeps drawing meanwhile
        self.notify("Download complete", icon="check")


Download().run()
```

Already inside async code? Use `await app.run_async()` instead of `app.run()`.

## Changing widgets while the app runs

Every widget has the same methods ("learn once, use everywhere"):

| Method | What it does |
|---|---|
| `w.update(content, **options)` | Change what it shows and/or its options: `card.update("Done", accent="mint")` |
| `w.load(content)` | Replace just the main content |
| `w.show()` / `w.hide()` | Show or hide (a hidden widget takes no space) |
| `w.focus()` | Give it the keyboard focus |
| `w.split(other, direction=...)` | Put another widget beside or below it ([Layouts](layout.md)) |
| `w.remove()` | Take it away |
| `w.value` | Its current value: typed text, checked state, selection… |

And on the app:

| | |
|---|---|
| `self.query("#id")` | Find a widget by id |
| `self.notify("Saved!", icon="check")` | A small message in the corner for a few seconds |
| `self.set_theme("paper-soft")` | Switch colors instantly |
| `self.focused`, `self.focused_pane` | The focused widget, and the split pane it is in |
| `self.mount(widget)` / `stage.add(widget)` | Add a new widget while running |
| `self.exit()` | Quit |
| `def on_mount(self): ...` | Runs once, right after the app appears |

## Keyboard shortcuts with `BINDINGS`

```python
from stagetui import StageApp, StageBar, StageCard


class Notes(StageApp):
    BINDINGS = [
        ("ctrl+s", "save", "Save"),
        ("ctrl+t", "next_theme", "Theme"),  # a built-in action
        ("f1", "help"),  # no description: not shown in the bar
    ]

    def compose(self):
        return [StageCard("Press Ctrl+S", id="note"), StageBar()]

    def action_save(self):
        self.notify("Saved", icon="check")

    def action_help(self):
        self.notify("Ctrl+S saves")


Notes().run()
```

Each entry is `(key, action)` or `(key, action, description)`. stagetui calls
`action_<name>` on your app (or a method called `<name>`). Keys look like
`"ctrl+s"`, `"ctrl+shift+d"`, `"alt+x"`, `"f5"`, `"enter"`, `"?"`, `"ctrl+backslash"`.

**Built-in actions:** `quit`, `next_theme`, `focus_next`, `focus_previous`, `close_modal`.

## Keys and mouse that always work

| Key / mouse | What it does |
|---|---|
| **Tab** / **Shift+Tab** | Move to the next / previous control |
| **Enter** or **Space** | Press the focused button, tick the checkbox, open the list |
| **↑ ↓ ← →** | Move inside lists, tables and trees; otherwise move between controls |
| **PageUp / PageDown / Home / End** | Scroll the screen (or the focused list) |
| **Ctrl+PageUp / Ctrl+PageDown** | Previous / next tab |
| **Esc** | Close the pop-up; otherwise leave the text box; otherwise quit |
| **q** | Quit (types a "q" when you're in a text box) |
| **Ctrl+C** | Always quits |
| Click | Focus and press |
| Mouse wheel | Scroll whatever is under the pointer |
| Drag a split line | Resize panes |

Hovering highlights what you can click.
