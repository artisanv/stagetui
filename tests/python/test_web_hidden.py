"""Web pages you can't see: a web pane out of sight makes no sound (`<audio>`, audio
only in script, Web Audio, a cross-site frame — all stop while it's hidden, and play on
when it's back), and a new window a page opens is shown in the pane instead of nowhere.

Runs when the web add-on (`stagetui-plugin-web`) and a browser are available, like
test_web.py. The page plays a very quiet tone; its clocks (`currentTime`) tell whether
sound is being made, so nothing has to be heard.
"""

import functools
import http.server
import json
import math
import os
import shutil
import struct
import tempfile
import threading
import time
import urllib.parse
import urllib.request
import wave
from collections.abc import Iterator
from pathlib import Path

import pytest
import stagetui as ts
from stagetui import StageApp, StagePane

# The main page (served from 127.0.0.1) and, inside it, a frame from `localhost`: another
# site, so the browser runs it in a process of its own. Both play the tone and an
# AudioContext after a click, and the frame reports its clocks to the page.
MAIN = """<!doctype html><meta charset=utf-8><title>Hidden sound</title><link rel=icon href="data:,">
<body style="margin:0;background:#fff;font:20px sans-serif">
<button id=b style="position:absolute;left:0;top:0;width:320px;height:96px">Play</button>
<iframe id=f src="http://localhost:{port}/inner.html" allow="autoplay"
  style="position:absolute;left:0;top:128px;width:320px;height:96px;border:0"></iframe>
<div id=tick style="position:absolute;left:400px;top:0">0</div>
<audio id=a></audio>
<script>
const a = document.getElementById('a');
let ctx = null;
window.inner = {{}};
fetch('tone.wav').then((r) => r.blob()).then((b) => {{ a.src = URL.createObjectURL(b); }});
addEventListener('message', (e) => {{ if (e.data && e.data.t === 'clocks') window.inner = e.data; }});
setInterval(() => {{ document.getElementById('tick').textContent = Date.now(); }}, 100);
const tone = (ctx) => {{
  const osc = ctx.createOscillator(), gain = ctx.createGain();
  gain.gain.value = 0.001; osc.connect(gain).connect(ctx.destination); osc.start();
  return ctx;
}};
let det = null, spare = null, ctx2 = null;
document.getElementById('b').onclick = () => {{
  a.volume = 0.05; a.play();
  ctx = tone(new AudioContext());
  // Sound only in script: never put on the page.
  det = new Audio(a.src); det.volume = 0.05; det.play();
  // Ready, but quiet for now: the app asks them to play while the pane is hidden.
  spare = new Audio(a.src); spare.volume = 0.05;
  ctx2 = tone(new AudioContext()); ctx2.suspend();
}};
window.clocks = () => ({{
  ready: !!a.src && !!window.inner.ready,
  audio: a.currentTime, ctx: ctx ? ctx.currentTime : 0, ctxState: ctx && ctx.state,
  det: det ? det.currentTime : 0, spare: spare ? spare.currentTime : 0,
  ctx2: ctx2 ? ctx2.currentTime : 0, ctx2State: ctx2 && ctx2.state,
  iaudio: window.inner.audio || 0, ictx: window.inner.ctx || 0, ipaused: window.inner.paused,
  idet: window.inner.det || 0,
  late: window.late ? window.late.currentTime : null, visible: document.visibilityState,
}});
</script>"""

INNER = """<!doctype html><meta charset=utf-8><link rel=icon href="data:,">
<body style="margin:0;background:#eef">
<button id=b style="width:320px;height:96px">Play inside</button>
<audio id=a></audio>
<script>
const a = document.getElementById('a');
let ctx = null, det = null;
fetch('tone.wav').then((r) => r.blob()).then((b) => { a.src = URL.createObjectURL(b); });
document.getElementById('b').onclick = () => {
  a.volume = 0.05; a.play();
  det = new Audio(a.src); det.volume = 0.05; det.play();
  ctx = new AudioContext();
  const osc = ctx.createOscillator(), gain = ctx.createGain();
  gain.gain.value = 0.001; osc.connect(gain).connect(ctx.destination); osc.start();
};
setInterval(() => parent.postMessage({t: 'clocks', ready: !!a.src, audio: a.currentTime,
  paused: a.paused, ctx: ctx ? ctx.currentTime : 0, det: det ? det.currentTime : 0}, '*'), 50);
</script>"""

SECOND = '<!doctype html><meta charset=utf-8><title>Second page</title><link rel=icon href="data:,"><h1>Two</h1>'

