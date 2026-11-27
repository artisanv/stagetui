"""The web add-on, end to end: a real page in a pane, from the Chrome/Chromium here.

Runs when the add-on (`stagetui-plugin-web`) and a browser are available — in CI the
"Web add-on" job builds it and points STAGETUI_PLUGIN_PATH at it.
"""

import json
import shutil
import time
from pathlib import Path

import pytest
import stagetui as ts
from stagetui import StageApp, StagePane

PAGE = """<!doctype html><meta charset=utf-8><title>Test مرحبا</title>
<body style="margin:0;font:20px sans-serif;background:#fff">
<button id=b style="position:absolute;left:0;top:0;width:320px;height:96px"
  onclick="window.n=(window.n||0)+1;document.getElementById('out').textContent='clicked '+window.n">
  Click me</button>
<input id=i style="position:absolute;left:0;top:128px;width:320px;height:64px;font-size:20px">
<div id=out style="position:absolute;left:0;top:224px">waiting</div>
<div style="height:4000px"></div>
"""

SECOND = "<!doctype html><meta charset=utf-8><title>Second page</title><h1>Two</h1>"


def browser_here() -> bool:
    import os

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
def pages(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> tuple[Path, Path]:
    monkeypatch.setenv("STAGETUI_HEADLESS", "1")
    if not any(p["name"] == "web" and p["installed"] for p in ts.plugins(refresh=True)):
        pytest.skip("the web add-on isn't built here")
    if not browser_here():
        pytest.skip("no Chrome/Chromium/Edge here")
    one = tmp_path / "مَرْحَبًا Hello 日本語 नमस्ते.html"
    one.write_text(PAGE, encoding="utf-8")
    two = tmp_path / "two.html"
    two.write_text(SECOND, encoding="utf-8")
    return one, two


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


def test_a_real_page_clicks_types_navigates_and_scrolls(pages: tuple[Path, Path]) -> None:
    one, two = pages
    events: list[tuple[str, object]] = []

    class Browser(StageApp):
        def compose(self):
            return [
                StagePane(
                    "web",
                    id="w",
                    path=str(one),
                    on_title=lambda t: events.append(("title", t)),
                    on_navigated=lambda u: events.append(("navigated", u)),
                )
            ]

    with Browser().drive((100, 30)) as d:
        web = d.app.query("#w").content
        node = lambda: d.query("#w")["children"][0]
        js = lambda code: web.call("evaluate", code)
        assert until(d, lambda: web.call("state")["title"] == "Test مرحبا"), web.call("state")
        assert until(d, lambda: node()["frames"] > 0), "a picture arrived"
        assert "Starting" not in d.snapshot().text
        assert ("title", "Test مرحبا") in events
        assert web.call("state")["url"].startswith("file://")

        # Click the button: the page's top-left is the pane's picture top-left.
        b = node()["bounds"]
        d.click((b["x"] + 2, b["y"] + 1))  # a cell inside the 320x96 px button
        assert until(d, lambda: js("document.getElementById('out').textContent") == "clicked 1")

        # Click the text box and type — any language.
        d.click((b["x"] + 2, b["y"] + 4))  # 128..192 px down: row 4 (cells are 32 px)
        d.type("hello مرحبا 日本語")
        assert until(d, lambda: js("document.getElementById('i').value") == "hello مرحبا 日本語")

        # The wheel scrolls the page.
        d.scroll(5, target="#w")
        assert until(d, lambda: js("window.scrollY") > 0)

        # Go somewhere else, and back.
        web.call("navigate", two.as_uri())
        assert until(d, lambda: web.call("state")["title"] == "Second page")
        web.call("back")
        assert until(d, lambda: web.call("state")["title"] == "Test مرحبا")
        assert any(e[0] == "navigated" for e in events)
        with pytest.raises(ts.StageError, match="no method"):
            web.call("explode")


def test_the_address_line(pages: tuple[Path, Path]) -> None:
    one, two = pages

    class Browser(StageApp):
        def compose(self):
            return [StagePane("web", id="w", path=str(one))]

    with Browser().drive((100, 30)) as d:
        web = d.app.query("#w").content
        assert until(d, lambda: web.call("state")["title"] == "Test مرحبا")
        d.click("#w")
        d.press("ctrl+l")
        d.type(str(two))
        assert until(d, lambda: "Go to:" in (d.query("#w")["children"][0]["status"] or ""))
        d.press("enter")
        assert until(d, lambda: web.call("state")["title"] == "Second page")
        state = json.dumps(web.call("state"), ensure_ascii=False)
        assert "two.html" in state
