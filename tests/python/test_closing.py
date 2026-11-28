"""Closing panes: what ran in them stops, and the last pane's × closes the app.

Hidden add-on panes (another pane type showing, another tab) are told so: they
stop drawing until they come back. The add-on parts run when the 3D add-on is
built (`STAGETUI_PLUGIN_PATH`, like tests/python/test_3d.py).
"""

import subprocess
import sys
import time
from pathlib import Path

import pytest
import stagetui as ts
from stagetui import StageApp, StagePane, StageSplit

# Text tests carry every script (AGENTS.md rule 4).
NOTES = "# مَرْحَبًا Hello 日本語 नमस्ते"


@pytest.fixture(autouse=True)
def headless(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("STAGETUI_HEADLESS", "1")


def quit_seen(app: StageApp) -> list[bool]:
    """Record the app's "quit" event (the Driver keeps running headless)."""
    seen: list[bool] = []
    dispatch = app._dispatch

    def spy(event):
        if event[2] == "quit":
            seen.append(True)
        return dispatch(event)

    app._dispatch = spy
    return seen


def close_button(d, pane: str) -> tuple[int, int]:
    """Where a pane's × is: the last one on its header row, inside the pane."""
    b = d.query(pane)["bounds"]
    row = d.snapshot().text.splitlines()[b["y"]]
    x = row.rindex("×", b["x"], b["x"] + b["w"])
    return x, b["y"]


def running(pid: int) -> bool:
    """Still running (a finished process waiting to be collected doesn't count)."""
    out = subprocess.run(
        ["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True, check=False
    )
    state = out.stdout.strip()
    return bool(state) and not state.startswith("Z")


def until(d, check, seconds: float = 10.0) -> bool:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        d.wait(0.05)
        if check():
            return True
        time.sleep(0.05)
    return False


def test_the_last_panes_close_button_closes_the_app() -> None:
    class Two(StageApp):
        def compose(self):
            return [
                StageSplit(
                    [StagePane("markdown", id="a", text=NOTES), StagePane("markdown", id="b")]
                )
            ]

    app = Two()
    seen = quit_seen(app)
    with app.drive((100, 20)) as d:
        d.click(close_button(d, "#a"))
        assert d.query("#a") is None, "the first × closes that pane"
        assert not seen, "…and the app keeps running"
        d.click(close_button(d, "#b"))
        assert seen, "× on the last pane closes the app"


@pytest.mark.skipif(sys.platform == "win32", reason="checks the process with ps")
def test_closing_a_terminal_pane_ends_its_program() -> None:
    class Shells(StageApp):
        def compose(self):
            return [
                StageSplit(
                    [
                        StagePane("terminal", id="t", command=["sleep", "60"]),
                        StagePane("markdown", id="m", text=NOTES),
                    ]
                )
            ]

    with Shells().drive((100, 20)) as d:
        program = lambda: (d.query("#t") or {"children": [{}]})["children"][0].get("pid")
        assert until(d, lambda: program() is not None), "the program started"
        pid = program()
        assert running(pid)
        d.click(close_button(d, "#t"))
        assert d.query("#t") is None
        assert until(d, lambda: not running(pid), 5), "closing the pane ended it at once"


@pytest.fixture
def model(tmp_path: Path) -> Path:
    if not any(p["name"] == "3d" and p["installed"] for p in ts.plugins(refresh=True)):
        pytest.skip("the 3D add-on isn't built here")
    path = tmp_path / f"{NOTES[2:]}.obj"
    path.write_text(
        "v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\n"
        "f 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 3 4 8 7\nf 2 3 7 6\nf 1 5 8 4\n",
        encoding="utf-8",
    )
    return path


@pytest.mark.skipif(sys.platform == "win32", reason="checks the process with ps")
def test_closing_an_add_on_pane_stops_it(model: Path) -> None:
    class Viewer(StageApp):
        def compose(self):
            return [
                StageSplit(
                    [
                        StagePane("3d", id="m", path=str(model)),
                        StagePane("markdown", id="n", text=NOTES),
                    ]
                )
            ]

    with Viewer().drive((100, 24)) as d:
        node = lambda: d.query("#m")["children"][0]
        assert until(d, lambda: node()["frames"] > 0 and node()["pid"])
        pid = node()["pid"]
        assert running(pid)
        started = time.monotonic()
        d.click(close_button(d, "#m"))
        assert time.monotonic() - started < 1.0, "the screen doesn't wait for it"
        assert d.query("#m") is None
        assert until(d, lambda: not running(pid), 5), "closing the pane stopped the add-on"


def test_a_hidden_add_on_pane_stops_drawing(model: Path) -> None:
    class Viewer(StageApp):
        def compose(self):
            return [StagePane("3d", id="m", path=str(model), spin=True)]

    with Viewer().drive((100, 24)) as d:
        pane = d.app.query("#m")
        node = lambda: next(c for c in d.query("#m")["children"] if c.get("plugin") == "3d")
        assert until(d, lambda: node()["frames"] > 2), "spinning: new pictures keep coming"

        pane.switch("markdown")  # the 3D view is stashed, still running
        d.wait(0.2)
        assert node()["shown"] is False
        frames = node()["frames"]
        d.wait(0.6)
        time.sleep(0.4)
        d.wait(0.1)
        assert node()["frames"] <= frames + 1, "hidden: it stops drawing"
        assert node()["pid"], "…but keeps its place"

        pane.switch("3d")
        d.wait(0.1)
        assert node()["shown"] is True
        assert until(d, lambda: node()["frames"] > frames + 2), "back: it draws again"
