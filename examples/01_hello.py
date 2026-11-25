"""Milestone 1: a styled card, Arabic included. Quit with q, Esc or Ctrl+C.

python examples/01_hello.py            # interactive
python examples/01_hello.py --headless # print the headless snapshot (what an agent sees)
"""

import json
import sys

from termstage import StageApp, StageCard, StageText


class Hello(StageApp):
    theme = "aurora-dark"
    accent = "violet"

    def compose(self):
        return [
            StageCard(
                "Welcome to termstage — a Rust-first TUI engine.\nPress q or Esc to quit.",
                id="hello",
                title="Hello",
                icon="sparkles",
            ),
            StageCard("مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج", id="arabic", title="عربي", accent="mint"),
            StageText("Milestone 1 · terminal core · headless snapshots · zero zombies"),
        ]


if __name__ == "__main__":
    if "--headless" in sys.argv:
        snap = Hello().run_headless((80, 24))
        print(snap.text)
        print(json.dumps(snap.tree, ensure_ascii=False, indent=2))
    else:
        Hello().run()
