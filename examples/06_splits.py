"""Split the screen into panes, like tmux or a code editor — then join them again.

Run:   python examples/06_splits.py
Keys:  |  split the focused pane side by side     -  split it top/bottom
       x  close the focused pane                    Tab  move between panes
       drag a divider with the mouse to resize      q  quit
"""

from termstage import StageApp, StageBar, StageCard, StageSplit

TIPS = [
    "Drag the line between two panes to resize them.",
    "Click a pane to focus it, or press Tab.",
    "Long text scrolls: use the mouse wheel or the arrow keys.",
    "Press x to close a pane — the others grow back.",
    "Arabic works too: مرحباً بك في النوافذ المقسّمة.",
]


class Splits(StageApp):
    theme = "aurora-dark"
    BINDINGS = [
        ("|", "split_right", "Split →"),
        ("ctrl+backslash", "split_right"),
        ("-", "split_down", "Split ↓"),
        ("x", "close_pane", "Close"),
        ("ctrl+t", "next_theme", "Theme"),
    ]

    def __init__(self):
        super().__init__()
        self.count = 1

    def compose(self):
        return [
            StageSplit(
                [StageCard(TIPS[0], title="Pane 1", icon="terminal", id="pane-1")],
                id="panes",
            ),
            StageBar("1 pane", id="status"),
        ]

    def new_pane(self):
        self.count += 1
        tip = TIPS[(self.count - 1) % len(TIPS)]
        return StageCard(tip, title=f"Pane {self.count}", icon="terminal", animate="pop")

    def target(self):
        # The pane with the keyboard focus, or the first pane.
        return self.focused_pane or self.query("#pane-1")

    def action_split_right(self):
        self.target().split(self.new_pane(), direction="horizontal")
        self.refresh_status()

    def action_split_down(self):
        self.target().split(self.new_pane(), direction="vertical")
        self.refresh_status()

    def action_close_pane(self):
        pane = self.focused_pane
        panes = self.panes()
        if pane is None or len(panes) <= 1:
            self.notify("This is the last pane", icon="info")
            return
        pane.remove()
        self.refresh_status()

    def panes(self):
        return [w for w in self._widgets.values() if isinstance(w, StageCard)]

    def refresh_status(self):
        n = len(self.panes())
        self.query("#status").update(f"{n} pane{'s' if n != 1 else ''}")


if __name__ == "__main__":
    Splits().run()
