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
    actions = {{"ping": ("p",), "jump": ("j", "up")}}
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

    def action(self, name):
        self.event("acted", name)

    def shown(self, shown):
        self.event("shown", shown)

    def call(self, method, args):
        if method == "count":
            return self.n
        if method == "typing":  # every key while typing, the usual ones after
            self.ready(text=args, keys=("esc",) if args else ("space",))
            return args
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
    # Official add-ons are listed even when they aren't installed, and all are on PyPI.
    assert found["3d"]["package"] == "stagetui-3d" and found["3d"]["available"]


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


def test_a_hidden_add_on_rests(addons: Path) -> None:
    ts.plugins(refresh=True)  # this test's add-on folder
    seen: list[bool] = []

    class App(StageApp):
        def compose(self):
            return [StagePane("dot", id="p", label="مَرْحَبًا 日本語 नमस्ते", on_shown=seen.append)]

    with App().drive((60, 16)) as d:
        pane = d.app.query("#p")
        node = lambda: next(c for c in d.query("#p")["children"] if c.get("plugin") == "dot")
        assert settle(d, lambda: node()["frames"] > 0)
        pane.switch("markdown")  # the add-on is stashed: still running, out of sight
        assert settle(d, lambda: seen == [False]), "it hears it's hidden"
        frames = node()["frames"]
        d.wait(0.4)
        time.sleep(0.3)
        d.wait(0.1)
        assert node()["frames"] <= frames + 1, "hidden: no frames"
        pane.switch("dot")
        assert settle(d, lambda: seen == [False, True]), "…and that it's back"
        assert settle(d, lambda: node()["frames"] > frames + 1), "drawing again"


def test_an_add_on_can_refuse_to_start(addons: Path) -> None:
    ts.plugins(refresh=True)

    class Bad(StageApp):
        def compose(self):
            return [StagePane("dot", id="d", fail="no file")]

    with Bad().drive((60, 16)) as d:
        assert settle(d, lambda: "could not start" in d.snapshot().text), d.snapshot().text
        assert "asked to fail: no file" in d.snapshot().text


def test_keymaps_rename_turn_off_and_explain() -> None:
    from stagetui.plugin import Keymap, key_name

    actions = {"spin": ("space",), "wires": "w", "grid": ("g",), "back": ("shift+left",)}
    plain = Keymap(actions)
    assert plain.action("space") == "spin" and plain.label("back") == "shift+←"
    k = Keymap(
        actions, {"path": "a.glb", "keys": {"wires": "x", "grid": None, "spin": ["s", "space"]}}
    )
    assert k.action("x") == "wires" and k.action("w") is None, "the old key does nothing"
    assert k.keys("grid") == [] and k.label("grid") is None, "turned off"
    assert k.keys("spin") == ["s", "space"] and k.label("spin") == "s"
    assert k.all_keys() == ["s", "space", "x", "shift+left"]
    # Written like stagetui names keys — also keys of other keyboards.
    # As in BINDINGS, a capital letter means Shift.
    capital = Keymap(actions, {"keys": {"wires": "X"}})
    assert capital.action("shift+x") == "wires" and capital.action("x") is None
    assert capital.spoken("wires") == "Shift+x" and capital.spoken("back") == "Shift+←"
    assert Keymap({"open": ("enter", "o"), "go": "ctrl+g"}).spoken("go") == "Ctrl+G"
    assert Keymap({"open": ("enter", "o")}).spoken("open") == "Enter"
    for written, named in [
        ("Ctrl+w", "ctrl+w"),
        ("Ctrl+W", "ctrl+shift+w"),
        ("W", "shift+w"),
        ("Ж", "shift+ж"),
        ("ENTER", "enter"),
        ("shift+ctrl+left", "ctrl+shift+left"),
        ("plus", "+"),
        ("ctrl++", "ctrl++"),
        ("shift+?", "?"),
        ("Escape", "esc"),
        ("F5", "f5"),
        ("ب", "ب"),
        ("ctrl+あ", "ctrl+あ"),
        ("क", "क"),
    ]:
        assert key_name(written) == named, written
        assert key_name(named) == named, named
    assert key_name("ctrl+banana") is None and key_name("f25") is None and key_name("") is None
    # Mistakes say what to do.
    with pytest.raises(ValueError, match='"wire".*spin, wires, grid, back'):
        Keymap(actions, {"keys": {"wire": "x"}})
    with pytest.raises(ValueError, match="مَرْحَبًا Hello 日本語 नमस्ते"):
        Keymap(actions, {"keys": {"مَرْحَبًا Hello 日本語 नमस्ते": "x"}})
    with pytest.raises(ValueError, match=r'"wires" and "grid".*keys=\{"grid": None\}'):
        Keymap(actions, {"keys": {"wires": "g"}})
    with pytest.raises(ValueError, match=r'"ctrl\+banana" \(for "wires"\)'):
        Keymap(actions, {"keys": {"wires": "ctrl+banana"}})
    with pytest.raises(TypeError, match="None to turn it off"):
        Keymap(actions, {"keys": {"wires": 5}})
    with pytest.raises(TypeError, match=r'like keys=\{"spin": "x"\}'):
        Keymap(actions, {"keys": "x"})


