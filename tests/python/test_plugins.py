"""Add-ons: pane types served by their own program (here: a tiny Python one)."""

import os
import sys
import time
from pathlib import Path

import pytest
import stagetui as ts
from stagetui import StageApp, StagePane, StageSplit

pytestmark = pytest.mark.skipif(sys.platform == "win32", reason="script add-ons need a shebang")

PLUGIN = """#!{python}
import math
from stagetui import Canvas
from stagetui.plugin import Plugin, serve


class Dot(Plugin):
    name, title, icon = "dot", "Dot", "dot"
    keys = ("space",)
    fps = 15

    def start(self, args):
        if args.get("fail"):
            raise ValueError("asked to fail: " + args["fail"])
        self.n = 0
        self.status("ready · " + args.get("label", ""))

    def frame(self, t):
        if self.n >= 3:
            raise SystemExit(3)  # a crash, on purpose
        c = Canvas(self.width, self.height, "#000000")
        c.circle(self.width / 2, self.height / 2, 5 + t, fill="#ff0000")
        return c

    def key(self, key, text):
        self.n += 1
        self.event("pressed", {{"key": key, "n": self.n}})

    def call(self, method, args):
        if method == "count":
            return self.n
        return super().call(method, args)


serve(Dot())
"""


@pytest.fixture
def addons(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    monkeypatch.setenv("STAGETUI_HEADLESS", "1")
    script = tmp_path / "stagetui-plugin-dot"
    script.write_text(PLUGIN.format(python=sys.executable), encoding="utf-8")
    script.chmod(0o755)
    monkeypatch.setenv("STAGETUI_PLUGIN_PATH", str(tmp_path))
    yield tmp_path
    monkeypatch.delenv("STAGETUI_PLUGIN_PATH")
    ts.plugins(refresh=True)


def settle(d, until, seconds: float = 10.0) -> bool:
    """Real time passes for the add-on's program; redraw until `until()` holds."""
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        d.wait(0.05)
        if until():
            return True
        time.sleep(0.05)
    return False


def test_add_ons_are_found_and_listed(addons: Path) -> None:
    found = {p["name"]: p for p in ts.plugins(refresh=True)}
    assert found["dot"]["installed"] and found["dot"]["title"] == "Dot"
    assert found["3d"]["package"] == "stagetui-3d" and not found["3d"]["installed"]


def test_an_add_on_draws_talks_and_restarts(addons: Path) -> None:
    ts.plugins(refresh=True)
    pressed = []

    class Desk(StageApp):
        def compose(self):
            return [
                StageSplit(
                    [
                        StagePane(
                            "dot",
                            id="d",
                            label="مَرْحَبًا Hello 日本語 नमस्ते",
                            on_pressed=pressed.append,
                        ),
                        StagePane("markdown", id="m", text="notes"),
                    ]
                )
            ]

    with Desk().drive((90, 24)) as d:
        dot = d.app.query("#d").content
        info = lambda: d.query("#d")["children"][0]
        assert settle(d, lambda: info()["status"] is not None), info()
        assert "Hello" in info()["status"] and "日本語" in info()["status"]
        # The status line can come before the first picture: wait for it.
        assert settle(d, lambda: "Starting" not in d.snapshot().text), "a frame arrived"
        d.click("#d")
        d.press("space")
        assert settle(d, lambda: pressed), "the add-on's event reached the app"
        assert pressed[0] == {"key": "space", "n": 1}
        assert dot.call("count") == 1
        with pytest.raises(ts.StageError, match="no method"):
            dot.call("nope")
        # It crashes after three key presses: the pane says so, Enter starts it again.
        d.press("space")
        d.press("space")
        assert settle(d, lambda: "stopped" in d.snapshot().text), d.snapshot().text
        d.press("enter")
        assert settle(d, lambda: info()["running"] and info()["error"] is None)
        assert dot.call("count") == 0, "a fresh start"


def test_an_add_on_can_refuse_to_start(addons: Path) -> None:
    ts.plugins(refresh=True)

    class Bad(StageApp):
        def compose(self):
            return [StagePane("dot", id="d", fail="no file")]

    with Bad().drive((60, 16)) as d:
        assert settle(d, lambda: "could not start" in d.snapshot().text), d.snapshot().text
        assert "asked to fail: no file" in d.snapshot().text


def test_official_add_ons_say_when_they_are_coming(addons: Path) -> None:
    ts.plugins(refresh=True)

    class Wants(StageApp):
        def compose(self):
            return [StageSplit([StagePane("3d", id="v"), StagePane("empty", id="e")])]

    with Wants().drive((90, 20)) as d:
        d.wait(0.1)
        assert "3D is coming soon" in d.snapshot().text
        d.click("#e")
        d.press("ctrl+space")
        d.press("space")  # the pane menu lists every add-on
        menu = d.snapshot().text
        assert "Dot" in menu and "3D  (coming soon)" in menu, menu


def test_writing_a_plugin_describes_itself(addons: Path) -> None:
    import subprocess

    out = subprocess.run(
        [str(addons / "stagetui-plugin-dot"), "--describe"],
        capture_output=True,
        encoding="utf-8",
        check=True,
        env={**os.environ},
    ).stdout
    assert '"name": "dot"' in out and '"protocol": 1' in out


def test_a_plugin_that_exits_does_not_crash_python(addons: Path) -> None:
    """Regression: a daemon thread blocked in a buffered stdin read made Python 3.14
    abort (SIGABRT, a core dump) when the plugin exited. It must exit cleanly."""
    import json
    import struct
    import subprocess

    def msg(m: dict) -> bytes:
        body = json.dumps(m).encode()
        return struct.pack("<IB", len(body) + 1, 1) + body

    start = {"t": "start", "width": 32, "height": 16, "cols": 4, "rows": 1, "args": {}}
    keys = b"".join(msg({"t": "key", "key": "space"}) for _ in range(3))
    proc = subprocess.Popen(
        [str(addons / "stagetui-plugin-dot")],
        stdin=subprocess.PIPE,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    proc.stdin.write(msg(start) + keys)
    proc.stdin.flush()  # stdin stays open: the reader thread is blocked in a read
    try:
        code = proc.wait(timeout=20)
    finally:
        proc.kill()
    err = proc.stderr.read().decode(errors="replace")
    assert code == 3, f"exit code {code} (-6 is SIGABRT): {err}"
    assert "Fatal Python error" not in err
