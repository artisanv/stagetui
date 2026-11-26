"""Milestone 2: show, draw, Canvas, images, PNG snapshots, Arabic/RTL in pixels."""

import json
import struct

import pytest
import stagetui as ts
from stagetui import Canvas, StageApp, StageCanvas, StageCard, StageError, StageImage

ARABIC = "مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج"
PERSIAN = "سلام دنیا، گل‌های زیبا"
HINDI = "नमस्ते दुनिया"
JAPANESE = "こんにちは世界"
SVG = b'<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32"><rect width="64" height="32" rx="6" fill="#7aa2f7"/></svg>'


def png_size(data: bytes) -> tuple[int, int]:
    assert data.startswith(b"\x89PNG\r\n\x1a\n")
    return struct.unpack(">II", data[16:24])


def ink(c: Canvas) -> int:
    """How many non-transparent pixels a canvas has (decoded via a round trip through PNG size)."""
    return len(c.png())


# --- show ---------------------------------------------------------------------------------


def test_show_text_card(capsys):
    ts.show(ARABIC, title="عربي", icon="sparkles")
    out = capsys.readouterr().out
    assert ARABIC in out and "عربي" in out and "╭" in out


def test_show_image_path_bytes_and_canvas(tmp_path, capsys):
    path = tmp_path / "logo.svg"
    path.write_bytes(SVG)
    ts.show(str(path))
    ts.show(path)  # pathlib.Path works too
    ts.show(SVG)
    ts.show(Canvas(30, 20, "mint"))
    out = capsys.readouterr().out.splitlines()
    assert out[0] == "[image: logo.svg 64×32]"
    assert out[1] == out[0]
    assert out[2].startswith("[image: image 64×32]")
    assert out[3] == "[image: canvas 30×20]"


def test_show_missing_image_is_friendly():
    with pytest.raises(StageError) as e:
        ts.show("nope/cat.png")
    assert "was not found" in e.value.what


# --- draw & Canvas ------------------------------------------------------------------------


def test_draw_saves_png(tmp_path):
    out = tmp_path / "hello.png"

    def scene(c):
        c.clear("#0f172a")
        c.text("Hello مرحبا नमस्ते", 10, 10, size=24)

    c = ts.draw(scene, size=(300, 60), out=out)
    assert isinstance(c, Canvas) and (c.width, c.height) == (300, 60)
    assert png_size(out.read_bytes()) == (300, 60)


def test_canvas_shapes():
    c = Canvas(200, 120, background="#0f172a")
    blank = ink(Canvas(200, 120, background="#0f172a"))
    c.rect(10, 10, 50, 30, fill="mint", radius=8)
    c.rounded_rect(70, 10, 50, 30, 10, stroke="violet", stroke_width=3)
    c.circle(150, 30, 20, fill="#ff000080")
    c.line(0, 60, 200, 60, color="amber", width=2)
    c.polyline([(0, 100), (50, 80), (100, 110), (150, 70)], color="rose", width=3)
    c.polyline([(10, 115), (30, 90), (50, 115)], fill="blue", color=None)
    c.gradient_rect(0, 70, 200, 10, colors=["black", "white"])
    c.gradient_circle(100, 100, 15, colors=["#7aa2f7", "#bb9af7"])
    c.text(ARABIC, 190, 85, size=14, align="right", bold=True)
    c.image(Canvas(10, 10, "white"), 180, 100, 10, 10)
    assert ink(c) > blank
    assert repr(c) == "Canvas(width=200, height=120)"


def test_canvas_friendly_errors():
    c = Canvas(10, 10)
    with pytest.raises(StageError) as e:
        c.rect(0, 0, 5, 5, fill="plaid")
    assert "Unknown color" in e.value.what and "#7aa2f7" in e.value.fix
    with pytest.raises(StageError):
        c.text("x", 0, 0, align="middle")
    with pytest.raises(StageError):
        c.gradient_rect(0, 0, 5, 5, colors=[])
    with pytest.raises(StageError):
        Canvas(0, 10)
    with pytest.raises(StageError):
        c.image("missing.png", 0, 0, 5, 5)


