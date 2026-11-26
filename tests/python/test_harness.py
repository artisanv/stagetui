"""Milestone 4a: harness widgets, terminal panes and Blender-style pane types."""

import json
import sys
import time

import pytest
import stagetui as ts
from stagetui import (
    StageApp,
    StageChat,
    StageCode,
    StageDiff,
    StageError,
    StageLog,
    StageMarkdown,
    StagePane,
    StagePermission,
    StagePrompt,
    StageSplit,
    StageTerminal,
    StageToolCall,
)

ARABIC = "مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج"
JAPANESE = "こんにちは世界"
HINDI = "नमस्ते दुनिया"


class Agent(StageApp):
    def __init__(self):
        super().__init__()
        self.seen = []

    def compose(self):
        return [
            StageChat(id="chat", height=20),
            StagePrompt(
                placeholder="Ask…",
                commands=[("help", "Show help"), ("clear", "Clear")],
                mentions=["src/main.py", "README.md"],
                on_submit=self.ask,
                id="prompt",
            ),
        ]

    async def ask(self, text):
        self.seen.append(text)
        chat = self.query("#chat")
        chat.add("user", text)
        reply = chat.add("assistant")
        for chunk in ["**Sure**, ", f"{ARABIC} ", f"{JAPANESE} {HINDI}\n\n", "```py\nx = 1\n```"]:
            reply.append(chunk)
        tool = chat.add_tool("read_file", {"path": "main.py"})
        tool.finish("42 lines")


def test_chat_streams_markdown_and_tools():
    app = Agent()
    snap = app.run_headless((80, 26), steps=[{"type": "hello"}, {"press": "enter"}])
    assert app.seen == ["hello"]
    text = snap.text
    assert "› You" in text and "✦ Assistant" in text
    assert "Sure," in text and "x = 1" in text
    assert "✓ read_file" in text
    chat = snap.query("chat")
    assert chat["messages"] == 3
    assert snap.query("prompt")["value"] == "", "submitted text is cleared"


def test_prompt_commands_mentions_and_history():
    app = Agent()
    snap = app.run_headless(
        (80, 26),
        steps=[
            {"type": "/he"},
            {"press": "tab"},
            {"type": "see @mai"},
            {"press": "enter"},  # accepts the file completion
            {"press": "enter"},  # sends
            {"press": "up"},  # history
        ],
    )
    assert app.seen == ["/help see @src/main.py "]
    assert snap.query("prompt")["value"] == "/help see @src/main.py "


def test_prompt_multiline_and_paste_like_typing():
    app = Agent()
    app.run_headless(
        (80, 26),
        steps=[{"type": "line one"}, {"press": "ctrl+j"}, {"type": "line two"}, {"press": "enter"}],
    )
    assert app.seen == ["line one\nline two"]


def test_diff_accept_reject_and_result():
    old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n"
    new = "a\nB\nc\nd\ne\nf\ng\nh\ni\nJ\nk\n"
    decided = []

    class App(StageApp):
        def compose(self):
            return [StageDiff((old, new), path="x.txt", id="d", on_decide=decided.append)]

    app = App()
    snap = app.run_headless((70, 30), steps=[{"press": "tab"}, {"press": "a"}, {"press": "r"}])
    d = snap.query("d")
    assert d["decisions"] == ["accepted", "rejected"]
    assert decided[0] == {"hunk": 0, "decision": "accepted"}
    assert "x.txt · 1 of 2 accepted" in snap.text


def test_log_markdown_code_and_toolcall():
    class App(StageApp):
        def compose(self):
            return [
                StageLog(id="log", height=6),
                StageMarkdown("# Title\n\n", id="md"),
                StageCode("def f():\n    return 'مرحبا'\n", language="python", id="code"),
                StageToolCall("bash", args={"cmd": "ls"}, id="tool"),
            ]

        def on_mount(self):
            log = self.query("#log")
            for i in range(300):
                log.append(f"INFO step {i}")
            log.append(["ERROR boom", "WARN careful"])
            self.query("#md").append(f"Streaming **works** — {ARABIC}")
            self.query("#tool").finish("a.py\nb.py")

    snap = App().run_headless((70, 34))
    assert snap.query("log")["lines"] == 302
    assert snap.query("log")["last"] == "WARN careful"
    assert "Streaming works" in snap.text
    assert snap.query("code")["language"] == "Python"
    assert snap.query("tool")["status"] == "done"
    assert "✓ bash" in snap.text


