"""Add-ons: pane types that live in their own program (video, web, 3D, your own).

Run:   python examples/10_plugins.py
       (with your virtual environment active: the add-on is a Python script that
       starts `python3`, and that Python needs stagetui too)
Try:   the left pane is "pulse", a tiny add-on written in Python
       (examples/plugins/stagetui-plugin-pulse): click it or press space
       the right pane is the Video add-on: if it isn't installed yet, press
       Enter in it and stagetui asks whether to install it
       click any pane's title to see every add-on in the menu
"""

import os
import subprocess
import tempfile
from pathlib import Path

from stagetui import StageApp, StageBar, StagePane, StageSplit

# stagetui looks for add-ons in STAGETUI_PLUGIN_PATH too: add the folder next to
# this file, where the "pulse" add-on lives.
PLUGINS = Path(__file__).resolve().parent / "plugins"
os.environ["STAGETUI_PLUGIN_PATH"] = os.pathsep.join(
    p for p in (str(PLUGINS), os.environ.get("STAGETUI_PLUGIN_PATH")) if p
)


def sample_video() -> str | None:
    """A short test clip, made with the ffmpeg that the Video add-on brings along.

    Before the add-on is installed there is no ffmpeg yet: the pane then offers to install it.
    """
    try:
        import imageio_ffmpeg
    except ImportError:
        return None
    out = Path(tempfile.gettempdir()) / "stagetui-sample.mp4"
    if not out.exists():
        subprocess.run(
            [imageio_ffmpeg.get_ffmpeg_exe(), "-loglevel", "error", "-y", "-f", "lavfi",
             "-i", "mandelbrot=size=640x360:rate=24", "-t", "8", "-pix_fmt", "yuv420p", str(out)],
            check=True,
        )  # fmt: skip
    return str(out)


class AddOns(StageApp):
    theme = "aurora-dark"

    def compose(self):
        return [
            StageSplit(
                [
                    StagePane("pulse", id="pulse", label="Hello", on_color=self.colored),
                    StagePane("video", id="video", path=sample_video()),
                ]
            ),
            StageBar("Click the heart · right: the Video add-on (missing? Enter installs it)"),
        ]

    def colored(self, color):
        self.notify(f"The add-on says: {color}")


if __name__ == "__main__":
    AddOns().run()