# New windows: a link with target=_blank, window.open, a link in a cross-site frame, and
# forms that post to a new window — sent by their button, and by `form.submit()`.
LINKS = """<!doctype html><meta charset=utf-8><title>Links</title><link rel=icon href="data:,">
<body style="margin:0;font:20px sans-serif">
<a href="pop.html?via=link" target=_blank
  style="position:absolute;left:0;top:0;width:320px;height:96px;display:block">link</a>
<button style="position:absolute;left:0;top:128px;width:320px;height:96px"
  onclick="window.open('pop.html?via=open')">open</button>
<iframe src="http://localhost:{port}/framelink.html"
  style="position:absolute;left:0;top:256px;width:320px;height:96px;border:0"></iframe>
<form method=post action="echo" target=_blank>
  <input type=hidden name=via value=form><input type=hidden name=v value="مَرْحَبًا 42">
  <button style="position:absolute;left:0;top:384px;width:320px;height:64px">post</button>
</form>
<form id=f2 method=post action="echo" target=_blank>
  <input type=hidden name=via value=submit><input type=hidden name=v value="日本語 7">
</form>
<button style="position:absolute;left:0;top:480px;width:320px;height:64px"
  onclick="document.getElementById('f2').submit()">submit()</button>"""

FRAMELINK = """<!doctype html><meta charset=utf-8><link rel=icon href="data:,">
<body style="margin:0"><a href="pop.html?via=frame" target=_blank
  style="display:block;width:320px;height:96px">frame link</a>"""

POP = """<!doctype html><meta charset=utf-8><title>Popup</title><link rel=icon href="data:,">
<script>document.title = 'Popup ' + new URLSearchParams(location.search).get('via')</script>"""

# Asked while the pane is hidden, these wait until it is shown.
ASK = "void spare.play(); void ctx2.resume(); 1"

# A new AudioContext made while the pane is hidden (by the app) must stay quiet too.
LATE = """window.late = new AudioContext();
{ const o = late.createOscillator(), g = late.createGain(); g.gain.value = 0.001;
  o.connect(g).connect(late.destination); o.start(); }
late.currentTime"""

CLOCKS = ("audio", "ctx", "det", "iaudio", "ictx", "idet")


def browser_here() -> bool:
    if os.environ.get("STAGETUI_CHROME"):
        return True
    names = ["chromium", "chromium-browser", "google-chrome", "google-chrome-stable",
             "microsoft-edge", "msedge", "chrome", "brave"]  # fmt: skip
    return any(shutil.which(n) for n in names) or any(
        Path(p).exists()
        for p in [
            "/Applications/Google Chrome.app",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        ]
    )


