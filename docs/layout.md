# Layouts: rows, columns, grids, splits, tabs and pop-ups

In `compose()` you return a list of widgets. To arrange them, put widgets
**inside other widgets**. The containers are listed below.

## Stacks, rows and grids with `Stage`

`Stage` holds other widgets. Its first argument is a **list of widgets**.

```python
from stagetui import Stage, StageApp, StageButton, StageCard


class Layouts(StageApp):
    def compose(self):
        return [
            # Top to bottom (the default):
            Stage([StageCard("one"), StageCard("two")]),
            # Side by side:
            Stage([StageButton("Yes"), StageButton("No")], direction="horizontal"),
            # A grid with 3 columns:
            Stage([StageCard(f"tile {n}") for n in range(6)], columns=3, width="100%"),
        ]


Layouts().run()
```

| Option | Values | What it does |
|---|---|---|
| `direction` | `"vertical"` (default), `"horizontal"` | Stack top to bottom, or side by side |
| `columns` | a number | Make a grid with this many columns |
| `gap` | cells | Space between children (default 1 row, or 2 columns side by side) |
| `align` | `"start"`, `"center"`, `"end"`, `"stretch"` | How children line up across the stack |
| `height` | cells | A fixed height; longer content **scrolls** |

### Sizes: `width`, `height` and `grow`

Every widget accepts these:

- `width=30` — exactly 30 cells wide. `width="50%"` — half of the space around it.
- `height=10` — exactly 10 rows tall.
- `grow=1` — take the leftover space. Two widgets with `grow=1` and `grow=2`
  share it 1 : 2.

```python
from stagetui import Stage, StageApp, StageCard


class Sidebar(StageApp):
    def compose(self):
        return [
            Stage(
                [
                    StageCard("Menu", width=20),
                    StageCard("The main area takes the rest.", grow=1),
                ],
                direction="horizontal",
                width="100%",
            )
        ]


Sidebar().run()
```

### When the window is too small

Nothing is squashed. When everything doesn't fit, the screen **scrolls**: use
the mouse wheel, **PageUp** / **PageDown**, **Home** / **End**, or the arrow
keys. A thin scrollbar on the right shows where you are. Moving the focus with
**Tab** scrolls the focused widget into view automatically.

## Split panes with `StageSplit`

A split shares its space between **panes**, with a line between them that you
can **drag with the mouse** to resize. A split fills all the space it gets.

```python
from stagetui import StageApp, StageCard, StageSplit


class Editor(StageApp):
    def compose(self):
        return [
            StageSplit(
                [
                    StageCard("Files", title="Explorer"),
                    StageSplit(
                        [StageCard("print('hi')", title="main.py"), StageCard("$", title="Shell")],
                        direction="vertical",
                    ),
                ],
                ratio=[1, 3],  # the explorer gets 1 part, the rest 3 parts
            )
        ]


Editor().run()
```

- `direction="horizontal"` (default) puts panes **side by side**; `"vertical"` stacks them.
- `ratio=[1, 3]` sets the shares, one number per pane.
- A card inside a split is a **pane**: it fills its share, can be focused with a
  click or **Tab**, and long text scrolls inside it.

### Splitting and closing while the app runs

Every widget has `.split()` and `.remove()`:

```python
from stagetui import StageApp, StageCard, StageSplit


class Panes(StageApp):
    BINDINGS = [("|", "split", "Split"), ("x", "close", "Close pane")]

    def compose(self):
        return [StageSplit([StageCard("First pane", id="first")])]

    def action_split(self):
        pane = self.focused_pane or self.query("#first")
        pane.split(StageCard("A new pane"), direction="horizontal")

    def action_close(self):
        if self.focused_pane is not None:
            self.focused_pane.remove()  # the other panes grow back


Panes().run()
```

`examples/06_splits.py` is a complete version of this.

## Pages with `StageTabs`

Each child is one page. Its `title` (and `icon`) name the tab.

```python
from stagetui import Stage, StageApp, StageCard, StageTabs, StageText


class Settings(StageApp):
    def compose(self):
        return [
            StageTabs(
                [
                    Stage([StageText("Your name and picture")], title="Profile", icon="file"),
                    Stage([StageText("Colors and fonts")], title="Look", icon="star"),
                    StageCard("Version 1.0", title="About", icon="info"),
                ],
                on_change=lambda page: print("now on page", page),
            )
        ]


Settings().run()
```

Change the page by clicking a tab, with **Ctrl+PageDown** / **Ctrl+PageUp**,
or from code with `tabs.update(active=2)`.

## Pop-up windows with `StageModal`

A modal sits **above** everything, and the rest of the screen dims. It starts
hidden: call `.show()` to open it and `.hide()` to close it. **Esc** or a click
outside also closes it, unless you pass `dismissable=False`.

```python
from stagetui import Stage, StageApp, StageButton, StageModal, StageText


class Confirm(StageApp):
    def compose(self):
        return [
            StageButton("Delete…", variant="danger", on_click=self.ask),
            StageModal(
                [
                    StageText("Really delete?"),
                    Stage(
                        [
                            StageButton("Delete", variant="danger"),
                            StageButton("Cancel", on_click=self.cancel),
                        ],
                        direction="horizontal",
                    ),
                ],
                title="Are you sure?",
                icon="warning",
                id="ask",
            ),
        ]

    def ask(self):
        self.query("#ask").show()

    def cancel(self):
        self.query("#ask").hide()


Confirm().run()
```

While a modal is open, **Tab** only moves between the controls inside it.
Pictures underneath are covered properly, even in terminals that draw pictures
on top of text.

## A status bar with `StageBar`

`StageBar("text")` is pinned to the bottom of the screen (or to the top, if it
comes first in your list). It shows your text and your keyboard shortcuts from
`BINDINGS` — so people can always see what they can do.

## Right-to-left layouts

When your app's text starts in Arabic, Persian, Hebrew or Urdu, stagetui
**mirrors the layout**: rows start on the right, the first pane of a split is on
the right, scrollbars move to the left, tree arrows flip. You can also choose:

- `StageApp.dir = "rtl"` (or `"ltr"`) for the whole app,
- `dir="rtl"` on one container, e.g. a single tab page.

See [World languages](world-languages.md) for text details.
