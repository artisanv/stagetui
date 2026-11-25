"""Milestone 3: layout, widgets, events, focus, splits, modals, motion, RTL."""

import asyncio
import inspect

import pytest
import termstage as ts
from termstage import (
    Stage,
    StageApp,
    StageBar,
    StageButton,
    StageCanvas,
    StageCard,
    StageCheckbox,
    StageInput,
    StageModal,
    StageProgress,
    StageSelect,
    StageSpinner,
    StageSplit,
    StageTable,
    StageTabs,
    StageText,
    StageTree,
    TermstageError,
)

ARABIC = "مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج"
MIXED = "Hello مرحبا world"
JAPANESE = "こんにちは世界"
HINDI = "नमस्ते दुनिया"


class Form(StageApp):
    BINDINGS = [("ctrl+s", "save", "Save"), ("ctrl+t", "next_theme", "Theme")]

    def __init__(self):
        super().__init__()
        self.log = []

    def compose(self):
        return [
            StageInput(placeholder="Name", id="name", on_submit=self.submitted),
            StageCheckbox("Agree", id="agree", on_change=lambda v: self.log.append(("agree", v))),
            StageSelect(
                ["S", "M", "L"], id="size", on_change=lambda v: self.log.append(("size", v))
            ),
            StageButton("Send", id="send", on_click=self.send),
            StageBar("ready"),
        ]

    def submitted(self, text):
        self.log.append(("submit", text))

    async def send(self):
        await asyncio.sleep(0)
        self.log.append(("send", self.query("#name").value))

    def action_save(self):
        self.log.append(("save",))


def test_typing_clicking_and_shortcuts():
    app = Form()
    snap = app.run_headless(
        (60, 20),
        steps=[
            {"type": f"{ARABIC} {JAPANESE}"},
            {"press": "enter"},
            {"click": "#agree"},
            {"press": "ctrl+s"},
            {"click": "#send"},
        ],
    )
    assert ("submit", f"{ARABIC} {JAPANESE}") in app.log
    assert ("agree", True) in app.log
    assert ("save",) in app.log
    assert ("send", f"{ARABIC} {JAPANESE}") in app.log, "async handler ran"
    assert snap.tree["focus"] == "send"
    assert snap.query("agree")["checked"] is True
    # The screen text gives Arabic back in reading order.
    assert ARABIC in snap.text or snap.query("name")["value"].startswith(ARABIC)


def test_select_with_the_keyboard():
    app = Form()
    app.run_headless(
        (60, 20),
        steps=[
            {"press": "tab"},
            {"press": "tab"},
            {"press": "enter"},
            {"press": "down"},
            {"press": "enter"},
        ],
    )
    assert ("size", "M") in app.log


def test_theme_switch_and_quit_key_typed_into_input():
    app = Form()
    snap = app.run_headless((60, 20), steps=[{"type": "q"}, {"press": "ctrl+t"}])
    assert snap.query("name")["value"] == "q", "q types into the focused input"
    assert snap.tree["theme"] == "nordic-light"


def test_handlers_take_zero_or_more_arguments():
    seen = []

    class App(StageApp):
        def compose(self):
            return [
                StageInput(id="a", on_submit=lambda: seen.append("no-arg")),
                StageInput(id="b", on_submit=lambda text: seen.append(text)),
            ]

    App().run_headless((40, 12), steps=[{"type": "x\n"}, {"press": "tab"}, {"type": "y\n"}])
    assert seen == ["no-arg", "y"]


class Panes(StageApp):
    BINDINGS = [("|", "split", "Split"), ("x", "close", "Close")]

    def compose(self):
        return [StageSplit([StageCard("one", id="one", title="One")], id="split")]

    def action_split(self):
        (self.focused_pane or self.query("#one")).split(StageCard("new", title="New"))

    def action_close(self):
        if self.focused_pane is not None:
            self.focused_pane.remove()


def test_dynamic_split_and_unsplit():
    snap = Panes().run_headless((80, 20), steps=[{"press": "|"}, {"press": "|"}])
    split = snap.query("split")
    assert len(split["children"]) == 3
    widths = [c["bounds"]["w"] for c in split["children"]]
    assert sum(widths) + 2 == 80, widths
    xs = [c["bounds"]["x"] for c in split["children"]]
    assert xs == sorted(xs)
    snap = Panes().run_headless((80, 20), steps=[{"press": "|"}, {"press": "x"}])
    assert snap.query("one") is not None
    assert snap.query("split") is None or len(snap.query("split").get("children", [])) == 1


