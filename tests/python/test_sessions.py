"""Sessions end to end: start an app in the background, attach a (pseudo) terminal,
detach, reattach — and nothing is lost: typed text, a running shell, the app itself.

Every test uses its own private session folder (XDG_RUNTIME_DIR), so it never sees
or touches the sessions of the person running the tests.
"""

import json
import os
import select
import shutil
import subprocess
import sys
import tempfile
import textwrap
import time
from collections.abc import Iterator
from pathlib import Path

import pytest

pytestmark = pytest.mark.skipif(sys.platform == "win32", reason="POSIX pty")

APP = textwrap.dedent(
    """
    from stagetui import StageApp, StageCard, StageInput, StagePane, StageSplit


    class Notes(StageApp):
        def compose(self):
            return [
                StageCard("Background test", title="Notes app"),
                StageInput(id="box", placeholder="write here"),
                StagePane("terminal", id="sh", command="sh", height=8),
            ]


    Notes().run()
    """
)


@pytest.fixture
def env() -> Iterator[dict[str, str]]:
    # Socket paths must be short (about 100 characters on macOS): not under tmp_path.
    runtime = Path(tempfile.mkdtemp(prefix="st-", dir="/tmp"))
    e = {
        **os.environ,
        "XDG_RUNTIME_DIR": str(runtime),
        "TERM": "xterm-256color",
        "STAGETUI_NO_PROBE": "1",
        "PS1": "$ ",
    }
    for key in ("STAGETUI_HEADLESS", "VTE_VERSION", "TMUX", "SSH_TTY", "SSH_CONNECTION"):
        e.pop(key, None)
    yield e
    shutil.rmtree(runtime, ignore_errors=True)


def cli(env: dict[str, str], *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-m", "stagetui", *args],
        check=False,
        env=env,
        capture_output=True,
        encoding="utf-8",
        timeout=60,
    )


def sessions(env: dict[str, str]) -> list[str]:
    return [s["name"] for s in json.loads(cli(env, "sessions", "--json").stdout)]


