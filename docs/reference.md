# Reference

Every public name in `termstage`, in one place. Import with `import termstage as ts`
or `from termstage import ...`.

Sizes: **cells** for things on screen (`width=20`), **pixels** for canvases (`size=(640, 360)`).

## Verbs

### `ts.show(content, *, title=None, icon=None, dir=None, theme=None, accent=None, width=None, height=None, fit=None)`
Show text (as a card) or a picture, inline. `content` can be text, an image path
(`str` or `pathlib.Path`), image `bytes`, or a `Canvas`. Returns `None`.

### `ts.draw(fn=None, *, size=(640, 360), background=None, out=None, show=True)`
Create a `Canvas`, call `fn(canvas)`, then save it to `out` (if given) or show it
(unless `show=False`). Returns the `Canvas`.

### `ts.inspect(*, headless=False, probe=True) -> dict`
What the current terminal can do (same as `termstage inspect`).

### `ts.fonts() -> dict`
Bundled fonts and which writing systems have fonts available.

## Apps

### `class StageApp`
| Member | Description |
|---|---|
| `theme = "aurora-dark"` | Class attribute: color scheme |
| `accent = None` | Class attribute: highlight color |
| `dir = None` | Class attribute: `"rtl"` / `"ltr"` layout; `None` guesses from the text |
| `BINDINGS = []` | Class attribute: shortcuts `(key, action)` or `(key, action, description)` |
| `reduce_motion = False` | Class attribute: turn animations off |
| `compose(self) -> list` | Override: return the widgets to show |
| `on_mount(self)` | Override (optional, may be `async`): runs once the app is on screen |
| `action_<name>(self)` | Called by a shortcut whose action is `<name>` |
| `run()` | Run in the terminal until q / Esc / Ctrl+C |
| `await run_async()` | The same, from async code |
| `run_headless(size=(80, 24), *, steps=None, cell=(16, 32), settle=True) -> Snapshot` | Run without a terminal; `steps` press keys, type, click, wait ([Snapshots](snapshots.md)) |
| `query("#id")` | The widget with that id |
| `focused`, `focused_pane` | The focused widget, and the split pane holding it (or `None`) |
| `notify(text, *, icon=None, accent=None, timeout=3.0)` | A short message in the corner |
| `set_theme(name)` | Switch the theme while running |
| `mount(widget)` | Add a widget at the end of the screen |
| `exit()` | Quit |

Built-in actions for `BINDINGS`: `quit`, `next_theme`, `focus_next`, `focus_previous`, `close_modal`.

### `class Snapshot`
| Member | Description |
|---|---|
| `text` | Screen as plain text, reading order |
| `tree` | `dict` describing every widget (nested `children`), plus `focus`, `dir`, `scroll`, `toasts` |
| `png` | Screen as PNG `bytes` (made on first use) |
| `cols`, `rows` | Size in cells |
| `query(id)` | One widget's `dict` by id (`"x"` or `"#x"`), at any depth, or `None` |
| `save(path)` | Save as `.png`, `.txt` or `.json`; returns the `Path` |

## Widgets

All widgets accept `id`, `theme`, `accent` and (most) `animate`, `visible`,
`width`, `height`, `grow`. Text widgets also accept `title`, `icon`, `rounded`,
`padding`, `dir`, `style`. Handlers are `on_<event>=` ([events](events.md)).

| Widget | First argument | Extra options | Events |
|---|---|---|---|
| `StageCard(text)` | text | `shadow` | — |
| `StageText(text)` | text | — | — |
| `StageImage(source)` | path, `bytes`, or `Canvas` | `width`, `height` (cells), `fit` | — |
| `StageCanvas(draw)` | a function `draw(canvas)` (or `None`) | `size` (pixels), `background`, `fps` | `on_frame(canvas, t)` |
| `StageButton(label)` | text | `icon`, `variant` | `on_click()` |
| `StageInput(value)` | starting text | `placeholder`, `title`, `password` | `on_change(text)`, `on_submit(text)` |
| `StageCheckbox(label)` | text | `checked` | `on_change(checked)` |
| `StageSelect(options)` | list of texts | `value`, `title` | `on_change(option)` |
| `StageProgress(value)` | 0–1, or `None` | `label` | — |
| `StageSpinner(label)` | text | — | — |
| `StageTable(rows)` | list of rows | `headers`, `height` | `on_select(row)`, `on_submit(row)` |
| `StageTree(items)` | nested dict / list | `height` | `on_select(item)`, `on_submit(item)` |
| `Stage(children)` | list of widgets | `direction`, `columns`, `gap`, `align` | — |
| `StageSplit(children)` | list of widgets | `direction`, `ratio` | — |
| `StageTabs(children)` | list of widgets | `active` | `on_change(index)` |
| `StageModal(children)` | list of widgets | `title`, `icon`, `dismissable` | `on_close()` |
| `StageBar(text)` | text | `hints` | — |

