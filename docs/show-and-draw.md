# Show & draw

Two one-line helpers for scripts:

- `ts.show(...)` — show a message or a picture right where your output is.
- `ts.draw(...)` — make a picture with code.

## `ts.show`: like print, but prettier

### Text

```python
import stagetui as ts

ts.show("Build finished in 3.2 seconds.")
ts.show("All tests passed.", title="Tests", icon="check", accent="mint")
ts.show("مرحبا بكم", title="عربي")
```

Text becomes a card. Arabic, Persian, Hindi, Chinese and Japanese all work.

### Pictures

Give `ts.show` the path of an image file. PNG, JPEG, GIF (first frame),
WebP, BMP and SVG are supported.

```python
import stagetui as ts

ts.show("logo.png")
ts.show("chart.svg", width=40)  # 40 cells wide; the height follows automatically
```

stagetui picks the best way your terminal can draw pictures:

| Your terminal | What you get |
|---|---|
| Kitty, Ghostty | Full-quality pictures (Kitty graphics) |
| WezTerm, iTerm2 | Full-quality pictures (iTerm2 images) |
| Windows Terminal, foot, xterm | Sixel pictures (up to 256 colors) |
| Anything else | Small colored blocks — always works |

### Options

| Option | What it does | Example |
|---|---|---|
| `title` | Title on the card's border | `title="Tests"` |
| `icon` | A small symbol before the title | `icon="check"` |
| `accent` | Highlight color | `accent="mint"` or `accent="#ff8800"` |
| `theme` | Color scheme | `theme="paper-soft"` |
| `dir` | Text direction: `"auto"`, `"ltr"`, `"rtl"` | `dir="rtl"` |
| `width`, `height` | Size in cells | `width=30` |
| `fit` | How a picture fills its box: `"contain"`, `"cover"`, `"stretch"` | `fit="cover"` |

## Drawing with a Canvas

A **Canvas** is a blank picture you draw on with code. Positions are in pixels,
measured from the **top-left corner**: `x` grows to the right, `y` grows downwards.

```
(0,0) ──────────► x
  │
  │      ● (60, 40)
  ▼
  y
```

The quickest way is `ts.draw`. You write a small function that receives the
canvas (usually called `c`) and draws on it:

```python
import stagetui as ts


def smiley(c):
    c.circle(100, 100, 80, fill="amber")              # face
    c.circle(70, 80, 10, fill="black")                # left eye
    c.circle(130, 80, 10, fill="black")               # right eye
    c.polyline([(60, 125), (100, 150), (140, 125)], color="black", width=6)  # smile


ts.draw(smiley, size=(200, 200))
```

Save it instead of showing it with `out=`:

```python
import stagetui as ts

ts.draw(lambda c: c.circle(50, 50, 40, fill="mint"), size=(100, 100), out="dot.png")
```

You can also make a canvas yourself:

```python
from stagetui import Canvas

c = Canvas(300, 100, background="#1a1b26")
c.text("Hello, canvas!", 20, 30, size=28, bold=True)
c.save("hello.png")
```

### Everything a canvas can draw

| Method | Draws | Example |
|---|---|---|
| `clear(color)` | Fills the whole canvas | `c.clear("#0f172a")` |
| `rect(x, y, width, height, fill=, stroke=, stroke_width=, radius=)` | A rectangle (rounded with `radius`) | `c.rect(10, 10, 80, 40, fill="blue", radius=8)` |
| `rounded_rect(x, y, width, height, radius=12, ...)` | Same, rounded by default | `c.rounded_rect(10, 10, 80, 40, 10, fill="rose")` |
| `circle(x, y, radius, fill=, stroke=, stroke_width=)` | A circle centered at `x, y` | `c.circle(50, 50, 20, fill="mint")` |
| `line(x1, y1, x2, y2, color=, width=)` | A straight line | `c.line(0, 0, 100, 100, color="white")` |
| `polyline(points, color=, width=, fill=, closed=)` | Lines through many points (charts!) | `c.polyline([(0, 50), (50, 10), (100, 40)])` |
| `gradient_rect(x, y, width, height, colors=, vertical=, radius=)` | A rectangle that blends colors | `c.gradient_rect(0, 0, 200, 50, colors=["blue", "violet"])` |
| `gradient_circle(x, y, radius, colors=)` | A circle that blends from the center | `c.gradient_circle(50, 50, 40, colors=["white", "blue"])` |
| `text(text, x, y, size=, color=, bold=, align=, max_width=)` | Text — Arabic letters join properly | `c.text("مرحبا", 190, 10, align="right")` |
| `image(path_or_canvas, x, y, width, height, fit=)` | A picture inside a box | `c.image("logo.png", 0, 0, 64, 64)` |
| `png()` / `save(path)` | The result as PNG bytes / a file | `c.save("art.png")` |

If you give neither `fill` nor `stroke` to `rect` or `circle`, it is filled white.

### Colors

Write colors in any of these ways:

- Hex codes: `"#7aa2f7"`, short `"#fff"`, or with transparency `"#7aa2f780"` (the last two digits: `00` invisible … `ff` solid)
- stagetui accent names: `"blue"`, `"mint"`, `"amber"`, `"rose"`, `"violet"`
- Basic names: `"white"`, `"black"`, `"gray"`, `"red"`, `"transparent"`

A misspelled color raises a friendly error that lists what you can use.

### Text on a canvas

`text` uses real fonts and proper shaping, so Arabic letters join and Hindi
letters combine correctly. `align` decides which side of `x` the text grows from:

```python
from stagetui import Canvas

c = Canvas(400, 120, background="#1a1b26")
c.text("Left", 10, 10)                                # starts at x=10
c.text("Center", 200, 10, align="center")             # centered on x=200
c.text("مرحبا بكم", 390, 10, align="right", size=20)  # ends at x=390 — natural for Arabic
c.save("text.png")
```
