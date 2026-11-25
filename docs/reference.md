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
| `compose(self) -> list` | Override: return the widgets to show |
| `run()` | Run in the terminal until q / Esc / Ctrl+C |
| `run_headless(size=(80, 24), *, cell=(16, 32)) -> Snapshot` | Draw without a terminal |

### `class Snapshot`
| Member | Description |
|---|---|
| `text` | Screen as plain text, reading order |
| `tree` | `dict` describing every widget |
| `png` | Screen as PNG `bytes` (made on first use) |
| `cols`, `rows` | Size in cells |
| `query(id)` | One widget's `dict` by id (`"x"` or `"#x"`), or `None` |
| `save(path)` | Save as `.png`, `.txt` or `.json`; returns the `Path` |

## Widgets

All widgets accept: `id`, `theme`, `accent`, `animate`; text widgets also accept
`title`, `icon`, `rounded`, `padding`, `dir`, `style`. See [Apps & widgets](apps.md).

| Widget | First argument | Extra options |
|---|---|---|
| `StageCard(content, ...)` | text | — |
| `StageText(content, ...)` | text | — |
| `StageImage(source, ...)` | path, `bytes`, or `Canvas` | `width`, `height` (cells), `fit` |
| `StageCanvas(draw, ...)` | a function `draw(canvas)` | `size` (pixels), `background`, `width`, `height` (cells) |

### `Style(padding=None, rounded=None, accent=None)`
Reusable look settings for `style=`.

### `@ts.component`
Marks a function that returns widgets as a reusable component.

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
| Direction | `auto`, `ltr`, `rtl` |
| Fit | `contain`, `cover`, `stretch` |
| Animate | `none`, `fade`, `slide-up`, `slide-left`, `pulse`, `pop` (*effect coming soon*) |
| Icons | `sparkles`, `check`, `x`, `play`, `pause`, `stop`, `globe`, `book`, `cube`, `sun`, `moon`, `terminal`, `star`, `bolt`, `info`, `warning`, `folder`, `file`, `chat`, `code`, `gear`, `heart` |
