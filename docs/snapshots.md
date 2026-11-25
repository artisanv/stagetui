# Snapshots — for tests, CI and AI agents

A **snapshot** is a picture of your app taken *without* a terminal. termstage
draws everything offscreen and gives you three views of the result:

| View | What it is | Best for |
|---|---|---|
| `snap.text` | The screen as plain text (Arabic in normal reading order) | Tests, and AI agents reading the screen |
| `snap.tree` | Every widget as a dictionary: id, type, position, text… | Checking details precisely |
| `snap.png` | The screen as a PNG picture | Humans, visual checks, AI agents with vision |

## Take a snapshot

```python
from termstage import StageApp, StageCard


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
from termstage import StageApp, StageCard


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
from termstage import StageApp, StageCard


class Greeting(StageApp):
    def compose(self):
        return [StageCard("Hello, Sara!", id="greeting")]


def test_greeting_is_shown():
    snap = Greeting().run_headless((40, 10))
    assert "Hello, Sara!" in snap.text
    assert snap.query("greeting")["bounds"]["w"] > 0


test_greeting_is_shown()
```

For byte-identical PNGs across different machines, use only termstage's bundled
fonts: set the environment variable `TERMSTAGE_SYSTEM_FONTS=0`.

## From the command line

```bash
termstage render my_app.py --size 100x30 --out app.png --text app.txt --tree app.json
```

This loads `my_app.py`, finds the `StageApp` in it (without running its
`if __name__ == "__main__":` part), and writes the three views. See
[Command line](command-line.md).

## For AI agents

An agent can build an app, look at it, and fix it — in a loop, with no human:

1. Write or edit the app.
2. Run `termstage render app.py --text screen.txt --tree tree.json` (or `run_headless` in Python).
3. Read `screen.txt` to see the layout; read `tree.json` for exact positions and ids.
4. Look at the PNG if the agent can see images.
5. Repeat.

> *Coming soon (Milestone 4):* `termstage drive` and `termstage mcp`, so agents
> can also press keys and click in a running app.
