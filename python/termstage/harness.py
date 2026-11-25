"""The harness kit: widgets for building AI coding-agent apps, and Blender-style panes.

- :class:`StageChat`, :class:`StageMarkdown`, :class:`StagePrompt` — the conversation.
- :class:`StageCode`, :class:`StageDiff` — code and proposed changes.
- :class:`StageToolCall`, :class:`StagePermission` — what the agent does, and asking first.
- :class:`StageTerminal`, :class:`StageLog` — a real shell, and fast logs.
- :class:`StagePane` and :func:`pane_type` — panes whose content you can switch.

All behaviour lives in Rust; these classes describe widgets and forward calls.
"""

from __future__ import annotations

import json
import os
from collections.abc import Callable, Iterable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .widgets import Animate, Dir, Handler, Size, Theme, _error, _text, _Widget

# ----- helpers ------------------------------------------------------------------


class _Callable(_Widget):
    """A widget with Rust-side methods (`append`, `add`, …)."""

    def _call(self, method: str, args: Any = None) -> Any:
        core = self._core()
        if core is None:
            return self._offline(method, args)
        return json.loads(core.call(self._key, method, json.dumps(args, ensure_ascii=False)))

    def _offline(self, method: str, args: Any) -> Any:
        raise _error(
            f"{type(self).__name__}.{method}() needs a running app",
            "this widget is not on screen yet",
            "set the starting content in the constructor, or call it from a handler / on_mount()",
        )


# ----- conversation -------------------------------------------------------------


class StageMarkdown(_Callable):
    """Markdown text: headings, **bold**, `code`, lists, tables, code blocks with colors.

    Made for streaming: ``md.append(chunk)`` adds text as an AI writes it, and the
    view follows the end until you scroll up.
    """

    _type = "markdown"

    def __init__(
        self,
        text: str = "",
        *,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
        width: Size | None = None,
        height: int | None = None,
        grow: float | None = None,
    ) -> None:
        super().__init__(
            _text(text, "StageMarkdown"),
            id=id,
            theme=theme,
            accent=accent,
            animate=animate,
            visible=visible,
            width=width,
            height=height,
            grow=grow,
        )

    def append(self, text: str) -> None:
        """Add text at the end (streaming)."""
        if self._core() is None:
            self._content = (self._content or "") + text
            return
        self._call("append", text)

    def clear(self) -> None:
        """Remove all text."""
        if self._core() is None:
            self._content = ""
            return
        self._call("clear")


class ChatMessage:
    """One message in a :class:`StageChat` (returned by ``chat.add(...)``)."""

    def __init__(self, chat: StageChat, index: int) -> None:
        self.chat = chat
        self.index = index

    def append(self, text: str) -> None:
        """Add text to this message (streaming), or output to a tool call."""
        self.chat._message("append", self.index, text=text)

    def update(self, text: str) -> None:
        """Replace this message's text."""
        self.chat._message("set", self.index, text=text)

    def finish(self, output: str = "", *, ok: bool = True) -> None:
        """Mark a tool call as done (✓) or failed (✗), with its output."""
        self.chat._message("tool", self.index, status="done" if ok else "error", output=output)

    def __repr__(self) -> str:
        return f"ChatMessage(index={self.index})"


