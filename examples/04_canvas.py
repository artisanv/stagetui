"""Draw your own graphics with Python: a little dashboard on a Canvas.

Run:   python examples/04_canvas.py
Quit:  q or Esc
"""

import math

from stagetui import StageApp, StageCanvas, StageCard

DATA = [3, 5, 4, 7, 6, 9, 8, 11, 10, 13]


def chart(c):
    w, h = c.width, c.height
    # No clear(): a new canvas is transparent, so it blends with the app's theme.
    c.rounded_rect(8, 8, w - 16, h - 16, 16, fill="#1f2335")
    c.text("Weekly visits", 24, 20, size=18, bold=True)
    c.text("الزيارات الأسبوعية", w - 24, 20, size=16, align="right", color="#a9b1d6")

    # A line chart: turn each value into an (x, y) point.
    left, top, right, bottom = 30, 60, w - 30, h - 30
    top_value = max(DATA)
    points = [
        (left + i * (right - left) / (len(DATA) - 1), bottom - v / top_value * (bottom - top))
        for i, v in enumerate(DATA)
    ]
    c.polyline(points + [(right, bottom), (left, bottom)], fill="#7aa2f740", color=None)
    c.polyline(points, color="#7aa2f7", width=3)
    for x, y in points:
        c.circle(x, y, 4, fill="#bb9af7")


def sun(c):
    c.gradient_circle(80, 80, 70, colors=["#ffd27a", "#e0af68", "#e0af6800"])
    for i in range(12):
        a = i * math.pi / 6
        c.line(
            80 + 40 * math.cos(a),
            80 + 40 * math.sin(a),
            80 + 62 * math.cos(a),
            80 + 62 * math.sin(a),
            color="#1a1b26",
            width=3,
        )


class Dashboard(StageApp):
    theme = "aurora-dark"

    def compose(self):
        return [
            StageCard(
                "Everything below is drawn by Python code — no image files.",
                title="Canvas",
                icon="code",
            ),
            StageCanvas(chart, size=(480, 260), id="chart"),
            StageCanvas(sun, size=(160, 160), id="sun", width=12),
        ]


if __name__ == "__main__":
    Dashboard().run()
