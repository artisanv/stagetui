"""Milestone 4b: driving apps (Driver, stagetui drive / mcp), live capture, snapshot tests."""

import json
import subprocess
import sys
from pathlib import Path

import pytest
from stagetui import Driver, StageApp, StageButton, StageCard, StageInput
from stagetui.testing import StageSnapshot

ROOT = Path(__file__).parents[2]
ARABIC = "مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج"

APP = """
from stagetui import StageApp, StageButton, StageCard, StageInput, StagePane, StageSplit

class Demo(StageApp):
    def compose(self):
        return [
            StageSplit([StagePane("markdown", text="# Notes", id="left"), StagePane("log", id="right")]),
            StageInput(placeholder="Name", id="name"),
            StageButton("Greet", id="greet", on_click=self.greet),
            StageCard("…", id="out"),
        ]

    def greet(self):
        print("this print must not break the protocol")
        self.query("#out").update(f"Hello, {self.query('#name').value}!")
"""


@pytest.fixture
def app_file(tmp_path):
    p = tmp_path / "demo_app.py"
    p.write_text(APP, encoding="utf-8")
    return p


class Form(StageApp):
    def compose(self):
        return [
            StageInput(placeholder="Name", id="name"),
            StageButton("Greet", id="greet", on_click=self.greet),
            StageCard("…", id="out"),
        ]

    def greet(self):
        self.query("#out").update(f"Hello, {self.query('#name').value}!")


def test_driver_steps_and_capture():
    app = Form()
    with app.drive((60, 16)) as d:
        assert isinstance(d, Driver)
        d.type(ARABIC)
        d.click("#greet")
        assert f"Hello, {ARABIC}!" in d.snapshot().text
        assert d.query("out")["text"] == f"Hello, {ARABIC}!"
        assert app.capture_text() == d.snapshot().text
        assert app.capture_tree()["focus"] == "greet"
        assert app.capture_png().startswith(b"\x89PNG")
        d.resize(40, 12)
        assert d.snapshot().cols == 40
        snap = d.snapshot(png=True)
    assert snap.png.startswith(b"\x89PNG")
    with pytest.raises(Exception, match="closed"):
        d.press("tab")


def test_drag_moves_a_split_line():
    from stagetui import StageCard, StageSplit

    class Two(StageApp):
        def compose(self):
            return [StageSplit([StageCard("مرحبا", id="a"), StageCard("日本語 नमस्ते", id="b")])]

    with Two().drive((81, 20)) as d:
        # Arabic first: the split is mirrored (right to left), so look at whichever is on the left.
        left = lambda: min((d.query(i)["bounds"] for i in ("#a", "#b")), key=lambda b: b["x"])["w"]
        line = left()
        d.drag((line, 5), (line + 20, 5))
        assert left() == line + 20
        d.step({"drag": {"from": [line + 20, 5], "to": [line - 10, 5]}})
        assert left() == line - 10
        with pytest.raises(Exception, match="No widget"):
            d.drag("#nope", (1, 1))


