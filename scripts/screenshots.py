"""Make the pictures in docs/images/ from the real examples — no terminal needed.

Run:   python scripts/screenshots.py            (all of them)
       python scripts/screenshots.py web world  (just these)

Each picture is an example in `examples/`, driven like a person would use it
(typing, clicking), then saved with stagetui's own headless renderer. The web and
video pictures need those add-ons built (`cargo build -p stagetui-web -p stagetui-video`)
and a Chrome/Chromium and ffmpeg on this computer; they are skipped otherwise.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import time
from collections.abc import Callable
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "docs" / "images"
EXAMPLES = ROOT / "examples"

# A calm, private setup: no real home folder, no personal shell prompt, no probing.
CALM = tempfile.mkdtemp(prefix="stagetui-shots-")
Path(CALM, ".bashrc").write_text("PS1='\\$ '\n")
os.environ.update(
    {
        "STAGETUI_HEADLESS": "1",
        "HOME": CALM,
        "SHELL": "/bin/bash",
        "STAGETUI_PLUGIN_PATH": os.pathsep.join(
            [str(EXAMPLES / "plugins"), str(ROOT / "target" / "debug")]
        ),
    }
)
os.environ.pop("STAGETUI_SESSION", None)

import stagetui as ts
from stagetui._render import find_app


def until(d, check: Callable[[], bool], seconds: float = 20.0) -> bool:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        d.wait(0.05)
        try:
            if check():
                return True
        except ts.StageError:
            pass
        time.sleep(0.05)
    return False


def settle(d, seconds: float) -> None:
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        d.wait(0.05)
        time.sleep(0.05)


def harness(d) -> None:
    d.type("/run ls")
    d.press("enter")
    until(d, lambda: "Allow" in d.snapshot().text, 10)
    d.press("y")
    settle(d, 1.5)
    d.click((20, d.snapshot().rows - 2))  # back to the prompt
    d.type("What can stagetui do?")
    d.press("enter")
    settle(d, 4)
    d.type("/diff")
    d.press("enter")  # picks the command from the list
    settle(d, 0.5)
    if "/diff" in d.snapshot().text.splitlines()[-3]:
        d.press("enter")  # sends it
    settle(d, 1.5)


def gallery(d) -> None:
    d.type("Sara · سارة")
    d.press("tab")
    d.press("space")
    settle(d, 0.5)


def gallery_data(d) -> None:
    d.click("#tab-data") if d.query("#tab-data") else d.press("ctrl+right")
    settle(d, 0.5)


def wait_pictures(d) -> None:
    until(d, lambda: any(p.get("frames", 0) > 0 for p in _plugin_nodes(d)), 30)
    until(d, lambda: "loading" not in d.snapshot().text, 20)  # the web page has finished
    settle(d, 2)


def _plugin_nodes(d) -> list[dict]:
    found: list[dict] = []

    def walk(node: dict) -> None:
        if node.get("type") == "plugin":
            found.append(node)
        for child in node.get("children", []):
            walk(child)

    walk(d.snapshot().tree)
    return found


def make_video() -> Path:
    clip = Path(CALM, "fractal.mp4")
    subprocess.run(
        ["ffmpeg", "-loglevel", "error", "-f", "lavfi", "-i", "mandelbrot=s=640x360:rate=30",
         "-t", "6", "-pix_fmt", "yuv420p", str(clip)],
        check=True,
    )  # fmt: skip
    return clip


# name: (example file, size, what to do, arguments, needs)
SHOTS: dict[str, tuple[str, tuple[int, int], Callable | None, list[str], str]] = {
    "harness": ("07_harness.py", (120, 34), harness, [], ""),
    "gallery": ("05_ui_gallery.py", (110, 32), gallery, [], ""),
    "workspace": ("08_workspace.py", (120, 34), None, [], ""),
    "world": ("03_arabic_rtl.py", (70, 26), None, [], ""),
    "canvas": ("04_canvas.py", (90, 28), None, [], ""),
    "plugins": ("10_plugins.py", (100, 28), wait_pictures, [], ""),
    "web": ("12_web.py", (120, 34), wait_pictures, ["https://example.com"], "web"),
    "video": ("11_video.py", (110, 32), wait_pictures, ["<video>"], "video"),
    "3d": ("13_3d.py", (110, 32), wait_pictures, [], "3d"),
}


def shoot(name: str) -> None:
    file, size, act, args, needs = SHOTS[name]
    if needs == "web" and not (shutil.which("chromium") or shutil.which("google-chrome")
                               or os.environ.get("STAGETUI_CHROME")):  # fmt: skip
        print(f"  skip {name}: no Chrome/Chromium")
        return
    if needs and not any(p["name"] == needs and p["installed"] for p in ts.plugins(refresh=True)):
        print(f"  skip {name}: the {needs} add-on isn't built")
        return
    if "<video>" in args:
        if not shutil.which("ffmpeg"):
            print(f"  skip {name}: no ffmpeg")
            return
        args = [str(make_video())]
    sys.argv = [file, *args]
    app = find_app(EXAMPLES / file)
    with app().drive(size) as d:
        settle(d, 1)
        if act:
            act(d)
        png = d.snapshot(settle=True, png=True).png
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / f"{name}.png").write_bytes(png)
    print(f"  ✓ docs/images/{name}.png")


if __name__ == "__main__":
    os.chdir(EXAMPLES)  # the file-browser panes show the examples folder
    for name in sys.argv[1:] or list(SHOTS):
        shoot(name)
