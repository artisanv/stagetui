"""``Stage*`` widgets: thin, typed descriptions. All behaviour lives in Rust.

Every widget follows the same pattern::

    Thing(what_to_show, id=..., title=..., icon=..., theme=..., accent=...,
          rounded=..., padding=..., dir=..., animate=..., style=..., on_<event>=...)

The first argument is always the main content: text, a picture, a value, or a
**list of child widgets** for containers. Everything else is optional.
Invalid values raise :class:`termstage.TermstageError`, which explains *what*
went wrong, *why*, and how to *fix* it.

Once the app is running, every widget has the same lifecycle methods:
``show()``, ``hide()``, ``load(content)``, ``update(**props)``, ``focus()``,
``split(widget)`` and ``remove()``.
"""

from __future__ import annotations

import itertools
import json
import os
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, Literal

from . import _core

if TYPE_CHECKING:
    from .app import StageApp

Theme = Literal["aurora-dark", "nordic-light", "velvet-dusk", "paper-soft", "hacker-mint"]
Padding = Literal["compact", "normal", "spacious"] | int
Dir = Literal["auto", "ltr", "rtl"]
Animate = Literal["none", "fade", "slide-up", "slide-left", "pulse", "pop"]
Fit = Literal["contain", "cover", "stretch"]
Size = int | str
Direction = Literal["horizontal", "vertical"]
Align = Literal["start", "center", "end", "stretch"]
Variant = Literal["primary", "secondary", "danger"]
Handler = Callable[..., Any]

_keys = itertools.count(1)


@dataclass(frozen=True, slots=True)
class Style:
    """Reusable look settings you can pass to any widget with ``style=``.

    Example::

        soft = Style(padding="spacious", shadow=True, accent="mint")
        StageCard("Hello", style=soft)
    """

    padding: Padding | None = None
    rounded: bool | None = None
    accent: str | None = None
    shadow: bool | None = None
    width: Size | None = None
    height: Size | None = None
    grow: float | None = None


def _error(what: str, why: str, fix: str) -> _core.TermstageError:
    err = _core.TermstageError(f"{what}\n  why: {why}\n  fix: {fix}")
    err.what, err.why, err.fix = what, why, fix  # type: ignore[attr-defined]
    return err