def test_an_add_on_s_keys_can_be_renamed(addons: Path) -> None:
    ts.plugins(refresh=True)
    acted, pressed = [], []

    class Desk(StageApp):
        def compose(self):
            return [
                StagePane(
                    "dot",
                    id="d",
                    label="مَرْحَبًا Hello 日本語 नमस्ते",
                    keys={"ping": "x", "jump": None},
                    on_acted=acted.append,
                    on_pressed=pressed.append,
                )
            ]

    with Desk().drive((60, 16)) as d:
        info = lambda: d.query("#d")["children"][0]
        assert settle(d, lambda: info()["status"] is not None), info()
        d.click("#d")
        for old in ["p", "j", "up"]:  # its old key, and a turned-off action's keys
            d.press(old)
        d.press("x")
        assert settle(d, lambda: acted), "the renamed key reached action()"
        d.press("space")  # a plain key still goes to key()
        assert settle(d, lambda: pressed)
        assert acted == ["ping"] and pressed == [{"key": "space", "n": 1}]


def test_an_add_on_can_ask_for_every_key(addons: Path) -> None:
    ts.plugins(refresh=True)
    pressed = []

    class Desk(StageApp):
        def compose(self):
            return [StagePane("dot", id="d", label="مَرْحَبًا 日本語 नमस्ते", on_pressed=pressed.append)]

    with Desk().drive((60, 16)) as d:
        dot = d.app.query("#d").content
        assert settle(d, lambda: d.query("#d")["children"][0]["status"] is not None)
        d.click("#d")
        d.press("z")  # not one of its keys: the app's
        assert dot.call("typing", True) is True
        d.press("z")  # now it gets every key
        assert settle(d, lambda: pressed), "a key it didn't list arrived"
        dot.call("typing", False)
        d.press("z")
        d.wait(0.3)
        time.sleep(0.2)
        d.wait(0.1)
        assert [p["key"] for p in pressed] == ["z"], pressed


def test_an_add_on_explains_bad_keys(addons: Path) -> None:
    ts.plugins(refresh=True)

    class Bad(StageApp):
        def compose(self):
            return [
                StagePane("dot", id="a", keys={"pong": "x"}),
                StagePane("dot", id="b", keys={"ping": "j"}),
            ]

    with Bad().drive((100, 16)) as d:
        error = lambda pane: d.query(pane)["children"][0]["error"] or ""
        assert settle(d, lambda: '"pong"' in error("#a")), error("#a")
        assert "Dot could not start" in error("#a") and "ping, jump" in error("#a")
        assert settle(d, lambda: '"j" is given to both "ping" and "jump"' in error("#b"))


def test_official_add_ons_offer_to_install(addons: Path) -> None:
    found = {p["name"]: p for p in ts.plugins(refresh=True)}
    if found["3d"]["installed"]:
        pytest.skip("the 3D add-on is installed here")

    class Wants(StageApp):
        def compose(self):
            return [
                StageSplit(
                    [
                        StagePane("3d", id="v"),
                        StagePane("empty", id="e"),
                        StagePane("markdown", id="m", text="مَرْحَبًا Hello 日本語 नमस्ते"),
                    ]
                )
            ]

    with Wants().drive((120, 20)) as d:
        d.wait(0.1)
        assert "3D isn't installed yet" in d.snapshot().text
        # A click asks first; nothing is installed until you say yes.
        d.click("#v")
        assert "Install 3D?" in d.snapshot().text, d.snapshot().text
        d.press("esc")
        assert "Install 3D?" not in d.snapshot().text
        d.click("#e")
        d.press("ctrl+space")
        d.press("space")  # the pane menu lists every add-on
        menu = d.snapshot().text
        assert "Dot" in menu and "3D  (install…)" in menu, menu


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