@pytest.mark.parametrize("size", [(80, 24), (40, 12), (120, 40), (20, 8)])
def test_split_reflow_never_overlaps(size):
    class Grid(StageApp):
        def compose(self):
            return [
                StageSplit(
                    [
                        StageCard(ARABIC, id="a"),
                        StageSplit(
                            [StageCard(MIXED, id="b"), StageCard(HINDI, id="c")],
                            direction="vertical",
                        ),
                    ],
                    ratio=[2, 1],
                )
            ]

    snap = Grid().run_headless(size)
    boxes = [snap.query(i)["bounds"] for i in "abc"]
    for i, a in enumerate(boxes):
        assert a["x"] + a["w"] <= size[0] and a["y"] + a["h"] <= size[1]
        for b in boxes[i + 1 :]:
            apart = (
                a["x"] + a["w"] <= b["x"]
                or b["x"] + b["w"] <= a["x"]
                or a["y"] + a["h"] <= b["y"]
                or b["y"] + b["h"] <= a["y"]
            )
            assert apart, (a, b)


def test_modal_opens_traps_focus_and_closes():
    closed = []

    class App(StageApp):
        def compose(self):
            return [
                StageButton("Open", id="open", on_click=lambda: self.query("#m").show()),
                StageModal(
                    [StageText("Sure?"), StageButton("Yes", id="yes")],
                    id="m",
                    title="Confirm",
                    on_close=lambda: closed.append(True),
                ),
            ]

    snap = App().run_headless((60, 20), steps=[{"click": "#open"}])
    assert snap.query("m")["on_screen"] is True
    assert snap.tree["focus"] == "yes"
    assert "Confirm" in snap.text
    snap = App().run_headless((60, 20), steps=[{"click": "#open"}, {"press": "esc"}])
    assert snap.query("m")["visible"] is False
    assert closed == [True]


def test_tabs_and_scrolling():
    class App(StageApp):
        def compose(self):
            return [
                StageTabs(
                    [
                        Stage([StageText(f"line {i}") for i in range(40)], title="Long", id="long"),
                        StageCard(JAPANESE, title="Other", id="other"),
                    ],
                    id="tabs",
                    grow=1,
                )
            ]

    snap = App().run_headless((50, 15), steps=[{"scroll": {"target": "#long", "rows": 6}}])
    assert snap.query("long")["scroll"]["offset"] == 6
    assert "line 0" not in snap.text and "line 6" in snap.text
    snap = App().run_headless((50, 15), steps=[{"press": "ctrl+pagedown"}])
    assert snap.query("tabs")["active"] == 1
    assert JAPANESE in snap.text


def test_runtime_updates_and_values():
    class App(StageApp):
        def compose(self):
            return [
                StageProgress(0.1, id="p", label="Work"),
                StageTable([["a", 1], ["b", 2]], headers=["K", "V"], id="t"),
                StageTree({"src": ["main.py"], "README.md": None}, id="tree"),
                StageSpinner("busy", id="spin"),
                StageButton("Go", id="go", on_click=self.go),
            ]

        def go(self):
            self.query("#p").update(0.9)
            self.query("#t").update([["a", 1], ["b", 2], ["c", 3]])
            self.query("#spin").update("done")
            self.notify("Finished", icon="check")

    snap = App().run_headless((60, 20), steps=[{"click": "#go"}])
    assert snap.query("p")["value"] == pytest.approx(0.9)
    assert snap.query("t")["rows"] == 3
    assert snap.query("spin")["label"] == "done"
    assert snap.tree["toasts"] == ["Finished"]


def test_tree_keyboard():
    picked = []

    class App(StageApp):
        def compose(self):
            return [StageTree({"src": ["main.py", "app.py"]}, id="t", on_submit=picked.append)]

    App().run_headless(
        (40, 10),
        steps=[{"press": "tab"}, {"press": "right"}, {"press": "down"}, {"press": "enter"}],
    )
    assert picked and picked[0]["path"] == ["src", "main.py"] and picked[0]["leaf"] is True


def test_live_canvas_on_frame():
    frames = []

    def tick(canvas, t):
        frames.append(t)
        canvas.clear("#203040")

    class App(StageApp):
        def compose(self):
            return [StageCanvas(on_frame=tick, size=(64, 32), fps=10, id="cv")]

    snap = App().run_headless((40, 10), steps=[{"wait": 0.5}])
    assert len(frames) >= 4, frames
    assert frames == sorted(frames)
    assert snap.png.startswith(b"\x89PNG")