class _Widget:
    """What every widget has in common. Use the ``Stage*`` classes, not this."""

    _type: str = ""

    def __init__(
        self,
        content: Any = None,
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
        visible: bool | None = None,
        width: Size | None = None,
        height: Size | None = None,
        grow: float | None = None,
        children: Sequence[_Widget] | None = None,
        handlers: Mapping[str, Handler | None] | None = None,
        **extra: Any,
    ) -> None:
        self._key = f"w{next(_keys)}"
        self._content = content
        self._app: StageApp | None = None
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
            "visible": visible,
            "width": width,
            "height": height,
            "grow": grow,
            **extra,
        }
        if style is not None:
            for key in ("padding", "rounded", "accent", "shadow", "width", "height", "grow"):
                if props.get(key) is None:
                    props[key] = getattr(style, key)
        self.props: dict[str, Any] = {k: v for k, v in props.items() if v is not None}
        self._handlers: dict[str, Handler] = {
            k: v for k, v in (handlers or {}).items() if v is not None
        }
        for name, fn in self._handlers.items():
            if not callable(fn):
                raise TypeError(f"on_{name} must be a function, got {type(fn).__name__}")
        self._children: list[_Widget] | None = None
        if children is not None:
            kids = list(children)
            for c in kids:
                if not isinstance(c, _Widget):
                    raise TypeError(
                        f"{type(self).__name__} children must be termstage widgets, got {c!r}"
                    )
            self._children = kids

    # ----- identity -------------------------------------------------------

    @property
    def id(self) -> str | None:
        """The id you gave this widget (``None`` until the app assigns one)."""
        return self.props.get("id")

    @property
    def children(self) -> list[_Widget]:
        """Child widgets (containers only)."""
        return list(self._children or [])

    def _events(self) -> list[str]:
        return sorted(self._handlers)

    def _content_json(self) -> Any:
        return self._content

    def _spec(self) -> dict[str, Any]:
        spec: dict[str, Any] = {
            "type": self._type,
            "content": self._content_json(),
            "props": self.props,
            "key": self._key,
        }
        events = self._events()
        if events:
            spec["events"] = events
        if self._children is not None:
            spec["children"] = [c._spec() for c in self._children]
        return spec

    def _walk(self) -> list[_Widget]:
        out: list[_Widget] = [self]
        for c in self._children or []:
            out.extend(c._walk())
        return out

    def _core(self) -> _core.App | None:
        app = self._app
        return app._core if app is not None else None

    # ----- lifecycle (Rule 4) ---------------------------------------------

    def update(self, content: Any = None, **props: Any) -> None:
        """Change the content and/or properties, e.g. ``card.update("Done!", accent="mint")``."""
        patch: dict[str, Any] = {}
        if content is not None:
            self._content = content
            patch["content"] = self._content_json()
        if props:
            for k, v in props.items():
                if v is None:
                    self.props.pop(k, None)
                else:
                    self.props[k] = v
            patch["props"] = props
        core = self._core()
        if core is not None and patch:
            core.update(self._key, json.dumps(patch, ensure_ascii=False))

    def load(self, content: Any) -> None:
        """Replace the main content (text, picture, rows…)."""
        self.update(content)

    def show(self) -> None:
        """Make the widget visible (modals open this way)."""
        self.props["visible"] = True
        core = self._core()
        if core is not None:
            core.show(self._key)

    def hide(self) -> None:
        """Hide the widget; it takes no space while hidden."""
        self.props["visible"] = False
        core = self._core()
        if core is not None:
            core.hide(self._key)

    def focus(self) -> None:
        """Give this widget keyboard focus."""
        core = self._require_running("focus()")
        core.focus(self._key)

    def split(self, widget: _Widget, direction: Direction = "horizontal") -> _Widget:
        """Put ``widget`` next to this one (``"horizontal"``) or below it (``"vertical"``).

        Returns the new widget. Use :meth:`remove` to close it again.
        """
        core = self._require_running("split()")
        if not isinstance(widget, _Widget):
            raise TypeError(f"split() needs a termstage widget, got {widget!r}")
        assert self._app is not None
        self._app._bind(widget)
        core.split(self._key, json.dumps(widget._spec(), ensure_ascii=False), direction)
        return widget

    def remove(self) -> None:
        """Remove this widget. Closing a pane joins its split back together."""
        core = self._require_running("remove()")
        core.remove(self._key)
        if self._app is not None:
            self._app._unbind(self)

    def detach(self) -> None:
        """Pop this pane out into its own terminal window (*coming in Milestone 5*)."""
        raise _error(
            "detach() isn't available yet",
            "popping panes into their own window arrives in Milestone 5",
            "keep the pane in the main window for now",
        )

    def attach(self) -> None:
        """Bring a detached pane back (*coming in Milestone 5*)."""
        self.detach()

    @property
    def value(self) -> Any:
        """The widget's current value: typed text, checked state, selection…"""
        core = self._core()
        if core is None:
            return self._content
        return json.loads(core.value(self._key))

    def _require_running(self, what: str) -> _core.App:
        core = self._core()
        if core is None:
            raise _error(
                f"{what} needs a running app",
                "this widget is not on screen yet",
                "call it from an event handler or on_mount(), after app.run() started",
            )
        return core

    def __repr__(self) -> str:
        args = ", ".join(f"{k}={v!r}" for k, v in self.props.items())
        content = self._content if self._children is None else f"[{len(self._children)} widgets]"
        return f"{type(self).__name__}({content!r}{', ' if args else ''}{args})"


def _text(content: object, owner: str) -> str:
    if content is None:
        return ""
    if not isinstance(content, str):
        raise TypeError(f"{owner} content must be text (a str), got {type(content).__name__}")
    return content


# ----- text & pictures ----------------------------------------------------------


