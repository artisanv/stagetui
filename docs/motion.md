# Motion: animations and live drawings

## Entrance animations with `animate=`

Any widget can arrive with a short, smooth effect:

```python
from stagetui import StageApp, StageCard


class Welcome(StageApp):
    def compose(self):
        return [
            StageCard("I fade in.", animate="fade"),
            StageCard("I slide up.", animate="slide-up"),
            StageCard("I slide in from the side.", animate="slide-left"),
            StageCard("I pop.", animate="pop"),
            StageCard("My border breathes.", animate="pulse"),
        ]


Welcome().run()
```

| `animate=` | Effect |
|---|---|
| `"fade"` | Fades in from the background |
| `"slide-up"` | Rises 3 rows while fading in |
| `"slide-left"` | Slides in from the right (from the left in right-to-left apps) |
| `"pop"` | Appears with a bright flash of its border |
| `"pulse"` | The border slowly glows and dims, forever — good for "needs attention" |

Effects play when the widget first appears, and again each time you `.show()` it.
They take about a third of a second. Pop-up windows (`StageModal`) use `"pop"` by default.

stagetui only redraws while something moves, so an idle app uses no CPU.

### Turning motion off

Some people find motion distracting. Either set `reduce_motion = True` on your
app, or set the environment variable `STAGETUI_REDUCE_MOTION=1` — every effect
then shows its final state immediately.

## Spinners and progress bars

`StageSpinner("Thinking…")` spins by itself. `StageProgress(None)` shows a
gliding "busy" bar; `StageProgress(0.6)` shows 60 %. Update them from a handler:
`self.query("#bar").update(0.8)`.

## Live drawings with `on_frame`

A `StageCanvas` can redraw itself many times a second. Give it an `on_frame`
function: it receives the canvas and the time in seconds since the app started.

```python
import math

from stagetui import StageApp, StageCanvas


def clock(canvas, t):
    canvas.clear("#1a1b26")
    angle = t * 2 * math.pi / 5  # one turn every 5 seconds
    x = 100 + 70 * math.cos(angle)
    y = 100 + 70 * math.sin(angle)
    canvas.circle(100, 100, 80, stroke="#3b4261", stroke_width=3)
    canvas.line(100, 100, x, y, color="mint", width=4)
    canvas.text(f"{t:.1f} s", 70, 180, size=16, color="gray")


class Clock(StageApp):
    def compose(self):
        return [StageCanvas(on_frame=clock, size=(200, 200), fps=30)]


Clock().run()
```

- `fps` (frames per second, up to 60) sets how often `on_frame` is called.
  It is only called while the canvas is on screen.
- If drawing a frame takes longer, stagetui simply waits for it — frames never pile up.
- Over SSH or on slow terminals, use a lower `fps`, like 10.

The **Canvas** tab of `examples/05_ui_gallery.py` shows a live drawing.

## Animations in tests

Headless runs use a **virtual clock**: time only moves when a step says so, so
every snapshot is exactly the same on every run.

```python
from stagetui import StageApp, StageCard


class Hello(StageApp):
    def compose(self):
        return [StageCard("Hello", animate="slide-up")]


# Snapshots normally show the finished state.
final = Hello().run_headless((40, 10))

# Or freeze the clock part-way: wait 0.05 s, then take the picture as it is.
early = Hello().run_headless((40, 10), steps=[{"wait": 0.05}], settle=False)
assert early.text != final.text  # the card is still sliding up
```