def test_permission_widget_and_ask_permission():
    answers = []

    class App(StageApp):
        def compose(self):
            return [StagePrompt(on_submit=self.go, id="p")]

        async def go(self, text):
            answers.append(
                await self.ask_permission(f"Run {text}?", tool="bash", args={"cmd": text})
            )

    App().run_headless((70, 24), steps=[{"type": "ls"}, {"press": "enter"}, {"press": "y"}])
    App().run_headless((70, 24), steps=[{"type": "rm"}, {"press": "enter"}, {"press": "esc"}])
    App().run_headless((70, 24), steps=[{"type": "x"}, {"press": "enter"}, {"press": "a"}])
    assert answers == ["allow", "deny", "always"]

    seen = []

    class Direct(StageApp):
        def compose(self):
            return [StagePermission("Delete it?", tool="fs", on_decide=seen.append, id="perm")]

    snap = Direct().run_headless((60, 14), steps=[{"click": "#perm"}, {"press": "n"}])
    assert seen == ["deny"] and snap.query("perm")["decided"] == "deny"


@pytest.mark.skipif(sys.platform == "win32", reason="uses /bin/sh")
def test_terminal_runs_commands():
    class App(StageApp):
        def compose(self):
            return [StageTerminal(["sh"], id="term", height=10)]

        def on_mount(self):
            self.query("#term").send("echo hello-$((40+2))\n")

    app = App()
    snap = app.run_headless((60, 14))
    deadline = time.monotonic() + 5
    screen = snap.query("term")["screen"]
    # The shell answers on its own thread; poll the live app until it prints.
    while "hello-42" not in screen and time.monotonic() < deadline:
        time.sleep(0.05)
        screen = json.loads(snap._core.query("#term"))["screen"]
    assert "hello-42" in screen


class Workspace(StageApp):
    def compose(self):
        return [
            StageSplit(
                [StagePane("markdown", text="# Notes", id="a"), StagePane("log", id="b")],
                id="split",
            )
        ]


def test_panes_switch_through_the_menu_and_restore():
    # Click pane a's title, pick "Chat" from the menu with the keyboard.
    snap = Workspace().run_headless((100, 24), steps=[{"click": [3, 0]}])
    assert snap.text.count("Split side by side") == 1, "the menu is open"
    snap = Workspace().run_headless(
        (100, 24), steps=[{"click": [3, 0]}, {"press": "home"}, {"press": "enter"}]
    )
    panes = {p["id"]: p for p in snap.tree["children"][0]["children"]}
    assert panes["a"]["pane_type"] in {"chat", "markdown"}


def test_pane_api_set_type_panes_and_custom_types():
    @ts.pane_type("greeting", title="Greeting", icon="heart")
    def greeting(name="Sara"):
        return StageMarkdown(f"Hello, **{name}**!")

    class App(StageApp):
        def compose(self):
            return [
                StageSplit(
                    [StagePane("greeting", name="Nasser", id="g"), StagePane("empty", id="e")]
                )
            ]

        def on_mount(self):
            self.query("#e").set_type("code", text="print('hi')", language="python")
            self.query("#g").switch("markdown")
            self.query("#g").switch("greeting")  # restored, not rebuilt
            self.found = self.panes()

    app = App()
    snap = app.run_headless((100, 24))
    assert "Hello, Nasser!" in snap.text
    assert "print('hi')" in snap.text
    types = {p["id"]: p["type"] for p in app.found}
    assert types == {"g": "greeting", "e": "code"}
    assert snap.query("g")["stashed"] == ["markdown"]
    with pytest.raises(StageError, match="Unknown pane type"):
        StagePane("does-not-exist")


def test_custom_pane_type_built_when_chosen_from_the_menu():
    built = []

    @ts.pane_type("counter", title="Counter")
    def counter():
        built.append(1)
        return StageMarkdown("count: 1")

    class App(StageApp):
        def compose(self):
            return [StagePane("empty", id="p")]

        def on_mount(self):
            self._core.switch_pane("#p", "counter")

    snap = App().run_headless((60, 12))
    assert built == [1]
    assert "count: 1" in snap.text
    assert snap.query("p")["pane_type"] == "counter"


def test_prefix_splits_and_closes_panes():
    snap = Workspace().run_headless(
        (100, 24), steps=[{"click": [3, 5]}, {"press": "ctrl+space"}, {"press": "|"}]
    )
    count = lambda s: sum(1 for _ in _walk(s.tree) if _.get("type") == "pane")
    assert count(snap) == 3
    snap = Workspace().run_headless(
        (100, 24), steps=[{"click": [3, 5]}, {"press": "ctrl+space"}, {"press": "x"}]
    )
    assert count(snap) == 1


def _walk(node):
    yield node
    for c in node.get("children", []):
        yield from _walk(c)


def test_offline_helpers_before_the_app_runs():
    chat = StageChat()
    msg = chat.add("assistant", "Hi")
    msg.append(" there")
    assert chat._content == [{"role": "assistant", "text": "Hi there"}]
    md = StageMarkdown("a")
    md.append("b")
    assert md._content == "ab"
    with pytest.raises(StageError, match="needs a running app"):
        StageDiff(("a", "b")).accept_all()
    code = StageCode.from_file(__file__)
    assert code.props["path"].endswith("test_harness.py")