class StageCard(_Widget):
    """A titled, bordered card with wrapped text. Arabic/RTL text mirrors the title automatically.

    Inside a :class:`StageSplit` a card becomes a *pane*: it fills its share,
    takes focus, and scrolls long text with the mouse wheel or arrow keys.
    """

    _type = "card"

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
        shadow: bool | None = None,
        visible: bool | None = None,
        width: Size | None = None,
        height: Size | None = None,
        grow: float | None = None,
    ) -> None:
        super().__init__(
            _text(content, "StageCard"),
            id=id,
            title=title,
            icon=icon,
            theme=theme,
            accent=accent,
            rounded=rounded,
            padding=padding,
            dir=dir,
            animate=animate,
            style=style,
            shadow=shadow,
            visible=visible,
            width=width,
            height=height,
            grow=grow,
        )


class StageText(_Widget):
    """Plain text, a little softer than card text. No border, no background."""

    _type = "text"

    def __init__(
        self,
        content: str = "",
        *,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        dir: Dir | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
        width: Size | None = None,
        grow: float | None = None,
    ) -> None:
        super().__init__(
            _text(content, "StageText"),
            id=id,
            theme=theme,
            accent=accent,
            dir=dir,
            animate=animate,
            visible=visible,
            width=width,
            grow=grow,
        )


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
        visible: bool | None = None,
        grow: float | None = None,
    ) -> None:
        super().__init__(
            _image_source(source, type(self).__name__),
            id=id,
            theme=theme,
            animate=animate,
            visible=visible,
            width=width,
            height=height,
            grow=grow,
            fit=fit,
        )

    def load(self, content: ImageSource) -> None:
        """Show a different picture."""
        self.update(_image_source(content, type(self).__name__))


class StageCanvas(StageImage):
    """A picture you draw with code. ``draw`` receives a :class:`termstage.Canvas`.

    ``size`` is the canvas size in **pixels**; ``width`` / ``height`` (optional)
    are how many terminal cells it takes on screen.

    Give it ``on_frame`` to animate: termstage calls ``on_frame(canvas, t)``
    up to ``fps`` times a second while it is on screen (``t`` = seconds since start).

    Example::

        def scene(c):
            c.clear("#0f172a")
            c.circle(80, 60, 40, fill="violet")

        StageCanvas(scene, size=(160, 120))
    """

    def __init__(
        self,
        draw: Callable[[_core.Canvas], object] | None = None,
        *,
        size: tuple[int, int] = (320, 180),
        background: str | None = None,
        width: int | None = None,
        height: int | None = None,
        on_frame: Callable[..., object] | None = None,
        fps: int = 30,
        id: str | None = None,
        theme: Theme | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
        grow: float | None = None,
    ) -> None:
        if draw is not None and not callable(draw):
            raise TypeError("StageCanvas needs a function that draws, like StageCanvas(my_drawing)")
        if draw is None and on_frame is None:
            raise TypeError("StageCanvas needs a drawing function or on_frame=")
        canvas = _core.Canvas(size[0], size[1], background)
        if draw is not None:
            draw(canvas)
        self.canvas = canvas
        super().__init__(
            canvas,
            width=width,
            height=height,
            fit="contain",
            id=id,
            theme=theme,
            animate=animate,
            visible=visible,
            grow=grow,
        )
        if on_frame is not None:
            if not callable(on_frame):
                raise TypeError("on_frame must be a function like def tick(canvas, t): ...")
            self._handlers["frame"] = on_frame
            self.props["fps"] = int(fps)


# ----- containers ---------------------------------------------------------------


class Stage(_Widget):
    """A container that lines widgets up.

    - ``Stage([a, b])`` — stacked top to bottom.
    - ``Stage([a, b], direction="horizontal")`` — side by side.
    - ``Stage([a, b, c, d], columns=2)`` — a grid.

    Give it a ``height`` and it scrolls when its content is taller.
    ``title`` / ``icon`` name it when it is a page of :class:`StageTabs`.
    """

    _type = "stage"

    def __init__(
        self,
        children: Sequence[_Widget] = (),
        *,
        direction: Direction = "vertical",
        gap: int | None = None,
        align: Align | None = None,
        columns: int | None = None,
        title: str | None = None,
        icon: str | None = None,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        dir: Dir | None = None,
        animate: Animate | None = None,
        style: Style | None = None,
        visible: bool | None = None,
        width: Size | None = None,
        height: Size | None = None,
        grow: float | None = None,
    ) -> None:
        super().__init__(
            None,
            id=id,
            title=title,
            icon=icon,
            theme=theme,
            accent=accent,
            dir=dir,
            animate=animate,
            style=style,
            visible=visible,
            width=width,
            height=height,
            grow=grow,
            direction=direction,
            gap=gap,
            align=align,
            columns=columns,
            children=children,
        )

    def add(self, widget: _Widget) -> _Widget:
        """Add a widget at the end (while the app runs, it appears immediately)."""
        if not isinstance(widget, _Widget):
            raise TypeError(f"add() needs a termstage widget, got {widget!r}")
        assert self._children is not None
        self._children.append(widget)
        core = self._core()
        if core is not None and self._app is not None:
            self._app._bind(widget)
            core.mount(self._key, json.dumps(widget._spec(), ensure_ascii=False))
        return widget


