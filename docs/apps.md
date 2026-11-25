# Apps & widgets

An **app** fills the terminal window and stays until you quit. You build one by
listing **widgets** (cards, text, pictures) in a `compose` method.

## The shape of every app

```python
from termstage import StageApp, StageCard, StageText


class MyApp(StageApp):
    theme = "nordic-light"  # optional: the color scheme
    accent = "violet"  # optional: the highlight color

    def compose(self):
        return [
            StageCard("Hello!", title="Greeting"),
            StageText("Press q to quit."),
        ]


MyApp().run()
```

- `compose` returns a **list**; widgets appear top to bottom, centered.
- `run()` shows the app. Quit with **q**, **Esc** or **Ctrl+C**.
- `run_headless()` draws the app without a terminal — see [Snapshots](snapshots.md).

> *Coming soon (Milestone 3):* side-by-side layouts, split panes, buttons,
> tabs, mouse clicks and animations.

## The widgets

Every widget follows the same pattern — learn it once, use it everywhere:

```
Widget(what_to_show, id=..., title=..., icon=..., theme=..., accent=..., ...)
```

The first thing you pass is always **what to show**. Everything else is optional.

### `StageCard` — a box with a title

```python
from termstage import StageCard

StageCard("Deploy finished.", title="Status", icon="check", accent="mint")
```

### `StageText` — plain text

```python
from termstage import StageText

StageText("Last updated 2 minutes ago.")
```

### `StageImage` — a picture

From a file, from bytes, or from a Canvas. `width` and `height` are in **cells**;
leave them out to use the picture's own size.

```python
from termstage import StageApp, StageImage


class Photo(StageApp):
    def compose(self):
        return [StageImage("logo.png", width=30)]


Photo().run()
```

`fit` controls how the picture fills its box: `"contain"` (default, whole
picture visible), `"cover"` (fills the box, edges may be cut), `"stretch"`.

### `StageCanvas` — a picture drawn by your code

Give it a function that draws on a [Canvas](show-and-draw.md#drawing-with-a-canvas).
`size` is the canvas size in **pixels**.

```python
from termstage import StageApp, StageCanvas


def sunrise(c):
    c.gradient_rect(0, 0, 320, 180, colors=["#0f172a", "#e0af68"], vertical=True)
    c.circle(160, 150, 50, fill="amber")


class Art(StageApp):
    def compose(self):
        return [StageCanvas(sunrise, size=(320, 180))]


Art().run()
```

## Options every widget understands

| Option | Values | What it does |
|---|---|---|
| `id` | a short name like `"status"` | Lets tests and agents find the widget. Without it you get `card-1`, `card-2`, … |
| `title` | text | Title on the border (cards) |
| `icon` | an icon name (below) | Symbol before the title |
| `theme` | a theme name (below) | Color scheme for just this widget |
| `accent` | `"blue"`, `"mint"`, `"amber"`, `"rose"`, `"violet"`, or `"#rrggbb"` | Highlight color |
| `rounded` | `True` / `False` | Rounded or square corners |
| `padding` | `"compact"`, `"normal"`, `"spacious"`, or a number of cells | Space inside the card |
| `dir` | `"auto"`, `"ltr"`, `"rtl"` | Text direction (auto-detected by default) |
| `animate` | `"none"`, `"fade"`, `"slide-up"`, `"slide-left"`, `"pulse"`, `"pop"` | *Coming soon:* entrance animation (accepted today, no effect yet) |
| `style` | a `Style(...)` | Reusable settings, see below |

### Themes

`"aurora-dark"` (default) · `"nordic-light"` · `"velvet-dusk"` · `"paper-soft"` · `"hacker-mint"`

### Icons

`sparkles` ✦ · `check` ✓ · `x` ✗ · `play` ▶ · `pause` ‖ · `stop` ■ · `globe` ◍ ·
`book` ≡ · `cube` ◆ · `sun` ☼ · `moon` ☾ · `terminal` ❯ · `star` ★ · `bolt` ϟ ·
`info` ℹ · `warning` ▲ · `folder` ▣ · `file` ▢ · `chat` » · `code` ⌘ · `gear` ⚙ · `heart` ♥

### Reusing a look with `Style`

```python
from termstage import StageCard, Style

calm = Style(padding="spacious", rounded=True, accent="mint")

StageCard("One", style=calm)
StageCard("Two", style=calm)
```

Options you pass directly win over the style: `StageCard("x", style=calm, accent="rose")`.

## Your own widgets with `@component`

A component is a normal function that returns a widget. Decorate it with
`@ts.component` so readers (and tools) know it is meant for reuse:

```python
import termstage as ts
from termstage import StageApp, StageCard


@ts.component
def Metric(label, value, accent="mint"):
    return StageCard(f"{value}\n{label}", accent=accent)


class Dashboard(StageApp):
    def compose(self):
        return [Metric("Users", "1,204"), Metric("Errors", "3", accent="rose")]


Dashboard().run()
```

## When something is wrong

termstage never shows a cryptic error. Every problem raises a `TermstageError`
with three parts: **what** happened, **why**, and how to **fix** it.

```python
import termstage as ts
from termstage import StageApp, StageCard


class Oops(StageApp):
    def compose(self):
        return [StageCard("Hi", icon="unicorn")]


try:
    Oops().run_headless()
except ts.TermstageError as e:
    print(e.what)  # Unknown icon "unicorn"
    print(e.fix)  # use one of: sparkles, check, x, ...
```
