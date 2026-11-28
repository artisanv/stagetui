"""An agent that keeps working while you're away — and panes that pop out.

Run:   stagetui start examples/09_detach.py
Try:   Ctrl+Space, then o   the focused pane opens in its own terminal window
                            (Ctrl+Shift+D too, in terminals that can tell it apart)
       close that window    the pane snaps back, nothing lost
       Ctrl+Space, then d   you get your terminal back; the agent keeps working
       stagetui sessions    it is listed as "night-shift"
       stagetui attach      come back: everything it did meanwhile is there
       Ctrl+Space, then q   asks: detach, or quit the app for good
                            (plain q types into the shell when it has the focus)

`python examples/09_detach.py` works too, but then the app lives in that
terminal and can't be detached (panes still pop out).
"""

import asyncio
import time

from stagetui import StageApp, StageBar, StagePane, StageProgress, StageSplit

STEPS = [
    "Read the issue",
    "Search the code",
    "Write a failing test",
    "Fix the bug",
    "Run the tests",
    "Update the docs",
    "Open a pull request",
]


class NightShift(StageApp):
    theme = "aurora-dark"
    BINDINGS = [("ctrl+shift+d", "pop_out", "Pop out")]

    def compose(self):
        self.progress = StageProgress(0, label="Starting…")
        return [
            self.progress,
            StageSplit(
                [StagePane("log", id="steps"), StagePane("terminal", id="shell")],
                ratio=[3, 2],
                grow=1,
            ),
            StageBar("Ctrl+Space, then: o pop a pane out · d detach · q leave"),
        ]

    @property
    def log(self):
        return self.query("#steps").content

    def on_mount(self):
        self.background(self.work())

    async def work(self):
        done = 0
        while True:
            for step in STEPS:
                done += 1
                self.progress.update(STEPS.index(step) / len(STEPS), label=step)
                self.log.append(f"{time.strftime('%H:%M:%S')}  #{done}  {step}")
                await asyncio.sleep(1.5)
            self.log.append(f"{time.strftime('%H:%M:%S')}  OK  round finished, starting again")


if __name__ == "__main__":
    NightShift().run()
