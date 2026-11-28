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
 ▣ Files ▾                               ⊞  ⊟  ↗  ×
```

- **Click the title** (with the ▾) to open the menu: choose another type, or split / close.
- **⊞** splits the pane side by side, **⊟** top / bottom, **↗** opens it in its own
  window (see below), **×** closes it.
- Drag the line between two panes to resize them.
- The focused pane's header is highlighted.

**Closing a pane stops what runs in it** — a shell, or an add-on such as a video —
right away, instead of when the app quits.

**Closing the last pane closes the app**, the same as pressing **q**:

- in a window [attached to a running session](sessions.md#leaving-detach-or-quit),
  stagetui first asks whether to *Detach* (the app keeps running) or *Quit* it;
- in a pane's [own window](#open-a-pane-in-its-own-window), closing it puts the
  pane back where it came from instead.

**Switching keeps things running.** When you switch a pane from *Terminal* to
*Notes*, the shell keeps running in the background; switch back and it's all there.
Add-ons (video, web, 3D) rest while you can't see them: a video pauses, and plays
on when you switch back (see [Add-ons](plugins.md#out-of-sight-an-add-on-rests)).

## From the keyboard

Press the **prefix key**, **Ctrl+Space**, then:

| Key | Does |
|---|---|
| **Space** or **Enter** | Open the pane menu (then ↑ ↓ Enter) |
| **\|** | Split side by side |
| **-** | Split top / bottom |
| **x** | Close the pane (the last one: close the app, like **×**) |
| **Tab** / arrows | Move to the next pane |
| **o** | Open the pane in its own window (again: bring it back) |
| **1**-**9** | Switch workspace (with [workspace tabs](#workspaces-whole-layouts-like-blenders-tabs)) |
| **d** | Detach (see [Sessions](sessions.md)) |
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

## Workspaces: whole layouts, like Blender's tabs

Blender has *Layout*, *Modeling*, *Shading*… screens at the top. Each is a whole
arrangement of areas. `StageWorkspaces` gives your app the same thing:

```python
from stagetui import StageApp, StagePane, StageSplit, StageWorkspaces


class Studio(StageApp):
    def compose(self):
        return [
            StageWorkspaces(
                {
                    "Code": StageSplit([StagePane("files"), StagePane("code", text="print(1)")]),
                    "Agent": StageSplit([StagePane("chat"), StagePane("terminal")]),
                }
            )
        ]


Studio().run()
```

- The tabs across the top switch workspaces: click them, or press
  **Ctrl+Space** then **1**, **2**, … (Ctrl+PageUp / Ctrl+PageDown work too).
- Every workspace keeps running while another is on screen — a shell in *Agent*
  keeps going while you read code in *Code*.
- `StageWorkspaces(["Code", "Review"])` (just names) starts each with an empty pane.

## Save a layout, bring it back

People rearrange panes the way they like. Save it, and restore it next time:

```python
from stagetui import StageApp, StageError, StagePane, StageSplit


class Desk(StageApp):
    BINDINGS = [("ctrl+s", "save", "Save layout")]

    def compose(self):
        return [StageSplit([StagePane("files", id="files"), StagePane("chat", id="chat")])]

    def on_mount(self):
        try:
            self.load_workspace()  # the layout saved last time, if any
        except StageError:
            pass  # nothing saved yet

    def save(self):
        self.save_workspace()
        self.notify("Layout saved")


Desk().run()
```

| | |
|---|---|
| `app.save_workspace(name=None)` | Save the layout. The name is the open workspace tab (or `"default"`). Returns the file |
| `app.load_workspace(name=None)` | Bring a saved layout back |
| `app.workspaces()` | The names you saved |
| `app.workspace()` | The current layout as a dict |
| `app.load_workspace({...})` | Arrange the panes like a dict you give |

Layouts are saved in `~/.config/stagetui/workspaces/<YourApp>.json`
(on Windows in `%APPDATA%\stagetui\workspaces`). Pass `path=` to use another file.

A layout is plain JSON, easy to read and to write by hand:

```json
{"split": "horizontal", "ratio": [0.3, 0.7], "children": [
    {"pane": "files", "id": "files"},
    {"pane": "terminal", "id": "shell", "args": {"command": "htop"}}
]}
```

**Loading never throws work away.** A pane whose `id` is still there is moved,
not rebuilt: its shell keeps running, its chat keeps its messages. Panes that are
not in the layout are closed; new ones are created with their `args`.

## Open a pane in its own window

Want the shell on your second monitor, or the chat big while you code? Any pane
can **pop out** into a terminal window of its own:

- click **↗** in the pane's header (or choose *Open in its own window* in its menu),
- or press **Ctrl+Space**, then **o**.

A new window of the terminal you are using opens (Kitty opens Kitty, foot opens
foot, Windows Terminal opens a new tab-window…) and shows just that pane. Its old
spot says where it went:

```
 ▣ Files ▾                               ⊞  ⊟  ↙  ×
                                      
  ▣ Files is open in its own window
                                      
   ↙ Bring it back
```

**Everything keeps running** — a shell keeps its history, a chat keeps streaming.
To bring the pane back, **close its window**, click **↙ Bring it back** (or
**↙ Put back** at the top of the window), or press **Ctrl+Space** then **o** again.

From code, every widget can do it for the pane it lives in:

```python
from stagetui import StageApp, StagePane, StageSplit


class Desk(StageApp):
    BINDINGS = [("ctrl+shift+d", "pop_out", "Pop out")]  # one key for it

    def compose(self):
        return [StageSplit([StagePane("chat", id="chat"), StagePane("terminal", id="shell")])]

    def on_mount(self):
        shell = self.query("#shell")
        shell.detach()  # open the shell in its own window
        print(shell.detached)  # True
        shell.attach()  # and back


Desk().run()
```

The window simply runs `stagetui attach <app>:<pane>` — you can run that yourself
in any terminal, too (see [Sessions](sessions.md)).

| Where you are | What opens |
|---|---|
| Kitty, Ghostty, WezTerm, foot, Alacritty, Konsole, GNOME Terminal, xterm | A new window of the same terminal |
| Windows Terminal | A new Windows Terminal window |
| Terminal.app, iTerm2 (macOS) | A new window, through AppleScript |
| Inside tmux | A new tmux window |
| Over SSH | Nothing can open on your computer: a message shows the command to run |

To choose the terminal yourself, set `LAUNCHER = "alacritty -e {cmd}"` on your
app, or the `STAGETUI_LAUNCHER` environment variable (`{cmd}` is where the
command goes; without it, the command goes at the end).

## Good to know

- "Any application" means **terminal programs**: shells, editors (`vim`, `nano`),
  `htop`, `git`, Python, AI command-line tools. Windowed apps such as a browser
  will come later as optional plug-ins.
- Pictures that a program *inside* a terminal pane draws are not shown yet (text only).
- Apps can keep running in the background and be opened again from any
  terminal: see [Sessions](sessions.md).