class StageSplit(Stage):
    """Panes that share the space, with dividers you can drag with the mouse.

    ``direction="horizontal"`` puts panes side by side; ``"vertical"`` stacks them.
    ``ratio=[2, 1]`` gives the first pane twice the space. A split fills the
    space it is given. Change it while running with ``pane.split(...)`` and
    ``pane.remove()``.
    """

    _type = "split"

    def __init__(
        self,
        children: Sequence[_Widget] = (),
        *,
        direction: Direction = "horizontal",
        ratio: Sequence[float] | None = None,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        dir: Dir | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
        width: Size | None = None,
        height: Size | None = None,
        grow: float | None = None,
    ) -> None:
        _Widget.__init__(
            self,
            None,
            id=id,
            theme=theme,
            accent=accent,
            dir=dir,
            animate=animate,
            visible=visible,
            width=width,
            height=height,
            grow=grow,
            direction=direction,
            ratio=list(ratio) if ratio is not None else None,
            children=children,
        )


class StageTabs(Stage):
    """Pages with clickable tabs. Each child's ``title`` (and ``icon``) names its tab.

    Switch with a click, with Ctrl+PageUp / Ctrl+PageDown, or ``tabs.update(active=2)``.
    ``on_change(index)`` runs when the tab changes.
    """

    _type = "tabs"

    def __init__(
        self,
        children: Sequence[_Widget] = (),
        *,
        active: int = 0,
        on_change: Handler | None = None,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        dir: Dir | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
        width: Size | None = None,
        height: Size | None = None,
        grow: float | None = None,
    ) -> None:
        _Widget.__init__(
            self,
            None,
            id=id,
            theme=theme,
            accent=accent,
            dir=dir,
            animate=animate,
            visible=visible,
            width=width,
            height=height,
            grow=grow,
            active=active,
            children=children,
            handlers={"change": on_change},
        )


class StageModal(Stage):
    """A pop-up window above everything else. Hidden until you call ``.show()``.

    While open, it keeps keyboard focus; Esc or a click outside closes it
    (unless ``dismissable=False``), and ``on_close()`` runs.
    """

    _type = "modal"

    def __init__(
        self,
        children: Sequence[_Widget] = (),
        *,
        title: str | None = None,
        icon: str | None = None,
        dismissable: bool = True,
        on_close: Handler | None = None,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        dir: Dir | None = None,
        animate: Animate | None = "pop",
        visible: bool = False,
        width: Size | None = None,
    ) -> None:
        _Widget.__init__(
            self,
            None,
            id=id,
            title=title,
            icon=icon,
            theme=theme,
            accent=accent,
            dir=dir,
            animate=animate,
            visible=visible,
            width=width,
            dismissable=dismissable,
            children=children,
            handlers={"close": on_close},
        )


class StageBar(_Widget):
    """A status line pinned to the bottom (or top, if it comes first) of the screen.

    It shows your text and the app's keyboard shortcuts (from ``BINDINGS``).
    """

    _type = "bar"

    def __init__(
        self,
        content: str = "",
        *,
        hints: bool = True,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        dir: Dir | None = None,
        visible: bool | None = None,
    ) -> None:
        super().__init__(
            _text(content, "StageBar"),
            id=id,
            theme=theme,
            accent=accent,
            dir=dir,
            visible=visible,
            hints=hints,
        )


# ----- controls -----------------------------------------------------------------