def rpc(mode, app_file, messages, size="80x24"):
    lines = "".join((m if isinstance(m, str) else json.dumps(m)) + "\n" for m in messages)
    proc = subprocess.run(
        [sys.executable, "-m", "stagetui._agent", mode, str(app_file), "--size", size],
        input=lines,
        capture_output=True,
        encoding="utf-8",
        timeout=60,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr
    return [json.loads(line) for line in proc.stdout.splitlines()], proc.stderr


def test_drive_protocol(app_file):
    replies, stderr = rpc(
        "drive",
        app_file,
        [
            {"jsonrpc": "2.0", "id": 1, "method": "type", "params": {"text": "Sara"}},
            {"jsonrpc": "2.0", "id": 2, "method": "click", "params": {"target": "#greet"}},
            {"jsonrpc": "2.0", "id": 3, "method": "query", "params": {"id": "out"}},
            {"jsonrpc": "2.0", "id": 4, "method": "panes"},
            {"jsonrpc": "2.0", "id": 5, "method": "snapshot", "params": {"png": True}},
            {
                "jsonrpc": "2.0",
                "id": 6,
                "method": "set_pane_type",
                "params": {"pane": "right", "type": "chat"},
            },
            {
                "jsonrpc": "2.0",
                "method": "press",
                "params": {"key": "tab"},
            },  # notification: no reply
            {"jsonrpc": "2.0", "id": 7, "method": "fly"},
            {"jsonrpc": "2.0", "id": 8, "method": "click", "params": {"target": "#nope"}},
            "not json",
            {"jsonrpc": "2.0", "id": 9, "method": "shutdown"},
        ],
    )
    by_id = {r.get("id"): r for r in replies}
    assert "Hello, Sara!" in by_id[2]["result"]["text"]
    assert by_id[3]["result"]["text"] == "Hello, Sara!"
    assert [p["type"] for p in by_id[4]["result"]] == ["markdown", "log"]
    assert by_id[5]["result"]["png"].startswith("iVBOR")
    assert by_id[6]["result"]["text"]
    assert by_id[7]["error"]["code"] == -32601
    assert by_id[8]["error"]["data"]["fix"]
    assert by_id[None]["error"]["code"] == -32700, "parse errors are reported"
    assert by_id[9]["result"] == {}
    assert "must not break the protocol" in stderr, "app prints go to stderr"


def test_mcp_protocol(app_file):
    replies, _ = rpc(
        "mcp",
        app_file,
        [
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "t", "version": "1"},
                },
            },
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
            {
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {"name": "type", "arguments": {"text": "Ali"}},
            },
            {
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {"name": "click", "arguments": {"target": "#greet"}},
            },
            {
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {"name": "snapshot", "arguments": {"png": True}},
            },
            {
                "jsonrpc": "2.0",
                "id": 6,
                "method": "tools/call",
                "params": {"name": "click", "arguments": {}},
            },
            {
                "jsonrpc": "2.0",
                "id": 7,
                "method": "tools/call",
                "params": {"name": "list_panes", "arguments": {}},
            },
            {"jsonrpc": "2.0", "id": 8, "method": "ping"},
        ],
    )
    by_id = {r["id"]: r for r in replies}
    init = by_id[1]["result"]
    assert init["serverInfo"]["name"] == "stagetui" and "tools" in init["capabilities"]
    names = {t["name"] for t in by_id[2]["result"]["tools"]}
    assert {"snapshot", "press", "type", "click", "drag", "list_panes", "set_pane_type"} <= names
    for t in by_id[2]["result"]["tools"]:
        assert t["inputSchema"]["type"] == "object" and t["description"]
    assert "Hello, Ali!" in by_id[4]["result"]["content"][0]["text"]
    kinds = [c["type"] for c in by_id[5]["result"]["content"]]
    assert kinds[:2] == ["text", "image"]
    assert by_id[6]["result"]["isError"] is True
    assert "markdown" in by_id[7]["result"]["content"][0]["text"]
    assert by_id[8]["result"] == {}
    assert len(replies) == 8, "the notification gets no reply"


def test_cli_drive_hands_over_to_python(app_file):
    proc = subprocess.run(
        [sys.executable, "-m", "stagetui", "drive", str(app_file), "--size", "60x20"],
        input=json.dumps({"jsonrpc": "2.0", "id": 1, "method": "snapshot"}) + "\n",
        capture_output=True,
        encoding="utf-8",
        timeout=60,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr
    assert json.loads(proc.stdout)["result"]["tree"]["size"] == {"cols": 60, "rows": 20}
    missing = subprocess.run(
        [sys.executable, "-m", "stagetui", "mcp", "nope.py"],
        capture_output=True,
        encoding="utf-8",
        check=False,
    )
    assert missing.returncode == 1 and "fix:" in missing.stderr


def test_golden_snapshots(tmp_path):
    snap = Form().run_headless((40, 12))
    golden = StageSnapshot(tmp_path, "test_form", update=False)
    golden.assert_match(snap, png=True)  # first run saves
    assert (tmp_path / "test_form.txt").exists() and (tmp_path / "test_form.png").exists()
    StageSnapshot(tmp_path, "test_form", update=False).assert_match(snap, png=True)  # same → passes
    changed = Form().run_headless((40, 12), steps=[{"type": "Zed"}])
    with pytest.raises(AssertionError, match="screen text changed") as e:
        StageSnapshot(tmp_path, "test_form", update=False).assert_match(changed, png=True)
    assert "pixels differ" in str(e.value) and "--stage-update" in str(e.value)
    assert (tmp_path / "test_form.actual.txt").exists()
    StageSnapshot(tmp_path, "test_form", update=True).assert_match(changed, png=True)
    StageSnapshot(tmp_path, "test_form", update=False).assert_match(changed, png=True)


def test_stage_snapshot_fixture(stage_snapshot):
    """The plugin's fixture, with a committed golden file."""
    stage_snapshot.assert_match(Form().run_headless((40, 12)))


def test_png_diff():
    from stagetui import _core

    a = Form().run_headless((30, 8)).png
    b = Form().run_headless((30, 8), steps=[{"type": "x"}]).png
    assert _core.png_diff(a, a) == 0.0
    assert 0 < _core.png_diff(a, b) < 0.1
    assert _core.png_diff(a, Form().run_headless((31, 8)).png) == 1.0
