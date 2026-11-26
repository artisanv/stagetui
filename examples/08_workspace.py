"""A Blender-style workspace: every pane can show anything, split, or close.

Run:   python examples/08_workspace.py
Try:   click a pane's title (▾) to switch what it shows — Terminal, Files, Chat, Clock…
       the ⊞ ⊟ × buttons on the right of each title split side by side, split top/bottom, close
       keyboard: Ctrl+Space, then Space (menu) · | (split →) · - (split ↓) · x (close) · Tab (next)
       switch a pane to Terminal, run `htop`, switch away and back: it is still running
"""

import math
import time

import stagetui as ts
from stagetui import StageApp, StageBar, StageCanvas, StagePane, StageSplit


# Your own pane type: it appears in every pane's menu.
@ts.pane_type("clock", title="Clock", icon="sun")
def clock_pane():
    def tick(canvas, t):
        w, h = canvas.width, canvas.height
        canvas.clear("#16161e")
        cx, cy, r = w / 2, h / 2, min(w, h) / 2 - 12
        canvas.circle(cx, cy, r, stroke="#3b4261", stroke_width=3)
        # Smooth hands: seconds with their fraction, from the same wall clock.
        now = time.time()
        local = time.localtime(now)
        seconds = local.tm_sec + now % 1
        minutes = local.tm_min + seconds / 60
        hours = local.tm_hour % 12 + minutes / 60
        for value, length, color, width in [
            (hours / 12, 0.5, "#bb9af7", 6),
            (minutes / 60, 0.75, "#7aa2f7", 4),
            (seconds / 60, 0.9, "#f7768e", 2),
        ]:
            a = value * 2 * math.pi - math.pi / 2
            canvas.line(
                cx,
                cy,
                cx + r * length * math.cos(a),
                cy + r * length * math.sin(a),
                color=color,
                width=width,
            )
        canvas.circle(cx, cy, 5, fill="#e0af68")
        canvas.text(
            time.strftime("%H:%M:%S"), cx, cy + r * 0.45, size=18, color="#a9b1d6", align="center"
        )

    return StageCanvas(on_frame=tick, size=(320, 320), fps=20)


WELCOME = """# Your workspace

Every pane works like an area in **Blender**:

- **Click a pane's title** (the ▾) to choose what it shows.
- **⊞** splits it side by side, **⊟** top/bottom, **×** closes it.
- Drag the lines between panes to resize.

From the keyboard, press **Ctrl+Space**, then:

| Key | Does |
|---|---|
| Space | open the pane menu |
| \\| | split side by side |
| - | split top / bottom |
| x | close the pane |
| Tab | next pane |

Switch a pane to *Terminal*, start `htop`, switch to *Notes* and back —
it is still running. There is also a custom **Clock** pane type.
"""


class Workspace(StageApp):
    theme = "aurora-dark"
    BINDINGS = [("ctrl+t", "next_theme", "Theme")]

    def compose(self):
        return [
            StageSplit(
                [
                    StagePane("files"),
                    StagePane("markdown", text=WELCOME),
                    StagePane("clock"),
                ],
                ratio=[1, 2, 1.2],
            ),
            StageBar("Workspace · click a pane title to change it"),
        ]


if __name__ == "__main__":
    Workspace().run()
