"""Write your own pane type as a plugin — in plain Python.

A plugin is a small program that draws pictures for a pane. stagetui starts it
when a pane of its type opens, shows every frame it draws, and sends it keys
and mouse clicks. Save this as a file named ``stagetui-plugin-<name>`` (make it
executable), put it on your ``PATH`` (or in ``STAGETUI_PLUGIN_PATH``), and the
new type appears in every pane's menu::

    #!/usr/bin/env python3
    import math
    from stagetui import Canvas
    from stagetui.plugin import Plugin, serve

    class Pulse(Plugin):
        name, title, icon = "pulse", "Pulse", "heart"

        def frame(self, t):
            c = Canvas(self.width, self.height, "#16161e")
            r = min(self.width, self.height) / 3 * (1 + 0.2 * math.sin(t * 3))
            c.circle(self.width / 2, self.height / 2, r, fill="#f7768e")
            return c

    serve(Pulse())

Name what its keys do in ``actions`` and the app can give them other keys —
``StagePane("pulse", keys={"bigger": "b"})`` — or turn them off (``None``)::

    actions = {"bigger": ("+", "="), "smaller": ("-",)}

    def action(self, name):        # "bigger" or "smaller", whatever key the app chose
        ...

The protocol is the same as the Rust crate ``stagetui-plugin-api``.
"""

from __future__ import annotations

import json
import os
import struct
import sys
import threading
import time
from collections.abc import Mapping, Sequence
from queue import Empty, Queue
from typing import Any, BinaryIO, ClassVar

from ._core import Canvas

PROTOCOL = 1
_JSON, _FRAME = 1, 2


def _write(out: BinaryIO, kind: int, payload: bytes) -> None:
    out.write(struct.pack("<IB", len(payload) + 1, kind))
    out.write(payload)
    out.flush()


def _read_exact(fd: int, n: int) -> bytes | None:
    buf = bytearray()
    while len(buf) < n:
        chunk = os.read(fd, n - len(buf))
        if not chunk:
            return None
        buf += chunk
    return bytes(buf)


def _read(fd: int) -> tuple[int, bytes] | None:
    # Raw os.read, not sys.stdin.buffer: a thread blocked in a buffered read
    # holds its lock, and Python 3.14 aborts at exit if it can't take that lock.
    head = _read_exact(fd, 4)
    if head is None:
        return None
    (size,) = struct.unpack("<I", head)
    body = _read_exact(fd, size) if size else None
    if not body:
        return None
    return body[0], body[1:]


# ----- keys the app can rename -------------------------------------------------

# Key names stagetui understands, and the name it sends for each.
_KEY_NAMES = {
    "enter": "enter",
    "return": "enter",
    "escape": "esc",
    "esc": "esc",
    "tab": "tab",
    "backspace": "backspace",
    "delete": "delete",
    "del": "delete",
    "insert": "insert",
    "up": "up",
    "down": "down",
    "left": "left",
    "right": "right",
    "home": "home",
    "end": "end",
    "pageup": "pageup",
    "page_up": "pageup",
    "pagedown": "pagedown",
    "page_down": "pagedown",
    "space": "space",
    "backslash": "\\",
    "slash": "/",
    "comma": ",",
    "period": ".",
    "minus": "-",
    "plus": "+",
    "equals": "=",
    "semicolon": ";",
    "left_bracket": "[",
    "right_bracket": "]",
}
_MODS = {
    "ctrl": "ctrl",
    "control": "ctrl",
    "alt": "alt",
    "option": "alt",
    "meta": "alt",
    "super": "super",
    "cmd": "super",
    "win": "super",
    "shift": "shift",
}
_ARROWS = {"left": "←", "right": "→", "up": "↑", "down": "↓"}


def key_name(text: str) -> str | None:
    """A key written the way stagetui names the keys it sends (``"Ctrl+s"`` →
    ``"ctrl+s"``, ``"plus"`` → ``"+"``, ``"Escape"`` → ``"esc"``); ``None`` if it
    isn't a key. As in an app's ``BINDINGS``, a capital letter means Shift:
    ``"W"`` is ``"shift+w"``."""
    lower = text.strip().lower()
    if not lower:
        return None
    # A lone "+" is the plus key; otherwise "+" joins the parts.
    if lower == "+":
        parts = ["+"]
    elif lower.endswith("++"):
        parts = [*lower[:-2].split("+"), "+"]
    else:
        parts = lower.split("+")
    *held, last = parts
    if any(m not in _MODS for m in held):
        return None
    mods = {_MODS[m] for m in held}
    digits = last[1:]
    if last in _KEY_NAMES:
        base = _KEY_NAMES[last]
    elif last.startswith("f") and digits.isascii() and digits.isdigit():
        if not 1 <= int(digits) <= 24:
            return None
        base = f"f{int(digits)}"
    elif len(last) == 1:
        if text.strip()[-1:].isupper():
            mods.add("shift")  # "W" is Shift+W, like stagetui reads it everywhere
        elif not last.isalpha() and last != " ":
            mods.discard("shift")  # symbols say shift already ("?", "+")
        base = last
    else:
        return None
    return "".join(f"{m}+" for m in ("ctrl", "alt", "super", "shift") if m in mods) + base


