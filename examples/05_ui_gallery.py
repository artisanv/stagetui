"""A tour of termstage's widgets: buttons, inputs, lists, tables, a live canvas,
a pop-up window, notifications, themes and Arabic right-to-left layout.

Run:   python examples/05_ui_gallery.py
Keys:  Tab / Shift+Tab move between controls · Enter or Space presses
       click anything · scroll with the mouse wheel · Ctrl+T changes the theme
       F1 opens the About window · q quits
"""

import asyncio
import math

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
    StageTable,
    StageTabs,
    StageText,
    StageTree,
)

PEOPLE = [
    ["Ada Lovelace", "Mathematician", 1815],
    ["Alan Turing", "Computer scientist", 1912],
    ["Grace Hopper", "Rear admiral", 1906],
    ["Ibn al-Haytham", "Scientist", 965],
    ["Al-Khwarizmi", "Mathematician", 780],
    ["Margaret Hamilton", "Engineer", 1936],
]

FILES = {
    "src": {"app.py": None, "widgets.py": None, "assets": ["logo.svg", "font.ttf"]},
    "docs": ["getting-started.md", "apps.md"],
    "README.md": None,
}


def orbit(canvas, t):
    """Called up to 30 times a second while the Canvas tab is open."""
    w, h = canvas.width, canvas.height
    canvas.clear("#16161e")
    canvas.gradient_circle(w / 2, h / 2, 90, colors=["#bb9af760", "#bb9af700"])
    for i, color in enumerate(["#7aa2f7", "#73daca", "#e0af68", "#f7768e"]):
        angle = t * (0.8 + i * 0.35) + i * math.pi / 2
        radius = 50 + i * 22
        x = w / 2 + radius * math.cos(angle)
        y = h / 2 + radius * math.sin(angle) * 0.6
        canvas.circle(x, y, 9 - i, fill=color)
    canvas.circle(w / 2, h / 2, 18, fill="#e0af68")
    canvas.text(f"{t:4.1f} s", 16, 12, size=16, color="#565f89")


