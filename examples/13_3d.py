"""A 3D model in a pane — turn it with the mouse.

Run:   python examples/13_3d.py [model.glb | model.gltf | model.obj | model.stl]
Needs: the 3D add-on: pip install stagetui-3d
Try:   drag to turn it · right-drag to move it · the mouse wheel zooms
       Space stops (or starts) the spin · W shows the wires · R starts over
       with no file, it makes a colorful knot to look at
"""

import math
import sys
import tempfile
from pathlib import Path

from stagetui import StageApp, StageBar, StagePane, StageSplit


def knot() -> str:
    """A (3, 2) torus knot as an OBJ file, with a rainbow of vertex colors."""
    rings, sides, radius = 240, 18, 0.32
    path = Path(tempfile.gettempdir()) / "stagetui-knot.obj"

    def center(t: float) -> tuple[float, float, float]:
        r = 2 + math.cos(3 * t)
        return (r * math.cos(2 * t), r * math.sin(2 * t), -math.sin(3 * t))

    lines = []
    for i in range(rings):
        t = 2 * math.pi * i / rings
        here, ahead = center(t), center(t + 0.001)
        tangent = [a - b for a, b in zip(ahead, here)]
        # Two directions across the tube, at right angles to where it goes.
        side = [tangent[1], -tangent[0], 0.0]
        length = math.hypot(*side) or 1.0
        side = [s / length for s in side]
        up = [
            tangent[1] * side[2] - tangent[2] * side[1],
            tangent[2] * side[0] - tangent[0] * side[2],
            tangent[0] * side[1] - tangent[1] * side[0],
        ]
        length = math.hypot(*up) or 1.0
        up = [u / length for u in up]
        hue = i / rings
        color = [0.5 + 0.5 * math.cos(2 * math.pi * (hue + k / 3)) for k in range(3)]
        for j in range(sides):
            a = 2 * math.pi * j / sides
            p = [here[k] + radius * (math.cos(a) * side[k] + math.sin(a) * up[k]) for k in range(3)]
            lines.append("v {:.4f} {:.4f} {:.4f} {:.3f} {:.3f} {:.3f}".format(*p, *color))
    for i in range(rings):
        for j in range(sides):
            a = i * sides + j + 1
            b = ((i + 1) % rings) * sides + j + 1
            c = ((i + 1) % rings) * sides + (j + 1) % sides + 1
            d = i * sides + (j + 1) % sides + 1
            lines.append(f"f {a} {b} {c} {d}")
    path.write_text("\n".join(lines) + "\n")
    return str(path)


MODEL = sys.argv[1] if len(sys.argv) > 1 else knot()

NOTES = """# Looking around

The model is drawn by your computer's
processor — no graphics card needed.

| Do | To |
|---|---|
| drag | turn it |
| right-drag | move it |
| wheel | zoom |
| Space | spin or stop |
| W | wires |
| G | grid |
| R | start over |

Click the model first for the keys.
"""


class Viewer(StageApp):
    theme = "aurora-dark"

    def compose(self):
        return [
            StageSplit(
                [
                    StagePane("3d", id="model", path=MODEL, on_loaded=self.loaded),
                    StagePane("markdown", text=NOTES),
                ],
                ratio=[3, 1],
                grow=1,
            ),
            StageBar(
                "Drag to turn · wheel zoom · Space spin · W wires · R reset · Ctrl+Space q quit"
            ),
        ]

    def loaded(self, info):
        self.notify(f"{Path(info['path']).name}: {info['triangles']:,} triangles", icon="cube")


if __name__ == "__main__":
    Viewer().run()
