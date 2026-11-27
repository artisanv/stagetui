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

The protocol is the same as the Rust crate ``stagetui-plugin-api``.
"""

from __future__ import annotations

import json
import os
import struct
import sys
import threading
import time
from queue import Empty, Queue
from typing import Any, BinaryIO

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
    #: Wants every key typed (like a text box).
    text = False
    #: Draws pictures (so it is best in a terminal with picture support).
    pictures = True

    def __init__(self) -> None:
        self.width = 1
        self.height = 1
        self.args: dict[str, Any] = {}
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
        """A key from ``keys`` (or any key with ``text = True``)."""

    def mouse(self, kind: str, x: int, y: int, button: str | None) -> None:
        """``kind``: down, up, drag, move, scroll_up, scroll_down. ``x``/``y`` in pixels."""

    def call(self, method: str, args: Any) -> Any:
        """The app called ``pane.content.call(method, args)``."""
        raise ValueError(f'this plugin has no method "{method}"')

    def stop(self) -> None:
        """The pane is closing."""

    # ----- use these -------------------------------------------------------

    def status(self, text: str) -> None:
        """A short line at the bottom of the pane, e.g. ``"▶ 01:23 / 04:56"``."""
        self._send({"t": "status", "text": text})

    def event(self, name: str, value: Any = None) -> None:
        """Tell the app something happened (``on_<name>`` handlers)."""
        self._send({"t": "event", "name": name, "value": value})

    def _send(self, msg: dict[str, Any]) -> None:
        if self._out is not None:
            _write(self._out, _JSON, json.dumps(msg, ensure_ascii=False).encode())


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
    every = 1 / max(1, plugin.fps)
    due = time.monotonic()
    try:
        while True:
            wait = max(0.0, due - time.monotonic()) if started else None
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
                plugin._send(
                    {
                        "t": "ready",
                        "keys": list(plugin.keys),
                        "text": plugin.text,
                        "fps": plugin.fps,
                    }
                )
                started, due = True, time.monotonic()
            elif kind == "resize":
                plugin.width, plugin.height = msg["width"], msg["height"]
                plugin.resize(plugin.width, plugin.height)
                due = time.monotonic()
            elif kind == "key":
                plugin.key(msg["key"], msg.get("text"))
                due = time.monotonic()
            elif kind == "mouse":
                plugin.mouse(msg["kind"], msg["x"], msg["y"], msg.get("button"))
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
            elif kind == "tick" and started:
                due = time.monotonic() + every
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
