"""Backend for ``stagetui drive`` and ``stagetui mcp`` (``python -m stagetui._agent``).

Both run the app headless with a :class:`stagetui.Driver` and speak JSON-RPC 2.0,
one message per line on stdin/stdout:

- ``drive``: plain methods (``snapshot``, ``press``, ``type``, ``click``…).
- ``mcp``: the Model Context Protocol — the same actions as MCP tools, so coding
  agents (Claude Code and others) can see and operate the app.

The app's own ``print()`` output goes to stderr so it never corrupts the protocol.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import sys
from collections.abc import Callable
from pathlib import Path
from typing import Any, TextIO

from . import __version__
from ._core import StageError
from ._render import _size, find_app
from .app import Driver

PROTOCOL_VERSION = "2025-06-18"


class ToolError(Exception):
    """A request the app could not carry out (bad arguments, unknown widget…)."""


def _err(e: BaseException) -> dict[str, Any]:
    if isinstance(e, StageError):
        return {
            "what": getattr(e, "what", str(e)),
            "why": getattr(e, "why", ""),
            "fix": getattr(e, "fix", ""),
        }
    if isinstance(e, ToolError):
        return {"what": str(e), "why": "", "fix": "check the tool's arguments"}
    return {"what": str(e), "why": type(e).__name__, "fix": ""}


# ----- the actions (shared by drive and mcp) ---------------------------------------


def _screen(d: Driver) -> dict[str, Any]:
    snap = d.snapshot()
    return {"text": snap.text, "focus": snap.tree.get("focus")}


def _need(args: dict[str, Any], name: str, kind: type | tuple[type, ...]) -> Any:
    if name not in args:
        raise ToolError(f"missing argument {name!r}")
    if not isinstance(args[name], kind):
        raise ToolError(f"argument {name!r} has the wrong type")
    return args[name]


def snapshot(d: Driver, args: dict[str, Any]) -> dict[str, Any]:
    snap = d.snapshot(settle=bool(args.get("settle", False)), png=bool(args.get("png", False)))
    out: dict[str, Any] = {"text": snap.text, "tree": snap.tree}
    if args.get("png"):
        out["png"] = base64.b64encode(snap.png).decode()
    return out


def press(d: Driver, args: dict[str, Any]) -> dict[str, Any]:
    keys = args.get("keys") or [_need(args, "key", str)]
    for k in keys:
        d.press(str(k))
    return _screen(d)


def type_(d: Driver, args: dict[str, Any]) -> dict[str, Any]:
    d.type(_need(args, "text", str))
    return _screen(d)


def click(d: Driver, args: dict[str, Any]) -> dict[str, Any]:
    if "target" in args:
        d.click(str(args["target"]))
    elif "x" in args and "y" in args:
        d.click((int(args["x"]), int(args["y"])))
    else:
        raise ToolError("give either 'target' (like \"#send\") or 'x' and 'y'")
    return _screen(d)


def scroll(d: Driver, args: dict[str, Any]) -> dict[str, Any]:
    d.scroll(int(args.get("rows", 3)), args.get("target"))
    return _screen(d)


def resize(d: Driver, args: dict[str, Any]) -> dict[str, Any]:
    d.resize(int(_need(args, "cols", int)), int(_need(args, "rows", int)))
    return _screen(d)


def wait(d: Driver, args: dict[str, Any]) -> dict[str, Any]:
    d.wait(float(args.get("seconds", 0.5)))
    return _screen(d)


def wait_idle(d: Driver, args: dict[str, Any]) -> dict[str, Any]:
    d.wait_idle()
    return _screen(d)


def query(d: Driver, args: dict[str, Any]) -> Any:
    return d.query(str(_need(args, "id", str)))


def panes(d: Driver, args: dict[str, Any]) -> Any:
    return d.panes()


def set_pane_type(d: Driver, args: dict[str, Any]) -> dict[str, Any]:
    pane = str(_need(args, "pane", str))
    kind = str(_need(args, "type", str))
    d.core.switch_pane(pane if pane.startswith("#") else f"#{pane}", kind)
    d.step({"wait": 0})  # builds app-defined pane types, lays out
    return _screen(d)


def workspace(d: Driver, args: dict[str, Any]) -> Any:
    return d.app.workspace()


def load_workspace(d: Driver, args: dict[str, Any]) -> dict[str, Any]:
    layout = args.get("layout")
    if isinstance(layout, dict):
        d.app.load_workspace(layout)
    else:
        d.app.load_workspace(str(args.get("name", "default")))
    d.step({"wait": 0})  # builds app-defined pane types, lays out
    return _screen(d)


def save_workspace(d: Driver, args: dict[str, Any]) -> dict[str, Any]:
    file = d.app.save_workspace(str(args.get("name", "default")))
    return {"saved": str(file)}


ACTIONS: dict[str, Callable[[Driver, dict[str, Any]], Any]] = {
    "snapshot": snapshot,
    "press": press,
    "type": type_,
    "click": click,
    "scroll": scroll,
    "resize": resize,
    "wait": wait,
    "wait_idle": wait_idle,
    "query": query,
    "panes": panes,
    "set_pane_type": set_pane_type,
    "workspace": workspace,
    "load_workspace": load_workspace,
    "save_workspace": save_workspace,
}

# ----- MCP tool descriptions ---------------------------------------------------------

_S = {"type": "string"}
_I = {"type": "integer"}

TOOLS: list[dict[str, Any]] = [
    {
        "name": "snapshot",
        "description": "See the app's screen: plain text in reading order plus the widget tree "
        "(ids, types, positions, focus). Set png=true to also get a picture.",
        "inputSchema": {
            "type": "object",
            "properties": {"png": {"type": "boolean"}, "settle": {"type": "boolean"}},
        },
    },
    {
        "name": "press",
        "description": 'Press a key, like "enter", "tab", "esc", "ctrl+s", "up", "f1", or several '
        'with keys=[...]. Prefix key for pane commands: "ctrl+space". Returns the new screen.',
        "inputSchema": {
            "type": "object",
            "properties": {"key": _S, "keys": {"type": "array", "items": _S}},
        },
    },
    {
        "name": "type",
        "description": "Type text into the focused widget. Returns the new screen.",
        "inputSchema": {"type": "object", "properties": {"text": _S}, "required": ["text"]},
    },
    {
        "name": "click",
        "description": 'Click a widget by id (target="#send") or a cell (x, y). Returns the new screen.',
        "inputSchema": {"type": "object", "properties": {"target": _S, "x": _I, "y": _I}},
    },
    {
        "name": "scroll",
        "description": "Scroll with the mouse wheel: rows > 0 scrolls down. Optional target widget id.",
        "inputSchema": {"type": "object", "properties": {"rows": _I, "target": _S}},
    },
    {
        "name": "resize",
        "description": "Change the screen size in cells.",
        "inputSchema": {
            "type": "object",
            "properties": {"cols": _I, "rows": _I},
            "required": ["cols", "rows"],
        },
    },
    {
        "name": "wait",
        "description": "Let time pass (seconds, virtual clock): animations, timers, live drawings.",
        "inputSchema": {"type": "object", "properties": {"seconds": {"type": "number"}}},
    },
    {
        "name": "query",
        "description": "Get one widget as JSON by id: type, bounds, text, value, focus…",
        "inputSchema": {"type": "object", "properties": {"id": _S}, "required": ["id"]},
    },
    {
        "name": "list_panes",
        "description": "List the app's panes (Blender-style): id, type, bounds, focused, stashed types.",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "set_pane_type",
        "description": 'Switch what a pane shows, e.g. pane="#right", type="terminal". '
        "Types: chat, terminal, files, diff, code, markdown, log, empty, or app-defined ones.",
        "inputSchema": {
            "type": "object",
            "properties": {"pane": _S, "type": _S},
            "required": ["pane", "type"],
        },
    },
    {
        "name": "workspace",
        "description": "Get the layout of splits and panes as JSON: "
        '{"split": "horizontal", "ratio": [...], "children": [{"pane": "chat", "id": "a"}, ...]}.',
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "load_workspace",
        "description": "Arrange the panes: give layout= (JSON like the workspace tool returns) "
        "or name= of a saved workspace. Panes with the same id keep their content.",
        "inputSchema": {
            "type": "object",
            "properties": {"layout": {"type": "object"}, "name": _S},
        },
    },
    {
        "name": "save_workspace",
        "description": "Save the current layout under a name (to load it later).",
        "inputSchema": {"type": "object", "properties": {"name": _S}, "required": ["name"]},
    },
]

_TOOL_ACTIONS = {t["name"]: t["name"] for t in TOOLS} | {"list_panes": "panes"}


# ----- servers -----------------------------------------------------------------------


class Server:
    def __init__(self, driver: Driver, out: TextIO) -> None:
        self.driver = driver
        self.out = out
        self.running = True

    def send(self, msg: dict[str, Any]) -> None:
        self.out.write(json.dumps(msg, ensure_ascii=False) + "\n")
        self.out.flush()

    def reply(self, id: Any, result: Any) -> None:
        self.send({"jsonrpc": "2.0", "id": id, "result": result})

    def error(self, id: Any, code: int, message: str, data: Any = None) -> None:
        err: dict[str, Any] = {"code": code, "message": message}
        if data is not None:
            err["data"] = data
        self.send({"jsonrpc": "2.0", "id": id, "error": err})

    def handle_line(self, line: str) -> None:
        line = line.strip()
        if not line:
            return
        try:
            msg = json.loads(line)
        except json.JSONDecodeError as e:
            self.error(None, -32700, f"Parse error: {e}")
            return
        if not isinstance(msg, dict) or not isinstance(msg.get("method"), str):
            self.error(msg.get("id") if isinstance(msg, dict) else None, -32600, "Invalid request")
            return
        id_ = msg.get("id")
        params = msg.get("params") or {}
        if not isinstance(params, dict):
            self.error(id_, -32602, "params must be an object")
            return
        self.dispatch(id_, msg["method"], params, notification="id" not in msg)

    def dispatch(self, id: Any, method: str, params: dict[str, Any], notification: bool) -> None:
        raise NotImplementedError


class DriveServer(Server):
    """Plain JSON-RPC: one method per action."""

    def dispatch(self, id: Any, method: str, params: dict[str, Any], notification: bool) -> None:
        if method == "shutdown":
            self.running = False
            if not notification:
                self.reply(id, {})
            return
        action = ACTIONS.get(method)
        if action is None:
            if not notification:
                self.error(
                    id, -32601, f"Unknown method {method!r}", {"methods": [*ACTIONS, "shutdown"]}
                )
            return
        try:
            result = action(self.driver, params)
        except ToolError as e:
            if not notification:
                self.error(id, -32602, str(e))
            return
        except Exception as e:  # noqa: BLE001 — every app error goes back to the caller
            if not notification:
                self.error(
                    id, -32000, str(e).splitlines()[0] if str(e) else type(e).__name__, _err(e)
                )
            return
        if not notification:
            self.reply(id, result)


class McpServer(Server):
    """Model Context Protocol over stdio."""

    def __init__(self, driver: Driver, out: TextIO, app_name: str) -> None:
        super().__init__(driver, out)
        self.app_name = app_name

    def dispatch(self, id: Any, method: str, params: dict[str, Any], notification: bool) -> None:
        if notification:
            return  # notifications/initialized, cancelled… need no answer
        if method == "initialize":
            self.reply(
                id,
                {
                    "protocolVersion": params.get("protocolVersion") or PROTOCOL_VERSION,
                    "capabilities": {"tools": {"listChanged": False}},
                    "serverInfo": {"name": "stagetui", "version": __version__},
                    "instructions": (
                        f"Tools to see and operate the terminal app {self.app_name!r}. "
                        "Start with `snapshot` to read the screen (text plus widget ids), then "
                        "`click`, `type`, `press` to act — each returns the new screen."
                    ),
                },
            )
        elif method == "ping":
            self.reply(id, {})
        elif method == "tools/list":
            self.reply(id, {"tools": TOOLS})
        elif method == "tools/call":
            self.reply(id, self.call(str(params.get("name", "")), params.get("arguments") or {}))
        else:
            self.error(id, -32601, f"Unknown method {method!r}")

    def call(self, name: str, args: dict[str, Any]) -> dict[str, Any]:
        action = ACTIONS.get(_TOOL_ACTIONS.get(name, ""))
        if action is None:
            return {
                "content": [{"type": "text", "text": f"Unknown tool {name!r}"}],
                "isError": True,
            }
        try:
            result = action(self.driver, args)
        except Exception as e:  # noqa: BLE001 — errors are shown to the agent
            info = _err(e)
            text = info["what"] + (f"\nwhy: {info['why']}" if info["why"] else "")
            text += f"\nfix: {info['fix']}" if info["fix"] else ""
            return {"content": [{"type": "text", "text": text}], "isError": True}
        content: list[dict[str, Any]] = []
        if isinstance(result, dict) and "text" in result:
            content.append({"type": "text", "text": result["text"]})
            if "png" in result:
                content.append({"type": "image", "data": result["png"], "mimeType": "image/png"})
            rest = {k: v for k, v in result.items() if k not in ("text", "png")}
            if rest:
                content.append({"type": "text", "text": json.dumps(rest, ensure_ascii=False)})
        else:
            content.append({"type": "text", "text": json.dumps(result, ensure_ascii=False)})
        return {"content": content, "isError": False}


def serve(mode: str, app_path: str, size: tuple[int, int], stdin: TextIO, out: TextIO) -> int:
    app = find_app(Path(app_path))()
    with app.drive(size) as driver:
        server: Server
        if mode == "mcp":
            server = McpServer(driver, out, Path(app_path).name)
        else:
            server = DriveServer(driver, out)
        for line in stdin:
            server.handle_line(line)
            if not server.running:
                break
    return 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="stagetui")
    p.add_argument("mode", choices=["drive", "mcp"])
    p.add_argument("app")
    p.add_argument("--size", default="100x30")
    a = p.parse_args(argv)
    # Windows pipes default to a legacy code page; stagetui always speaks UTF-8.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8", errors="replace")
    # The protocol owns stdout; anything else the app prints goes to stderr.
    proto = os.fdopen(os.dup(sys.stdout.fileno()), "w", encoding="utf-8", buffering=1)
    os.dup2(sys.stderr.fileno(), sys.stdout.fileno())
    sys.stdout = sys.stderr
    os.environ["STAGETUI_HEADLESS"] = "1"
    try:
        return serve(a.mode, a.app, _size(a.size, "--size"), sys.stdin, proto)
    except StageError as e:
        print(f"stagetui: {e}", file=sys.stderr)
        return 1
    finally:
        proto.flush()


if __name__ == "__main__":
    sys.exit(main())
