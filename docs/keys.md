# Keys: rename them, turn them off

Every key that stagetui itself answers to — quitting, moving between widgets,
splitting panes — has a **name**, like `quit` or `close_pane`. With the name you
can give it other keys, or turn it off:

- **in your app**, with `KEYS = {...}` (for everyone who uses your app);
- **for yourself**, in a small `keys.json` file (for every stagetui app you use).

stagetui keeps this safe: there is always a key that quits, the prefix key never
goes away, and one key never does two things at once.

## The built-in keys

These keys work on their own:

| Name | Default keys | What it does |
|---|---|---|
| `quit` | `q`, `esc`, `ctrl+c` | Quit the app (Esc first closes a dialog or leaves the focused widget). After the prefix the same keys quit too, all but Esc |
| `prefix` | `ctrl+space` | Press it, then a pane key. It reaches the app even from inside a shell pane |
| `focus_next` | `tab` | Move to the next widget or pane (after the prefix: leave a shell pane) |
| `focus_previous` | `shift+tab` | Move to the previous widget or pane |
| `next_tab` | `ctrl+pagedown` | Show the next tab or workspace |
| `previous_tab` | `ctrl+pageup` | Show the previous tab or workspace |

These work **after the prefix**: press **Ctrl+Space**, let go, then the key:

| Name | Default keys | What it does |
|---|---|---|
| `pane_menu` | `space`, `enter` | Choose what the pane shows |
| `split_right` | `\|`, `\` | Split the pane: a new one on the right |
| `split_down` | `-` | Split the pane: a new one below |
| `close_pane` | `x` | Close the pane (closing the last one closes the app) |
| `pop_out` | `o` | Pop the pane out into its own window, or back |
| `detach` | `d` | Leave the app running in the background (stagetui sessions) |

After the prefix, the **arrow keys** always move between panes and **1**–**9**
always show a workspace; those can't be moved.

Keys are written the same way everywhere in stagetui: `"q"`, `"ctrl+q"`,
`"ctrl+shift+d"`, `"alt+x"`, `"f10"`, `"enter"`, `"esc"`, `"space"`, `"?"`. In
Python, the backslash key is written `"\\"`.

You don't need to remember this table: `stagetui keys` prints it (see
[below](#see-the-keys-stagetui-keys)).

## Rename keys in your app: `KEYS`

Put `KEYS` on your app, next to `BINDINGS`. Each entry is a key name, then:

- **one key** — `"quit": "ctrl+q"`,
- **several keys** — `"split_right": ["|", "v"]`,
- **`None`** to turn it off — `"pane_menu": None`.

A name you give replaces all of its default keys: with `"quit": "ctrl+q"`, the
keys **q**, **Esc** and **Ctrl+C** no longer quit, so your app can use them.

```python
from stagetui import StageApp, StageBar, StagePane, StageSplit


class Editor(StageApp):
    KEYS = {
        "quit": "ctrl+q",  # Ctrl+Q quits; q, Esc and Ctrl+C are free now
        "prefix": "ctrl+a",  # like tmux
        "close_pane": "w",  # Ctrl+A, then w, closes a pane
        "split_right": ["|", "v"],  # two keys for one job
        "pane_menu": None,  # turned off
    }

    def compose(self):
        return [
            StageSplit(
                [
                    StagePane("markdown", text="# Notes"),
                    StagePane("code", text="print('hi')", language="python"),
                ]
            ),
            StageBar(),  # the bar shows the keys in effect: "Ctrl+Q Quit"
        ]


Editor().run()
```

`PREFIX = "ctrl+a"` (from [Panes](panes.md#from-the-keyboard)) still works; it
is the same as `KEYS = {"prefix": "ctrl+a"}`.

Try your keys without opening a terminal — `drive()` presses them for you:

```python
from stagetui import StageApp, StagePane, StageSplit


class Editor(StageApp):
    KEYS = {"quit": "ctrl+q", "close_pane": "w"}

    def compose(self):
        return [
            StageSplit(
                [
                    StagePane("markdown", id="notes", text="# مَرْحَبًا Hello"),
                    StagePane("markdown", id="todo", text="- 日本語 नमस्ते"),
                ]
            )
        ]


