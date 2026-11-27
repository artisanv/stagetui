"""stagetui — a Rust-first TUI engine for agent harnesses and humans.

Scripts use verbs::

    import stagetui as ts
    ts.show("logo.png")                 # a picture, right in the terminal
    ts.show("مرحبا بكم", title="Hi")    # a text card (Arabic just works)
    ts.draw(lambda c: c.circle(50, 50, 40, fill="mint"), size=(100, 100))

Apps use ``Stage*`` nouns::

    from stagetui import StageApp, StageCard

    class Hello(StageApp):
        def compose(self):
            return [StageCard("Welcome!", title="Hello", icon="sparkles")]

    Hello().run()                          # interactive
    Hello().run_headless((80, 24)).save("hello.png")   # for CI and agents
"""

from __future__ import annotations

from ._core import Canvas, StageError, __version__
from ._inspect import fonts, inspect, plugins, sessions
from .app import Driver, Snapshot, StageApp
from .component import component
from .harness import (
    ChatMessage,
    StageChat,
    StageCode,
    StageDiff,
    StageLog,
    StageMarkdown,
    StagePane,
    StagePermission,
    StagePlugin,
    StagePrompt,
    StageTerminal,
    StageToolCall,
    StageWorkspaces,
    pane_type,
)
from .verbs import draw, show
from .widgets import (
    Stage,
    StageBar,
    StageButton,
    StageCanvas,
    StageCard,
    StageCheckbox,
    StageImage,
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
    Style,
)

__all__ = [
    "Canvas",
    "ChatMessage",
    "Driver",
    "Snapshot",
    "Stage",
    "StageApp",
    "StageBar",
    "StageButton",
    "StageCanvas",
    "StageCard",
    "StageChat",
    "StageCheckbox",
    "StageCode",
    "StageDiff",
    "StageError",
    "StageImage",
    "StageInput",
    "StageLog",
    "StageMarkdown",
    "StageModal",
    "StagePane",
    "StagePermission",
    "StagePlugin",
    "StageProgress",
    "StagePrompt",
    "StageSelect",
    "StageSpinner",
    "StageSplit",
    "StageTable",
    "StageTabs",
    "StageTerminal",
    "StageText",
    "StageToolCall",
    "StageTree",
    "StageWorkspaces",
    "Style",
    "__version__",
    "component",
    "draw",
    "fonts",
    "inspect",
    "pane_type",
    "plugins",
    "sessions",
    "show",
]