class StageChat(_Callable):
    """A conversation: your messages, the assistant's (Markdown, streamed), and tool calls.

    ::

        chat = StageChat(id="chat")
        msg = chat.add("assistant")
        msg.append("Hello")          # streams in
        tool = chat.add_tool("read_file", {"path": "main.py"})
        tool.finish("42 lines")

    Keys: ↑/↓ select a message, ``y`` copies it, Enter opens a tool call, End follows new messages.
    """

    _type = "chat"

    def __init__(
        self,
        messages: Sequence[Mapping[str, Any]] = (),
        *,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
        width: Size | None = None,
        height: int | None = None,
        grow: float | None = None,
        on_copy: Handler | None = None,
    ) -> None:
        super().__init__(
            [dict(m) for m in messages],
            id=id,
            theme=theme,
            accent=accent,
            animate=animate,
            visible=visible,
            width=width,
            height=height,
            grow=grow,
            handlers={"copy": on_copy},
        )

    def add(self, role: str, text: str = "", *, name: str | None = None) -> ChatMessage:
        """Add a message. ``role`` is ``"user"``, ``"assistant"`` or ``"system"``."""
        msg: dict[str, Any] = {"role": role, "text": text}
        if name is not None:
            msg["name"] = name
        return self._add(msg)

    def add_tool(self, name: str, args: Any = None) -> ChatMessage:
        """Show a tool call (spinning until ``.finish()``)."""
        return self._add({"role": "tool", "tool": {"name": name, "args": args}})

    def _add(self, msg: dict[str, Any]) -> ChatMessage:
        if self._core() is None:
            self._content.append(msg)
            return ChatMessage(self, len(self._content) - 1)
        return ChatMessage(self, int(self._call("add", msg)))

    def _message(self, method: str, index: int, **fields: Any) -> None:
        if self._core() is None:
            m = self._content[index]
            if method == "append" and "tool" in m:
                m["tool"]["output"] = m["tool"].get("output", "") + fields["text"]
            elif method == "append":
                m["text"] = m.get("text", "") + fields["text"]
            elif method == "set":
                m["text"] = fields["text"]
            else:
                m.setdefault("tool", {}).update(fields)
            return
        self._call(method, {"index": index, **fields})

    def clear(self) -> None:
        """Remove every message."""
        if self._core() is None:
            self._content = []
            return
        self._call("clear")

    @property
    def messages(self) -> list[dict[str, Any]]:
        """All messages as dicts (``role``, ``text``)."""
        return list(self.value)


class StagePrompt(_Callable):
    """Where you talk to the agent: multi-line text, history, commands and file mentions.

    - **Enter** sends (``on_submit(text)``); **Shift+Enter**, **Alt+Enter** or **Ctrl+J** adds a line.
    - **↑ / ↓** on the first / last line walk through what you sent before.
    - Type ``/`` for ``commands=[("help", "Show help"), ...]``; type ``@`` to mention a file.
    - Paste works, including many lines.
    """

    _type = "prompt"

    def __init__(
        self,
        text: str = "",
        *,
        placeholder: str | None = None,
        title: str | None = None,
        commands: Iterable[tuple[str, str]] | None = None,
        mentions: Iterable[str] | None = None,
        max_lines: int | None = None,
        on_submit: Handler | None = None,
        on_change: Handler | None = None,
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
            _text(text, "StagePrompt"),
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
            commands=[list(c) for c in commands] if commands is not None else None,
            mentions=list(mentions) if mentions is not None else None,
            max_lines=max_lines,
            handlers={"submit": on_submit, "change": on_change},
        )

    def clear(self) -> None:
        """Empty the prompt."""
        self.update("")


# ----- code ------------------------------------------------------------------------


class StageCode(_Widget):
    """Source code with syntax colors and line numbers.

    ``StageCode(source, language="python")`` — or give ``path="app.py"`` and the
    language is guessed from the name. Use ``StageCode.from_file("app.py")`` to load a file.
    """

    _type = "code"

    def __init__(
        self,
        source: str = "",
        *,
        language: str | None = None,
        path: str | None = None,
        line_numbers: bool = True,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
        width: Size | None = None,
        height: int | None = None,
        grow: float | None = None,
    ) -> None:
        super().__init__(
            _text(source, "StageCode"),
            id=id,
            theme=theme,
            accent=accent,
            animate=animate,
            visible=visible,
            width=width,
            height=height,
            grow=grow,
            language=language,
            path=path,
            line_numbers=line_numbers,
        )

    @classmethod
    def from_file(cls, path: str | os.PathLike[str], **options: Any) -> StageCode:
        """Show a file's contents."""
        p = Path(path)
        return cls(p.read_text(encoding="utf-8", errors="replace"), path=str(path), **options)


