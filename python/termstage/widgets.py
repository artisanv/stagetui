"""``Stage*`` widgets: thin, typed descriptions. All behaviour lives in Rust.

Every widget follows the same pattern::

    Thing(what_to_show, id=..., title=..., icon=..., theme=..., accent=...,
          rounded=..., padding=..., dir=..., animate=..., style=...)

The first argument is always the main content. Everything else is optional.
Invalid values raise :class:`termstage.TermstageError`, which explains *what*
went wrong, *why*, and how to *fix* it.
"""

from __future__ import annotations

import os
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any, Literal

from . import _core

Theme = Literal["aurora-dark", "nordic-light", "velvet-dusk", "paper-soft", "hacker-mint"]
Padding = Literal["compact", "normal", "spacious"] | int
Dir = Literal["auto", "ltr", "rtl"]
Animate = Literal["none", "fade", "slide-up", "slide-left", "pulse", "pop"]
Fit = Literal["contain", "cover", "stretch"]


@dataclass(frozen=True, slots=True)
class Style:
    """Reusable look settings you can pass to any widget with ``style=``."""

    padding: Padding | None = None
    rounded: bool | None = None
    accent: str | None = None


class _Widget:
    _type: str = ""

    def __init__(
        self,
        content: str = "",
        *,
        id: str | None = None,
        title: str | None = None,
        icon: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        rounded: bool | None = None,
        padding: Padding | None = None,
        dir: Dir | None = None,
        animate: Animate | None = None,
        style: Style | None = None,
    ) -> None:
        if not isinstance(content, str):
            raise TypeError(
                f"{type(self).__name__} content must be a str, got {type(content).__name__}"
            )
        self.content = content
        props: dict[str, Any] = {
            "id": id,
            "title": title,
            "icon": icon,
            "theme": theme,
            "accent": accent,
            "rounded": rounded,
            "padding": padding,
            "dir": dir,
            "animate": animate,
        }
        if style is not None:
            for key in ("padding", "rounded", "accent"):
                if props[key] is None:
                    props[key] = getattr(style, key)
        self.props = {k: v for k, v in props.items() if v is not None}

    @property
    def id(self) -> str | None:
        return self.props.get("id")

    def _spec(self) -> dict[str, Any]:
        return {"type": self._type, "content": self.content, "props": self.props}

    def __repr__(self) -> str:
        args = ", ".join(f"{k}={v!r}" for k, v in self.props.items())
        return f"{type(self).__name__}({self.content!r}{', ' if args else ''}{args})"


class StageCard(_Widget):
    """A titled, bordered card with wrapped text. Arabic/RTL text mirrors the title automatically."""

    _type = "card"


class StageText(_Widget):
    """Plain, muted text on the app background."""

    _type = "text"


ImageSource = str | os.PathLike[str] | bytes | _core.Canvas


def _image_source(source: ImageSource, owner: str) -> str:
    if isinstance(source, _core.Canvas):
        return source._register()
    if isinstance(source, (bytes, bytearray, memoryview)):
        return _core.register_image(bytes(source))
    if isinstance(source, (str, os.PathLike)):
        return os.fspath(source)
    raise TypeError(
        f"{owner} needs a file path, image bytes or a Canvas, got {type(source).__name__}"
    )


class StageImage(_Widget):
    """A picture: PNG, JPEG, GIF, WebP, BMP or SVG — from a file path, bytes, or a Canvas.

    ``width`` / ``height`` are measured in terminal **cells** (character boxes).
    Leave them out to use the picture's natural size.
    """

    _type = "image"

    def __init__(
        self,
        source: ImageSource,
        *,
        width: int | None = None,
        height: int | None = None,
        fit: Fit = "contain",
        id: str | None = None,
        theme: Theme | None = None,
        animate: Animate | None = None,
    ) -> None:
        super().__init__(
            _image_source(source, type(self).__name__), id=id, theme=theme, animate=animate
        )
        extra = {"width": width, "height": height, "fit": fit}
        self.props.update({k: v for k, v in extra.items() if v is not None})


class StageCanvas(StageImage):
    """A picture you draw with code. ``draw`` receives a :class:`termstage.Canvas`.

    ``size`` is the canvas size in **pixels**; ``width`` / ``height`` (optional)
    are how many terminal cells it takes on screen.

    Example::

        def scene(c):
            c.clear("#0f172a")
            c.circle(80, 60, 40, fill="violet")

        StageCanvas(scene, size=(160, 120))
    """

    _type = "image"

    def __init__(
        self,
        draw: Callable[[_core.Canvas], object],
        *,
        size: tuple[int, int] = (320, 180),
        background: str | None = None,
        width: int | None = None,
        height: int | None = None,
        id: str | None = None,
        theme: Theme | None = None,
        animate: Animate | None = None,
    ) -> None:
        if not callable(draw):
            raise TypeError("StageCanvas needs a function that draws, like StageCanvas(my_drawing)")
        canvas = _core.Canvas(size[0], size[1], background)
        draw(canvas)
        self.canvas = canvas
        super().__init__(
            canvas, width=width, height=height, fit="contain", id=id, theme=theme, animate=animate
        )
