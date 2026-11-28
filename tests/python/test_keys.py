"""Keys: every built-in key has a name; apps (KEYS) and people (keys.json) rename
them. It stays safe: there's always a way to quit, and one key never means two
things at once. Agents can ask which keys an app answers to."""

import json
import subprocess
import sys
from pathlib import Path

import pytest
import stagetui as ts
from stagetui import StageApp, StageBar, StagePane, StageSplit

TEXT = "مَرْحَبًا Hello 日本語 नमस्ते"


@pytest.fixture(autouse=True)
def headless(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("STAGETUI_HEADLESS", "1")
    monkeypatch.delenv("STAGETUI_KEYS", raising=False)


def quit_seen(app: StageApp) -> list[bool]:
    seen: list[bool] = []
    dispatch = app._dispatch

    def spy(event):
        if event[2] == "quit":
            seen.append(True)
        return dispatch(event)

    app._dispatch = spy
    return seen


class Panes(StageApp):
    def compose(self):
        return [
            StageSplit([StagePane("markdown", id="a", text=TEXT), StagePane("markdown", id="b")]),
            StageBar(TEXT),
        ]


def test_an_app_renames_and_turns_off_keys() -> None:
    class Mine(Panes):
        KEYS = {"quit": "ctrl+q", "prefix": "ctrl+a", "close_pane": "w", "pane_menu": None}

    app = Mine()
    seen = quit_seen(app)
    with app.drive((100, 20)) as d:
        for key in ("q", "esc", "ctrl+c"):
            d.press(key)
        assert not seen, "the old quit keys are free"
        assert "Ctrl+Q" in d.snapshot().text, "the bar shows the new quit key"
        d.click("#a")
        d.press("ctrl+a")
        d.press("w")
        assert d.query("#a") is None, "Ctrl+A then w closes the pane"
        keys = {k["name"]: k for k in d.keys()["keys"]}
        assert keys["quit"]["keys"] == ["ctrl+q"]
        assert keys["pane_menu"]["keys"] == []
        assert keys["close_pane"]["after_prefix"] is True
        d.press("ctrl+q")
        assert seen, "Ctrl+Q quits"


@pytest.mark.parametrize(
    ("keys", "message"),
    [
        ({"quit": None}, "Every app needs a way to quit"),
        ({"prefix": None}, "prefix key can't be turned off"),
        ({"focus_next": "q"}, "has two jobs"),
        ({"close_pane": "1"}, "workspace 1"),
        ({"quitt": "q"}, 'no key called "quitt"'),
        ({"quit": "ctrl+nope"}, "Unknown key"),
    ],
)
def test_unsafe_or_mistyped_keys_are_explained(keys: dict, message: str) -> None:
    class Mine(Panes):
        KEYS = keys

    with pytest.raises(ts.StageError, match=message), Mine().drive((80, 20)):
        pass


def test_an_apps_own_shortcut_comes_first() -> None:
    saved: list[bool] = []

    class Mine(Panes):
        BINDINGS = [("q", "save", "Save")]

        def save(self):
            saved.append(True)

    app = Mine()
    seen = quit_seen(app)
    with app.drive((100, 20)) as d:
        d.press("q")
        assert saved and not seen, "q is the app's now"
        text = d.snapshot().text
        assert "Save" in text and "Esc" in text, "the bar offers the next quit key"
        assert d.keys()["shortcuts"][0]["action"] == "save"


def test_an_app_can_ask_before_quitting() -> None:
    class Careful(Panes):
        CONFIRM_QUIT = True

    app = Careful()
    seen = quit_seen(app)
    with app.drive((100, 24)) as d:
        d.press("q")
        assert "Quit this app?" in d.snapshot().text and not seen
        d.press("esc")
        assert "Quit this app?" not in d.snapshot().text and not seen
        d.press("q")
        d.press("enter")
        assert seen, "Enter says yes"
        assert d.keys()["confirm_quit"] is True


def test_a_persons_keys_file_applies_and_never_breaks_an_app(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = tmp_path / f"{TEXT} keys.json"
    path.write_text(
        json.dumps({"close_pane": "w", "quit": None, "nope": "x", "split_down": "ctrl+s"}),
        encoding="utf-8",
    )
    monkeypatch.setenv("STAGETUI_KEYS", str(path))

    class Mine(Panes):
        BINDINGS = [("ctrl+s", "save", "Save")]

        def save(self):
            pass

    with Mine().drive((120, 24)) as d:
        info = d.keys()
        keys = {k["name"]: k["keys"] for k in info["keys"]}
        assert keys["close_pane"] == ["w"], "a good entry applies"
        assert "q" in keys["quit"], "quit can't be taken away"
        assert keys["split_down"] == ["-"], "the app's own shortcut stays the app's"
        assert len(info["problems"]) == 3 and info["file"] == str(path)
        assert "skipped" in d.snapshot().text, "the person is told, the app runs"
    assert ts.keys()["file"] == str(path)
    assert {k["name"]: k["keys"] for k in ts.keys()["keys"]}["close_pane"] == ["w"]


def test_keys_from_the_command_line(tmp_path: Path) -> None:
    app = tmp_path / "app.py"
    app.write_text(
        "from stagetui import StageApp, StagePane\n"
        "class App(StageApp):\n"
        "    KEYS = {'quit': 'ctrl+q'}\n"
        "    BINDINGS = [('ctrl+s', 'save', 'Save')]\n"
        "    def compose(self):\n"
        "        return [StagePane('markdown', text='hi')]\n"
        "    def save(self):\n"
        "        pass\n",
        encoding="utf-8",
    )
    run = lambda *a: subprocess.run(
        [sys.executable, "-m", "stagetui", "keys", *a],
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )
    plain = run()
    assert plain.returncode == 0 and "close_pane" in plain.stdout and "Ctrl+Space" in plain.stdout
    one = run(str(app))
    assert one.returncode == 0, one.stderr
    assert "Ctrl+Q" in one.stdout and "Save" in one.stdout
    data = json.loads(run(str(app), "--json").stdout)
    assert {k["name"]: k["keys"] for k in data["keys"]}["quit"] == ["ctrl+q"]
    missing = run("nope.py")
    assert missing.returncode != 0 and "was not found" in missing.stderr
