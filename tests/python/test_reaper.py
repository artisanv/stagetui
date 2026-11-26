"""Zero zombies: tracked children die even when the Python parent is SIGKILLed."""

import os
import signal
import subprocess
import sys
import time

import pytest

pytestmark = pytest.mark.skipif(
    sys.platform == "win32", reason="Unix signals; Windows uses Job Objects"
)

PARENT = """
import sys, time
import stagetui._core as c
pid = c._spawn_tracked(["sleep", "600"])
print(pid, flush=True)
time.sleep(600)
"""


def alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    if sys.platform.startswith("linux"):
        try:
            with open(f"/proc/{pid}/stat") as f:
                return f.read().rsplit(")", 1)[1].split()[0] != "Z"
        except (FileNotFoundError, ProcessLookupError):  # reaped while we looked
            return False
    return True


def wait_dead(pid: int, timeout: float = 5.0) -> bool:
    """True once the process is gone (or a zombie, or already tearing down)."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if not alive(pid):
            return True
        time.sleep(0.05)
    return False


def test_children_die_with_sigkilled_parent():
    parent = subprocess.Popen(
        [sys.executable, "-c", PARENT], stdout=subprocess.PIPE, text=True, stdin=subprocess.DEVNULL
    )
    try:
        assert parent.stdout is not None
        child = int(parent.stdout.readline())
        assert alive(child)
        parent.send_signal(signal.SIGKILL)
        parent.wait(timeout=10)
        assert wait_dead(child), f"child {child} survived"
    finally:
        if parent.poll() is None:
            parent.kill()


def test_tracked_child_killed_on_normal_exit():
    code = PARENT.replace("time.sleep(600)", "")
    out = subprocess.run(
        [sys.executable, "-c", code],
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
        stdin=subprocess.DEVNULL,
    )
    child = int(out.stdout.strip())
    assert wait_dead(child), f"child {child} survived"
