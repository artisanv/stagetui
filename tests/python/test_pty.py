"""End-to-end in a real pseudo-terminal: this test *is* the terminal.

It answers stagetui's capability queries like a Kitty-compatible terminal,
checks the first frame is drawn with synchronized output, presses `q`, and
verifies every terminal mode is restored.
"""

import os
import select
import sys
import time
from pathlib import Path

import pytest

pytestmark = pytest.mark.skipif(sys.platform == "win32", reason="POSIX pty")

ROOT = Path(__file__).parents[2]
FAKE_TERMINAL_REPLIES = {
    b"\x1b[>0q": b"\x1bP>|kitty(0.39.1)\x1b\\",
    b"a=q": b"\x1b_Gi=31;OK\x1b\\",
    b"\x1b[?u": b"\x1b[?0u",
    b"\x1b[?2026$p": b"\x1b[?2026;2$y",
    b"\x1b[16t": b"\x1b[6;20;10t",
    b"\x1b[c": b"\x1b[?62;22c",
}


def read_until(fd: int, needle: bytes, timeout: float, buf: bytearray) -> bool:
    deadline = time.monotonic() + timeout
    answered: set[bytes] = set()
    while time.monotonic() < deadline:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                return needle in buf
            if not chunk:
                return needle in buf
            buf.extend(chunk)
        for query, reply in FAKE_TERMINAL_REPLIES.items():
            if query in buf and query not in answered:
                answered.add(query)
                os.write(fd, reply)
        if needle in buf:
            return True
    return False


def run_app_in_pty(extra_env: dict[str, str]) -> bytes:
    import fcntl
    import pty
    import struct
    import termios

    env = {**os.environ, "TERM": "xterm-kitty", "STAGETUI_PROBE_TIMEOUT_MS": "2000", **extra_env}
    for key in ("STAGETUI_HEADLESS", "VTE_VERSION", "TMUX", "SSH_TTY", "SSH_CONNECTION"):
        if key not in extra_env:
            env.pop(key, None)
    pid, fd = pty.fork()
    if pid == 0:  # child: becomes the app
        os.execve(sys.executable, [sys.executable, str(ROOT / "examples" / "01_hello.py")], env)

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 1000, 600))
    out = bytearray()
    try:
        assert read_until(fd, b"Welcome to stagetui", 15, out), out[-2000:]
        os.write(
            fd, b"\x1b[113u" if b"\x1b[>1u" in out else b"q"
        )  # 'q' (kitty key event if enabled)
        read_until(fd, b"\x1b[?1049l", 10, out)
        _, status = os.waitpid(pid, 0)
    finally:
        try:
            os.kill(pid, 9)
        except ProcessLookupError:
            pass
        os.close(fd)
    assert os.waitstatus_to_exitcode(status) == 0
    return bytes(out)


def restore_tail(out: bytes) -> bytes:
    return out[out.rfind(b"\x1b[?2026l") :]


def test_interactive_app_draws_and_restores_terminal():
    out = run_app_in_pty({})
    assert b"\x1b[?1049h" in out, "alternate screen not entered"
    assert b"\x1b[?2026h" in out, "frame not wrapped in synchronized output"
    assert b"\x1b[>1u" in out, "kitty keyboard protocol not enabled"
    assert b"a=t,f=100" in out, "Arabic should be sent as pixels on Kitty"
    tail = restore_tail(out)
    for seq in (
        b"\x1b[<u",
        b"\x1b[?1004l",
        b"\x1b[?2004l",
        b"\x1b[?1000l",
        b"\x1b[?25h",
        b"\x1b[?1049l",
    ):
        assert seq in tail, f"mode not restored: {seq!r}"
    assert b"\x1b[8l" not in out, "BiDi mode must not be touched on non-BiDi terminals"


def test_vte_gets_presentation_forms_and_explicit_bidi():
    out = run_app_in_pty({"VTE_VERSION": "8002", "STAGETUI_TEXT_MODE": "cells"})
    assert b"\x1b[8l" in out, "explicit BiDi mode not enabled"
    assert b"\x1b[8h" in restore_tail(out), "BiDi mode not restored"
    assert "\ufee3".encode() in out, "Arabic should be shaped into presentation forms"


INTERACTIVE_APP = """
import sys
from stagetui import Stage, StageApp, StageButton, StageCard

class App(StageApp):
    def __init__(self):
        super().__init__()
        self.clicks = 0

    def compose(self):
        return [
            StageCard("waiting", id="status", title="Status"),
            StageButton("Press me", id="btn", on_click=self.pressed),
        ]

    async def pressed(self):
        self.clicks += 1
        self.query("#status").update(["", "first-click", "second-press"][min(self.clicks, 2)])

app = App()
if len(sys.argv) > 1:  # print where the button is (headless, same size)
    b = app.run_headless((100, 30)).query("btn")["bounds"]
    print(b["x"] + b["w"] // 2, b["y"])
else:
    app.run()
    print("RESULT", app.clicks)
"""


def test_mouse_and_keys_reach_python_handlers(tmp_path):
    import fcntl
    import pty
    import struct
    import subprocess
    import termios

    script = tmp_path / "interactive.py"
    script.write_text(INTERACTIVE_APP, encoding="utf-8")
    where = subprocess.run(
        [sys.executable, str(script), "where"],
        capture_output=True,
        encoding="utf-8",
        env={**os.environ, "STAGETUI_HEADLESS": "1"},
        check=True,
    )
    bx, by = map(int, where.stdout.split())

    env = {**os.environ, "TERM": "xterm-kitty", "STAGETUI_PROBE_TIMEOUT_MS": "2000"}
    for key in ("STAGETUI_HEADLESS", "VTE_VERSION", "TMUX", "SSH_TTY", "SSH_CONNECTION"):
        env.pop(key, None)
    pid, fd = pty.fork()
    if pid == 0:
        os.execve(sys.executable, [sys.executable, str(script)], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 1000, 600))
    out = bytearray()
    try:
        assert read_until(fd, b"waiting", 15, out), out[-2000:]
        # A real SGR mouse click (press + release) on the button.
        os.write(fd, f"\x1b[<0;{bx + 1};{by + 1}M\x1b[<0;{bx + 1};{by + 1}m".encode())
        assert read_until(fd, b"first-click", 10, out), out[-2000:]
        # Keyboard: Tab to the button, Enter to press it.
        os.write(fd, b"\t")
        time.sleep(0.2)
        os.write(fd, b"\r")
        # Only changed cells are sent, so wait for a redraw rather than the whole word.
        n = len(out)
        assert read_until(fd, b"\x1b[?2026l", 10, out) and len(out) > n
        time.sleep(0.3)
        os.write(fd, b"\x1b[113u" if b"\x1b[>1u" in out else b"q")
        assert read_until(fd, b"RESULT 2", 10, out), out[-2000:]
        _, status = os.waitpid(pid, 0)
    finally:
        try:
            os.kill(pid, 9)
        except ProcessLookupError:
            pass
        os.close(fd)
    assert os.waitstatus_to_exitcode(status) == 0
    assert b"\x1b[?1049l" in restore_tail(bytes(out)) or b"\x1b[?1049l" in out