class StageDiff(_Callable):
    """Proposed changes to a text, hunk by hunk. Each hunk can be accepted or rejected.

    ``StageDiff((old_text, new_text), path="app.py")``. Keys: **a** accept, **r** reject,
    **A** accept all, **j/k** next/previous hunk. ``diff.result`` is the text with the
    accepted hunks applied. ``on_decide({"hunk": 0, "decision": "accepted"})`` runs on each choice.
    """

    _type = "diff"

    def __init__(
        self,
        changes: tuple[str, str] | None = None,
        *,
        path: str | None = None,
        on_decide: Handler | None = None,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
        width: Size | None = None,
        height: int | None = None,
        grow: float | None = None,
    ) -> None:
        super().__init__(
            self._pair(changes),
            id=id,
            theme=theme,
            accent=accent,
            animate=animate,
            visible=visible,
            width=width,
            height=height,
            grow=grow,
            path=path,
            handlers={"decide": on_decide},
        )

    @staticmethod
    def _pair(changes: Any) -> list[str] | None:
        if changes is None:
            return None
        if isinstance(changes, (tuple, list)) and len(changes) == 2:
            return [str(changes[0]), str(changes[1])]
        raise TypeError("StageDiff needs (old_text, new_text), like StageDiff((before, after))")

    def load(self, content: tuple[str, str]) -> None:
        """Show new changes: ``diff.load((old_text, new_text))``."""
        self.update(self._pair(content))

    def accept_all(self) -> None:
        self._call("accept_all")

    def reject_all(self) -> None:
        self._call("reject_all")

    def decide(self, hunk: int, decision: str) -> None:
        """``decision`` is ``"accept"`` or ``"reject"``."""
        self._call("decide", {"hunk": hunk, "decision": decision})

    @property
    def result(self) -> str:
        """The new text with only the accepted hunks applied."""
        return str(self.value["result"])


# ----- agent actions -------------------------------------------------------------------


class StageToolCall(_Callable):
    """A collapsible card for a tool the agent ran: name, arguments, status, output.

    ``tool = StageToolCall("read_file", args={"path": "main.py"})`` spins until
    ``tool.finish(output)`` (✓) or ``tool.finish(error, ok=False)`` (✗). Enter or a click opens it.
    """

    _type = "toolcall"

    def __init__(
        self,
        name: str,
        *,
        args: Any = None,
        status: str = "running",
        output: str = "",
        expanded: bool = False,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
    ) -> None:
        super().__init__(
            _text(name, "StageToolCall"),
            id=id,
            theme=theme,
            accent=accent,
            animate=animate,
            visible=visible,
            args=args,
            status=status,
            output=output,
            expanded=expanded,
        )

    def finish(self, output: str = "", *, ok: bool = True) -> None:
        self._call("finish", {"status": "done" if ok else "error", "output": output})

    def append(self, text: str) -> None:
        """Add output while it runs."""
        self._call("append", text)


class StagePermission(_Widget):
    """ "May the agent do this?" — Allow (**y**), Always allow (**a**) or Deny (**n**).

    ``on_decide(choice)`` receives ``"allow"``, ``"always"`` or ``"deny"``. The easy way
    to use it is ``await app.ask_permission(...)``.
    """

    _type = "permission"

    def __init__(
        self,
        question: str,
        *,
        tool: str | None = None,
        args: Any = None,
        on_decide: Handler | None = None,
        border: bool = True,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
    ) -> None:
        super().__init__(
            _text(question, "StagePermission"),
            border=border,
            id=id,
            theme=theme,
            accent=accent,
            animate=animate,
            visible=visible,
            tool=tool,
            args=args,
            handlers={"decide": on_decide},
        )


# ----- shell & logs ----------------------------------------------------------------