def _short(key: str) -> str:
    """``"shift+left"`` → ``"shift+←"``; other keys stay as they are."""
    if key == "+":
        return key
    if key.endswith("++"):
        return key
    held, _, base = key.rpartition("+")
    return (held + "+" if held else "") + _ARROWS.get(base, base)


_SPOKEN = {
    "space": "Space",
    "enter": "Enter",
    "esc": "Esc",
    "tab": "Tab",
    "backspace": "Backspace",
    "delete": "Del",
    "insert": "Ins",
    "home": "Home",
    "end": "End",
    "pageup": "PgUp",
    "pagedown": "PgDn",
    **_ARROWS,
}


def _spoken(key: str) -> str:
    """``"ctrl+o"`` → ``"Ctrl+O"``, ``"enter"`` → ``"Enter"``, ``"shift+left"`` → ``"Shift+←"``."""
    if key == "+":
        held, base = "", "+"
    elif key.endswith("++"):
        held, base = key[:-2], "+"
    else:
        held, _, base = key.rpartition("+")
    mods = [m for m in held.split("+") if m]
    if base in _SPOKEN:
        base = _SPOKEN[base]
    elif len(base) > 1 and base.startswith("f"):
        base = "F" + base[1:]
    elif "ctrl" in mods or "alt" in mods:
        base = base.upper()
    return "".join(m[:1].upper() + m[1:] + "+" for m in mods) + base


def _json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


class Keymap:
    """A plugin's keys after the app's changes (the same as the Rust ``Keymap``).

    ``actions`` names what keys do and their usual keys, ``{"spin": ("space",)}``;
    ``args["keys"]`` (from ``StagePane(..., keys={"spin": "s", "grid": None})``) changes
    them. A name that isn't an action, a key stagetui doesn't know, or one key given to
    two actions raises ``ValueError`` (``TypeError`` for something that isn't a key or a
    dict at all) with a message for the app's author.
    """

    def __init__(
        self,
        actions: Mapping[str, str | Sequence[str]],
        args: Mapping[str, Any] | None = None,
    ) -> None:
        self._map: dict[str, list[str]] = {
            name: [key_name(k) or k.lower() for k in ((keys,) if isinstance(keys, str) else keys)]
            for name, keys in actions.items()
        }
        names = ", ".join(self._map)
        given = (args or {}).get("keys")
        if isinstance(given, Mapping):
            for name, value in given.items():
                if name not in self._map:
                    raise ValueError(
                        f'keys= names "{name}", but there is no action by that name; '
                        f"the actions are: {names}"
                    )
                self._map[name] = self._given(name, value)
        elif given is not None:
            first = next(iter(self._map), "name")
            raise TypeError(
                f'keys= should name actions and their keys, like keys={{"{first}": "x"}}, '
                f"not {_json(given)}; the actions are: {names}"
            )
        for name, keys in self._map.items():
            self._map[name] = list(dict.fromkeys(keys))
        pairs = list(self._map.items())
        for i, (first, keys) in enumerate(pairs):
            for key in keys:
                for second, others in pairs[i + 1 :]:
                    if key in others:
                        raise ValueError(
                            f'"{key}" is given to both "{first}" and "{second}"; give one of '
                            f'them another key (keys={{"{second}": "…"}}) or turn it off '
                            f'(keys={{"{second}": None}})'
                        )

    @staticmethod
    def _given(action: str, value: Any) -> list[str]:
        def one(v: Any) -> str:
            if not isinstance(v, str):
                raise TypeError(
                    f'keys={{"{action}": {_json(value)}}}: give a key like "x", "space" or '
                    f'"ctrl+s", a list of keys, or None to turn it off'
                )
            name = key_name(v)
            if name is None:
                raise ValueError(
                    f'"{v}" (for "{action}") isn\'t a key stagetui knows; keys are written '
                    f'like "x", "space", "ctrl+s", "shift+left", "f5" or "+"'
                )
            return name

        if value is None:
            return []
        if isinstance(value, list | tuple):
            return [one(v) for v in value]
        return [one(value)]

    def action(self, key: str) -> str | None:
        """The action a key does (``None``: none, or it was turned off)."""
        key = key_name(key) or key.lower()
        return next((name for name, keys in self._map.items() if key in keys), None)

    def keys(self, action: str) -> list[str]:
        """The keys an action has now (empty: turned off, or no such action)."""
        return list(self._map.get(action, ()))

    def label(self, action: str) -> str | None:
        """The action's first key, short for a status line (arrows as ``← → ↑ ↓``);
        ``None`` when the action is off."""
        keys = self._map.get(action)
        return _short(keys[0]) if keys else None

    def spoken(self, action: str) -> str | None:
        """The action's first key written for a sentence, the way stagetui shows keys
        (``"Enter"``, ``"Ctrl+O"``, ``"Shift+←"``): "press Enter to …". ``None`` when off."""
        keys = self._map.get(action)
        return _spoken(keys[0]) if keys else None

    def all_keys(self) -> list[str]:
        """Every key of every action (stagetui sends these on to the plugin)."""
        return [k for keys in self._map.values() for k in keys]


