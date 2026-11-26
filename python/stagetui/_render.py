"""Backend for ``stagetui render my_app.py`` (run as ``python -m stagetui._render``).

Loads the file without running its ``if __name__ == "__main__":`` block,
finds the ``StageApp`` it defines, and renders it headlessly.
"""

from __future__ import annotations

import argparse
import json
import runpy
import sys
from pathlib import Path

from ._core import StageError
from .app import StageApp

RUN_NAME = "__stagetui_render__"


def _size(text: str, flag: str) -> tuple[int, int]:
    try:
        w, h = text.lower().replace("×", "x").split("x")
        return int(w), int(h)
    except ValueError:
        raise SystemExit(
            f"stagetui: can't read {flag} {text!r} — use something like 100x30"
        ) from None


def find_app(path: Path) -> type[StageApp]:
    sys.path.insert(0, str(path.resolve().parent))
    namespace = runpy.run_path(str(path), run_name=RUN_NAME)
    apps = [
        v
        for v in namespace.values()
        if isinstance(v, type) and issubclass(v, StageApp) and v.__module__ == RUN_NAME
    ]
    if not apps:
        raise SystemExit(
            f"stagetui: {path} does not define a StageApp.\n"
            "  fix: add a class like `class MyApp(StageApp): ...` to the file"
        )
    return apps[-1]


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="stagetui render")
    p.add_argument("target")
    p.add_argument("--out")
    p.add_argument("--text")
    p.add_argument("--tree")
    p.add_argument("--size", default="80x24")
    p.add_argument("--cell", default="16x32")
    a = p.parse_args(argv)
    try:
        app = find_app(Path(a.target))()
        snap = app.run_headless(_size(a.size, "--size"), cell=_size(a.cell, "--cell"))
        written = []
        if a.out:
            Path(a.out).write_bytes(snap.png)
            written.append(a.out)
        if a.text:
            Path(a.text).write_text(snap.text + "\n", encoding="utf-8")
            written.append(a.text)
        if a.tree:
            Path(a.tree).write_text(
                json.dumps(snap.tree, ensure_ascii=False, indent=2), encoding="utf-8"
            )
            written.append(a.tree)
        for path in written:
            print(f"✓ saved {Path(path).resolve()}", file=sys.stderr)
        if not (a.out or a.text or a.tree):
            print(snap.text)
    except StageError as e:
        print(f"stagetui: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
