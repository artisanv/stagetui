"""One-line verbs for scripts: ``ts.show(...)`` and ``ts.draw(...)``."""

from __future__ import annotations

import os
import sys
from collections.abc import Callable
from typing import Any

from . import _core
from .widgets import Dir, Fit, Theme


def show(
    content: Any,
    *,
    title: str | None = None,
    icon: str | None = None,
    dir: Dir | None = None,
    theme: Theme | None = None,
    accent: str | None = None,
    width: int | None = None,
    height: int | None = None,
    fit: Fit | None = None,
) -> None:
    """Show something right here in the terminal, like a fancy ``print()``.

    - A path to an image (``"cat.png"``, ``"logo.svg"``) shows the picture.
    - Image ``bytes`` or a :class:`~termstage.Canvas` shows that picture.
    - Anything else is shown as text in a card (Arabic and other scripts included).

    ``width`` / ``height`` are in terminal cells.
    """
    if isinstance(content, os.PathLike):
        content = os.fspath(content)
    out = _core.show(
        content,
        title=title,
        icon=icon,
        dir=dir,
        theme=theme,
        accent=accent,
        width=width,
        height=height,
        fit=fit,
    )
    sys.stdout.write(out)
    sys.stdout.flush()


def draw(
    fn: Callable[[_core.Canvas], object] | None = None,
    *,
    size: tuple[int, int] = (640, 360),
    background: str | None = None,
    out: str | os.PathLike[str] | None = None,
    show: bool = True,
) -> _core.Canvas:
    """Make a picture with code.

    ``fn`` receives a :class:`~termstage.Canvas` to draw on. Afterwards the
    picture is saved to ``out`` (if given) and shown in the terminal (unless
    ``show=False``). The canvas is returned so you can keep drawing.

    Example::

        def hello(c):
            c.clear("#0f172a")
            c.text("Hello!", 20, 20, size=32)

        ts.draw(hello, size=(300, 80))
    """
    canvas = _core.Canvas(size[0], size[1], background)
    if fn is not None:
        fn(canvas)
    if out is not None:
        canvas.save(os.fspath(out))
    elif show:
        globals()["show"](canvas)
    return canvas