class Gallery(StageApp):
    theme = "aurora-dark"
    accent = "violet"
    BINDINGS = [
        ("ctrl+t", "next_theme", "Theme"),
        ("f1", "about", "About"),
    ]

    def compose(self):
        form = Stage(
            [
                Stage(
                    [
                        StageButton("Save", icon="check", variant="primary", on_click=self.save),
                        StageButton("Upload", icon="play", on_click=self.upload),
                        StageButton("Delete", icon="x", variant="danger", on_click=self.confirm),
                    ],
                    direction="horizontal",
                ),
                StageInput(
                    placeholder="Your name (try Arabic: اكتب اسمك)",
                    title="Name",
                    id="name",
                    on_change=self.named,
                    on_submit=self.greet,
                ),
                StageCheckbox("Send me updates", on_change=self.toggled),
                StageSelect(["Small", "Medium", "Large"], title="Size", on_change=self.sized),
                StageProgress(0.35, label="Upload", id="progress"),
                StageSpinner("Waiting for you…", id="spinner"),
            ]
        )
        controls = Stage(
            [
                form,
                # Everything you do on the left shows up here, live.
                StageCard(self.summary(), title="What you did", icon="info", id="result", grow=1),
            ],
            direction="horizontal",
            align="start",
            title="Controls",
            icon="gear",
        )
        data = Stage(
            [
                Stage(
                    [
                        StageTable(
                            PEOPLE,
                            headers=["Name", "Known for", "Born"],
                            height=5,
                            on_select=self.picked,
                        ),
                        StageTree(FILES, on_submit=self.opened),
                    ],
                    direction="horizontal",
                ),
                StageText(
                    "Pick a row with ↑ ↓ or the mouse; open a file in the tree.", id="data-status"
                ),
            ],
            title="Data",
            icon="book",
        )
        canvas = Stage(
            [
                StageText("Drawn by Python 30 times a second — Rust does the rest."),
                StageCanvas(on_frame=orbit, size=(420, 220), fps=30),
            ],
            title="Canvas",
            icon="cube",
        )
        arabic = Stage(
            [
                StageCard(
                    "مَرْحَباً بِكُمْ! هذا التبويب كله من اليمين إلى اليسار.",
                    title="أهلاً",
                    icon="sparkles",
                    animate="slide-left",
                ),
                StageInput(
                    placeholder="اكتب هنا…",
                    title="رسالة",
                    dir="rtl",
                    id="message",
                    on_submit=self.send,
                ),
                Stage(
                    [
                        StageButton("إرسال", variant="primary", on_click=self.send),
                        StageButton("إلغاء", on_click=self.clear_message),
                    ],
                    direction="horizontal",
                ),
            ],
            title="عربي",
            icon="globe",
            dir="rtl",
        )
        return [
            StageCard(
                "Everything here is drawn by termstage's Rust engine.\n"
                "Use the mouse or the keyboard — both work everywhere.",
                title="termstage gallery",
                icon="sparkles",
                animate="slide-up",
            ),
            StageTabs([controls, data, canvas, arabic], id="tabs", grow=1),
            StageModal(
                [
                    StageText("This deletes nothing — it's a demo. Continue?"),
                    Stage(
                        [
                            StageButton("Yes, delete", variant="danger", on_click=self.deleted),
                            StageButton("Cancel", on_click=self.close_modal),
                        ],
                        direction="horizontal",
                    ),
                ],
                title="Are you sure?",
                icon="warning",
                id="confirm",
            ),
            StageModal(
                [
                    StageText(
                        "termstage: a Rust-first TUI engine for humans and AI agents.\n"
                        "Press Esc or click outside to close."
                    ),
                ],
                title="About",
                icon="info",
                id="about",
            ),
            StageBar("Gallery"),
        ]

    # ----- what the Controls tab remembers -----------------------------------

    def __init__(self):
        super().__init__()
        self.form = {"name": "", "updates": False, "size": "Small", "last": "nothing yet"}

    def summary(self):
        f = self.form
        return (
            f"Name:     {f['name'] or '—'}\n"
            f"Updates:  {'yes' if f['updates'] else 'no'}\n"
            f"Size:     {f['size']}\n"
            f"\nLast action: {f['last']}"
        )

    def remember(self, **changes):
        self.form.update(changes)
        self.query("#result").update(self.summary())

    # ----- event handlers --------------------------------------------------

    def named(self, name):
        self.remember(name=name)  # runs on every key you type

    def greet(self, name):
        self.remember(last=f"pressed Enter → Hello, {name or 'stranger'}!")
        self.notify(f"Hello, {name}!" if name else "Type a name first", icon="sparkles")

    def toggled(self, checked):
        self.remember(updates=checked, last="ticked the box" if checked else "unticked the box")

    def sized(self, size):
        self.remember(size=size, last=f"chose {size}")

    def save(self):
        self.remember(last="saved ✓")
        self.notify("Saved!", icon="check", accent="mint")

    async def upload(self):
        bar = self.query("#progress")
        spinner = self.query("#spinner")
        spinner.update("Uploading…")
        self.remember(last="uploading…")
        for step in range(36, 101, 4):
            bar.update(step / 100)
            await asyncio.sleep(0.05)
        spinner.update("Done!")
        self.remember(last="uploaded ✓")
        self.notify("Upload finished", icon="check")

    def confirm(self):
        self.query("#confirm").show()

    def deleted(self):
        self.query("#confirm").hide()
        self.remember(last="deleted (not really)")
        self.notify("Deleted (not really)", icon="x", accent="rose")

    def close_modal(self):
        self.query("#confirm").hide()
        self.remember(last="cancelled the delete")

    def action_about(self):
        self.query("#about").show()

    def picked(self, row):
        name, known_for, born = row["row"]
        self.query("#data-status").update(f"Selected: {name} — {known_for}, born {born}")

    def opened(self, item):
        path = "/".join(item["path"])
        self.query("#data-status").update(f"Opened {path}")
        self.notify(f"Opened {path}", icon="file")

    def send(self):
        box = self.query("#message")
        text = box.value.strip()
        if text:
            self.notify(f"أُرسلت: {text}", icon="send", accent="mint")
            box.update("")
        else:
            self.notify("اكتب رسالة أولاً", icon="info")

    def clear_message(self):
        self.query("#message").update("")


if __name__ == "__main__":
    Gallery().run()
