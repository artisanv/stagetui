"""Show things in the terminal with one line each — like a fancy print().

Run:  python examples/02_show.py
"""

import tempfile
from pathlib import Path

import termstage as ts

# 1. Text becomes a neat card. Arabic and other languages just work.
ts.show("Hello! This card was made with one line of Python.", title="ts.show", icon="sparkles")
ts.show("مرحبا بكم في termstage", title="عربي", accent="mint")


# 2. Draw a picture with code and show it.
def badge(c):
    c.clear("#1a1b26")
    c.gradient_rect(0, 0, 260, 90, colors=["#7aa2f7", "#bb9af7"], radius=18)
    c.text("termstage", 130, 18, size=30, bold=True, align="center", color="#1a1b26")
    c.text("صُنع بلغة بايثون", 130, 56, size=18, align="center", color="#1a1b26")


picture = ts.draw(badge, size=(260, 90), show=False)
ts.show(picture)

# 3. Show an image file (PNG, JPEG, GIF, WebP, BMP or SVG).
path = Path(tempfile.gettempdir()) / "termstage_badge.png"
picture.save(path)
ts.show(path, width=30)
print(f"(saved a copy at {path})")