class Window:
    """A terminal window running `stagetui attach`."""

    def __init__(self, env: dict[str, str], name: str) -> None:
        import fcntl
        import pty
        import struct
        import termios

        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.execve(sys.executable, [sys.executable, "-m", "stagetui", "attach", name], env)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 1000, 600))
        self.out = bytearray()

    def wait_for(self, needle: str, timeout: float = 20) -> bool:
        want = needle.encode()
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if want in self.out:
                return True
            r, _, _ = select.select([self.fd], [], [], 0.05)
            if r:
                try:
                    chunk = os.read(self.fd, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                self.out.extend(chunk)
        return want in self.out

    def send(self, data: str) -> None:
        os.write(self.fd, data.encode())
        time.sleep(0.15)

    def close(self) -> int:
        self.wait_for("\x00never", timeout=0.5)
        deadline = time.monotonic() + 15
        status = None
        while time.monotonic() < deadline:
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid:
                break
            self.wait_for("\x00never", timeout=0.2)
        else:
            os.kill(self.pid, 9)
            os.waitpid(self.pid, 0)
        os.close(self.fd)
        return os.waitstatus_to_exitcode(status) if status is not None else -9


def test_detach_and_reattach_keep_everything(tmp_path: Path, env: dict[str, str]) -> None:
    app = tmp_path / "notes_app.py"
    app.write_text(APP, encoding="utf-8")
    started = cli(env, "start", str(app), "--name", "notes", "--detached")
    assert started.returncode == 0, started.stderr
    assert "stagetui attach notes" in started.stderr
    try:
        assert sessions(env) == ["notes"]

        # First window: type into the box and run something in the shell.
        w = Window(env, "notes")
        assert w.wait_for("Notes app"), bytes(w.out[-2000:])
        w.send("hello مَرْحَبًا 日本語 नमस्ते")
        assert w.wait_for("hello"), bytes(w.out[-2000:])
        w.send("\t")  # to the shell
        w.send("echo stagetui-$((6*7))\r")
        assert w.wait_for("stagetui-42"), bytes(w.out[-3000:])
        w.send("\x00")  # Ctrl+Space (the prefix) …
        w.send("d")  # … then d: detach
        assert w.wait_for("Detached from"), bytes(w.out[-2000:])
        assert w.close() == 0
        assert b"\x1b[?1049l" in w.out, "the window's terminal is restored"

        # The app kept running; a new window sees the same state.
        assert sessions(env) == ["notes"]
        w = Window(env, "notes")
        assert w.wait_for("stagetui-42"), "the shell kept its output"
        assert w.wait_for("hello"), "the text box kept its text"
        w.send("\x00")
        w.send("q")  # Ctrl+Space then q: leaving asks first …
        assert w.wait_for("Quit app"), bytes(w.out[-2000:])
        w.send("q")  # … q again quits the app
        assert w.wait_for("has quit"), bytes(w.out[-2000:])
        assert w.close() == 0
        deadline = time.monotonic() + 10
        while sessions(env) and time.monotonic() < deadline:
            time.sleep(0.1)
        assert sessions(env) == []
    finally:
        cli(env, "stop", "notes")


def test_stop_attach_errors_and_crash_report(tmp_path: Path, env: dict[str, str]) -> None:
    app = tmp_path / "notes_app.py"
    app.write_text(APP, encoding="utf-8")
    assert cli(env, "start", str(app), "-d").returncode == 0
    assert sessions(env) == ["notes"], "named after the app's class"
    listed = cli(env, "sessions").stdout
    assert "notes" in listed and "stagetui attach" in listed
    stopped = cli(env, "stop")  # the only session
    assert stopped.returncode == 0 and "stopped" in stopped.stderr
    assert sessions(env) == []

    missing = cli(env, "attach", "nope")
    assert missing.returncode == 1 and "fix:" in missing.stderr

    broken = tmp_path / "broken.py"
    broken.write_text("raise SystemExit('this app is broken')\n", encoding="utf-8")
    failed = cli(env, "start", str(broken), "-d")
    assert failed.returncode == 1
    assert "this app is broken" in failed.stderr, failed.stderr

    bad = cli(env, "start", str(app), "--name", "no spaces")
    assert bad.returncode == 1 and "session name" in bad.stderr


def test_session_folder_is_private(tmp_path: Path, env: dict[str, str]) -> None:
    app = tmp_path / "notes_app.py"
    app.write_text(APP, encoding="utf-8")
    assert cli(env, "start", str(app), "-d").returncode == 0
    try:
        folder = Path(env["XDG_RUNTIME_DIR"]) / "stagetui"
        assert folder.stat().st_mode & 0o777 == 0o700
        for f in folder.iterdir():
            if f.suffix in (".sock", ".token", ".json", ".log"):
                assert f.stat().st_mode & 0o077 == 0, f"{f.name} is readable by others"
    finally:
        cli(env, "stop")


PANES_APP = textwrap.dedent(
    """
    from stagetui import StageApp, StagePane, StageSplit


    class Desk(StageApp):
        def compose(self):
            return [
                StageSplit(
                    [
                        StagePane("markdown", id="doc", text="# Notes"),
                        StagePane("terminal", id="sh", command="sh"),
                    ]
                )
            ]


    Desk().run()
    """
)

RECORDER = "import sys, pathlib; pathlib.Path(sys.argv[1]).write_text(' '.join(sys.argv[2:]))\n"


def test_a_pane_pops_out_into_its_own_window(tmp_path: Path, env: dict[str, str]) -> None:
    app = tmp_path / "desk.py"
    app.write_text(PANES_APP, encoding="utf-8")
    recorder, asked = tmp_path / "rec.py", tmp_path / "asked.txt"
    recorder.write_text(RECORDER, encoding="utf-8")
    # Instead of opening a real terminal window, write down what would run in it.
    env = {**env, "STAGETUI_LAUNCHER": f"{sys.executable} {recorder} {asked} {{cmd}}"}
    assert cli(env, "start", str(app), "-d").returncode == 0
    try:
        main = Window(env, "desk")
        assert main.wait_for("Notes"), bytes(main.out[-2000:])
        main.send("\x00")  # Ctrl+Space …
        main.send("o")  # … o: open the focused pane (the shell) in its own window
        assert main.wait_for("Bring it back"), bytes(main.out[-2000:])
        deadline = time.monotonic() + 10
        while not asked.exists() and time.monotonic() < deadline:
            time.sleep(0.05)
        assert asked.read_text(encoding="utf-8").endswith("-m stagetui attach desk:sh")

        pop = Window(env, "desk:sh")  # what the new window runs
        assert pop.wait_for("Put back"), bytes(pop.out[-2000:])
        pop.send("echo pop-$((6*7))\r")
        assert pop.wait_for("pop-42"), bytes(pop.out[-2000:])

        # Closing the window brings the pane back, shell and all.
        os.kill(pop.pid, 1)  # SIGHUP: the window is closed
        pop.close()
        main.out.clear()
        main.send("\x00")
        main.send("\t")  # a key, so the screen is drawn again
        assert main.wait_for("pop-42"), bytes(main.out[-2000:])
        main.send("\x00")
        main.send("d")
        assert main.close() == 0
    finally:
        cli(env, "stop")
