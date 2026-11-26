# Panes: a Blender-style workspace

Like the areas in Blender, every **pane** can show anything. Its header has a menu
to switch what it shows, and buttons to split or close it. People can arrange the
screen the way they like — and so can your code, or an AI agent.

`examples/08_workspace.py` is a ready-made workspace to try.

## Make panes

```python
from stagetui import StageApp, StageBar, StagePane, StageSplit


class Workspace(StageApp):
    def compose(self):
        return [
            StageSplit(
                [
                    StagePane("files"),  # the folder you started in
                    StagePane("markdown", text="# Notes\n\nAnything you like."),
                    StagePane("chat"),
                ],
                ratio=[1, 2, 2],
            ),
            StageBar("Click a pane's title to change it"),
        ]


Workspace().run()
```

The first argument is the **pane type**. Extra options go to that type:

| Type | Shows | Options |
|---|---|---|
| `"chat"` | A [chat](harness.md) | — |
| `"terminal"` | A real shell or a program | `command="htop"`, `cwd="src"` |
| `"files"` | The files in a folder | `root="."` |
| `"diff"` | Proposed changes | `old=`, `new=`, `path=` |
| `"code"` | Source code with colors | `path="app.py"` or `text=`, `language=` |
| `"markdown"` | A Markdown document | `text=` |
| `"log"` | A log | — |
| `"empty"` | A hint to pick something | — |

## The pane header

```
 ▣ Files ▾                                  ⊞  ⊟  ×
```

- **Click the title** (with the ▾) to open the menu: choose another type, or split / close.
- **⊞** splits the pane side by side, **⊟** top / bottom, **×** closes it.
- Drag the line between two panes to resize them.
- The focused pane's header is highlighted.

**Switching keeps things running.** When you switch a pane from *Terminal* to
*Notes*, the shell keeps running in the background; switch back and it's all there.

## From the keyboard

Press the **prefix key**, **Ctrl+Space**, then:

| Key | Does |
|---|---|
| **Space** or **Enter** | Open the pane menu (then ↑ ↓ Enter) |
| **\|** | Split side by side |
| **-** | Split top / bottom |
| **x** | Close the pane |
| **Tab** / arrows | Move to the next pane |
| **q** | Quit |

While the prefix is pressed, the status bar lists these keys. The prefix works
everywhere — even inside a terminal pane, which otherwise receives every key.
Change it with `PREFIX = "ctrl+a"` on your app.

## Control panes from code

```python
from stagetui import StageApp, StagePane, StageSplit


class Agent(StageApp):
    def compose(self):
        return [StageSplit([StagePane("chat", id="left"), StagePane("empty", id="right")])]

    def on_mount(self):
        right = self.query("#right")
        right.set_type("code", text="print('hello')", language="python")
        print(right.type)  # "code"
        print(self.panes())  # every pane: id, type, bounds, focused, stashed


Agent().run()
```

| | |
|---|---|
| `pane.set_type("diff", old=..., new=...)` | Show a new type (built fresh with these options) |
| `pane.switch("terminal")` | Show a type again, restoring what it had |
| `pane.type` | The type on show |
| `pane.content` | The widget inside, e.g. the `StageChat` of a `"chat"` pane |
| `pane.split(StagePane("log"))` / `pane.remove()` | Split / close, like every widget |
| `app.panes()` | All panes, for code and agents |

## Your own pane types

Register a function that returns a widget; it appears in every pane's menu:

```python
import stagetui as ts
from stagetui import StageApp, StageCard, StagePane


@ts.pane_type("weather", title="Weather", icon="sun")
def weather(city="Kuwait"):
    return StageCard(f"Sunny in {city}", title="Weather", icon="sun")


class Dashboard(StageApp):
    def compose(self):
        return [StagePane("weather", city="Riyadh")]


Dashboard().run()
```

## Good to know

- "Any application" means **terminal programs**: shells, editors (`vim`, `nano`),
  `htop`, `git`, Python, AI command-line tools. Windowed apps such as a browser
  will come later as optional plug-ins.
- Pictures that a program *inside* a terminal pane draws are not shown yet (text only).
- Saving and restoring whole layouts ("workspaces") and popping a pane out into
  its own window are *coming in Milestone 5*.
