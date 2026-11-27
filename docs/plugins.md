# Add-ons: video, web, 3D — and your own

Some pane types are too big to live inside stagetui itself: a video player, a
web browser, a 3D viewer. They are **add-ons**: separate programs you install
only if you want them. Once installed, they appear in every pane's menu, like
*Chat* or *Terminal*.

| Add-on | What it adds | Status |
|---|---|---|
| **Video** | Play video files and links in a pane, with sound | `pip install stagetui-video` ([below](#video)) |
| **Web** | A real web page in a pane (uses your Chrome / Chromium) | `pip install stagetui-web` ([below](#web)) |
| **3D** | Look at 3D models (glTF, OBJ, STL) and turn them with the mouse | `pip install stagetui-3d` ([below](#3d)) |

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

## Video

```python
from stagetui import StageApp, StagePane


class Cinema(StageApp):
    def compose(self):
        return [StagePane("video", id="movie", path="trip.mp4")]


Cinema().run()
```

`examples/11_video.py` is a ready-made player (it even makes a test clip).

| Option | Meaning |
|---|---|
| `path="movie.mp4"` | A file — or a link (`url="https://…/clip.mp4"`) |
| `autoplay=False` | Wait for Space (default: plays at once) |
| `loop=True` | Start over at the end |
| `volume=0.5` | 0 to 1 |
| `muted=True` | No sound |
| `start=30` | Begin at 30 seconds |

**Keys** (with the pane focused — click it): **Space** pause · **← →** seek 5 s
(**Shift** 30 s) · **↑ ↓** volume · **M** mute · **Home** start over. Clicking the
picture pauses; the mouse wheel changes the volume. The line at the bottom shows
where you are: `▶ 01:23 / 04:56 · volume 80%`.

From code, the pane's `content` does it too:

<!-- not-tested -->
```python
video = self.query("#movie").content
video.call("pause")              # also: "play", "toggle"
video.call("seek", 90)           # seconds
video.call("volume", 0.5)        # 0 to 1
video.call("mute", True)
video.call("state")              # {"position", "duration", "paused", "volume", …}
StagePane("video", path="a.mp4", on_ended=self.next_clip, on_state=self.changed)
```

The picture and the sound are decoded by **ffmpeg**, which comes with the add-on
(through the `imageio-ffmpeg` package) — nothing else to install. On Linux the sound
plays through your system's ALSA library (PipeWire and PulseAudio included).

## Web

```python
from stagetui import StageApp, StagePane


class Browse(StageApp):
    def compose(self):
        return [StagePane("web", id="page", url="https://example.com")]


Browse().run()
```

`examples/12_web.py` is a ready-made browser pane.

It is a **real page**: the Chrome, Chromium, Edge or Brave already on your
computer draws it (hidden), and stagetui shows it — links, forms, CSS and
JavaScript all work. Nothing is downloaded.

| Option | Meaning |
|---|---|
| `url="https://…"` | A web address — or `path="page.html"` for a file |
| `zoom=1.5` | Bigger (or smaller, below 1) |

Click the page, then **type** — in any language. **Ctrl+L** types an address at
the bottom (Enter goes there), **Alt+← / Alt+→** go back and forward, **F5**
reloads. The mouse wheel scrolls.

From code:

<!-- not-tested -->
```python
page = self.query("#page").content
page.call("navigate", "github.com")
page.call("back")                      # also "forward", "reload"
title = page.call("evaluate", "document.title")   # run JavaScript, get the result
page.call("state")                     # {"url", "title", "loading", "zoom"}
StagePane("web", url="…", on_title=self.titled, on_navigated=self.went, on_loaded=self.ready)
```

## 3D

```python
from stagetui import StageApp, StagePane


class Look(StageApp):
    def compose(self):
        return [StagePane("3d", id="model", path="robot.glb")]


Look().run()
```

`examples/13_3d.py` is a ready-made viewer (with no file, it makes a colorful knot).

It reads the common 3D files: **glTF** (`.glb`, or `.gltf` with its `.bin` and
pictures next to it), **OBJ** (with its `.mtl` colors and pictures) and **STL**
(3D printing). Colors, pictures on the surface and every part of the model are
shown. It is drawn by your computer's processor, on all its cores — no graphics
card or driver needed.

| Option | Meaning |
|---|---|
| `path="model.glb"` | The model to show |
| `spin=False` | Hold still (default: it turns slowly until you touch it) |
| `wireframe=True` | Show the triangles' edges |
| `grid=False` | No floor grid |
| `up="z"` | Which way is up in the file (STL files are usually `"z"`; others `"y"`) |

**Mouse:** drag to turn it, right-drag to move it, the wheel zooms. **Keys** (click
the model first): **← → ↑ ↓** turn · **+ −** zoom · **Space** spin or stop · **W**
wires · **G** grid · **F** fit it in the pane · **R** start over.

From code:

<!-- not-tested -->
```python
model = self.query("#model").content
model.call("load", "other.stl")           # show another model
model.call("view", {"yaw": 90, "pitch": 20, "distance": 4})   # degrees; distance 3 ≈ fits
model.call("spin", False)                 # also "wireframe", "grid" (True/False)
model.call("reset")
model.call("save", "view.png")            # what it shows, as a picture
model.call("state")                       # {"triangles", "yaw", "pitch", "distance", "spin", …}
StagePane("3d", path="a.glb", on_loaded=self.loaded)   # {"path", "triangles", "format"}
```

Compressed glTF files (Draco, meshopt) aren't read yet: the pane says so — export
the model without compression.

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