def test_motion_is_deterministic():
    class App(StageApp):
        def compose(self):
            return [
                StageCard("hello", animate="slide-up", id="c"),
                StageCard("pulse", animate="pulse"),
                StageCard("pop", animate="pop"),
            ]

    a = App().run_headless((40, 24), steps=[{"wait": 0.1}])
    b = App().run_headless((40, 24), steps=[{"wait": 0.1}])
    assert a == b
    assert a.png == b.png


def test_rtl_app_mirrors_layout():
    class App(StageApp):
        def compose(self):
            return [
                Stage(
                    [StageButton("أولاً", id="first"), StageButton("ثانياً", id="second")],
                    direction="horizontal",
                ),
                StageCard(ARABIC, title="عربي"),
            ]

    snap = App().run_headless((60, 16))
    assert snap.tree["dir"] == "rtl"
    assert snap.query("first")["bounds"]["x"] > snap.query("second")["bounds"]["x"]


def test_component_decorator():
    @ts.component
    def Tile(label: str, value: str):
        return [StageText(label), StageCard(value)]

    class App(StageApp):
        def compose(self):
            return [Stage([Tile("Users", "1,204"), Tile("Errors", "3")], direction="horizontal")]

    snap = App().run_headless((60, 14))
    assert "1,204" in snap.text and "Errors" in snap.text
    with pytest.raises(TypeError):
        ts.component(lambda: 42)()


def test_friendly_errors():
    class Bad(StageApp):
        def compose(self):
            return [StageSplit([StageCard("a"), StageCard("b")], ratio=[1])]

    with pytest.raises(TermstageError) as e:
        Bad().run_headless()
    assert "ratio" in e.value.what and e.value.fix

    with pytest.raises(TermstageError, match="Unknown key"):

        class Keys(StageApp):
            BINDINGS = [("hyper+x", "go")]

        Keys().run_headless()

    with pytest.raises(TermstageError, match="needs a running app"):
        StageCard("x").split(StageCard("y"))

    with pytest.raises(TermstageError, match="Milestone 5"):
        StageCard("x").detach()

    class NoAction(StageApp):
        BINDINGS = [("ctrl+g", "missing")]

        def compose(self):
            return [StageCard("x")]

    with pytest.raises(TermstageError, match="action_missing"):
        NoAction().run_headless(steps=[{"press": "ctrl+g"}])

    with pytest.raises(TermstageError, match="Unknown step"):
        Form().run_headless(steps=[{"dance": True}])


# Grammar conformance: every Stage* widget takes its primary content first and
# the universal keyword properties; every widget has the same lifecycle verbs.
WIDGETS = [
    (StageCard, "text"),
    (StageText, "text"),
    (StageButton, "Go"),
    (StageInput, ""),
    (StageCheckbox, "Agree"),
    (StageSelect, ["a", "b"]),
    (StageProgress, 0.5),
    (StageSpinner, "busy"),
    (StageTable, [["a"]]),
    (StageTree, ["a"]),
    (Stage, []),
    (StageSplit, []),
    (StageTabs, []),
    (StageModal, []),
    (StageBar, "status"),
    (ts.StageMarkdown, "# hi"),
    (ts.StageCode, "x = 1"),
    (ts.StageDiff, ("a", "b")),
    (ts.StageLog, "line"),
    (ts.StageToolCall, "bash"),
    (ts.StagePermission, "Allow?"),
    (ts.StageChat, []),
    (ts.StagePrompt, ""),
    (ts.StagePane, "markdown"),
]


@pytest.mark.parametrize(("cls", "content"), WIDGETS, ids=[w[0].__name__ for w in WIDGETS])
def test_widget_grammar(cls, content):
    params = inspect.signature(cls).parameters
    first = next(iter(params.values()))
    assert first.kind is inspect.Parameter.POSITIONAL_OR_KEYWORD, "primary content comes first"
    for name in ("id", "theme", "accent"):
        assert name in params and params[name].kind is inspect.Parameter.KEYWORD_ONLY, name
    w = cls(content, id="w")
    for verb in ("show", "hide", "load", "update", "focus", "split", "remove", "detach", "attach"):
        assert callable(getattr(w, verb)), verb
    assert w.id == "w"
    # Every widget renders headlessly on its own.

    class One(StageApp):
        def compose(self):
            return [w]

    snap = One().run_headless((50, 16))
    assert snap.query("w") is not None
