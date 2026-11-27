# Add-ons: video, web, 3D — and your own

Some pane types are too big to live inside stagetui itself: a video player, a
web browser, a 3D viewer. They are **add-ons**: separate programs you install
only if you want them. Once installed, they appear in every pane's menu, like
*Chat* or *Terminal*.

| Add-on | What it adds | Status |
|---|---|---|
| **Video** | Play video files and links in a pane, with sound | coming soon |
| **Web** | A real web page in a pane (uses your Chrome / Chromium) | coming soon |
| **3D** | Look at 3D models and turn them with the mouse | coming soon |

`examples/10_plugins.py` shows a tiny add-on in action.

## Using an add-on

Put it in a pane like any other type:

```python
from stagetui import StageApp, StagePane, StageSplit


class Cinema(StageApp):
    def compose(self):
        return [StageSplit([StagePane("video", path="trip.mp4"), StagePane("chat")])]


Cinema().run()
```

The options (`path=…`) go to the add-on. If it isn't installed, the pane says so —
**press Enter in it (or pick it in a pane's menu) and stagetui asks:**

```
╭─ Install Video? ───────────────────────────────────────╮
│                                                        │
│  Video needs the stagetui-video add-on.                │
│  It goes into this app's Python. No password needed.   │
│                                                        │
│              Install         Not now                   │
╰────────────────────────────────────────────────────────╯
```

Say yes and it installs right there (you can watch it in the pane), then the
pane turns into the add-on. Or install ahead of time:

```bash
stagetui plugins                  # what is installed, what you can add
stagetui plugins install video    # install one
```

## Real pictures

Video, web pages and 3D are pictures, so they look best in a terminal that can
show real pictures: **Ghostty, Kitty, WezTerm or foot** (Linux), **Ghostty,
Kitty, WezTerm or iTerm2** (macOS), **WezTerm or Windows Terminal** (Windows).
Other terminals draw pictures with blocks.

stagetui handles this for you. When a picture add-on appears in a terminal that
draws blocks:

- if you have one of those terminals, the pane **opens there by itself** (see
  [Panes → Open a pane in its own window](panes.md#open-a-pane-in-its-own-window));
  close that window and the pane comes back;
- if you don't, stagetui offers to install one (Ghostty or Kitty on Linux and
  macOS, WezTerm on Windows). The install runs in the pane — on Linux, type
  your password there when it asks.

You can keep your favorite terminal as your everyday one: only the picture
panes open elsewhere.

## In your code

```python
import stagetui as ts

for addon in ts.plugins():
    print(addon["name"], "installed" if addon["installed"] else "not installed")
```

A pane's add-on can do things on request and tell your app what happened:

<!-- not-tested -->
```python
video = self.query("#movie").content   # the add-on inside the pane
video.call("pause")                    # ask it to do something
StagePane("video", path="a.mp4", on_ended=self.next_clip)   # its events
```

## Write your own add-on

An add-on is a small program named `stagetui-plugin-<name>` that draws pictures.
stagetui starts it when a pane of its type opens, shows every frame it draws, and
sends it the keys and clicks. If it crashes, only its pane notices (press Enter
to start it again).

The easiest way is Python. Save this as `stagetui-plugin-pulse`:

<!-- not-tested -->
```python
#!/usr/bin/env python3
import math

from stagetui import Canvas
from stagetui.plugin import Plugin, serve


class Pulse(Plugin):
    name, title, icon = "pulse", "Pulse", "heart"
    keys = ("space",)          # keys it wants while its pane has focus

    def start(self, args):     # the pane opened; args are its options
        self.color = "#f7768e"

    def frame(self, t):        # draw (up to `fps` times a second)
        c = Canvas(self.width, self.height, "#16161e")
        r = min(self.width, self.height) / 3 * (1 + 0.15 * math.sin(t * 4))
        c.circle(self.width / 2, self.height / 2, r, fill=self.color)
        return c

    def key(self, key, text):  # "space"
        self.color = "#7aa2f7"
        self.event("color", self.color)   # → on_color in the app


serve(Pulse())
```

Then make it runnable and tell stagetui where it is:

```bash
chmod +x stagetui-plugin-pulse
STAGETUI_PLUGIN_PATH=. python my_app.py      # or put it anywhere on your PATH
```

`Pulse` now shows up in every pane's menu. What a plugin can do:

| Method | When |
|---|---|
| `start(args)` | The pane opened (raise an error to show a message instead) |
| `frame(t)` | Draw: return a `Canvas` (or raw RGBA bytes), or `None` if nothing changed |
| `resize(width, height)` | The pane changed size (`self.width`, `self.height` are pixels) |
| `key(key, text)` | A key from `keys` (`text = True` gets every key) |
| `mouse(kind, x, y, button)` | Clicks, drags, moves and the wheel, in pixels |
| `call(method, args)` | The app called `pane.content.call(...)` |
| `self.status(text)` | Show a line at the bottom of the pane |
| `self.event(name, value)` | Tell the app (`on_<name>` handlers) |

For speed (video, 3D), write it in Rust with the `stagetui-plugin-api` crate: the
same idea — implement `PaneSource` and call `serve(...)`. Under the hood both
speak a tiny protocol over stdin/stdout: JSON messages and raw RGBA frames.

## Good to know

| Setting | Effect |
|---|---|
| `STAGETUI_PLUGIN_PATH=dir1:dir2` | Extra folders to look for add-ons in |
| `STAGETUI_NO_PLUGINS=1` | Don't add add-ons to pane menus |

- Add-ons are looked for next to your Python (where `pip install` puts them),
  in `STAGETUI_PLUGIN_PATH`, and on your `PATH`.
- `ts.plugins(refresh=True)` looks again after installing one while your
  program runs.
