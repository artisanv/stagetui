"""termstage — a Rust-first TUI engine for agent harnesses and humans.

Scripts use verbs::

    import termstage as ts
    ts.show("logo.png")                 # a picture, right in the terminal
    ts.show("مرحبا بكم", title="Hi")    # a text card (Arabic just works)
    ts.draw(lambda c: c.circle(50, 50, 40, fill="mint"), size=(100, 100))

Apps use ``Stage*`` nouns::

    from termstage import StageApp, StageCard

    class Hello(StageApp):
        def compose(self):
            return [StageCard("Welcome!", title="Hello", icon="sparkles")]

    Hello().run()                          # interactive
    Hello().run_headless((80, 24)).save("hello.png")   # for CI and agents
"""

from __future__ import annotations

from ._core import Canvas, TermstageError, __version__
from ._inspect import fonts, inspect
from .app import Snapshot, StageApp
from .component import component
from .verbs import draw, show
from .widgets import StageCanvas, StageCard, StageImage, StageText, Style

__all__ = [
    "Canvas",
    "Snapshot",
    "StageApp",
    "StageCanvas",
    "StageCard",
    "StageImage",
    "StageText",
    "Style",
    "TermstageError",
    "__version__",
    "component",
    "draw",
    "fonts",
    "inspect",
    "show",
]