@pytest.fixture
def site(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Iterator[int]:
    """A little web server on 127.0.0.1 (also reachable as `localhost`); its port."""
    monkeypatch.setenv("STAGETUI_HEADLESS", "1")
    if not any(p["name"] == "web" and p["installed"] for p in ts.plugins(refresh=True)):
        pytest.skip("the web add-on isn't built here")
    if not browser_here():
        pytest.skip("no Chrome/Chromium/Edge here")
    # 20 seconds of a 440 Hz tone, very quiet (long, so its clock never wraps).
    with wave.open(str(tmp_path / "tone.wav"), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(8000)
        w.writeframes(
            b"".join(
                struct.pack("<h", int(60 * math.sin(2 * math.pi * 440 * i / 8000)))
                for i in range(8000 * 20)
            )
        )

    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *args: object) -> None:
            pass

        def do_POST(self) -> None:
            """Show what a form sent, in the page's title."""
            sent = self.rfile.read(int(self.headers.get("Content-Length", 0))).decode()
            form = {k: v[0] for k, v in urllib.parse.parse_qs(sent).items()}
            page = f"<!doctype html><meta charset=utf-8><title>Posted {form['via']}: {form['v']}"
            body = (page + '</title><link rel=icon href="data:,">').encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    class Server(http.server.ThreadingHTTPServer):
        def handle_error(self, *args: object) -> None:
            pass  # the browser hanging up early is fine

    server = Server(("127.0.0.1", 0), functools.partial(Quiet, directory=str(tmp_path)))
    port = server.server_address[1]
    (tmp_path / "main.html").write_text(MAIN.format(port=port), encoding="utf-8")
    (tmp_path / "inner.html").write_text(INNER, encoding="utf-8")
    (tmp_path / "second.html").write_text(SECOND, encoding="utf-8")
    (tmp_path / "links.html").write_text(LINKS.format(port=port), encoding="utf-8")
    (tmp_path / "framelink.html").write_text(FRAMELINK, encoding="utf-8")
    (tmp_path / "pop.html").write_text(POP, encoding="utf-8")
    threading.Thread(target=server.serve_forever, daemon=True).start()
    yield port
    server.shutdown()
    server.server_close()


def pump(d, seconds: float) -> None:
    """Let real time pass while the app keeps running."""
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        d.wait(0.05)
        time.sleep(0.05)


def until(d, check, seconds: float = 30.0) -> bool:
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


def test_a_hidden_page_makes_no_sound_and_plays_on_when_back(site: int) -> None:
    class Browser(StageApp):
        def compose(self):
            return [StagePane("web", id="w", url=f"http://127.0.0.1:{site}/main.html")]

    with Browser().drive((100, 30)) as d:
        pane = d.app.query("#w")
        web = pane.content
        clocks = lambda: web.call("evaluate", "clocks()")

        def node() -> dict:
            return next(c for c in d.query("#w")["children"] if "frames" in c)

        assert until(d, lambda: web.call("state")["title"] == "Hidden sound"), web.call("state")
        assert until(d, lambda: node()["frames"] > 0 and clocks()["ready"]), clocks()
        b = node()["bounds"]
        d.click((b["x"] + 2, b["y"] + 1))  # the page's button (0..96 px down)
        d.click((b["x"] + 2, b["y"] + 4))  # the frame's button (128..224 px down)
        assert until(d, lambda: all(clocks()[k] > 0.3 for k in CLOCKS), 15), clocks()

        for cycle in (1, 2):
            before, hid = clocks(), time.monotonic()
            pane.switch("markdown")
            pump(d, 0.3)
            if cycle == 1:
                # The app can still read a resting page, at once.
                asked = time.monotonic()
                mid = clocks()
                assert time.monotonic() - asked < 1.0
                assert mid["audio"] - before["audio"] < 0.4, (before, mid)
                late = web.call("evaluate", LATE)
                web.call("evaluate", ASK)
            pump(d, 1.6)
            if cycle == 1:
                still = clocks()
                for k in ("audio", "ctx", "det", "spare", "ctx2"):
                    assert still[k] == pytest.approx(mid[k], abs=0.05), (k, mid, still)
                assert still["late"] - late < 0.2, ("a new AudioContext played", late, still)
            pane.switch("web")
            pump(d, 0.2)
            back = clocks()
            hidden = time.monotonic() - hid
            assert hidden > 1.9
            # Out of sight for about 2 s, the clocks moved only around the switches.
            moved = {k: round(back[k] - before[k], 3) for k in CLOCKS}
            assert all(m < 0.6 for m in moved.values()), (cycle, moved, before, back)

            frames = node()["frames"]
            pump(d, 1.2)
            after = clocks()
            # Back in sight, everything that was playing plays on — in the frame too.
            ran = {k: round(after[k] - back[k], 3) for k in CLOCKS}
            assert all(r > 0.6 for r in ran.values()), (cycle, ran, back, after)
            assert after["ctxState"] == "running" and after["ipaused"] is False, after
            assert after["visible"] == "visible", after
            assert node()["frames"] > frames, "pictures come again"
            if cycle == 1:
                # What was asked while hidden happens now.
                for k in ("late", "spare", "ctx2"):
                    assert after[k] - still[k] > 0.6, (k, "plays once shown", still, after)
                assert after["ctx2State"] == "running", after

        # Going somewhere waits while the pane is hidden.
        pane.switch("markdown")
        pump(d, 0.3)
        web.call("navigate", f"http://127.0.0.1:{site}/second.html")
        pump(d, 1.0)
        assert web.call("state")["title"] == "Hidden sound"
        pane.switch("web")
        assert until(d, lambda: web.call("state")["title"] == "Second page"), web.call("state")


def pages_open() -> int:
    """How many pages (tabs) the add-on's browser has, from Chrome's own list."""
    ports = sorted(
        Path(tempfile.gettempdir()).glob("stagetui-web-*/DevToolsActivePort"),
        key=lambda p: p.stat().st_mtime,
    )
    port = ports[-1].read_text().split()[0]
    with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/list") as r:
        return sum(t["type"] == "page" for t in json.load(r))


def test_a_new_window_opens_in_the_pane(site: int) -> None:
    links = f"http://127.0.0.1:{site}/links.html"

    class Browser(StageApp):
        def compose(self):
            return [StagePane("web", id="w", url=links)]

    with Browser().drive((100, 30)) as d:
        web = d.app.query("#w").content
        title = lambda: web.call("state")["title"]
        assert until(d, lambda: title() == "Links"), web.call("state")
        pump(d, 1.0)  # the frame inside loads too
        # The browser holds the pane's page and nothing else (Chrome's first tab is closed).
        assert until(d, lambda: pages_open() == 1, 10), pages_open()
        b = next(c for c in d.query("#w")["children"] if "frames" in c)["bounds"]
        shown = (
            (1, "Popup link"),  # a link with target=_blank
            (5, "Popup open"),  # window.open()
            (9, "Popup frame"),  # a link with target=_blank in a cross-site frame
            (12, "Posted form: مَرْحَبًا 42"),  # a form posting to a new window: it keeps its data
            (15, "Posted submit: 日本語 7"),  # the same, sent by form.submit()
        )
        for row, want in shown:
            d.click((b["x"] + 2, b["y"] + row))
            shows = lambda want=want: title() == want
            assert until(d, shows, 15), (want, web.call("state"))
            # No new window is left where nobody can see it.
            assert until(d, lambda: pages_open() == 1, 10), (want, pages_open())
            web.call("navigate", links)
            assert until(d, lambda: title() == "Links")
            pump(d, 1.0)
