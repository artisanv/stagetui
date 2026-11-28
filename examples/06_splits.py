"""Split the screen into panes, like tmux or a code editor — then join them again.

Run:   python examples/06_splits.py
Keys:  |  split the focused pane side by side     -  split it top/bottom
       x  close the focused pane                    Tab  move between panes
       drag a divider with the mouse to resize      q  quit
"""

from stagetui import StageApp, StageBar, StageCard, StageSplit

TIPS = [
    "Drag the line between two panes to resize them.",
    "Click a pane to focus it, or press Tab.",
    "Long text scrolls: use the mouse wheel or the arrow keys.",
    "Press x to close a pane — the others grow back.",
    "Arabic works too: مرحباً بك في النوافذ المقسّمة.",
]


class Splits(StageApp):
    theme = "aurora-dark"
    # Our own action names (not the built-in "close_pane", "split_pane_right"…):
    # a built-in name would run stagetui's version, never the methods below.
    # "next_theme" is built in on purpose.
    BINDINGS = [
        ("|", "add_right", "Split →"),
        ("ctrl+backslash", "add_right"),
        ("-", "add_below", "Split ↓"),
        ("x", "close", "Close"),
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
        # The pane with the keyboard focus, or else the first one on screen.
        return self.focused_pane or self.query("#" + self.pane_ids()[0])

    def action_add_right(self):
        self.target().split(self.new_pane(), direction="horizontal")
        self.refresh_status()

    def action_add_below(self):
        self.target().split(self.new_pane(), direction="vertical")
        self.refresh_status()

    def action_close(self):
        if len(self.pane_ids()) <= 1:
            self.notify("This is the last pane", icon="info")
            return
        pane = self.focused_pane
        if pane is None:
            self.notify("Click a pane (or press Tab) first, then x closes it", icon="info")
            return
        pane.remove()  # the other panes grow back
        self.refresh_status()

    def pane_ids(self):
        # The cards on screen right now, read from the app's live widget tree.
        def cards(node):
            for child in node.get("children", []):
                if child["type"] == "card":
                    yield child["id"]
                else:
                    yield from cards(child)

        return list(cards(self.capture_tree()))

    def refresh_status(self):
        n = len(self.pane_ids())
        self.query("#status").update(f"{n} pane{'s' if n != 1 else ''}")


if __name__ == "__main__":
    Splits().run()