class StageButton(_Widget):
    """A button. Click it, or focus it with Tab and press Enter or Space.

    ``variant``: ``"secondary"`` (default, quiet), ``"primary"`` (filled) or ``"danger"``.
    """

    _type = "button"

    def __init__(
        self,
        label: str = "",
        *,
        on_click: Handler | None = None,
        icon: str | None = None,
        variant: Variant | None = None,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
        width: Size | None = None,
        grow: float | None = None,
    ) -> None:
        super().__init__(
            _text(label, "StageButton"),
            id=id,
            icon=icon,
            theme=theme,
            accent=accent,
            animate=animate,
            visible=visible,
            width=width,
            grow=grow,
            variant=variant,
            handlers={"click": on_click},
        )


class StageInput(_Widget):
    """A one-line text box. ``on_change(text)`` runs as you type, ``on_submit(text)`` on Enter.

    The first argument is the starting text. ``password=True`` shows dots.
    """

    _type = "input"

    def __init__(
        self,
        value: str = "",
        *,
        placeholder: str | None = None,
        title: str | None = None,
        password: bool | None = None,
        on_change: Handler | None = None,
        on_submit: Handler | None = None,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        dir: Dir | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
        width: int | None = None,
        grow: float | None = None,
    ) -> None:
        super().__init__(
            _text(value, "StageInput"),
            id=id,
            title=title,
            theme=theme,
            accent=accent,
            dir=dir,
            animate=animate,
            visible=visible,
            width=width,
            grow=grow,
            placeholder=placeholder,
            password=password,
            handlers={"change": on_change, "submit": on_submit},
        )


class StageCheckbox(_Widget):
    """A box you can tick. ``on_change(checked)`` runs with ``True`` or ``False``."""

    _type = "checkbox"

    def __init__(
        self,
        label: str = "",
        *,
        checked: bool = False,
        on_change: Handler | None = None,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
    ) -> None:
        super().__init__(
            _text(label, "StageCheckbox"),
            id=id,
            theme=theme,
            accent=accent,
            animate=animate,
            visible=visible,
            checked=checked,
            handlers={"change": on_change},
        )


class StageSelect(_Widget):
    """Pick one option from a dropdown. ``on_change(option)`` runs with the chosen text."""

    _type = "select"

    def __init__(
        self,
        options: Sequence[str],
        *,
        value: str | None = None,
        title: str | None = None,
        on_change: Handler | None = None,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
        width: int | None = None,
    ) -> None:
        if isinstance(options, str) or not isinstance(options, Sequence):
            raise TypeError(
                'StageSelect needs a list of options, like StageSelect(["S", "M", "L"])'
            )
        super().__init__(
            [str(o) for o in options],
            id=id,
            title=title,
            theme=theme,
            accent=accent,
            animate=animate,
            visible=visible,
            width=width,
            value=value,
            handlers={"change": on_change},
        )


class StageProgress(_Widget):
    """A progress bar. ``value`` goes from ``0`` to ``1``; ``None`` shows a moving bar.

    Update it with ``bar.update(0.75)``.
    """

    _type = "progress"

    def __init__(
        self,
        value: float | None = None,
        *,
        label: str | None = None,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
        width: int | None = None,
        grow: float | None = None,
    ) -> None:
        if value is not None and not isinstance(value, (int, float)):
            raise TypeError("StageProgress value must be a number from 0 to 1, or None")
        super().__init__(
            None if value is None else float(value),
            id=id,
            theme=theme,
            accent=accent,
            animate=animate,
            visible=visible,
            width=width,
            grow=grow,
            label=label,
        )

    def update(self, content: Any = None, **props: Any) -> None:
        """Set a new value (``bar.update(0.5)``) and/or properties."""
        if isinstance(content, (int, float)):
            content = float(content)
        super().update(content, **props)


class StageSpinner(_Widget):
    """A small spinner with a label, for "working on it…" moments."""

    _type = "spinner"

    def __init__(
        self,
        label: str = "",
        *,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        visible: bool | None = None,
    ) -> None:
        super().__init__(
            _text(label, "StageSpinner"), id=id, theme=theme, accent=accent, visible=visible
        )


def _cell(v: object) -> str:
    return "" if v is None else str(v)


