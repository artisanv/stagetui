"""Workspaces: save a layout of panes, change it, bring it back — from Python and for agents."""

import json
from pathlib import Path

import pytest
import stagetui as ts
from stagetui import StageApp, StageBar, StageCard, StagePane, StageSplit, StageWorkspaces
from stagetui._agent import ACTIONS, TOOLS


@pytest.fixture(autouse=True)
def private_config(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("STAGETUI_HEADLESS", "1")
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "config"))
    monkeypatch.setenv("APPDATA", str(tmp_path / "config"))


@ts.pane_type("greeting", title="Greeting")
def greeting(name: str = "world"):
    return StageCard(f"Hello, {name}", title="Greeting")


class Desk(StageApp):
    def compose(self):
        return [
            StageWorkspaces(
                {
                    "Code": StageSplit(
                        [
                            StagePane("files", id="files"),
                            StagePane("greeting", id="g", name="Salma"),
                        ],
                        ratio=[1, 2],
                    ),
                    "مراجعة": StagePane("chat", id="chat"),
                }
            ),
            StageBar("desk"),
        ]


def test_save_change_and_load_by_name(tmp_path: Path) -> None:
    with Desk().drive((100, 28)) as d:
        app = d.app
        layout = app.workspace()
        assert layout["children"][1] == {"pane": "greeting", "id": "g", "args": {"name": "Salma"}}
        file = app.save_workspace()  # named after the open tab
        assert file == tmp_path / "config" / "stagetui" / "workspaces" / "desk.json"
        assert app.workspaces() == ["Code"]

        app.load_workspace({"pane": "log", "id": "files"})
        d.wait(0.1)
        assert app.workspace() == {"pane": "log", "id": "files"}
        assert "Salma" not in d.snapshot().text

        app.load_workspace("Code")
        d.wait(0.1)
        assert app.workspace() == layout, "the same tree comes back"
        assert "Hello, Salma" in d.snapshot().text, "custom panes are rebuilt with their options"


def test_workspace_tabs_names_switching_and_errors(tmp_path: Path) -> None:
    with Desk().drive((100, 28)) as d:
        assert d.snapshot().text.splitlines()[0].split() == ["Code", "مراجعة"]
        d.press("ctrl+space")
        d.press("2")
        assert d.app.workspace() == {"pane": "chat", "id": "chat"}
        d.app.save_workspace()
        assert d.app.workspaces() == ["مراجعة"]  # named after the open tab
        with pytest.raises(ts.StageError, match="No workspace named"):
            d.app.load_workspace("nope")
        with pytest.raises(ts.StageError, match="layout has a mistake"):
            d.app.load_workspace({"pain": "chat"})
        mine = tmp_path / "elsewhere.json"
        d.app.save_workspace("mine", path=mine)
        assert json.loads(mine.read_text(encoding="utf-8"))["workspaces"]["mine"]["pane"] == "chat"


def test_list_of_names_starts_empty_panes() -> None:
    class Blank(StageApp):
        def compose(self):
            return [StageWorkspaces(["One", "Two"])]

    with Blank().drive((60, 12)) as d:
        assert d.app.workspace()["pane"] == "empty"
        assert "One" in d.snapshot().text.splitlines()[0]


def test_agents_can_read_and_arrange_workspaces() -> None:
    names = {t["name"] for t in TOOLS}
    assert {"workspace", "load_workspace", "save_workspace"} <= names
    with Desk().drive((100, 28)) as d:
        layout = ACTIONS["workspace"](d, {})
        assert layout["split"] == "horizontal"
        layout["children"].reverse()
        screen = ACTIONS["load_workspace"](d, {"layout": layout})
        assert "Hello, Salma" in screen["text"]
        assert d.app.workspace()["children"][0]["id"] == "g"
        saved = ACTIONS["save_workspace"](d, {"name": "flipped"})
        assert saved["saved"].endswith("desk.json")


def test_background_work_runs_while_the_app_does() -> None:
    import asyncio

    class Counter(StageApp):
        count = 0

        def compose(self):
            self.card = StageCard("0", id="n")
            return [self.card]

        def on_mount(self):
            self.background(self.tick())

        async def tick(self):
            while True:
                self.count += 1
                self.card.update(str(self.count))
                await asyncio.sleep(0.05)

    with Counter().drive((30, 6)) as d:
        d.wait(0.5)
        assert d.app.count >= 2
    with pytest.raises(ts.StageError, match="needs a running app"):

        async def nothing() -> None: ...

        coro = nothing()
        try:
            Counter().background(coro)
        finally:
            coro.close()