class StageTerminal(_Callable):
    """A real terminal running your shell — or any command: ``StageTerminal("htop")``.

    While it has focus it gets **every** key (so Ctrl+C, Esc and Tab work in it). To
    reach the app, press the prefix key **Ctrl+Space** first, then e.g. **Tab** to move
    to another pane or **q** to quit. The program stops when the app exits.
    """

    _type = "terminal"

    def __init__(
        self,
        command: str | Sequence[str] = "",
        *,
        cwd: str | os.PathLike[str] | None = None,
        title: str | None = None,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
        width: Size | None = None,
        height: int | None = None,
        grow: float | None = None,
    ) -> None:
        content: Any = command if isinstance(command, str) else [str(c) for c in command]
        super().__init__(
            content,
            id=id,
            title=title,
            theme=theme,
            accent=accent,
            animate=animate,
            visible=visible,
            width=width,
            height=height,
            grow=grow,
            cwd=os.fspath(cwd) if cwd is not None else None,
        )

    def send(self, text: str) -> None:
        """Type text into the terminal (``"ls\\n"`` runs ``ls``)."""
        self._call("send", text)

    def restart(self) -> None:
        self._call("restart")

    @property
    def screen(self) -> str:
        """What the terminal shows right now, as text."""
        return str(self._call("screen"))


class StageLog(_Callable):
    """A fast log view: ``log.append(line)`` thousands of times a second is fine.

    Lines with ERROR / WARN / DEBUG / OK are colored. ``log.filter("error")`` shows only
    matching lines. Keeps the last ``max_lines`` lines (default 5000).
    """

    _type = "log"

    def __init__(
        self,
        text: str = "",
        *,
        max_lines: int | None = None,
        filter: str | None = None,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        animate: Animate | None = None,
        visible: bool | None = None,
        width: Size | None = None,
        height: int | None = None,
        grow: float | None = None,
    ) -> None:
        super().__init__(
            _text(text, "StageLog"),
            id=id,
            theme=theme,
            accent=accent,
            animate=animate,
            visible=visible,
            width=width,
            height=height,
            grow=grow,
            max_lines=max_lines,
            filter=filter,
        )

    def append(self, text: str | Sequence[str]) -> None:
        if self._core() is None:
            lines = [text] if isinstance(text, str) else list(text)
            self._content = (
                "\n".join([self._content, *lines]) if self._content else "\n".join(lines)
            )
            return
        self._call("append", text if isinstance(text, str) else list(text))

    def clear(self) -> None:
        self._call("clear")

    def filter(self, text: str) -> None:
        """Show only lines containing ``text`` (``""`` shows everything)."""
        self._call("filter", text)


# ----- Blender-style panes --------------------------------------------------------


@dataclass(frozen=True, slots=True)
class PaneType:
    name: str
    title: str
    icon: str
    build: Callable[..., _Widget]
    builtin: bool


_PANE_TYPES: dict[str, PaneType] = {}


def pane_type(
    name: str, *, title: str | None = None, icon: str = "cube"
) -> Callable[[Callable[..., _Widget]], Callable[..., _Widget]]:
    """Register your own pane type, offered in every pane's menu.

    ::

        @ts.pane_type("weather", title="Weather", icon="sun")
        def weather_pane(city="Kuwait"):
            return StageCard(f"Sunny in {city}")

        StagePane("weather", city="Riyadh")
    """

    def register(fn: Callable[..., _Widget]) -> Callable[..., _Widget]:
        _PANE_TYPES[name] = PaneType(name, title or name.title(), icon, fn, builtin=False)
        return fn

    return register


def _files(root: str = ".", depth: int = 3) -> dict[str, Any]:
    skip = {"node_modules", "target", "__pycache__", "dist", "build", ".venv", "venv"}

    def walk(p: Path, d: int) -> dict[str, Any]:
        out: dict[str, Any] = {}
        try:
            entries = sorted(p.iterdir(), key=lambda e: (not e.is_dir(), e.name))
        except OSError:
            return out
        for e in entries[:300]:
            if e.name.startswith(".") or e.name in skip:
                continue
            out[e.name + ("/" if e.is_dir() else "")] = (
                walk(e, d - 1) if e.is_dir() and d > 1 else None
            )
        return out

    return walk(Path(root), depth)


def _builtin(name: str, title: str, icon: str) -> Callable[[Callable[..., _Widget]], None]:
    def register(fn: Callable[..., _Widget]) -> None:
        _PANE_TYPES[name] = PaneType(name, title, icon, fn, builtin=True)

    return register


