"""Fixes before 1.0: what the examples promise, what agents and the command line
see, and what is left running after a headless run.

The add-on parts run when the 3D add-on is built (`STAGETUI_PLUGIN_PATH`, like
tests/python/test_3d.py); they are skipped otherwise.
"""

import ast
import json
import os
import runpy
import select
import subprocess
import sys
import time
from pathlib import Path

import pytest
import stagetui as ts
from stagetui import StageApp, StageBar, StageCard, StagePane, StageSplit

ROOT = Path(__file__).parents[2]
EXAMPLES = ROOT / "examples"
# Text tests carry every script (AGENTS.md rule 4).
TEXT = "مَرْحَبًا Hello مرحبا 日本語 中文 नमस्ते"
posix_only = pytest.mark.skipif(sys.platform == "win32", reason="checks processes with ps / a pty")


@pytest.fixture(autouse=True)
def private(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("STAGETUI_HEADLESS", "1")
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "config"))
    monkeypatch.setenv("APPDATA", str(tmp_path / "config"))
    monkeypatch.delenv("STAGETUI_KEYS", raising=False)


def running(pid: int) -> bool:
    """Still running (a finished process waiting to be collected doesn't count)."""
    out = subprocess.run(
        ["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True, check=False
    )
    state = out.stdout.strip()
    return bool(state) and not state.startswith("Z")


def gone(pid: int, seconds: float = 5.0) -> bool:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if not running(pid):
            return True
        time.sleep(0.05)
    return False


def descendants(pattern: str) -> list[int]:
    """Running processes started (directly or not) by this test, whose command has ``pattern``."""
    out = subprocess.run(
        ["ps", "-e", "-o", "pid=,ppid=,stat=,args="], capture_output=True, text=True, check=False
    )
    kids: dict[int, list[tuple[int, str, str]]] = {}
    for line in out.stdout.splitlines():
        parts = line.split(None, 3)
        if len(parts) == 4:
            kids.setdefault(int(parts[1]), []).append((int(parts[0]), parts[2], parts[3]))
    found, todo = [], [os.getpid()]
    while todo:
        for pid, stat, args in kids.get(todo.pop(), []):
            todo.append(pid)
            if pattern in args and not stat.startswith("Z"):
                found.append(pid)
    return found


def until(d, check, seconds: float = 10.0) -> bool:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        d.wait(0.05)
        if check():
            return True
        time.sleep(0.05)
    return False


# ----- the examples -----------------------------------------------------------------

BUILT_IN_ACTIONS = {
    "quit",
    "detach",
    "pop_out",
    "detach_focused",
    "focus_next",
    "focus_previous",
    "next_theme",
    "close_modal",
    "pane_menu",
    "split_pane_right",
    "split_pane_down",
    "close_pane",
}


@pytest.mark.parametrize("example", sorted(EXAMPLES.glob("*.py")), ids=lambda p: p.name)
def test_no_example_binds_a_key_to_a_method_that_can_never_run(example: Path) -> None:
    """A shortcut named like a built-in action runs stagetui's version: an
    ``action_<name>`` method for it would never be called."""
    tree = ast.parse(example.read_text(encoding="utf-8"))
    for cls in (n for n in ast.walk(tree) if isinstance(n, ast.ClassDef)):
        methods = {f.name for f in cls.body if isinstance(f, ast.FunctionDef)}
        for stmt in cls.body:
            if isinstance(stmt, ast.Assign) and any(
                isinstance(t, ast.Name) and t.id == "BINDINGS" for t in stmt.targets
            ):
                for binding in ast.literal_eval(stmt.value):
                    action = binding[1]
                    assert not (action in BUILT_IN_ACTIONS and f"action_{action}" in methods), (
                        f"{example.name}: {action!r} is built in, so action_{action} never runs"
                    )


def test_the_splits_example_counts_its_panes() -> None:
    splits = runpy.run_path(str(EXAMPLES / "06_splits.py"), run_name="example")["Splits"]
    app = splits()
    status = lambda d: d.query("#status")["text"]
    with app.drive((100, 24)) as d:
        d.press("|")
        d.press("-")
        assert status(d) == "3 panes"
        d.press("x")  # closes the focused (newest) pane
        assert status(d) == "2 panes" and len(app.pane_ids()) == 2
        d.press("tab")
        d.press("ctrl+space")
        d.press("x")  # the prefix way runs the app's own close too
        assert status(d) == "1 pane"
        d.press("tab")
        d.press("x")
        assert status(d) == "1 pane", "the last pane stays"
        assert any("last pane" in t for t in d.snapshot().tree["toasts"])
        d.press("|")
        assert status(d) == "2 panes"


@pytest.mark.parametrize(
    "name",
    ["01_hello.py", "03_arabic_rtl.py", "04_canvas.py", "05_ui_gallery.py", "06_splits.py"]
    + ["07_harness.py", "08_workspace.py", "09_detach.py"],
)
def test_every_example_starts_and_quits_with_ctrl_space_q(name: str) -> None:
    namespace = runpy.run_path(str(EXAMPLES / name), run_name="example")
    app_class = [
        v
        for v in namespace.values()
        if isinstance(v, type) and issubclass(v, StageApp) and v.__module__ == "example"
    ][-1]
    with app_class().drive((100, 30)) as d:
        d.wait(0.2)
        assert d.snapshot().text.strip()
        d.press("ctrl+space")
        d.press("q")
        assert d.has_quit, f"{name}: Ctrl+Space then q quits from anywhere"


# ----- the command line gives the app its own sys.argv ------------------------------

ARGV_APP = """
import sys
from stagetui import StageApp, StageCard

SEEN = list(sys.argv)

class Echo(StageApp):
    def compose(self):
        return [StageCard(repr((SEEN, list(sys.argv))) + " {text}", id="argv")]
"""


@pytest.mark.parametrize("tool", ["drive", "render"])
def test_command_line_tools_give_the_app_its_own_argv(tmp_path: Path, tool: str) -> None:
    app = tmp_path / "echo_app.py"
    app.write_text(ARGV_APP.format(text=TEXT), encoding="utf-8")
    if tool == "drive":
        proc = subprocess.run(
            [sys.executable, "-m", "stagetui._agent", "drive", str(app), "--size", "200x8"],
            input=json.dumps(
                {"jsonrpc": "2.0", "id": 1, "method": "query", "params": {"id": "argv"}}
            )
            + "\n",
            capture_output=True,
            encoding="utf-8",
            timeout=60,
            check=False,
        )
        assert proc.returncode == 0, proc.stderr
        shown = json.loads(proc.stdout)["result"]["text"]
    else:
        out = tmp_path / "tree.json"
        proc = subprocess.run(
            [
                sys.executable,
                "-m",
                "stagetui._render",
                str(app),
                "--size",
                "200x8",
                "--tree",
                str(out),
            ],
            capture_output=True,
            encoding="utf-8",
            timeout=60,
            check=False,
        )
        assert proc.returncode == 0, proc.stderr
        node = next(c for c in json.loads(out.read_text(encoding="utf-8"))["children"])
        shown = node["text"]
    assert shown == repr(([str(app)], [str(app)])) + f" {TEXT}"


def test_keys_for_one_app_read_the_persons_keys_file(tmp_path: Path) -> None:
    """`stagetui keys app.py` reads the same keys file as plain `stagetui keys`."""
    config = tmp_path / "config"
    keys_file = config / "stagetui" / "keys.json"
    keys_file.parent.mkdir(parents=True)
    keys_file.write_text(json.dumps({"close_pane": "w"}), encoding="utf-8")
    app = tmp_path / "keys_app.py"
    app.write_text(
        "from stagetui import StageApp, StageBar, StagePane\n"
        "class App(StageApp):\n"
        "    def compose(self):\n"
        f"        return [StagePane('markdown', text={TEXT!r}), StageBar({TEXT!r})]\n",
        encoding="utf-8",
    )
    env = {**os.environ, "XDG_CONFIG_HOME": str(config), "APPDATA": str(config)}
    env.pop("STAGETUI_HEADLESS", None)  # a person's run (the app itself still runs headless)
    run = lambda *args: subprocess.run(
        [sys.executable, "-m", "stagetui", "keys", *args],
        capture_output=True,
        encoding="utf-8",
        env=env,
        timeout=60,
        check=False,
    )
    data = json.loads(run(str(app), "--json").stdout)
    assert data["file"] == str(keys_file)
    assert {k["name"]: k["keys"] for k in data["keys"]}["close_pane"] == ["w"]
    text = run(str(app)).stdout
    assert str(keys_file) in text and "not read here" not in text
    assert str(keys_file) in run().stdout, "the same file as plain `stagetui keys`"


# ----- headless runs stop what they started -----------------------------------------


class Shell(StageApp):
    def compose(self):
        return [
            StagePane(
                "terminal", id="t", command=[sys.executable, "-c", "import time; time.sleep(120)"]
            ),
            StageBar(TEXT),
        ]


@posix_only
def test_a_driver_stops_its_shells_when_it_closes() -> None:
    d = Shell().drive((80, 20))
    pid_of = lambda: d.query("#t")["children"][0].get("pid")
    assert until(d, lambda: pid_of() is not None), "the program started"
    pid = pid_of()
    snap = d.snapshot()
    assert running(pid)
    d.close()
    assert gone(pid), "closing the driver stopped the shell's program at once"
    d.close()  # closing twice does nothing
    assert snap.png.startswith(b"\x89PNG"), "a snapshot still makes its picture"
    assert snap.query("#t")["type"] == "pane" and TEXT in snap.text


@posix_only
def test_run_headless_leaves_nothing_running() -> None:
    snap = Shell().run_headless((80, 20), steps=[{"wait": 0.2}])
    pid = snap.query("#t")["children"][0].get("pid")
    assert pid, "the shell had started"
    assert gone(pid), "run_headless() returned: the program is gone"
    assert snap.png.startswith(b"\x89PNG")
    assert not running(pid), "making the picture doesn't start it again"


@pytest.fixture
def model(tmp_path: Path) -> Path:
    if not any(p["name"] == "3d" and p["installed"] for p in ts.plugins(refresh=True)):
        pytest.skip("the 3D add-on isn't built here")
    path = tmp_path / f"{TEXT}.obj"
    path.write_text(
        "v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\n"
        "f 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 3 4 8 7\nf 2 3 7 6\nf 1 5 8 4\n",
        encoding="utf-8",
    )
    return path


@posix_only
def test_an_add_on_stops_when_the_run_ends_and_its_picture_stays(model: Path) -> None:
    class Viewer(StageApp):
        def compose(self):
            return [StagePane("3d", id="m", path=str(model)), StageBar(TEXT)]

    with Viewer().drive((80, 24)) as d:
        node = lambda: d.query("#m")["children"][0]
        assert until(d, lambda: node().get("frames", 0) > 0 and node().get("pid"))
        pid = node()["pid"]
        snap = d.snapshot()
        live = d.snapshot(png=True).png
    assert gone(pid), "the add-on stopped when the driver closed"
    before = set(descendants("stagetui-plugin-3d"))
    late = snap.png  # painted after the run: the last picture, no new add-on
    assert ts._core.png_diff(live, late) < 0.2
    time.sleep(0.3)
    assert not set(descendants("stagetui-plugin-3d")) - before, "painting didn't restart it"


# ----- quitting a headless app --------------------------------------------------------


@posix_only
def test_after_quitting_a_driver_can_look_but_not_act() -> None:
    with Shell().drive((80, 20)) as d:
        pid_of = lambda: d.query("#t")["children"][0].get("pid")
        assert until(d, lambda: pid_of() is not None)
        pid = pid_of()
        assert not d.has_quit
        d.press("ctrl+space")
        d.press("q")
        assert d.has_quit
        assert gone(pid), "quitting stops the app's programs, like a real run"
        assert TEXT in d.snapshot().text, "the last screen can still be seen"
        d.wait(0.1)  # time passes; nothing happens
        with pytest.raises(ts.StageError, match="The app has quit") as e:
            d.type(TEXT)
        assert "snapshot" in e.value.fix


def test_background_work_never_holds_up_a_headless_step() -> None:
    """An agent loop in background() runs as time passes; steps don't wait for it."""
    import asyncio

    class Busy(StageApp):
        ticks = 0

        def compose(self):
            return [StageCard(TEXT, id="n"), StageBar(TEXT)]

        def on_mount(self):
            self.background(self.forever())

        async def forever(self):
            while True:
                self.ticks += 1
                await asyncio.sleep(0.05)

    with Busy().drive((60, 10)) as d:
        started = time.monotonic()
        d.press("tab")
        d.click("#n")
        assert time.monotonic() - started < 0.4, "steps don't wait for background work"
        started = time.monotonic()
        d.wait(0.5)
        assert time.monotonic() - started < 2.0, "wait(0.5) takes about half a second"
        assert d.app.ticks >= 3, "the background work ran meanwhile"


# ----- unknown pane types ---------------------------------------------------------------


def test_an_unknown_pane_type_lists_every_kind_and_how_to_get_add_ons() -> None:
    @ts.pane_type("fix-test-clock", title=TEXT)
    def clock():
        return StageCard(TEXT)

    try:
        with pytest.raises(ts.StageError) as e:
            StagePane("pulse-not-here")
    finally:
        from stagetui import harness

        harness._PANE_TYPES.pop("fix-test-clock", None)
    message = str(e.value)
    assert 'Unknown pane type "pulse-not-here"' in message
    for part in ("chat", "terminal", "markdown", "fix-test-clock", "add-ons installed here"):
        assert part in e.value.why, part
    for part in (
        "stagetui plugins",
        "pip install stagetui-pulse-not-here",
        "stagetui-plugin-pulse-not-here",
        "STAGETUI_PLUGIN_PATH",
        sys.executable,
        "Python where stagetui is installed",
    ):
        assert part in e.value.fix, part


def test_an_add_on_that_does_not_answer_is_named_not_lost(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A `stagetui-plugin-*` file that fails `--describe` (say, its #! Python has
    no stagetui) is listed with the reason, and the pane error names the file."""
    says = "ModuleNotFoundError: No module named 'stagetui' " + TEXT
    folder = tmp_path / "add-ons"
    folder.mkdir()
    if sys.platform == "win32":
        plugin = folder / "stagetui-plugin-pulse-test.cmd"
        plugin.write_text(f"@echo off\r\necho {says} 1>&2\r\nexit /b 1\r\n", encoding="utf-8")
    else:
        plugin = folder / "stagetui-plugin-pulse-test"
        plugin.write_text(
            f"#!/bin/sh\necho 'Traceback (most recent call last):' >&2\necho \"{says}\" >&2\nexit 1\n",
            encoding="utf-8",
        )
        plugin.chmod(0o755)
    extra = os.pathsep.join(filter(None, [str(folder), os.environ.get("STAGETUI_PLUGIN_PATH")]))
    try:
        with monkeypatch.context() as m:
            m.setenv("STAGETUI_PLUGIN_PATH", extra)
            entry = next(p for p in ts.plugins(refresh=True) if p["name"] == "pulse-test")
            assert entry["installed"] is False and entry["broken"]["path"] == str(plugin)
            if sys.platform != "win32":
                assert entry["broken"]["why"] == says, "the error it printed, not the traceback"
            assert all("broken" not in p for p in ts.plugins() if p["installed"])

            with pytest.raises(ts.StageError) as e:
                StagePane("pulse-test")
            assert 'The add-on "pulse-test" was found' in e.value.what
            assert str(plugin) in e.value.why and "--describe" in e.value.fix
            assert "#!" in e.value.fix and sys.executable in e.value.fix

            run = lambda *a: subprocess.run(
                [sys.executable, "-m", "stagetui", "plugins", *a],
                capture_output=True,
                encoding="utf-8",
                env={**os.environ, "STAGETUI_PLUGIN_PATH": extra},
                timeout=60,
                check=False,
            )
            listed = {p["name"]: p for p in json.loads(run("--json").stdout)}
            assert listed["pulse-test"]["broken"]["path"] == str(plugin)
            assert {"video", "web", "3d"} <= set(listed), "the official add-ons are still offered"
            assert f"⚠ found {plugin} but it didn't answer" in run().stdout
    finally:
        ts.plugins(refresh=True)  # forget the folder for the tests that follow


# ----- a pane's options come back with a saved layout ----------------------------------


def test_set_type_options_come_back_with_a_saved_layout(tmp_path: Path) -> None:
    class Desk(StageApp):
        def compose(self):
            return [
                StageSplit(
                    [StagePane("markdown", id="notes", text=TEXT), StagePane("empty", id="e")]
                )
            ]

    file = tmp_path / "layouts.json"
    source = f"print({TEXT!r})\n"
    first = Desk()
    with first.drive((120, 20)):
        first.query("#e").set_type("code", text=source, language="python")
        first.save_workspace("mine", path=file)
    saved = json.loads(file.read_text(encoding="utf-8"))["workspaces"]["mine"]["children"]
    assert saved[1] == {"pane": "code", "id": "e", "args": {"text": source, "language": "python"}}

    again = Desk()
    with again.drive((120, 20)) as d:
        assert d.query("#e")["pane_type"] == "empty"
        again.load_workspace("mine", path=file)
        d.wait(0.1)
        pane = d.query("#e")
        assert pane["pane_type"] == "code"
        assert [c["text"] for c in pane["children"] if c["type"] == "code"] == [source]


# ----- crashes are shown once, cleanly -------------------------------------------------

CRASH = """
from stagetui import StageApp, StageCard

class Boom(StageApp):
    def compose(self):
        return [StageCard("{text}")]

    def on_mount(self):
        raise ValueError("{text}")

Boom().run()
"""


@posix_only
@pytest.mark.filterwarnings("ignore:This process .* is multi-threaded:DeprecationWarning")
def test_an_error_in_the_app_is_shown_once_without_asyncio_noise(tmp_path: Path) -> None:
    import pty

    script = tmp_path / "boom.py"
    script.write_text(CRASH.format(text=TEXT), encoding="utf-8")
    env = {
        **os.environ,
        "TERM": "xterm-256color",
        "STAGETUI_NO_PROBE": "1",  # this test is the terminal: no questions to answer
        "STAGETUI_NO_SESSION": "1",
        "PYTHON_COLORS": "0",  # plain tracebacks, even in a terminal
    }
    env.pop("STAGETUI_HEADLESS", None)
    pid, fd = pty.fork()
    if pid == 0:  # child: becomes the app
        os.execve(sys.executable, [sys.executable, str(script)], env)
    out = bytearray()
    deadline = time.monotonic() + 30
    try:
        while time.monotonic() < deadline:
            if select.select([fd], [], [], 0.1)[0]:
                try:
                    chunk = os.read(fd, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                out.extend(chunk)
        _, status = os.waitpid(pid, 0)
    finally:
        try:
            os.kill(pid, 9)
        except ProcessLookupError:
            pass
        os.close(fd)
    text = out.decode("utf-8", "replace")
    assert os.waitstatus_to_exitcode(status) == 1
    assert text.count("Traceback (most recent call last)") == 1, text[-3000:]
    assert "During handling" not in text and "no running event loop" not in text
    assert "ValueError: " + TEXT in text