with Editor().drive((80, 20)) as d:
    d.click("#notes")
    d.press("ctrl+space")  # the prefix…
    d.press("w")  # …then w: the pane closes
    assert d.query("#notes") is None
    keys = {k["name"]: k["keys"] for k in d.keys()["keys"]}
    print(keys["quit"])  # ['ctrl+q']
```

## Your shortcuts come first: `BINDINGS` and `KEYS`

They do different jobs:

- **`BINDINGS`** add *your own* shortcuts: `("ctrl+s", "save", "Save")` calls your
  `save` method (see [Making apps interactive](events.md#keyboard-shortcuts-with-bindings)).
- **`KEYS`** move *stagetui's* keys.

When a shortcut uses a key that stagetui also uses, **your shortcut wins**.
`("q", "save", "Save")` makes **q** save instead of quit. The app can still be
quit with the other quit keys (Esc, Ctrl+C), and the status bar shows one of
those instead of q. This works after the prefix too: a shortcut `("x", …)` runs
when someone presses Ctrl+Space, then x.

```python
from stagetui import StageApp, StageBar, StageCard


class Notes(StageApp):
    BINDINGS = [("q", "save", "Save")]  # q is this app's now

    def compose(self):
        return [StageCard("Press q to save", id="note"), StageBar()]

    def save(self):
        self.notify("Saved", icon="check")


with Notes().drive((80, 12)) as d:
    d.press("q")  # saves; the app keeps running
    bar = d.snapshot().text
    assert "Save" in bar and "Esc" in bar  # the bar shows the next quit key
```

The one key a shortcut can't take is the prefix key — it would never reach your
app. stagetui says so when the app starts:

```text
Ctrl+Space is the prefix key
  why: the shortcut for "save" could never be pressed
  fix: give it another key in BINDINGS, or move the prefix with KEYS = {"prefix": …}
```

## The safety rules

stagetui checks `KEYS` when your app starts. If something isn't safe, the app
doesn't start; you get a `StageError` that says what is wrong, why, and how to
fix it:

| Rule | What you see |
|---|---|
| There is always a way to quit | `Every app needs a way to quit` — why: `"quit" has no keys left` — fix: `give "quit" at least one key, like "ctrl+q" (CONFIRM_QUIT = True asks first)` |
| The prefix key can't be turned off | `The prefix key can't be turned off` — why: `it's how people reach the pane keys and leave a shell pane` — fix: `give "prefix" another key instead, like "ctrl+a"` |
| One key, one job | `q has two jobs` — why: `"quit" and "focus_next" both use it` — fix: `give one of them another key in KEYS` |
| One key, one job (after the prefix) | `1 has two jobs after the prefix` — why: `workspace 1 and "close_pane" both use it` |
| Only names that exist | `KEYS has no key called "quitt"` — why: `the built-in keys are: quit, prefix, focus_next, …` |
| Only real keys | `Unknown key "ctrl+nope"` — why: `keys are written like "ctrl+s", "ctrl+shift+d", "f5", "enter" or "?"` |

"One key, one job" counts the keys that work after the prefix as their own group,
and the quit and Tab keys work there too: `"close_pane": "q"` is refused
(`q has two jobs after the prefix`), because Ctrl+Space then q already quits.

```python
import stagetui as ts
from stagetui import StageApp, StageCard


class NoWayOut(StageApp):
    KEYS = {"quit": None}  # not allowed

    def compose(self):
        return [StageCard("Hello")]


try:
    NoWayOut().run()
except ts.StageError as e:
    print(e.what)  # Every app needs a way to quit
    print(e.fix)  # give "quit" at least one key, like "ctrl+q" (CONFIRM_QUIT = True asks first)
```

A few more things always hold:

- **Esc steps back first.** It closes a dialog, then leaves the focused widget,
  and only then quits.
- **In a text box**, **q** types a "q". Quit keys with **Ctrl** or **Alt** (like
  **Ctrl+C**, or your `ctrl+q`) still quit.
- **In a shell pane**, every key goes to the shell — even Ctrl+C — except the
  prefix key. Press the prefix, then **Tab**, to leave the shell.

## Ask before quitting: `CONFIRM_QUIT`

If quitting by accident would lose work, ask first:

```python
from stagetui import StageApp, StageCard


