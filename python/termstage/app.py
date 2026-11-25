"""``StageApp``: describe your screen in ``compose()``, react to events, run it.

The Rust engine does the drawing, layout, focus, mouse and animation work on
its own threads; Python only runs your event handlers. Handlers can be normal
functions or ``async def`` — both work.
"""

from __future__ import annotations

import asyncio
import inspect
import json
import os
from collections.abc import Callable, Mapping, Sequence
from pathlib import Path
from typing import Any, ClassVar

from . import _core
from .widgets import Dir, Theme, _error, _Widget

Binding = tuple[str, str] | tuple[str, str, str]
Step = Mapping[str, Any]


def _arity(fn: Callable[..., Any]) -> int | None:
    """How many positional arguments ``fn`` accepts (``None`` = any number)."""
    try:
        params = inspect.signature(fn).parameters.values()
    except (TypeError, ValueError):
        return None
    if any(p.kind is inspect.Parameter.VAR_POSITIONAL for p in params):
        return None
    return sum(
        1
        for p in params
        if p.kind in (inspect.Parameter.POSITIONAL_ONLY, inspect.Parameter.POSITIONAL_OR_KEYWORD)
    )


def _call(fn: Callable[..., Any], *args: Any) -> Any:
    """Call a handler with as many arguments as it wants (0, 1, 2…)."""
    n = _arity(fn)
    return fn(*(args if n is None else args[:n]))


class Snapshot:
    """A picture of your app taken without a terminal.

    - ``snap.text`` — the screen as plain text (Arabic in normal reading order).
    - ``snap.tree`` — every widget as a dictionary: id, type, position, text…
    - ``snap.png`` — the screen as a PNG image (bytes).
    - ``snap.save("shot.png")`` — save as ``.png``, ``.txt`` or ``.json``.
    """

    def __init__(
        self, text: str, tree: dict[str, Any], core: _core.App, cell: tuple[int, int]
    ) -> None:
        self.text = text
        self.tree = tree
        self.cols: int = tree["size"]["cols"]
        self.rows: int = tree["size"]["rows"]
        self._core = core
        self._cell = cell
        self._png: bytes | None = None

    @property
    def png(self) -> bytes:
        """The screen as PNG bytes (rendered on first use)."""
        if self._png is None:
            self._png = self._core.png(self.cols, self.rows)
        return self._png

    def query(self, id: str) -> dict[str, Any] | None:
        """Find a widget in the tree by id (``"hello"`` or ``"#hello"``), at any depth."""
        wanted = id.removeprefix("#")

        def find(nodes: list[dict[str, Any]]) -> dict[str, Any] | None:
            for n in nodes:
                if n.get("id") == wanted:
                    return n
                hit = find(n.get("children", []))
                if hit is not None:
                    return hit
            return None

        return find(self.tree.get("children", []))

    def save(self, path: str | os.PathLike[str]) -> Path:
        """Save as PNG, text or JSON — chosen by the file extension."""
        p = Path(path)
        suffix = p.suffix.lower()
        if suffix == ".png":
            p.write_bytes(self.png)
        elif suffix == ".txt":
            p.write_text(self.text + "\n", encoding="utf-8")
        elif suffix == ".json":
            p.write_text(json.dumps(self.tree, ensure_ascii=False, indent=2), encoding="utf-8")
        else:
            raise ValueError(
                f"Can't save a snapshot as {suffix or 'a file without extension'!r}: "
                "use a name ending in .png, .txt or .json"
            )
        return p

    def __eq__(self, other: object) -> bool:
        return isinstance(other, Snapshot) and (self.text, self.tree) == (other.text, other.tree)

    def __repr__(self) -> str:
        return f"Snapshot(cols={self.cols}, rows={self.rows}, widgets={len(self.tree.get('children', []))})"