### Methods on every widget
`update(content=None, **options)` · `load(content)` · `show()` · `hide()` ·
`focus()` · `split(widget, direction="horizontal")` · `remove()` · `value` ·
`detach()` / `attach()` (*coming in Milestone 5*). Containers also have `add(widget)`.

### `Style(padding=None, rounded=None, accent=None, shadow=None, width=None, height=None, grow=None)`
Reusable look settings for `style=`.

### `@ts.component`
Turns a function that returns a widget (or a list of widgets) into a reusable widget.

## Canvas

### `Canvas(width=640, height=360, background=None)`
| Member | Description |
|---|---|
| `width`, `height` | Size in pixels |
| `clear(color)` | Fill everything |
| `rect(x, y, width, height, *, fill=None, stroke=None, stroke_width=1.0, radius=0.0)` | Rectangle |
| `rounded_rect(x, y, width, height, radius=12.0, *, fill=None, stroke=None, stroke_width=1.0)` | Rounded rectangle |
| `circle(x, y, radius, *, fill=None, stroke=None, stroke_width=1.0)` | Circle |
| `line(x1, y1, x2, y2, *, color="white", width=2.0)` | Line |
| `polyline(points, *, color="white", width=2.0, fill=None, closed=False)` | Connected lines |
| `gradient_rect(x, y, width, height, *, colors, vertical=False, radius=0.0)` | Gradient rectangle |
| `gradient_circle(x, y, radius, *, colors)` | Radial gradient circle |
| `text(text, x, y, *, size=16.0, color="white", bold=False, align="left", max_width=None)` | Shaped text |
| `image(source, x, y, width, height, *, fit="contain")` | Draw a file or another Canvas |
| `png() -> bytes` | PNG bytes |
| `save(path)` | Write a PNG file |

**Colors:** `"#rgb"`, `"#rrggbb"`, `"#rrggbbaa"`, `"blue"`, `"mint"`, `"amber"`,
`"rose"`, `"violet"`, `"white"`, `"black"`, `"gray"`, `"red"`, `"transparent"`.

## Errors

### `TermstageError`
Raised for every problem termstage can explain. Attributes: `what`, `why`, `fix`.
`str(error)` contains all three.

## Values

| Kind | Allowed values |
|---|---|
| Themes | `aurora-dark`, `nordic-light`, `velvet-dusk`, `paper-soft`, `hacker-mint` |
| Accents | `blue`, `mint`, `amber`, `rose`, `violet`, or `#rrggbb` |
| Padding | `compact`, `normal`, `spacious`, or a number of cells |
| Direction (text) | `auto`, `ltr`, `rtl` |
| Direction (layout) | `vertical`, `horizontal` |
| Align | `start`, `center`, `end`, `stretch` |
| Sizes | a number of cells, or `"50%"` |
| Fit | `contain`, `cover`, `stretch` |
| Button variants | `secondary`, `primary`, `danger` |
| Animate | `none`, `fade`, `slide-up`, `slide-left`, `pulse`, `pop` |
| Keys | `"ctrl+s"`, `"ctrl+shift+d"`, `"alt+x"`, `"f1"`…`"f24"`, `"enter"`, `"esc"`, `"tab"`, `"space"`, `"up"`, `"pageup"`, `"home"`, `"backslash"`, any single character |
| Icons | `sparkles`, `check`, `x`, `play`, `pause`, `stop`, `globe`, `book`, `cube`, `sun`, `moon`, `terminal`, `star`, `bolt`, `info`, `warning`, `folder`, `file`, `chat`, `code`, `gear`, `heart`, `send`, `arrow-right`, `arrow-left`, `plus`, `minus`, `dot`, `circle`, `home` |