class Careful(StageApp):
    CONFIRM_QUIT = True

    def compose(self):
        return [StageCard("Unsaved work!")]


with Careful().drive((80, 20)) as d:
    d.press("q")
    assert "Quit this app?" in d.snapshot().text  # it asks
    d.press("esc")  # no: back to the app
    assert "Quit this app?" not in d.snapshot().text
```

The question comes for every quit key and for **×** on the last pane.
**Enter** or **y** quits, **Esc** or **n** goes back, and pressing **Ctrl+C**
again also quits. Your own code quitting with `self.exit()` doesn't ask. In a
window attached to a [session](sessions.md#leaving-detach-or-quit), the
*Detach / Quit* question comes instead.

## Your own keys for every app: `keys.json`

If you use stagetui apps (yours or other people's), you can move the keys for
all of them at once, in one file. It is plain JSON with the same names — `null`
turns a key off:

```json
{
  "quit": "ctrl+q",
  "close_pane": "w",
  "split_right": ["|", "v"],
  "pane_menu": null
}
```

Save it as `keys.json` in stagetui's settings folder:

| System | Where |
|---|---|
| Linux, macOS | `~/.config/stagetui/keys.json` (or `$XDG_CONFIG_HOME/stagetui/keys.json`) |
| Windows | `%APPDATA%\stagetui\keys.json` |
| Anywhere | Set `STAGETUI_KEYS` to the file's path, e.g. `STAGETUI_KEYS=~/my-keys.json` |

How it fits together:

- Your file comes **after** the app's `KEYS`, so your keys win.
- But a key the app uses for **its own shortcut** (`BINDINGS`) stays the app's:
  that entry of your file is skipped.
- **A mistake never stops an app.** An entry with a problem (a typo, a key with
  two jobs, `"quit": null`) is skipped, the rest still apply, and the app shows
  one short message: *1 of your keys was skipped — run `stagetui keys` to see
  why*.
- **Tests and agents** (`run_headless`, `drive`, `stagetui mcp`) don't read
  `keys.json`, so they behave the same on every computer. They do read
  `STAGETUI_KEYS` if you set it.

## See the keys: `stagetui keys`

```bash
stagetui keys               # the keys every app starts with here, and your keys file
stagetui keys my_app.py     # one app's keys, with its own shortcuts
stagetui keys --json        # the same, for programs
```

```text
Keys
  quit            q, Esc, Ctrl+C       Quit the app (Esc first closes a dialog or leaves the focused widget). …
  prefix          Ctrl+Space           Press it, then a pane key. It reaches the app even from inside a shell pane
  focus_next      Tab                  Move to the next widget or pane (after the prefix: leave a shell pane)
  …

After the prefix (Ctrl+Space, then…)
  pane_menu       Space, Enter         Choose what the pane shows
  split_right     |, \                 Split the pane: a new one on the right
  …
                  ← ↑ ↓ →              Move between panes (always)
                  1-9                  Show a workspace (always)

Your keys for every app: /home/you/.config/stagetui/keys.json (none yet)
```

Problems in your file are listed at the bottom, each starting with `!`.

From Python:

```python
import stagetui as ts

for key in ts.keys()["keys"]:  # the keys every app starts with
    print(key["name"], key["keys"])  # quit ['q', 'esc', 'ctrl+c'] …
```

Inside an app, `self.keys()` gives the same for that app, plus its shortcuts:
a dict with `keys` (each with `name`, `keys`, `labels`, `after_prefix`,
`about`), `shortcuts` (your `BINDINGS`), `confirm_quit`, `file` (your keys file)
and `problems`.

## For AI agents

An app may have moved its keys, so an agent should ask before pressing them:

- `stagetui mcp my_app.py` has a **`keys`** tool;
- `stagetui drive my_app.py` has a **`keys`** method;
- in Python, `d.keys()` on a [driver](agents.md#in-python-appdrive).

Each gives the dict above. See [AI agents](agents.md).

## Keys inside add-on panes

Add-ons — video, web, 3D, or your own — have keys of their own (Space to pause
a video, W for the wires of a 3D model…). Every add-on's keys can be renamed or
turned off too, per pane: `StagePane("video", keys={...})`. See
[Add-ons → Change an add-on's keys](plugins.md#change-an-add-ons-keys).