class StageTable(_Widget):
    """Rows and columns. Move with ↑/↓, click a row, press Enter.

    ``on_select(row)`` runs when the selected row changes; ``row`` is a dict with
    ``index`` and ``row`` (the cell texts). ``height`` limits the visible rows.
    """

    _type = "table"

    def __init__(
        self,
        rows: Sequence[Sequence[object]] = (),
        *,
        headers: Sequence[str] | None = None,
        height: int | None = None,
        on_select: Handler | None = None,
        on_submit: Handler | None = None,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
        width: Size | None = None,
        grow: float | None = None,
    ) -> None:
        super().__init__(
            self._rows(rows),
            id=id,
            theme=theme,
            accent=accent,
            animate=animate,
            visible=visible,
            width=width,
            height=height,
            grow=grow,
            headers=[str(h) for h in headers] if headers is not None else None,
            handlers={"select": on_select, "submit": on_submit},
        )

    @staticmethod
    def _rows(rows: Sequence[Sequence[object]]) -> list[list[str]]:
        if isinstance(rows, str):
            raise TypeError('StageTable needs a list of rows, like [["Ada", 36], ["Linus", 54]]')
        return [[_cell(c) for c in row] for row in rows]

    def update(self, content: Any = None, **props: Any) -> None:
        """Replace the rows (``table.update([[...], ...])``) and/or properties."""
        super().update(None if content is None else self._rows(content), **props)


TreeData = Mapping[str, Any] | Sequence[Any]


def _tree_items(data: Any) -> list[Any]:
    """Turn friendly nested data into ``[{"label", "children"}]``."""
    if data is None:
        return []
    if isinstance(data, str):
        return [data]
    if isinstance(data, Mapping):
        out: list[Any] = []
        for label, kids in data.items():
            if kids is None or kids == [] or kids == {}:
                out.append(str(label))
            else:
                out.append({"label": str(label), "children": _tree_items(kids)})
        return out
    if isinstance(data, Sequence):
        items: list[Any] = []
        for item in data:
            if isinstance(item, Mapping) and "label" in item:
                d = dict(item)
                d["children"] = _tree_items(d.get("children"))
                items.append(d)
            elif isinstance(item, Mapping):
                items.extend(_tree_items(item))
            else:
                items.append(str(item))
        return items
    raise TypeError(f"StageTree can't show {type(data).__name__}; use a dict or a list")


class StageTree(_Widget):
    """A folding outline, e.g. files and folders.

    Give it nested data: ``{"src": ["main.py", "app.py"], "README.md": None}``.
    ``on_select(item)`` runs when the selection moves; ``on_submit(item)`` when a
    leaf is opened with Enter or a click. ``item`` has ``label``, ``path`` and ``leaf``.
    """

    _type = "tree"

    def __init__(
        self,
        items: TreeData = (),
        *,
        height: int | None = None,
        on_select: Handler | None = None,
        on_submit: Handler | None = None,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
        width: Size | None = None,
        grow: float | None = None,
    ) -> None:
        super().__init__(
            _tree_items(items),
            id=id,
            theme=theme,
            accent=accent,
            animate=animate,
            visible=visible,
            width=width,
            height=height,
            grow=grow,
            handlers={"select": on_select, "submit": on_submit},
        )

    def update(self, content: Any = None, **props: Any) -> None:
        """Replace the items and/or properties (open folders stay open)."""
        super().update(None if content is None else _tree_items(content), **props)


class RemoteWidget(_Widget):
    """A handle to a widget that termstage created by itself (for example the
    content of a pane you split). It has the same methods as every widget,
    plus ``call(method, args)`` for widget-specific ones."""

    _type = "remote"

    def __init__(self, widget_id: str) -> None:
        super().__init__(None, id=widget_id)
        self._key = f"#{widget_id}"

    def call(self, method: str, args: Any = None) -> Any:
        """Run a widget method, e.g. ``handle.call("append", "text")``."""
        core = self._require_running(f"{method}()")
        return json.loads(core.call(self._key, method, json.dumps(args, ensure_ascii=False)))

    @property
    def info(self) -> dict[str, Any]:
        """The widget as termstage describes it (type, bounds, text…)."""
        core = self._require_running("info")
        node = core.query(self._key)
        return json.loads(node) if node else {}