# --- StageImage / StageCanvas ---------------------------------------------------------------


def test_stage_image_from_path_bytes_and_canvas(tmp_path):
    path = tmp_path / "logo.svg"
    path.write_bytes(SVG)

    class Gallery(StageApp):
        def compose(self):
            return [
                StageImage(str(path), id="file", width=12),
                StageImage(SVG, id="bytes", height=3, fit="stretch"),
                StageImage(Canvas(40, 40, "violet"), id="canvas"),
            ]

    snap = Gallery().run_headless((60, 24))
    f = snap.query("file")
    assert f["type"] == "image" and f["source"].endswith("logo.svg")
    assert f["pixels"] == {"w": 64, "h": 32} and f["bounds"]["w"] == 12
    assert snap.query("bytes")["fit"] == "stretch" and snap.query("bytes")["bounds"]["h"] == 3
    assert snap.query("canvas")["source"].startswith("mem:")
    assert "[image: logo" in snap.text  # labels wrap inside narrow images


def test_stage_canvas_draws_once():
    calls = []

    def scene(c):
        calls.append((c.width, c.height))
        c.circle(20, 20, 15, fill="mint")

    class App(StageApp):
        def compose(self):
            return [StageCanvas(scene, size=(40, 40), id="art")]

    snap = App().run_headless((30, 10))
    assert calls == [(40, 40)]
    assert snap.query("art")["pixels"] == {"w": 40, "h": 40}


def test_stage_image_errors(tmp_path):
    class Missing(StageApp):
        def compose(self):
            return [StageImage(str(tmp_path / "ghost.png"))]

    with pytest.raises(StageError) as e:
        Missing().run_headless()
    assert "Could not open image" in e.value.what

    with pytest.raises(TypeError):
        StageImage(123)  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        StageCanvas("not a function")  # type: ignore[arg-type]

    class BadFit(StageApp):
        def compose(self):
            return [StageImage(SVG, fit="squash")]  # type: ignore[arg-type]

    with pytest.raises(StageError) as e:
        BadFit().run_headless()
    assert "contain" in e.value.fix


# --- snapshots ----------------------------------------------------------------------------


class World(StageApp):
    def compose(self):
        return [
            StageCard(ARABIC, id="ar", title="عربي"),
            StageCard(PERSIAN, id="fa"),
            StageCard(f"{HINDI} · {JAPANESE}", id="mix"),
        ]


def test_snapshot_text_is_in_reading_order():
    snap = World().run_headless((60, 20))
    assert ARABIC in snap.text
    assert PERSIAN in snap.text
    assert HINDI in snap.text and JAPANESE in snap.text
    assert snap.query("ar")["dir"] == "rtl" and snap.query("mix")["dir"] == "ltr"


def test_snapshot_png_and_save(tmp_path):
    snap = World().run_headless((50, 16), cell=(8, 16))
    assert png_size(snap.png) == (400, 256)
    assert snap.png is snap.png  # cached
    assert png_size(snap.save(tmp_path / "w.png").read_bytes()) == (400, 256)
    assert ARABIC in snap.save(tmp_path / "w.txt").read_text(encoding="utf-8")
    tree = json.loads(snap.save(tmp_path / "w.json").read_text(encoding="utf-8"))
    assert tree["children"][0]["id"] == "ar"
    with pytest.raises(ValueError):
        snap.save(tmp_path / "w.gif")


def test_png_is_deterministic():
    a = World().run_headless((40, 14)).png
    b = World().run_headless((40, 14)).png
    assert a == b


def test_fonts_cover_bundled_scripts():
    report = ts.fonts()
    covered = {s["script"] for s in report["scripts"] if s["covered"]}
    assert {"latin", "arabic", "devanagari"} <= covered
    assert "NotoSansArabic-Regular.ttf" in report["bundled"]
