from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any, ClassVar

from . import _core
from .widgets import Theme, _Widget


class Snapshot:
    """A picture of your app taken without a terminal.

    - ``snap.text`` — the screen as plain text (Arabic in normal reading order).
    - ``snap.tree`` — every widget as a dictionary: id, type, position, text…
    - ``snap.png`` — the screen as a PNG image (bytes).
    - ``snap.save("shot.png")`` — save as ``.png``, ``.txt`` or ``.json``.
    """

    def __init__(self, text: str, tree: dict[str, Any], spec: str, cell: tuple[int, int]) -> None:
        self.text = text
        self.tree = tree
        self.cols: int = tree["size"]["cols"]
        self.rows: int = tree["size"]["rows"]
        self._spec = spec
        self._cell = cell
        self._png: bytes | None = None

    @property
    def png(self) -> bytes:
        """The screen as PNG bytes (rendered on first use)."""
        if self._png is None:
            self._png = _core.render_png(self._spec, self.cols, self.rows, self._cell)
        return self._png

    def query(self, id: str) -> dict[str, Any] | None:
        """Find a widget in the tree by id (``"hello"`` or ``"#hello"``)."""
        wanted = id.removeprefix("#")
        return next((n for n in self.tree.get("children", []) if n.get("id") == wanted), None)

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
    """Base class for apps. Override :meth:`compose` to return a list of widgets."""

    theme: ClassVar[Theme] = "aurora-dark"
    accent: ClassVar[str | None] = None

    def compose(self) -> list[_Widget]:
        return []

    def _spec_json(self) -> str:
        children = self.compose()
        if not isinstance(children, (list, tuple)):
            raise TypeError("compose() must return a list of widgets")
        for c in children:
            if not isinstance(c, _Widget):
                raise TypeError(f"compose() returned {c!r}, which is not a termstage widget")
        spec: dict[str, Any] = {"theme": self.theme, "children": [c._spec() for c in children]}
        if self.accent is not None:
            spec["accent"] = self.accent
        return json.dumps(spec, ensure_ascii=False)

    def run(self) -> None:
        """Run in the current terminal. Press ``q``, ``Esc`` or ``Ctrl+C`` to quit."""
        _core.run_app(self._spec_json())

    def run_headless(
        self, size: tuple[int, int] = (80, 24), *, cell: tuple[int, int] = (16, 32)
    ) -> Snapshot:
        """Render without a terminal at ``(columns, rows)`` and return a :class:`Snapshot`.

        ``cell`` is the pixel size of one character box, used for PNGs.
        """
        cols, rows = size
        spec = self._spec_json()
        text, tree = _core.run_headless(spec, cols, rows, cell)
        return Snapshot(text, json.loads(tree), spec, cell)
