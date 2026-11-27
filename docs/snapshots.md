# Snapshots — for tests, CI and AI agents

A **snapshot** is a picture of your app taken *without* a terminal. stagetui
draws everything offscreen and gives you three views of the result:

| View | What it is | Best for |
|---|---|---|
| `snap.text` | The screen as plain text (Arabic in normal reading order) | Tests, and AI agents reading the screen |
| `snap.tree` | Every widget as a dictionary: id, type, position, text… | Checking details precisely |
| `snap.png` | The screen as a PNG picture | Humans, visual checks, AI agents with vision |

## Take a snapshot

```python
from stagetui import StageApp, StageCard


class Status(StageApp):
    def compose(self):
        return [StageCard("All systems normal.", title="Status", id="status")]


snap = Status().run_headless(size=(60, 12))  # 60 columns × 12 rows

print(snap.text)
print(snap.query("#status")["title"])  # "Status"
snap.save("status.png")  # also: .txt and .json
```

`size` is in cells (characters). `cell=(16, 32)` (the default) sets how many pixels one cell
is in the PNG; bigger numbers make bigger pictures.

## What's in the tree

```python
from stagetui import StageApp, StageCard


class Status(StageApp):
    def compose(self):
        return [StageCard("All systems normal.", title="Status", id="status")]


card = Status().run_headless((60, 12)).query("status")
print(card["type"])  # "card"
print(card["bounds"])  # {"x": 17, "y": 3, "w": 25, "h": 5} — position and size in cells
print(card["text"], card["dir"], card["theme"], card["accent"])
```

Every widget has an `id` (yours, or an automatic one like `card-1`), a `type`,
and `bounds` — where it sits on screen.

## Use it in tests

Snapshots are **deterministic**: the same app always gives the same result.
That makes them perfect for `pytest`:

```python
from stagetui import StageApp, StageCard


class Greeting(StageApp):
    def compose(self):
        return [StageCard("Hello, Sara!", id="greeting")]


def test_greeting_is_shown():
    snap = Greeting().run_headless((40, 10))
    assert "Hello, Sara!" in snap.text
    assert snap.query("greeting")["bounds"]["w"] > 0


test_greeting_is_shown()
```

## Press keys and click — without a terminal

`steps` drive your app like a person would. Handlers run, animations advance on
a virtual clock, and you get the snapshot at the end:

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


snap = Greeter().run_headless(
    (50, 14),
    steps=[
        {"type": "Sara"},  # type into the focused text box
        {"click": "#greet"},  # click a widget by id (or {"click": [x, y]})
    ],
)
assert "Hello, Sara!" in snap.text
```

| Step | What it does |
|---|---|
| `{"press": "tab"}` | Press a key (`"enter"`, `"ctrl+s"`, `"esc"`, `"f1"`…) |
| `{"type": "hello"}` | Type text |
| `{"click": "#id"}` | Click the middle of a widget (or `[x, y]`) |
| `{"drag": {"from": [40, 5], "to": [60, 5]}}` | Drag with the mouse (`from` can be `"#id"`): split lines, 3D models |
| `{"scroll": 5}` | Scroll down 5 rows (`{"scroll": {"target": "#list", "rows": -3}}` scrolls a widget up) |
| `{"wait": 0.5}` | Let 0.5 seconds of (virtual) time pass |
| `{"wait_idle": True}` | Let animations finish |
| `{"resize": [100, 30]}` | Change the screen size |

## Golden snapshot tests with pytest

stagetui comes with a pytest plugin. The `stage_snapshot` fixture saves a copy
of the screen the first time a test runs, and checks against it afterwards:

<!-- not-tested -->
```python
def test_home_screen(stage_snapshot):
    snap = MyApp().run_headless((80, 24))
    stage_snapshot.assert_match(snap)  # compares the text
    stage_snapshot.assert_match(snap, "picture", png=True)  # also the picture
```

The copies live in `__snapshots__/` next to the test file — commit them. When
the screen changes on purpose, refresh them with `pytest --stage-update` and
look at the diff before committing. Picture comparisons allow a tiny difference
(0.2 % of pixels by default, `tolerance=`) so font rendering on another machine
doesn't fail the test.

For byte-identical PNGs across different machines, use only stagetui's bundled
fonts: set the environment variable `STAGETUI_SYSTEM_FONTS=0`.

## From the command line

```bash
stagetui render my_app.py --size 100x30 --out app.png --text app.txt --tree app.json
```

This loads `my_app.py`, finds the `StageApp` in it (without running its
`if __name__ == "__main__":` part), and writes the three views. See
[Command line](command-line.md).

## For AI agents

An agent can build an app, look at it, and fix it — in a loop, with no human:

1. Write or edit the app.
2. Run `stagetui render app.py --text screen.txt --tree tree.json` (or `run_headless` in Python).
3. Read `screen.txt` to see the layout; read `tree.json` for exact positions and ids.
4. Look at the PNG if the agent can see images.
5. Repeat.

AI agents can also operate apps themselves with `stagetui mcp` and
`stagetui drive` — see [Let AI agents see and use your app](agents.md).
