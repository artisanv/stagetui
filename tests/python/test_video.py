"""The video add-on, end to end: a clip made on the spot, played (silently) in a pane.

Runs when the add-on (`stagetui-plugin-video`) and ffmpeg are available — in CI the
"Video add-on" job builds it and points STAGETUI_PLUGIN_PATH at it.
"""

import shutil
import subprocess
import time
from pathlib import Path

import pytest
import stagetui as ts
from stagetui import StageApp, StagePane


def ffmpeg() -> str | None:
    try:
        import imageio_ffmpeg

        return imageio_ffmpeg.get_ffmpeg_exe()
    except ImportError:
        return shutil.which("ffmpeg")


@pytest.fixture
def clip(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    monkeypatch.setenv("STAGETUI_HEADLESS", "1")
    monkeypatch.setenv("STAGETUI_VIDEO_NO_AUDIO", "1")  # tests never make noise
    if not any(p["name"] == "video" and p["installed"] for p in ts.plugins(refresh=True)):
        pytest.skip("the video add-on isn't built here")
    exe = ffmpeg()
    if exe is None:
        pytest.skip("no ffmpeg")
    monkeypatch.setenv("STAGETUI_FFMPEG", exe)
    out = tmp_path / "مَرْحَبًا Hello 日本語 नमस्ते.mp4"
    subprocess.run(
        [exe, "-hide_banner", "-loglevel", "error", "-y",
         "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=12:duration=2",
         "-f", "lavfi", "-i", "sine=frequency=440:duration=2",
         "-c:v", "mpeg4", "-q:v", "5", "-c:a", "aac", "-shortest", str(out)],
        check=True,
    )  # fmt: skip
    return out


def until(d, check, seconds: float = 15.0) -> bool:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        d.wait(0.05)
        if check():
            return True
        time.sleep(0.05)
    return False


def cinema(**options):
    events: list[tuple[str, object]] = []

    class Cinema(StageApp):
        def compose(self):
            return [
                StagePane(
                    "video",
                    id="v",
                    on_ended=lambda path: events.append(("ended", path)),
                    on_state=lambda s: events.append(("state", s)),
                    **options,
                )
            ]

    return Cinema(), events


def test_a_video_plays_pauses_seeks_and_ends(clip: Path) -> None:
    app, events = cinema(path=str(clip))
    with app.drive((90, 24)) as d:
        video = d.app.query("#v").content
        info = lambda: d.query("#v")["children"][0]
        assert until(d, lambda: (info()["status"] or "").startswith("▶")), info()
        assert until(d, lambda: "Starting" not in d.snapshot().text), "a picture arrived"
        assert until(d, lambda: video.call("state")["position"] > 0.3), "time moves"
        state = video.call("state")
        assert state["duration"] == pytest.approx(2.0, abs=0.2)
        assert state["path"].endswith("नमस्ते.mp4")
        # Space pauses; time stands still.
        d.click("#v")  # a click also pauses: two toggles = playing again
        d.press("space")
        assert until(d, lambda: video.call("state")["paused"] is False)
        d.press("space")
        assert until(d, lambda: video.call("state")["paused"])
        held = video.call("state")["position"]
        time.sleep(0.3)
        assert video.call("state")["position"] == pytest.approx(held, abs=0.05)
        # Seek, volume, mute — by key and by call.
        assert video.call("seek", 1.5)["position"] == pytest.approx(1.5, abs=0.1)
        d.press("down")
        assert until(d, lambda: video.call("state")["volume"] == pytest.approx(0.9))
        d.press("m")
        assert until(d, lambda: video.call("state")["muted"])
        assert "no sound" in info()["status"]  # turned off for tests
        # Play to the end: an "ended" event.
        video.call("play")
        assert until(d, lambda: ("ended", str(clip)) in events), events
        assert video.call("state")["ended"]
        assert ("state", "paused") in events
        with pytest.raises(ts.StageError, match="no method"):
            video.call("explode")


def test_loop_and_autoplay(clip: Path) -> None:
    app, events = cinema(path=str(clip), loop=True, autoplay=False)
    with app.drive((60, 16)) as d:
        video = d.app.query("#v").content
        assert until(d, lambda: d.query("#v")["children"][0]["status"] is not None)
        assert video.call("state")["paused"], "autoplay=False waits"
        video.call("seek", 1.6)
        video.call("play")
        assert until(d, lambda: video.call("state")["position"] < 1.0, seconds=10), "it loops"
        assert not any(e[0] == "ended" for e in events)


def test_friendly_problems(clip: Path) -> None:
    for options, message in [
        ({"path": "nope.mp4"}, "was not found"),
        ({}, "no video given"),
    ]:
        app, _ = cinema(**options)
        with app.drive((80, 16)) as d:
            assert until(d, lambda m=message: m in d.snapshot().text, seconds=10), (
                message,
                d.snapshot().text,
            )