# ----- writing a plugin ------------------------------------------------------------


class Plugin:
    """Override what you need. ``frame`` is the heart of it."""

    #: The pane type (also the end of the file name: ``stagetui-plugin-<name>``).
    name = "plugin"
    #: Shown in pane menus.
    title = "Plugin"
    #: One of stagetui's icon names.
    icon = "sparkles"
    #: Most frames per second.
    fps = 30
    #: Keys this plugin wants while its pane has focus, e.g. ``["space", "left"]``.
    keys: tuple[str, ...] = ()
    #: What its keys do, with their usual keys: ``{"spin": ("space",), "reset": ("r", "home")}``.
    #: The app can change them (``StagePane(..., keys={"spin": "s", "reset": None})``);
    #: these keys go to :meth:`action`, not :meth:`key`.
    actions: ClassVar[Mapping[str, str | tuple[str, ...]]] = {}
    #: Wants every key typed (like a text box).
    text = False
    #: Draws pictures (so it is best in a terminal with picture support).
    pictures = True

    def __init__(self) -> None:
        self.width = 1
        self.height = 1
        self.args: dict[str, Any] = {}
        #: Which key does what, after the app's ``keys={...}`` (ready before ``start``).
        self.keymap = Keymap({})
        self._out: BinaryIO | None = None

    # ----- override these --------------------------------------------------

    def start(self, args: dict[str, Any]) -> None:
        """The pane opened; ``args`` are its options. Raise to refuse with a message."""

    def resize(self, width: int, height: int) -> None:
        """The pane changed size (``self.width`` / ``self.height`` are already updated)."""

    def frame(self, t: float) -> Canvas | bytes | None:
        """Draw: return a ``Canvas`` (or RGBA bytes) — or ``None`` when nothing changed."""
        return None

    def key(self, key: str, text: str | None) -> None:
        """A key from ``keys`` (or any key with ``text = True``) that isn't an action's."""

    def action(self, name: str) -> None:
        """A key of one of ``actions`` was pressed — whichever key the app gave it."""

    def paste(self, text: str) -> None:
        """Text was pasted into the pane (or a file dragged in)."""

    def mouse(self, kind: str, x: int, y: int, button: str | None) -> None:
        """``kind``: down, up, drag, move, scroll_up, scroll_down. ``x``/``y`` in pixels."""

    def call(self, method: str, args: Any) -> Any:
        """The app called ``pane.content.call(method, args)``."""
        raise ValueError(f'this plugin has no method "{method}"')

    def shown(self, shown: bool) -> None:
        """The pane went out of sight (``False``: another pane type or tab is
        showing) or came back (``True``). ``frame`` isn't called while hidden:
        pause anything that makes sound or costs work."""

    def stop(self) -> None:
        """The pane is closing."""

    # ----- use these -------------------------------------------------------

    def status(self, text: str) -> None:
        """A short line at the bottom of the pane, e.g. ``"▶ 01:23 / 04:56"``."""
        self._send({"t": "status", "text": text})

    def event(self, name: str, value: Any = None) -> None:
        """Tell the app something happened (``on_<name>`` handlers)."""
        self._send({"t": "event", "name": name, "value": value})

    def ready(
        self,
        *,
        keys: Sequence[str] | None = None,
        text: bool | None = None,
        fps: int | None = None,
    ) -> None:
        """Change what the plugin wants while it runs. ``self.ready(text=True,
        keys=("esc", "tab"))`` while a text line is open gets every key (Tab and Esc
        only when listed); ``self.ready(text=False, keys=())`` gives them back."""
        if keys is not None:
            self.keys = tuple(keys)
        if text is not None:
            self.text = text
        if fps is not None:
            self.fps = fps
        self._send(_ready(self))

    def _send(self, msg: dict[str, Any]) -> None:
        if self._out is not None:
            _write(self._out, _JSON, json.dumps(msg, ensure_ascii=False).encode())


