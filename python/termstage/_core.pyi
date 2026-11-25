"""Type stubs for the Rust extension ``termstage._core``.

Kept in sync by ``tests/python/test_stubs.py``.
"""

from typing import Any

__version__: str

class TermstageError(Exception):
    what: str
    why: str
    fix: str

class Canvas:
    """A picture you draw on with code. Coordinates are pixels from the top-left."""

    def __init__(
        self, width: int = 640, height: int = 360, background: str | None = None
    ) -> None: ...
    @property
    def width(self) -> int: ...
    @property
    def height(self) -> int: ...
    def clear(self, color: str) -> None: ...
    def rect(
        self,
        x: float,
        y: float,
        width: float,
        height: float,
        *,
        fill: str | None = None,
        stroke: str | None = None,
        stroke_width: float = 1.0,
        radius: float = 0.0,
    ) -> None: ...
    def rounded_rect(
        self,
        x: float,
        y: float,
        width: float,
        height: float,
        radius: float = 12.0,
        *,
        fill: str | None = None,
        stroke: str | None = None,
        stroke_width: float = 1.0,
    ) -> None: ...
    def circle(
        self,
        x: float,
        y: float,
        radius: float,
        *,
        fill: str | None = None,
        stroke: str | None = None,
        stroke_width: float = 1.0,
    ) -> None: ...
    def line(
        self,
        x1: float,
        y1: float,
        x2: float,
        y2: float,
        *,
        color: str = "white",
        width: float = 2.0,
    ) -> None: ...
    def polyline(
        self,
        points: list[tuple[float, float]],
        *,
        color: str | None = "white",
        width: float = 2.0,
        fill: str | None = None,
        closed: bool = False,
    ) -> None: ...
    def gradient_rect(
        self,
        x: float,
        y: float,
        width: float,
        height: float,
        *,
        colors: list[str],
        vertical: bool = False,
        radius: float = 0.0,
    ) -> None: ...
    def gradient_circle(self, x: float, y: float, radius: float, *, colors: list[str]) -> None: ...
    def text(
        self,
        text: str,
        x: float,
        y: float,
        *,
        size: float = 16.0,
        color: str = "white",
        bold: bool = False,
        align: str = "left",
        max_width: float | None = None,
    ) -> None: ...
    def image(
        self,
        source: str | Canvas,
        x: float,
        y: float,
        width: float,
        height: float,
        *,
        fit: str = "contain",
    ) -> None: ...
    def png(self) -> bytes: ...
    def save(self, path: str) -> None: ...
    def _register(self) -> str: ...

def inspect(*, headless: bool = False, probe: bool = True) -> str: ...
def fonts() -> str: ...
def run_app(spec_json: str) -> None: ...
def run_headless(
    spec_json: str, cols: int, rows: int, cell: tuple[int, int] = (10, 20)
) -> tuple[str, str]: ...
def render_png(spec_json: str, cols: int, rows: int, cell: tuple[int, int] = (10, 20)) -> bytes: ...
def register_image(data: bytes, name: str = "image") -> str: ...
def show(
    content: Any,
    *,
    title: str | None = None,
    icon: str | None = None,
    dir: str | None = None,
    theme: str | None = None,
    accent: str | None = None,
    width: int | None = None,
    height: int | None = None,
    fit: str | None = None,
) -> str: ...
def cli_main() -> int: ...
def _spawn_tracked(argv: list[str]) -> int: ...
def _reaper_main() -> int: ...