def _register_builtins() -> None:
    from .widgets import StageText, StageTree

    _builtin("chat", "Chat", "chat")(lambda **_: StageChat())
    _builtin("terminal", "Terminal", "terminal")(
        lambda command="", cwd=None, **_: StageTerminal(command, cwd=cwd)
    )
    _builtin("files", "Files", "folder")(lambda root=".", **_: StageTree(_files(root)))
    _builtin("diff", "Changes", "code")(
        lambda old=None, new=None, path=None, **_: StageDiff(
            (old, new) if old is not None and new is not None else None, path=path
        )
    )
    _builtin("code", "Code", "file")(
        lambda path=None, text="", language=None, **_: (
            StageCode.from_file(path, language=language)
            if path
            else StageCode(text, language=language)
        )
    )
    _builtin("markdown", "Notes", "book")(
        lambda text="*Nothing here yet.*", **_: StageMarkdown(text)
    )
    _builtin("log", "Log", "info")(lambda **_: StageLog())
    _builtin("empty", "Empty", "circle")(
        lambda **_: StageText(
            "Empty pane — click its title (or press Ctrl+Space, then Space) to choose what it shows."
        )
    )


_register_builtins()


def _pane_types_spec() -> list[dict[str, Any]]:
    return [
        {"name": t.name, "title": t.title, "icon": t.icon, "custom": not t.builtin}
        for t in _PANE_TYPES.values()
    ]


def _build_pane_content(name: str, args: Mapping[str, Any]) -> _Widget:
    t = _PANE_TYPES.get(name)
    if t is None:
        raise _error(
            f'Unknown pane type "{name}"',
            "pane types are the built-in ones plus those you register with @ts.pane_type",
            "use one of: " + ", ".join(_PANE_TYPES),
        )
    widget = t.build(**args)
    if not isinstance(widget, _Widget):
        raise TypeError(f"pane type {name!r} must return a termstage widget, got {widget!r}")
    return widget


class StagePane(_Widget):
    """A Blender-style pane: its header has a menu to switch what it shows.

    ``StagePane("chat")``, ``StagePane("terminal", command="htop")``,
    ``StagePane("files", root="src")``, ``StagePane("code", path="app.py")`` — or any type
    you registered with :func:`pane_type`. Put panes in a :class:`StageSplit`.

    Click the title (or press **Ctrl+Space** then **Space**) to switch type, split or close.
    Switching away keeps the old content running; switching back restores it.
    """

    _type = "pane"

    def __init__(
        self,
        type: str = "empty",
        *,
        id: str | None = None,
        theme: Theme | None = None,
        accent: str | None = None,
        visible: bool | None = None,
        width: Size | None = None,
        height: Size | None = None,
        grow: float | None = None,
        on_change: Handler | None = None,
        **args: Any,
    ) -> None:
        content = _build_pane_content(type, args)
        super().__init__(
            type,
            id=id,
            theme=theme,
            accent=accent,
            visible=visible,
            width=width,
            height=height,
            grow=grow,
            children=[content],
            handlers={"pane": on_change},
        )

    @property
    def type(self) -> str:
        """The pane type on show."""
        core = self._core()
        if core is None:
            return str(self._content)
        node = core.query(self._key)
        return str(json.loads(node)["pane_type"]) if node else str(self._content)

    @property
    def content(self) -> _Widget | None:
        """The widget the pane shows now (if termstage knows it)."""
        app = self._app
        core = self._core()
        if app is None or core is None:
            return self._children[0] if self._children else None
        node = core.query(self._key)
        if not node:
            return None
        info = json.loads(node)
        for child in info.get("children", []):
            if child.get("on_screen"):
                return app._remote(child["id"])
        return None

    def set_type(self, type: str, **args: Any) -> _Widget:
        """Show another kind of content: ``pane.set_type("terminal", command="htop")``."""
        core = self._require_running("set_type()")
        widget = _build_pane_content(type, args)
        assert self._app is not None
        self._app._bind(widget)
        core.pane_set(self._key, type, json.dumps(widget._spec(), ensure_ascii=False), True)
        self._content = type
        return widget

    def switch(self, type: str) -> None:
        """Switch type, restoring the stashed content if there is one."""
        self._require_running("switch()").switch_pane(self._key, type)
