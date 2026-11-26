"""An agent that keeps working while you're away: detach, come back later.

Run:   stagetui start examples/09_detach.py
Try:   Ctrl+Space, then d   you get your terminal back; the agent keeps working
       stagetui sessions    it is listed as "night-shift"
       stagetui attach      come back: everything it did meanwhile is there
       a second terminal can `stagetui attach` at the same time: both show it
       q                    quit for good (or `stagetui stop night-shift`)

`python examples/09_detach.py` works too, but then the app lives in that
terminal and can't be detached.
"""

import asyncio
import time

from stagetui import StageApp, StageBar, StageCard, StageLog, StageProgress

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

    def compose(self):
        self.progress = StageProgress(0, label="Starting…")
        self.log = StageLog(id="log", grow=1)
        return [
            StageCard(
                "A pretend agent that works in the background. "
                "Press Ctrl+Space then d to detach — it keeps going.",
                title="Night shift",
            ),
            self.progress,
            self.log,
            StageBar("Ctrl+Space d — detach · q — quit"),
        ]

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