class StageApp:
    """Base class for apps. Override :meth:`compose` to return a list of widgets.

    Class settings:

    - ``theme`` — one of the five themes (switch while running with :meth:`set_theme`).
    - ``accent`` — highlight color.
    - ``dir`` — ``"rtl"`` mirrors the whole layout; ``None`` guesses from your text.
    - ``BINDINGS`` — keyboard shortcuts: ``[("ctrl+s", "save", "Save")]`` calls
      ``self.action_save()`` (or ``self.save()``) when Ctrl+S is pressed.
    """

    theme: ClassVar[Theme] = "aurora-dark"
    accent: ClassVar[str | None] = None
    dir: ClassVar[Dir | None] = None
    BINDINGS: ClassVar[Sequence[Binding]] = ()
    reduce_motion: ClassVar[bool] = False

    def __init__(self) -> None:
        self._core: _core.App | None = None
        self._widgets: dict[str, _Widget] = {}
        self._tasks: set[asyncio.Task[Any]] = set()
        self._failure: BaseException | None = None
        self._loop: asyncio.AbstractEventLoop | None = None
        self._headless = False

    # ----- what to override -------------------------------------------------

    def compose(self) -> list[_Widget]:
        """Return the widgets to show, top to bottom."""
        return []

    def on_mount(self) -> Any:
        """Runs once the app is on screen (may be ``async``)."""
        return None

    # ----- talking to the running app ----------------------------------------

    def query(self, selector: str) -> _Widget:
        """The widget with this id: ``app.query("#chat")``."""
        wanted = selector.removeprefix("#")
        for w in self._widgets.values():
            if w.id == wanted:
                return w
        core = self._core
        if core is not None:
            found = core.find(f"#{wanted}")
            if found is not None and found[0] in self._widgets:
                return self._widgets[found[0]]
        raise _error(
            f'No widget matches "{selector}"',
            'queries look widgets up by id, like "#chat"',
            "check the id= you gave the widget (ids are case-sensitive)",
        )

    @property
    def focused(self) -> _Widget | None:
        """The widget that has keyboard focus, if any."""
        return self._lookup(self._core.focused() if self._core else None)

    @property
    def focused_pane(self) -> _Widget | None:
        """The split pane that holds the focus (or the focused widget)."""
        return self._lookup(self._core.focused_pane() if self._core else None)

    def _lookup(self, found: tuple[str | None, str] | None) -> _Widget | None:
        if found is None or found[0] is None:
            return None
        return self._widgets.get(found[0])

    def notify(
        self, text: str, *, icon: str | None = None, accent: str | None = None, timeout: float = 3.0
    ) -> None:
        """Show a short message in the corner for ``timeout`` seconds."""
        if self._core is not None:
            self._core.notify(text, icon, accent, timeout)

    def set_theme(self, name: Theme) -> None:
        """Switch the color theme while the app runs."""
        if self._core is not None:
            self._core.set_theme(name)
        self.__dict__["theme"] = name

    def mount(self, widget: _Widget) -> _Widget:
        """Add a widget at the end of the screen while the app runs."""
        core = self._require_core("mount()")
        self._bind(widget)
        core.mount(None, json.dumps(widget._spec(), ensure_ascii=False))
        return widget

    def exit(self) -> None:
        """Quit the app (like pressing q)."""
        if self._core is not None:
            self._core.quit()

    # ----- running ---------------------------------------------------------

    def run(self) -> None:
        """Run in the current terminal. Press ``q``, ``Esc`` or ``Ctrl+C`` to quit."""
        try:
            asyncio.get_running_loop()
        except RuntimeError:
            asyncio.run(self.run_async())
            return
        raise _error(
            "run() can't be used inside async code",
            "an asyncio event loop is already running in this thread",
            "use `await app.run_async()` instead",
        )

    async def run_async(self) -> None:
        """Run in the current terminal from ``async`` code (``await app.run_async()``)."""
        core = self._create(cell=(16, 32))
        self._loop = asyncio.get_running_loop()
        self._failure = None
        core.start()
        try:
            await self._await(self.on_mount())
            while True:
                self._raise_failure()
                event = await self._loop.run_in_executor(None, core.next_event, 0.1)
                if event is None:
                    if not core.running():
                        break
                    continue
                if not self._dispatch(event):
                    break
                self._raise_failure()
        finally:
            for t in self._tasks:
                t.cancel()
            core.stop()
            self._core = None
            self._loop = None

    def run_headless(
        self,
        size: tuple[int, int] = (80, 24),
        *,
        steps: Sequence[Step] | None = None,
        cell: tuple[int, int] = (16, 32),
        settle: bool = True,
    ) -> Snapshot:
        """Run without a terminal at ``(columns, rows)`` and return a :class:`Snapshot`.

        ``steps`` drive the app like a person would — handy for tests and agents::

            app.run_headless((80, 24), steps=[
                {"press": "tab"},
                {"type": "hello"},
                {"click": "#send"},
                {"wait": 0.5},          # seconds of (virtual) time
            ])

        Time is virtual, so animations and snapshots are the same on every run.
        With ``settle=True`` (the default) entrance animations are played to the
        end before the snapshot; ``settle=False`` captures the exact moment.
        ``cell`` is the pixel size of one character box, used for PNGs.
        """
        cols, rows = size
        core = self._create(cell=cell)
        self._headless = True
        loop = asyncio.new_event_loop()
        self._loop = loop
        try:
            core.render(cols, rows)
            self._drain(loop)
            loop.run_until_complete(self._await(self.on_mount()))
            core.render(cols, rows)
            self._drain(loop)
            for step in steps or ():
                cols, rows = self._step(core, step, cols, rows)
                core.render(cols, rows)
                self._drain(loop)
                core.render(cols, rows)  # lay out what the handlers changed
            if settle:
                core.settle(cols, rows)
                self._drain(loop)
            text, tree = core.snapshot(cols, rows)
            return Snapshot(text, json.loads(tree), core, cell)
        finally:
            loop.close()
            self._loop = None
            self._headless = False

    # ----- internals -------------------------------------------------------

    def _spec(self) -> dict[str, Any]:
        children = self.compose()
        if not isinstance(children, (list, tuple)):
            raise TypeError("compose() must return a list of widgets")
        for c in children:
            if not isinstance(c, _Widget):
                raise TypeError(f"compose() returned {c!r}, which is not a termstage widget")
        bindings = []
        for b in self.BINDINGS:
            if not isinstance(b, (tuple, list)) or len(b) not in (2, 3):
                raise TypeError(f'BINDINGS entries look like ("ctrl+s", "save", "Save"), got {b!r}')
            entry = {"key": b[0], "action": b[1]}
            if len(b) == 3:
                entry["description"] = b[2]
            bindings.append(entry)
        spec: dict[str, Any] = {
            "theme": self.theme,
            "children": [c._spec() for c in children],
            "bindings": bindings,
        }
        if self.accent is not None:
            spec["accent"] = self.accent
        if self.dir is not None:
            spec["dir"] = self.dir
        if self.reduce_motion:
            spec["reduce_motion"] = True
        self._roots = list(children)
        return spec

    def _spec_json(self) -> str:
        return json.dumps(self._spec(), ensure_ascii=False)

    def _create(self, cell: tuple[int, int]) -> _core.App:
        if self._core is not None:
            raise _error(
                "This app is already running",
                "an app runs in one place at a time",
                "wait for it to finish, or create a second instance",
            )
        self._widgets = {}
        spec = self._spec_json()
        core = _core.App(spec, cell)
        self._core = core
        for root in self._roots:
            self._bind(root)
        return core

    def _bind(self, widget: _Widget) -> None:
        for w in widget._walk():
            w._app = self
            self._widgets[w._key] = w

    def _unbind(self, widget: _Widget) -> None:
        for w in widget._walk():
            self._widgets.pop(w._key, None)
            w._app = None

    def _require_core(self, what: str) -> _core.App:
        if self._core is None:
            raise _error(
                f"{what} needs a running app",
                "the app is not on screen yet",
                "call it from an event handler or on_mount()",
            )
        return self._core

    def _dispatch(self, event: tuple[str | None, str, str, str]) -> bool:
        """Handle one event. Returns ``False`` when the app should stop."""
        key, _id, name, raw = event
        value = json.loads(raw)
        if name == "quit":
            return False
        if key is None:
            if name == "action":
                self._run_action(str(value))
            elif name == "theme":
                self.__dict__["theme"] = value
            return True
        widget = self._widgets.get(key)
        if widget is None:
            return True
        if name == "frame":
            self._frame(widget, value)
            return True
        handler = widget._handlers.get(name)
        if handler is None:
            return True
        args: tuple[Any, ...] = () if name in ("click", "close") else (value,)
        self._handle(_call(handler, *args))
        return True

    def _frame(self, widget: _Widget, value: Mapping[str, Any]) -> None:
        canvas = getattr(widget, "canvas", None)
        handler = widget._handlers.get("frame")
        core = self._core
        if canvas is None or handler is None or core is None:
            return
        result = _call(handler, canvas, float(value.get("t", 0.0)))
        if inspect.isawaitable(result):

            async def finish() -> None:
                await result
                if self._core is not None:
                    self._core.set_canvas(widget._key, canvas)

            self._handle(finish())
        else:
            core.set_canvas(widget._key, canvas)

    def _run_action(self, name: str) -> None:
        method = getattr(self, f"action_{name}", None) or getattr(self, name, None)
        if method is None or not callable(method):
            raise _error(
                f'No method for the action "{name}"',
                "a shortcut in BINDINGS names an action, and the app needs a method for it",
                f"add `def action_{name}(self): ...` to your app class",
            )
        self._handle(_call(method))

    def _handle(self, result: Any) -> None:
        """Run an async handler's coroutine; plain results are ignored."""
        if not inspect.isawaitable(result):
            return
        loop = self._loop
        if loop is None:
            return
        if self._headless:
            loop.run_until_complete(self._await(result))
            return
        task = loop.create_task(self._await(result))
        self._tasks.add(task)

        def done(t: asyncio.Task[Any]) -> None:
            self._tasks.discard(t)
            if not t.cancelled() and t.exception() is not None:
                self._failure = t.exception()

        task.add_done_callback(done)

    @staticmethod
    async def _await(result: Any) -> Any:
        if inspect.isawaitable(result):
            return await result
        return result

    def _raise_failure(self) -> None:
        if self._failure is not None:
            failure, self._failure = self._failure, None
            raise failure

    def _drain(self, loop: asyncio.AbstractEventLoop) -> None:
        core = self._core
        assert core is not None
        for _ in range(1000):
            events = core.poll_events()
            if not events:
                return
            for e in events:
                self._dispatch(e)

    def _step(self, core: _core.App, step: Step, cols: int, rows: int) -> tuple[int, int]:
        if not isinstance(step, Mapping) or len(step) != 1:
            raise _error(
                f"Can't run the step {step!r}",
                'each step is a dict with one action, like {"press": "tab"}',
                "use press, type, click, scroll, wait, wait_idle or resize",
            )
        ((action, arg),) = step.items()
        if action == "press":
            core.press(str(arg))
        elif action == "type":
            core.type_text(str(arg))
        elif action == "click":
            if isinstance(arg, (list, tuple)) and len(arg) == 2:
                core.click_at(int(arg[0]), int(arg[1]))
            else:
                target = str(arg)
                core.click(target if target.startswith("#") else f"#{target}")
        elif action == "scroll":
            if isinstance(arg, Mapping):
                target = arg.get("target")
                core.scroll(int(arg.get("rows", 3)), None if target is None else str(target))
            else:
                core.scroll(int(arg))
        elif action == "wait":
            remaining = int(float(arg) * 1000)
            while remaining > 0:
                step_ms = min(remaining, 16)
                core.advance(step_ms)
                core.render(cols, rows)
                if self._loop is not None:
                    self._drain(self._loop)
                remaining -= step_ms
        elif action == "wait_idle":
            core.settle(cols, rows)
        elif action == "resize":
            cols, rows = int(arg[0]), int(arg[1])
        else:
            raise _error(
                f'Unknown step "{action}"',
                "headless runs understand a fixed set of actions",
                "use press, type, click, scroll, wait, wait_idle or resize",
            )
        return cols, rows