def _ready(plugin: Plugin) -> dict[str, Any]:
    return {
        "t": "ready",
        "keys": list(dict.fromkeys([*plugin.keys, *plugin.keymap.all_keys()])),
        "text": plugin.text,
        "fps": plugin.fps,
    }


def _describe(plugin: Plugin) -> dict[str, Any]:
    return {
        "name": plugin.name,
        "title": plugin.title,
        "icon": plugin.icon,
        "protocol": PROTOCOL,
        "pictures": plugin.pictures,
    }


def serve(plugin: Plugin) -> None:
    """Run ``plugin``: answer ``--describe``, then draw until the pane closes."""
    if sys.argv[1:2] == ["--describe"]:
        print(json.dumps(_describe(plugin), ensure_ascii=False))
        return
    out = sys.stdout.buffer
    plugin._out = out
    inbox: Queue[dict[str, Any] | None] = Queue()

    def reader() -> None:
        fd = sys.stdin.fileno()
        try:
            while (msg := _read(fd)) is not None:
                if msg[0] == _JSON:
                    inbox.put(json.loads(msg[1]))
        except OSError:
            pass
        inbox.put(None)

    threading.Thread(target=reader, daemon=True).start()
    begun = time.monotonic()
    started = False
    shown = True
    due = time.monotonic()
    try:
        while True:
            wait = max(0.0, due - time.monotonic()) if started and shown else None
            try:
                msg = inbox.get(timeout=wait)
            except Empty:
                msg = {"t": "tick"}
            if msg is None or msg["t"] == "stop":
                plugin.stop()
                return
            kind = msg["t"]
            if kind == "start":
                plugin.width, plugin.height = msg["width"], msg["height"]
                plugin.args = msg.get("args") or {}
                try:
                    plugin.keymap = Keymap(plugin.actions, plugin.args)
                    plugin.start(plugin.args)
                except Exception as e:  # noqa: BLE001 — any mistake becomes a message in the pane
                    plugin._send(
                        {
                            "t": "error",
                            "what": f"{plugin.title} could not start",
                            "why": str(e),
                            "fix": "check the pane's options, then press Enter to try again",
                        }
                    )
                    continue
                plugin._send(_ready(plugin))
                started, due = True, time.monotonic()
            elif kind == "resize":
                plugin.width, plugin.height = msg["width"], msg["height"]
                plugin.resize(plugin.width, plugin.height)
                due = time.monotonic()
            elif kind == "key":
                action = plugin.keymap.action(msg["key"])
                if action is not None:
                    plugin.action(action)
                else:
                    plugin.key(msg["key"], msg.get("text"))
                due = time.monotonic()
            elif kind == "mouse":
                plugin.mouse(msg["kind"], msg["x"], msg["y"], msg.get("button"))
                due = time.monotonic()
            elif kind == "paste":
                plugin.paste(msg.get("text", ""))
                due = time.monotonic()
            elif kind == "shown":
                if bool(msg.get("shown", True)) != shown:
                    shown = not shown
                    plugin.shown(shown)
                    due = time.monotonic()
            elif kind == "call":
                try:
                    reply = {
                        "t": "reply",
                        "id": msg["id"],
                        "value": plugin.call(msg["method"], msg.get("args")),
                    }
                except Exception as e:  # noqa: BLE001 — the caller gets the error
                    reply = {"t": "reply", "id": msg["id"], "value": None, "error": str(e)}
                plugin._send(reply)
            elif kind == "tick" and started and shown:
                due = time.monotonic() + 1 / max(1, plugin.fps)  # it can change: ready()
                pic = plugin.frame(time.monotonic() - begun)
                if pic is not None:
                    w, h = plugin.width, plugin.height
                    if isinstance(pic, Canvas):
                        w, h, pic = pic.width, pic.height, pic.rgba()
                    _write(out, _FRAME, struct.pack("<II", w, h) + bytes(pic))
    except (BrokenPipeError, OSError):
        plugin.stop()
    except Exception as e:  # noqa: BLE001 — a mistake in the plugin: say so in its pane
        import traceback

        traceback.print_exc()  # goes to the app's log of this plugin
        try:
            plugin._send(
                {
                    "t": "error",
                    "what": f"{plugin.title} stopped",
                    "why": f"{type(e).__name__}: {e}",
                    "fix": "press Enter to start it again",
                }
            )
        except OSError:
            pass
        plugin.stop()
        raise SystemExit(1) from None
