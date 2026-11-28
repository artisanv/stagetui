# stagetui-video

The video add-on for [stagetui](https://github.com/artisanv/stagetui): play video
files and links in any pane, with sound.

```bash
pip install stagetui-video      # also installs stagetui itself (1.0 or newer)
```

```python
from stagetui import StageApp, StagePane


class Cinema(StageApp):
    def compose(self):
        return [StagePane("video", path="trip.mp4")]


Cinema().run()
```

**Space** pause · **← →** seek 5 s (**Shift** 30 s) · **↑ ↓** volume · **M** mute ·
**Home** start over · click to pause · wheel for volume.

ffmpeg comes with it (through `imageio-ffmpeg`), so there is nothing else to install.
Videos look best in a terminal with real pictures (Ghostty, Kitty, WezTerm, foot,
iTerm2, Windows Terminal); in other terminals stagetui can open the video pane in
one of those for you.

See the [add-ons guide](https://github.com/artisanv/stagetui/blob/main/docs/plugins.md).
