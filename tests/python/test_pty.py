"""End-to-end in a real pseudo-terminal: this test *is* the terminal.

It answers termstage's capability queries like a Kitty-compatible terminal,
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

    env = {**os.environ, "TERM": "xterm-kitty", "TERMSTAGE_PROBE_TIMEOUT_MS": "2000", **extra_env}
    for key in ("TERMSTAGE_HEADLESS", "VTE_VERSION", "TMUX", "SSH_TTY", "SSH_CONNECTION"):
        if key not in extra_env:
            env.pop(key, None)
    pid, fd = pty.fork()
    if pid == 0:  # child: becomes the app
        os.execve(sys.executable, [sys.executable, str(ROOT / "examples" / "01_hello.py")], env)

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 1000, 600))
    out = bytearray()
    try:
        assert read_until(fd, b"Welcome to termstage", 15, out), out[-2000:]
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
    out = run_app_in_pty({"VTE_VERSION": "8002", "TERMSTAGE_TEXT_MODE": "cells"})
    assert b"\x1b[8l" in out, "explicit BiDi mode not enabled"
    assert b"\x1b[8h" in restore_tail(out), "BiDi mode not restored"
    assert "\ufee3".encode() in out, "Arabic should be shaped into presentation forms"
