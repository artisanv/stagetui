import termstage as ts
from termstage import StageApp, StageCard, StageText, Style, TermstageError

ARABIC = "مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج"
MIXED = "Hello مرحبا world"
JAPANESE = "こんにちは世界"
HINDI = "नमस्ते दुनिया"


class Hello(StageApp):
    theme = "aurora-dark"

    def compose(self):
        return [
            StageCard("Welcome to termstage.", id="hello", title="Hello", icon="sparkles"),
            StageCard(ARABIC, id="arabic", title="عربي", accent="mint"),
            StageText(f"{JAPANESE} · {HINDI} · {MIXED}"),
        ]


def test_version_and_exports():
    from importlib.metadata import version

    assert ts.__version__ == version("termstage"), "matches what pip shows"
    assert ts.__version__.count(".") == 2
    for name in ts.__all__:
        assert hasattr(ts, name), name


def test_inspect_headless():
    caps = ts.inspect()
    assert caps["headless"] is True
    assert caps["headless_reason"] == "TERMSTAGE_HEADLESS=1"
    assert caps["graphics"]["best"] == "halfblocks"
    assert caps["platform"]["os"] in {"linux", "macos", "windows"}
    assert caps["probe"]["performed"] is False


def test_headless_snapshot_text_and_tree():
    snap = Hello().run_headless((60, 20))
    assert (snap.cols, snap.rows) == (60, 20)
    assert "Hello" in snap.text and "Welcome to termstage." in snap.text
    assert len(snap.text.splitlines()) <= 20
    hello = snap.query("#hello")
    assert hello is not None and hello["type"] == "card" and hello["icon"] == "✦"
    assert hello["bounds"]["w"] > 0
    assert snap.query("arabic")["dir"] == "rtl"
    assert snap.query("text-1")["dir"] == "ltr"
    assert snap.query("missing") is None


def test_headless_is_deterministic():
    a = Hello().run_headless((70, 22))
    b = Hello().run_headless((70, 22))
    assert a == b


def test_global_scripts_text_survives_the_roundtrip():
    snap = Hello().run_headless((100, 24))
    assert snap.query("arabic")["text"] == ARABIC
    assert JAPANESE in snap.text and HINDI in snap.text


def test_style_and_theme_accent():
    class Themed(StageApp):
        theme = "paper-soft"
        accent = "rose"

        def compose(self):
            return [StageCard("x", style=Style(padding="spacious", rounded=False))]

    snap = Themed().run_headless((40, 12))
    assert snap.tree["theme"] == "paper-soft"
    card = snap.query("card-1")
    assert card["rounded"] is False
    assert "┌" in snap.text


def test_friendly_errors_have_three_parts():
    class Bad(StageApp):
        theme = "neon"  # type: ignore[assignment]

    try:
        Bad().run_headless()
    except TermstageError as e:
        assert "Unknown theme" in e.what
        assert e.why and "aurora-dark" in e.fix
        assert "fix:" in str(e)
    else:
        raise AssertionError("expected TermstageError")


def test_bad_icon_lists_alternatives():
    class Bad(StageApp):
        def compose(self):
            return [StageCard("x", icon="unicorn")]

    try:
        Bad().run_headless()
    except TermstageError as e:
        assert "sparkles" in e.fix
    else:
        raise AssertionError("expected TermstageError")


def test_compose_type_errors():
    class NotWidgets(StageApp):
        def compose(self):
            return ["just a string"]

    try:
        NotWidgets().run_headless()
    except TypeError as e:
        assert "not a termstage widget" in str(e)
    else:
        raise AssertionError("expected TypeError")


def test_interactive_run_refuses_headless():
    try:
        Hello().run()
    except TermstageError as e:
        assert "headless" in e.why
    else:
        raise AssertionError("expected TermstageError")


def test_component_decorator():
    @ts.component
    def Tile(label: str, value: str):
        return StageCard(f"{value}\n{label}", accent="mint", animate="pop")

    assert Tile.__termstage_component__ is True
    assert Tile("Users", "42").props["animate"] == "pop"
