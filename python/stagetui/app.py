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
import sys
from collections.abc import Awaitable, Callable, Mapping, Sequence
from pathlib import Path
from typing import Any, ClassVar, Self

from . import _core
from .widgets import Dir, Theme, _error, _Widget

_HEADLESS_TASK_BUDGET = 0.5  # seconds of real time async handlers get per headless step

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


def _contains(node: Mapping[str, Any], id: str | None) -> bool:
    return id is not None and any(
        c.get("id") == id or _contains(c, id) for c in node.get("children", [])
    )


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
    #: The key that reaches the app from inside a terminal pane (then press Tab, |, -, x, Space, q).
    PREFIX: ClassVar[str] = "ctrl+space"

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
            if found is not None:
                return self._remote(found[1])
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

    @property
    def session(self) -> dict[str, Any] | None:
        """While the app runs: its session (``name``, ``pid``…), which other
        terminals can open with ``stagetui attach <name>``. ``None`` otherwise."""
        info = self._core.session() if self._core else None
        return json.loads(info) if info else None

    # ----- workspaces ------------------------------------------------------

    def workspace(self) -> dict[str, Any]:
        """The current layout of splits and panes, as a dict (see :meth:`load_workspace`)."""
        return json.loads(self._require_core("workspace()").workspace(None))

    def save_workspace(
        self, name: str | None = None, *, path: str | os.PathLike[str] | None = None
    ) -> Path:
        """Save the current layout of splits and panes.

        ``name`` defaults to the open workspace tab's name (or ``"default"``).
        It goes to ``~/.config/stagetui/workspaces/<YourApp>.json`` (or ``path``).
        Returns the file it was saved in.
        """
        core = self._require_core("save_workspace()")
        file = self._workspace_file(path)
        core.save_workspace(None, name or core.workspace_name(), str(file))
        return file

    def load_workspace(
        self,
        name: str | Mapping[str, Any] | None = None,
        *,
        path: str | os.PathLike[str] | None = None,
    ) -> None:
        """Bring back a saved layout by name — or arrange the panes like a layout dict.

        ``name`` defaults to the open workspace tab's name (or ``"default"``).
        Panes that still exist (same id) are moved, not rebuilt: a running shell
        or a chat keeps everything.
        """
        core = self._require_core("load_workspace()")
        if isinstance(name, Mapping):
            core.load_workspace(None, json.dumps(name, ensure_ascii=False))
        else:
            core.open_workspace(
                None, name or core.workspace_name(), str(self._workspace_file(path))
            )

    def workspaces(self, *, path: str | os.PathLike[str] | None = None) -> list[str]:
        """The names of the saved workspaces."""
        return list(json.loads(_core.workspace_names(str(self._workspace_file(path)))))

    def _workspace_file(self, path: str | os.PathLike[str] | None) -> Path:
        if path is not None:
            return Path(path)
        return Path(_core.workspace_file(type(self).__name__))

    def _lookup(self, found: tuple[str | None, str] | None) -> _Widget | None:
        if found is None:
            return None
        if found[0] is not None and found[0] in self._widgets:
            return self._widgets[found[0]]
        return self._remote(found[1])

    def _remote(self, id: str) -> _Widget:
        """The widget with Rust id ``id``: your own object if you created it,
        otherwise a typed handle (stagetui created it, e.g. a pane you switched)."""
        from . import harness, widgets

        core = self._core
        found = core.find(f"#{id}") if core is not None else None
        if found is not None and found[0] in self._widgets:
            return self._widgets[found[0]]
        kind = ""
        if core is not None:
            node = core.query(f"#{id}")
            kind = json.loads(node).get("type", "") if node else ""
        classes: dict[str, type[_Widget]] = {
            "chat": harness.StageChat,
            "markdown": harness.StageMarkdown,
            "log": harness.StageLog,
            "terminal": harness.StageTerminal,
            "diff": harness.StageDiff,
            "toolcall": harness.StageToolCall,
            "prompt": harness.StagePrompt,
            "code": harness.StageCode,
        }
        cls = classes.get(kind)
        if cls is None:
            handle: _Widget = widgets.RemoteWidget(id)
        else:
            handle = object.__new__(cls)
            handle._key = f"#{id}"
            handle._content = None
            handle._children = None
            handle._handlers = {}
            handle.props = {"id": id}
        handle._app = self
        return handle

    def panes(self) -> list[dict[str, Any]]:
        """Every Blender-style pane: ``id``, ``type``, ``bounds``, ``focused``, ``stashed``."""
        core = self._core
        if core is None:
            return []
        tree = json.loads(core.tree())
        out: list[dict[str, Any]] = []

        def walk(nodes: list[dict[str, Any]]) -> None:
            for n in nodes:
                if n.get("type") == "pane":
                    focus = tree.get("focus")
                    inside = focus == n["id"] or any(
                        c.get("id") == focus or _contains(c, focus) for c in n.get("children", [])
                    )
                    out.append(
                        {
                            "id": n["id"],
                            "type": n["pane_type"],
                            "bounds": n["bounds"],
                            "focused": inside,
                            "stashed": n.get("stashed", []),
                        }
                    )
                walk(n.get("children", []))

        walk(tree.get("children", []))
        return out

    async def ask_permission(
        self, question: str, *, tool: str | None = None, args: Any = None
    ) -> str:
        """Ask the user before the agent acts. Returns ``"allow"``, ``"always"`` or ``"deny"``.

        ::

            if await self.ask_permission("Run the tests?", tool="bash", args={"cmd": "pytest"}) != "deny":
                ...
        """
        from .harness import StagePermission
        from .widgets import StageModal

        loop = asyncio.get_running_loop()
        answer: asyncio.Future[str] = loop.create_future()

        def decided(choice: str) -> None:
            if not answer.done():
                answer.set_result(choice)

        def closed() -> None:
            if not answer.done():
                answer.set_result("deny")

        modal = StageModal(
            [StagePermission(question, tool=tool, args=args, on_decide=decided, border=False)],
            title="Permission needed",
            icon="warning",
            accent="amber",
            on_close=closed,
            visible=True,
        )
        self.mount(modal)
        try:
            return await answer
        finally:
            if self._core is not None:
                modal.remove()

    def background(self, work: Awaitable[Any]) -> None:
        """Run ``async`` work alongside the app — say, an agent loop — without
        blocking the screen. It is cancelled when the app stops::

            def on_mount(self):
                self.background(self.watch_files())
        """
        if self._loop is None:
            raise _error(
                "background() needs a running app",
                "the app is not on screen yet",
                "call it from on_mount() or an event handler",
            )
        self._handle(work)

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
        core.start(type(self).__name__, sys.argv[0] if sys.argv else "")
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
            # Freeze the snapshot: handlers still running are cancelled without
            # touching the app (their cleanup sees no running app).
            self._core = None
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
        with self.drive(size, cell=cell) as d:
            for step in steps or ():
                d.step(step)
            return d.snapshot(settle=settle)

    def drive(
        self, size: tuple[int, int] = (80, 24), *, cell: tuple[int, int] = (16, 32)
    ) -> Driver:
        """Start the app without a terminal and control it step by step::

        with MyApp().drive((80, 24)) as d:
            d.type("hello")
            d.press("enter")
            print(d.snapshot().text)
        """
        return Driver(self, size, cell)

    # ----- capturing the live screen (agents, bug reports) ----------------

    def capture_text(self) -> str:
        """The screen right now as plain text (works while the app runs)."""
        return self._capture()[0]

    def capture_tree(self) -> dict[str, Any]:
        """Every widget right now as a dictionary (works while the app runs)."""
        return json.loads(self._capture()[1])

    def capture_png(self) -> bytes:
        """The screen right now as PNG bytes (works while the app runs)."""
        core = self._require_core("capture_png()")
        tree = json.loads(core.tree())
        return core.png(tree["size"]["cols"], tree["size"]["rows"])

    def _capture(self) -> tuple[str, str]:
        core = self._require_core("capture_text()")
        tree = json.loads(core.tree())
        return core.snapshot(tree["size"]["cols"], tree["size"]["rows"])

    # ----- internals -------------------------------------------------------

    def _spec(self) -> dict[str, Any]:
        children = self.compose()
        if not isinstance(children, (list, tuple)):
            raise TypeError("compose() must return a list of widgets")
        for c in children:
            if not isinstance(c, _Widget):
                raise TypeError(f"compose() returned {c!r}, which is not a stagetui widget")
        bindings = []
        for b in self.BINDINGS:
            if not isinstance(b, (tuple, list)) or len(b) not in (2, 3):
                raise TypeError(f'BINDINGS entries look like ("ctrl+s", "save", "Save"), got {b!r}')
            entry = {"key": b[0], "action": b[1]}
            if len(b) == 3:
                entry["description"] = b[2]
            bindings.append(entry)
        from .harness import _pane_types_spec

        spec: dict[str, Any] = {
            "theme": self.theme,
            "children": [c._spec() for c in children],
            "bindings": bindings,
            "pane_types": _pane_types_spec(),
            "prefix": self.PREFIX,
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
            elif name == "pane":
                self._pane_event(f"#{_id}", None, value)
            return True
        widget = self._widgets.get(key)
        if widget is None:
            return True
        if name == "pane":
            self._pane_event(key, widget, value)
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

    def _pane_event(self, target: str, widget: _Widget | None, value: Mapping[str, Any]) -> None:
        """A pane switched type; build custom content when stagetui can't."""
        from .harness import _build_pane_content

        kind = str(value.get("type", ""))
        core = self._core
        if not value.get("built") and core is not None:
            content = _build_pane_content(kind, dict(value.get("args") or {}))
            self._bind(content)
            core.pane_set(target, kind, json.dumps(content._spec(), ensure_ascii=False), False)
        if widget is not None:
            widget._content = kind
            handler = widget._handlers.get("pane")
            if handler is not None:
                self._handle(_call(handler, kind))

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
        """Handle every pending event; let async handlers run (headless: briefly)."""
        core = self._core
        assert core is not None
        for _ in range(1000):
            events = core.poll_events()
            for e in events:
                self._dispatch(e)
            if not self._tasks:
                if not events:
                    return
                continue
            pending = list(self._tasks)
            loop.run_until_complete(asyncio.wait(pending, timeout=_HEADLESS_TASK_BUDGET))
            self._raise_failure()
            if not events and all(not t.done() for t in pending):
                more = core.poll_events()
                for e in more:
                    self._dispatch(e)
                if not more:
                    return  # waiting on the user (a permission prompt…): next step

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


class Driver:
    """An app running without a terminal, controlled one step at a time.

    Made by :meth:`StageApp.drive`. Every action runs the app's handlers and lets
    animations advance on a virtual clock, exactly like ``run_headless(steps=...)``.
    Used by ``stagetui drive``, ``stagetui mcp`` and the pytest plugin.
    """

    def __init__(self, app: StageApp, size: tuple[int, int], cell: tuple[int, int]) -> None:
        self.app = app
        self.cols, self.rows = size
        self.cell = cell
        self.core = app._create(cell=cell)
        app._headless = True
        self.loop = asyncio.new_event_loop()
        app._loop = self.loop
        self.closed = False
        try:
            self._render()
            self.loop.run_until_complete(app._await(app.on_mount()))
            self._render()
        except BaseException:
            self.close()
            raise

    def _render(self) -> None:
        self.core.render(self.cols, self.rows)
        self.app._drain(self.loop)
        self.core.render(self.cols, self.rows)  # lay out what the handlers changed

    # ----- actions -------------------------------------------------------

    def step(self, step: Step) -> None:
        """Run one step dict, e.g. ``{"press": "tab"}`` (see ``run_headless``)."""
        self._check()
        self.cols, self.rows = self.app._step(self.core, step, self.cols, self.rows)
        self._render()

    def press(self, key: str) -> None:
        self.step({"press": key})

    def type(self, text: str) -> None:
        self.step({"type": text})

    def click(self, target: str | tuple[int, int]) -> None:
        """Click a widget (``"#send"``) or a cell (``(x, y)``)."""
        self.step({"click": list(target) if isinstance(target, tuple) else target})

    def scroll(self, rows: int, target: str | None = None) -> None:
        self.step({"scroll": {"rows": rows, "target": target}})

    def wait(self, seconds: float) -> None:
        """Let (virtual) time pass: animations, timers, live canvases."""
        self.step({"wait": seconds})

    def wait_idle(self) -> None:
        """Let entrance animations finish."""
        self.step({"wait_idle": True})

    def resize(self, cols: int, rows: int) -> None:
        self.step({"resize": [cols, rows]})

    # ----- looking -------------------------------------------------------

    def snapshot(self, *, settle: bool = False, png: bool = False) -> Snapshot:
        """The screen now. ``png=True`` renders the picture immediately."""
        self._check()
        if settle:
            self.core.settle(self.cols, self.rows)
            self.app._drain(self.loop)
        text, tree = self.core.snapshot(self.cols, self.rows)
        snap = Snapshot(text, json.loads(tree), self.core, self.cell)
        if png:
            snap._png = self.core.png(self.cols, self.rows)
        return snap

    def query(self, id: str) -> dict[str, Any] | None:
        """One widget as a dictionary, or ``None``."""
        node = self.core.query(id if id.startswith("#") else f"#{id}")
        return json.loads(node) if node else None

    def panes(self) -> list[dict[str, Any]]:
        return self.app.panes()

    # ----- lifecycle -----------------------------------------------------

    def _check(self) -> None:
        if self.closed:
            raise _error(
                "This driver is closed",
                "the app it controlled has stopped",
                "start a new one with app.drive()",
            )

    def close(self) -> None:
        """Stop the app. Handlers still running are cancelled without touching it."""
        if self.closed:
            return
        self.closed = True
        app = self.app
        app._core = None
        for t in app._tasks:
            t.cancel()
        if app._tasks:
            self.loop.run_until_complete(asyncio.gather(*app._tasks, return_exceptions=True))
        app._tasks.clear()
        self.loop.close()
        app._loop = None
        app._headless = False

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()
