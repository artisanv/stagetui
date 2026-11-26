"""pytest plugin: golden snapshots for stagetui apps.

Installed with stagetui and loaded by pytest automatically. Use the
``stage_snapshot`` fixture::

    def test_home_screen(stage_snapshot):
        snap = MyApp().run_headless((80, 24))
        stage_snapshot.assert_match(snap)            # text
        stage_snapshot.assert_match(snap, png=True)  # text and picture

The first run saves ``__snapshots__/<test file>/<test name>.txt`` (and ``.png``)
next to the test. Later runs compare against them. When a change is intended,
update the saved copies with ``pytest --stage-update`` (or
``STAGETUI_UPDATE_SNAPSHOTS=1``) and review the diff before committing.
"""

from __future__ import annotations

import difflib
import os
import re
from pathlib import Path
from typing import Any

import pytest

from . import _core
from .app import Snapshot


def pytest_addoption(parser: pytest.Parser) -> None:
    group = parser.getgroup("stagetui")
    group.addoption(
        "--stage-update",
        action="store_true",
        default=False,
        help="stagetui: save new golden snapshots instead of comparing",
    )


class StageSnapshot:
    """Compares snapshots with saved ("golden") copies. See the module docs."""

    def __init__(self, folder: Path, test_name: str, update: bool) -> None:
        self.folder = folder
        self.test_name = test_name
        self.update = update
        self.count = 0

    def _path(self, name: str | None, suffix: str) -> Path:
        stem = re.sub(r"[^\w.-]+", "_", self.test_name)
        if name:
            stem += "-" + re.sub(r"[^\w.-]+", "_", name)
        return self.folder / f"{stem}{suffix}"

    def assert_match(
        self,
        snap: Snapshot,
        name: str | None = None,
        *,
        png: bool = False,
        tolerance: float = 0.002,
    ) -> None:
        """Check ``snap`` against the saved copy (created on the first run).

        ``name`` tells several snapshots in one test apart. ``png=True`` also
        compares the picture: at most ``tolerance`` (a fraction, 0.002 = 0.2 %)
        of the pixels may differ, so tiny font-rendering differences between
        machines don't fail the test.
        """
        if name is None and self.count:
            name = str(self.count + 1)
        self.count += 1
        self.folder.mkdir(parents=True, exist_ok=True)
        text_path = self._path(name, ".txt")
        problems = []
        if self.update or not text_path.exists():
            text_path.write_text(snap.text + "\n", encoding="utf-8")
        else:
            expected = text_path.read_text(encoding="utf-8").removesuffix("\n")
            if expected != snap.text:
                diff = "\n".join(
                    difflib.unified_diff(
                        expected.splitlines(),
                        snap.text.splitlines(),
                        "saved",
                        "now",
                        lineterm="",
                    )
                )
                actual = self._path(name, ".actual.txt")
                actual.write_text(snap.text + "\n", encoding="utf-8")
                problems.append(f"The screen text changed ({text_path.name}):\n{diff}")
        if png:
            png_path = self._path(name, ".png")
            if self.update or not png_path.exists():
                png_path.write_bytes(snap.png)
            else:
                changed = _core.png_diff(png_path.read_bytes(), snap.png)
                if changed > tolerance:
                    actual = self._path(name, ".actual.png")
                    actual.write_bytes(snap.png)
                    problems.append(
                        f"The picture changed: {changed:.2%} of pixels differ "
                        f"(allowed {tolerance:.2%}). Compare {png_path.name} with {actual.name}."
                    )
        if problems:
            raise AssertionError(
                "\n\n".join(problems)
                + "\n\nIf this change is intended, run pytest --stage-update and review the result."
            )


@pytest.fixture
def stage_snapshot(request: pytest.FixtureRequest) -> StageSnapshot:
    """Golden-snapshot helper for stagetui apps (see ``stagetui.testing``)."""
    test_file = Path(str(request.node.fspath))
    folder = test_file.parent / "__snapshots__" / test_file.stem
    update = bool(request.config.getoption("--stage-update", default=False)) or (
        os.environ.get("STAGETUI_UPDATE_SNAPSHOTS") == "1"
    )
    return StageSnapshot(folder, request.node.name, update)


@pytest.fixture
def stage_headless(monkeypatch: pytest.MonkeyPatch) -> Any:
    """Make sure nothing in the test touches the real terminal."""
    monkeypatch.setenv("STAGETUI_HEADLESS", "1")
    return True
