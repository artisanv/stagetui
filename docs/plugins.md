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

All three are ready to install. You can also ask for them together with stagetui
itself — the name in the square brackets is the add-on (the quotes keep your shell
from reading the brackets):

```bash
pip install "stagetui[3d]"          # stagetui and the 3D add-on
pip install "stagetui[video,web]"   # stagetui, video and web
```

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

### Change an add-on's keys

Each add-on's keys have **names** — the tables below list them (`wires`, `spin`,
`mute`…). Give `keys=` to the pane to use other keys, or to turn some off:

```python
from stagetui import StageApp, StagePane


class Look(StageApp):
    def compose(self):
        return [
            StagePane(
                "3d",
                path="robot.glb",
                keys={
                    "wires": "x",              # x shows the edges now (w does nothing)
                    "grid": None,              # no key turns the grid on or off
                    "spin": ["s", "space"],    # two keys for one thing
                },
            )
        ]


Look().run()
```

- A name you don't list keeps its usual keys.
- Keys are written like everywhere else in stagetui: `"x"`, `"space"`,
  `"ctrl+s"`, `"shift+left"`, `"f5"`, `"+"` (see
  [Keyboard shortcuts](events.md#keyboard-shortcuts-with-bindings)). As there, a
  capital letter means Shift: `"X"` is Shift+X, `"x"` is the plain key.
- The line at the bottom of the pane shows your keys (`x wires`), and leaves out
  the ones you turned off.
- A key the add-on no longer uses goes to your app instead (your `BINDINGS`).

If something is wrong, the pane says what instead of starting — for example
`keys= names "wire", but there is no action by that name; the actions are:
turn_left, turn_right, …`, or `"g" is given to both "wires" and "grid"` when two
names would share a key (give the other one a new key, or `None`). Fix it and
press Enter in the pane.

### No file yet? The pane asks for one

Pick **Video** or **3D** from a pane's menu and there is no file to show yet. The
pane says so at the bottom — `No model yet — press Enter to type a file path` —
and you can carry on right there:

1. Click the pane, then press **Enter** (or the **o** key). A line opens at the bottom,
   like a web browser's address bar: `Open a model: █`.
2. Type the file's path. **Tab** finishes a file name for you, `~` is your home
   folder (`~/Downloads/robot.glb`), **Backspace** rubs out, **Ctrl+U** clears the
   line. You can also paste a path, or drag a file from your file manager onto the
   terminal.
3. Press **Enter**. The file opens. If it can't — not found, or not a file the
   add-on reads — the line says why and stays open, so you can fix the path.
   **Esc** closes the line.

The same happens when the file your app gave isn't there: the pane says what is
wrong, and **Enter** opens the line with that path already typed, ready to fix.
With a model or video already showing, **Enter** opens the line in its folder, to
pick another one.

### Out of sight, an add-on rests

An add-on works only while you can see its pane. Switch the pane to another type
(say, from *Video* to *Notes*) or go to another tab, and stagetui tells the add-on
it is hidden: a video pauses its sound and picture, a web page rests, and 3D
stops drawing — so nothing plays where you can't hear it, and your computer
doesn't work for nothing. Come back and it carries on (a video that was playing
plays on).

A resting web page is frozen: no pictures, no scripts or timers, and no sound —
not its videos, not sound made only in script (`new Audio()`), not its Web Audio,
not the frames of other sites inside it. When you come back, what was playing
plays on. [Web → Out of sight](#out-of-sight) has the details.

**Closing** the pane — **×** in its header, or **Ctrl+Space** then **x** — stops
the add-on's program right away. See
[Panes → The pane header](panes.md#the-pane-header) for what happens when it is
the last pane.

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

**Keys** (with the pane focused — click it). Rename them or turn them off with
`keys={...}` ([how](#change-an-add-ons-keys)):

| Name | Keys (as you write them) | What it does |
|---|---|---|
| `open` | `enter` or `o` | Type a video's path or link at the bottom, then Enter plays it ([how](#no-file-yet-the-pane-asks-for-one)) |
| `play_pause` | `space` | Pause, or play on |
| `back` | `left` (←) | Back 5 seconds |
| `forward` | `right` (→) | Forward 5 seconds |
| `back_more` | `shift+left` | Back 30 seconds |
| `forward_more` | `shift+right` | Forward 30 seconds |
| `louder` | `up` (↑) | Volume up |
| `quieter` | `down` (↓) | Volume down |
| `mute` | `m` | Sound off, or on again |
| `restart` | `home` | Start over from the beginning |

For example `StagePane("video", path="trip.mp4", keys={"play_pause": "k", "back": "j",
"forward": "l"})`. Clicking the picture pauses; the mouse wheel changes the volume.
The line at the bottom shows where you are, and what Space does now (`‖` pause,
`▶` play): `▶ 01:23 / 04:56 · volume 80% · space ‖ · ← → seek · …`.

From code, the pane's `content` does it too:

<!-- not-tested -->
```python
video = self.query("#movie").content
video.call("pause")              # also: "play", "toggle"
video.call("seek", 90)           # seconds
video.call("volume", 0.5)        # 0 to 1
video.call("mute", True)
video.call("open", "other.mp4")  # play another file (or a link)
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

Click the page, then **type** — in any language. Keys reach the page as real key
presses: **Tab** moves to the next field (**Shift+Tab** back), **Esc** reaches the
page, and the page's own shortcuts work. To leave the pane, press **Ctrl+Space**
then **Tab**. The mouse wheel scrolls.

A link or form that opens a **new window** (`target="_blank"`, `window.open` in
the page — in a frame of another site too) opens **in the pane** instead: a hidden
browser has no other window you could see. A form keeps what it sends.
(**Alt+←** goes back to where you were.) The browser holds the pane's page and
nothing else.

**Keys** of the pane itself. Rename them or turn them off with `keys={...}`
([how](#change-an-add-ons-keys)):

| Name | Keys | What it does |
|---|---|---|
| `address` | Ctrl+L | Type an address at the bottom: Enter goes there, Esc (or Ctrl+L again) never minds |
| `back` | Alt+← | Back to the page before |
| `forward` | Alt+→ | Forward again |
| `reload` | F5 or Ctrl+R | Load the page again |

For example `StagePane("web", url="https://example.com", keys={"address": "ctrl+o",
"reload": None})`. A key you rename away, or turn off, is the page's: here Ctrl+L
and F5 go to the page.

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

### Out of sight

When its pane is out of sight (another pane type, another tab), the page **rests**:

- It is frozen — nothing in it runs, and it makes **no sound**: the sound stops
  within about 0.1–0.2 seconds of the pane going out of sight. What was playing
  (videos, audio, Web Audio, in every frame) is paused, and plays on when you come
  back. Something the page or your code asks to play meanwhile (`play()`,
  `resume()` on an AudioContext, a new AudioContext) waits, and starts once the
  pane is shown.
- `navigate`, `back`, `forward` and `reload` **wait**: the page goes there when
  the pane is shown again, and `on_navigated`, `on_title` and `on_loaded` come
  then. `call("state")` shows the page it is resting on until then.
- `evaluate` still works for plain JavaScript and answers at once. What it waits
  for (a timer, a download, a promise) only finishes once the pane is shown; if
  the answer doesn't come within 9 seconds, the call fails with *"the page didn't
  answer: it rests while its pane is hidden, and what it waits for finishes once
  it's shown"*.
- A page stuck in an endless script can't be told to rest. Then the line at the
  bottom of the pane says *"This page isn't responding — it may keep playing;
  close the pane to stop it"*. Closing the pane always stops it.

Two small limits remain. A sound that plays by itself (`autoplay`) from an
element that was never put on the page is quiet while hidden but stays paused when
you come back — press play again. And a form inside a frame from **another site**
that posts to a new window opens in the pane without what it sent (that frame may
not be allowed to change the whole page, so the browser's new window is followed
instead).

## 3D

```python
from stagetui import StageApp, StagePane


class Look(StageApp):
    def compose(self):
        return [StagePane("3d", id="model", path="robot.glb")]


Look().run()
```

`examples/13_3d.py` is a ready-made viewer (with no file, it makes a colorful knot).

Install it with `pip install stagetui-3d` (or `pip install "stagetui[3d]"`) — or
just open a 3D pane and say yes when stagetui asks.

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

**Mouse:** drag to turn it, right-drag to move it, the wheel zooms.

**Keys** (click the model first). Rename them or turn them off with `keys={...}`
([how](#change-an-add-ons-keys)):

| Name | Keys (as you write them) | What it does |
|---|---|---|
| `open` | `enter` or `o` | Type a model's path at the bottom, then Enter opens it ([how](#no-file-yet-the-pane-asks-for-one)) |
| `turn_left` | `left` (←) | Turn the model to the left |
| `turn_right` | `right` (→) | Turn the model to the right |
| `turn_up` | `up` (↑) | Look from higher up |
| `turn_down` | `down` (↓) | Look from lower down |
| `zoom_in` | `+` or `=` | Come closer |
| `zoom_out` | `-` | Move away |
| `spin` | `space` | Spin slowly, or stop |
| `reset` | `r` or `home` | Start over: the first angle, the whole model in view |
| `wires` | `w` | Show the triangles' edges, or hide them |
| `grid` | `g` | Show the floor grid, or hide it |
| `fit` | `f` | Fit the model in the pane, keeping the angle |

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

`Pulse` now shows up in every pane's menu.

#### Keys the app can rename

Instead of plain `keys`, give your keys **names** in `actions`. Then an app can
rename them or turn them off, exactly like the add-ons above
(`StagePane("pulse", keys={"bigger": "b", "smaller": None})`):

<!-- not-tested -->
```python
class Pulse(Plugin):
    name, title, icon = "pulse", "Pulse", "heart"
    actions = {
        "bigger": ("+", "="),      # name: its usual keys
        "smaller": ("-",),
    }

    def start(self, args):
        self.size = 1.0
        # self.keymap knows the app's keys: show them at the bottom of the pane.
        hints = [f"{self.keymap.label(a)} {a}" for a in self.actions if self.keymap.label(a)]
        self.status(" · ".join(hints))         # e.g. "+ bigger · - smaller"

    def action(self, name):        # "bigger" or "smaller" — whatever key the app chose
        self.size *= 1.2 if name == "bigger" else 1 / 1.2

    # frame() draws, as in the first Pulse (a circle self.size times as big).
```

- `action(name)` gets the keys of `actions`; `key(key, text)` still gets the
  plain `keys` (and every other key with `text = True`).
- stagetui sends your plugin only the keys it asked for, so a key the app moved
  away goes back to the app.
- If the app names an action you don't have, or gives one key to two actions,
  your pane shows what is wrong (you don't need to check).
- `self.keymap.keys("bigger")` is the list of keys, `self.keymap.label("bigger")`
  the first one, short for a status line (arrows as `← → ↑ ↓`), or `None` when it
  is turned off. `self.keymap.spoken("bigger")` writes it for a sentence ("press
  Enter to …"): `Enter`, `Ctrl+O`, `Shift+←`.

#### Asking for every key for a while

A plugin that opens a text line (like the path line above) needs every key while
you type, then its usual keys again. Say so with `self.ready(...)` at any time:

<!-- not-tested -->
```python
def action(self, name):
    if name == "open":
        self.typing = ""
        self.ready(text=True, keys=("esc", "tab"))   # every key; Esc and Tab too

def key(self, key, text):
    if key == "esc":
        self.ready(text=False, keys=())              # back to the usual keys
    elif text:
        self.typing += text
```

What a plugin can do:

| Method | When |
|---|---|
| `start(args)` | The pane opened (raise an error to show a message instead) |
| `frame(t)` | Draw: return a `Canvas` (or raw RGBA bytes), or `None` if nothing changed |
| `resize(width, height)` | The pane changed size (`self.width`, `self.height` are pixels) |
| `key(key, text)` | A key from `keys` (`text = True` gets every key) |
| `action(name)` | A key of one of `actions` (whichever key the app gave it) |
| `mouse(kind, x, y, button)` | Clicks, drags, moves and the wheel, in pixels |
| `paste(text)` | Text was pasted into the pane (or a file dragged in) |
| `call(method, args)` | The app called `pane.content.call(...)` |
| `shown(shown)` | The pane went out of sight (`False`) or came back (`True`); `frame` isn't called while hidden |
| `self.status(text)` | Show a line at the bottom of the pane |
| `self.event(name, value)` | Tell the app (`on_<name>` handlers) |
| `self.ready(text=…, keys=…, fps=…)` | Change what it wants while it runs (every key while typing) |

### In Rust

For speed (video, web, 3D), write it in Rust with the `stagetui-plugin-api` crate:
the same idea — implement `PaneSource` and call `serve(...)`. Build it as a program
named `stagetui-plugin-<name>`.

```rust
use stagetui_plugin_api::{Describe, Frame, PaneSource, Ready, Start, serve};
use std::time::Duration;

struct Gradient { w: u32, h: u32 }

impl PaneSource for Gradient {
    fn describe(&self) -> Describe {
        Describe::new("gradient", "Gradient", "sparkles")
    }
    fn start(&mut self, s: &Start) -> Result<Ready, String> {
        (self.w, self.h) = (s.width, s.height);
        Ok(Ready::default())
    }
    fn resize(&mut self, width: u32, height: u32) {
        (self.w, self.h) = (width, height);
    }
    fn frame(&mut self, now: Duration) -> Option<Frame> {
        let shift = (now.as_secs_f32() * 60.0) as u32;
        Some(Frame::from_fn(self.w, self.h, |x, _| [((x + shift) % 256) as u8, 80, 200, 255]))
    }
    fn shown(&mut self, shown: bool) {
        // false: out of sight, and `frame` isn't called until it's back.
        // Pause sound or other work here (true: carry on).
        let _ = shown;
    }
}

fn main() {
    std::process::exit(serve(Gradient { w: 1, h: 1 }));
}
```

`describe`, `start` and `frame` are required; the others do nothing unless you
write them:

| Method | When |
|---|---|
| `describe()` | Its name, title and icon (for `--describe` and the pane menu) |
| `start(&Start)` | The pane opened: its size and options (`args`); return `Ready` (or an error message) |
| `frame(now)` | Draw: return a `Frame` when the picture changed, or `None` |
| `resize(width, height)` | The pane changed size (pixels) |
| `key(key, text)` / `mouse(&Mouse)` / `paste(text)` | Keys, clicks and pasted text |
| `focus(bool)` | The pane got or lost the keyboard |
| `shown(bool)` | The pane went **out of sight** (`false`: its type was switched, or it is in a tab you aren't looking at) or came back (`true`). `frame` isn't called while it's hidden — pause sound and other work here, like the video, web and 3D add-ons do |
| `call(method, args)` | The app called `pane.content.call(...)` |
| `status()` | The line at the bottom of the pane (`None`: unchanged) |
| `ready()` | What it wants now, if that changed (`None`: unchanged) — e.g. every key while a text line is open, its usual keys after |

**Keys the app can rename**, in Rust: list your actions once, read the app's
`keys=` in `start` with `Keymap::new`, and ask it which action a key does:

```rust
use stagetui_plugin_api::{KeyAction, Keymap, Ready, Start};

const ACTIONS: &[KeyAction] = &[
    KeyAction { name: "bigger", keys: &["+", "="], about: "grow" },
    KeyAction { name: "smaller", keys: &["-"], about: "shrink" },
];

// In `start`: a mistake in the app's keys= becomes the pane's message (the `?`).
fn start(&mut self, s: &Start) -> Result<Ready, String> {
    self.keys = Keymap::new(ACTIONS, &s.args)?;
    Ok(Ready { keys: self.keys.all_keys(), ..Ready::default() })
}

fn key(&mut self, key: &str, _text: Option<&str>) -> bool {
    match self.keys.action(key) {
        Some("bigger") => self.size *= 1.2,
        Some("smaller") => self.size /= 1.2,
        _ => return false,
    }
    true
}
```

`self.keys.label("bigger")` is the first key for your status line (`None` when the
app turned it off), `self.keys.spoken("bigger")` the same for a sentence
(`Enter`, `Ctrl+O`); `self.keys.keys("bigger")` all of them.

**A line for typing a path**, like the video and 3D add-ons have: `PathLine` does
the typing, Tab completion, `~`, pasting and quotes; you open the file. While it is
open, ask for every key with the `ready()` hook:

```rust
use stagetui_plugin_api::{PathLine, Ready, Typed};

// In `key`: an `open` action opens the line; while it's open, it gets every key.
fn key(&mut self, key: &str, text: Option<&str>) -> bool {
    if self.line.is_open() {
        if let Typed::Open(path) = self.line.key(key, text) {
            match self.load(&path) {
                Ok(()) => self.line.close(),
                Err(why) => self.line.fail(why),   // shown on the line, which stays open
            }
        }
        return true;
    }
    if self.keys.action(key) == Some("open") {
        self.line.open("");
    }
    true
}

// What stagetui should send now; `serve` asks after every message.
fn ready(&mut self) -> Option<Ready> {
    let want = if self.line.is_open() { self.line.ready(30) } else { self.usual() };
    (self.sent != want).then(|| { self.sent = want.clone(); want })
}

// In `status`: `self.line.status("Open a file")` while it is open.
```

`Frame::placeholder(width, height, &theme)` is a quiet picture for a pane with
nothing to show yet (send it once, so the pane doesn't say "Starting…").

Under the hood both Python and Rust add-ons speak a tiny protocol over
stdin/stdout: JSON messages and raw RGBA frames. stagetui sends `{"t": "start", …}`,
`{"t": "key", …}`, `{"t": "shown", "shown": false}` and so on; an add-on that doesn't
know a message simply ignores it, so older add-ons keep working.

## Good to know

| Setting | Effect |
|---|---|
| `STAGETUI_PLUGIN_PATH=dir1:dir2` | Extra folders to look for add-ons in |
| `STAGETUI_NO_PLUGINS=1` | Don't add add-ons to pane menus |

- Add-ons are looked for next to your Python (where `pip install` puts them),
  in `STAGETUI_PLUGIN_PATH`, and on your `PATH`.
- `ts.plugins(refresh=True)` looks again after installing one while your
  program runs.
