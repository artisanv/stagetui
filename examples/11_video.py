"""Video in a pane, with sound.

Run:   python examples/11_video.py path/to/movie.mp4
       (no file? it makes a short test clip with ffmpeg)
Needs: the video add-on: pip install stagetui-video
Try:   Space pause · ← → seek (Shift: 30 s) · ↑ ↓ volume · M mute · Home start over
       click the video to pause, scroll the wheel for volume
       click ↗ in the pane's title to open it in its own window
"""

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from stagetui import StageApp, StageBar, StagePane, StageSplit

NOTES = """# Now playing

Every pane can show a video — or switch to
anything else from its menu (click the title).

**Keys** (click the video first):

| Key | Does |
|---|---|
| Space | pause / play |
| ← → | seek 5 s |
| ↑ ↓ | volume |
| M | mute |
"""


def demo_clip() -> str:
    """A 10-second test clip (moving pattern and a soft tone)."""
    try:
        import imageio_ffmpeg

        ffmpeg = imageio_ffmpeg.get_ffmpeg_exe()
    except ImportError:
        ffmpeg = shutil.which("ffmpeg")
    if ffmpeg is None:
        sys.exit("Give a video file: python examples/11_video.py movie.mp4")
    out = Path(tempfile.gettempdir()) / "stagetui-demo-clip.mp4"
    if not out.exists():
        subprocess.run(
            [ffmpeg, "-hide_banner", "-loglevel", "error", "-y",
             "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=24:duration=10",
             "-f", "lavfi", "-i", "sine=frequency=330:duration=10",
             "-af", "volume=0.15", "-c:v", "mpeg4", "-q:v", "4", "-c:a", "aac",
             "-shortest", str(out)],
            check=True,
        )  # fmt: skip
    return str(out)


VIDEO = sys.argv[1] if len(sys.argv) > 1 else demo_clip()


class Cinema(StageApp):
    theme = "aurora-dark"

    def compose(self):
        return [
            StageSplit(
                [
                    StagePane("video", id="video", path=VIDEO, on_ended=self.ended),
                    StagePane("markdown", text=NOTES),
                ],
                ratio=[3, 1],
                grow=1,
            ),
            StageBar("Space pause · ← → seek · ↑ ↓ volume · M mute · q quit"),
        ]

    def ended(self, path):
        self.notify("The end — press Space to play it again")


if __name__ == "__main__":
    Cinema().run()
