# Reference

Every public name in `stagetui`, in one place. Import with `import stagetui as ts`
or `from stagetui import ...`.

Sizes: **cells** for things on screen (`width=20`), **pixels** for canvases (`size=(640, 360)`).

## Verbs

### `ts.show(content, *, title=None, icon=None, dir=None, theme=None, accent=None, width=None, height=None, fit=None)`
Show text (as a card) or a picture, inline. `content` can be text, an image path
(`str` or `pathlib.Path`), image `bytes`, or a `Canvas`. Returns `None`.

### `ts.draw(fn=None, *, size=(640, 360), background=None, out=None, show=True)`
Create a `Canvas`, call `fn(canvas)`, then save it to `out` (if given) or show it
(unless `show=False`). Returns the `Canvas`.

### `ts.inspect(*, headless=False, probe=True) -> dict`
What the current terminal can do (same as `stagetui inspect`).

### `ts.fonts() -> dict`
Bundled fonts and which writing systems have fonts available.


### `ts.plugins(*, refresh=False) -> list[dict]`
Add-ons: installed ones and the official ones you can install — `name`, `title`,
`installed`, `package`, `about`, `available` (it can be installed now). A
`stagetui-plugin-<name>` file that was found but didn't answer (say, its `#!` Python
has no stagetui) is listed too, with `"broken": {"path", "why"}`. Same as
`stagetui plugins --json` ([Add-ons](plugins.md)).

### `ts.keys() -> dict`
The keys every app starts with on this computer: `keys` (each built-in key with `name`,
`keys`, `labels`, `after_prefix`, `about`), the person's keys `file` and any `problems`
in it. Same as `stagetui keys --json` ([Keys](keys.md)).

### `stagetui.plugin.Plugin` / `serve(plugin)`
Write an add-on in Python: override `start`, `frame`, `key`, `mouse`, `call`; use
`self.status(...)` and `self.event(...)`; finish the file with `serve(MyPlugin())`.

### `ts.sessions() -> list[dict]`
The apps running right now that a terminal can attach to: `name`, `pid`, `app`, `cwd`, `started`.
Same as `stagetui sessions --json` ([Sessions](sessions.md)).

## Apps

### `class StageApp`
| Member | Description |
|---|---|
| `theme = "aurora-dark"` | Class attribute: color scheme |
| `accent = None` | Class attribute: highlight color |
| `dir = None` | Class attribute: `"rtl"` / `"ltr"` layout; `None` guesses from the text |
| `BINDINGS = []` | Class attribute: shortcuts `(key, action)` or `(key, action, description)`; they come before the built-in keys |
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
| `panes()` | Every pane: `id`, `type`, `bounds`, `focused`, `stashed` |
| `drive(size=(80, 24), *, cell=(16, 32)) -> Driver` | Run without a terminal, step by step (use with `with`) |
| `capture_text()`, `capture_tree()`, `capture_png()` | The live screen while the app runs |
| `await ask_permission(question, *, tool=None, args=None)` | Pop-up: returns `"allow"`, `"always"` or `"deny"` |
| `PREFIX = "ctrl+space"` | Class attribute: the key that reaches the app from a terminal pane (same as `KEYS = {"prefix": …}`) |
| `KEYS = {}` | Class attribute: rename or turn off built-in keys by name — `{"quit": "ctrl+q", "close_pane": "w", "pane_menu": None}` ([Keys](keys.md)) |
| `CONFIRM_QUIT = False` | Class attribute: `True` asks "Quit this app?" before a quit key or the last pane's × quits |
| `keys()` | Every key this app answers to: `keys`, `shortcuts`, `confirm_quit`, `file`, `problems` ([Keys](keys.md)) |
| `LAUNCHER = None` | Class attribute: how a pane opens in its own window, e.g. `"foot {cmd}"` (`None`: the terminal in use) |
| `background(work)` | Run `async` work (an agent loop…) alongside the app; cancelled when it quits. Without a terminal it runs while time passes (`wait`), and no step waits for it |
| `session` | While running: `{"name", "pid", "app", …}` — other terminals open it with `stagetui attach <name>` ([Sessions](sessions.md)) |
| `workspace()` | The layout of splits and panes, as a dict |
| `save_workspace(name=None, *, path=None) -> Path` | Save the layout (name: the open workspace tab, or `"default"`) |
| `load_workspace(name_or_layout=None, *, path=None)` | Bring a saved layout back, or apply a layout dict ([Workspaces](panes.md#save-a-layout-bring-it-back)) |
| `workspaces(*, path=None) -> list[str]` | Names of the saved layouts |

Built-in actions for `BINDINGS`: `quit`, `next_theme`, `focus_next`, `focus_previous`, `close_modal`,
`pane_menu`, `split_pane_right`, `split_pane_down`, `close_pane` (on the last pane it
closes the app, like `quit`), `pop_out` (open the focused pane in its own window, or
bring it back), `detach`. A shortcut with one of these names always does the built-in
job — an `action_close_pane` method of yours would never run — so give your own
actions other names (`("x", "close", "Close")` calls `action_close`).

### `class Driver`
Made by `app.drive()`; use it in a `with` block.

| Member | Description |
|---|---|
| `press(key)`, `type(text)`, `click("#id" or (x, y))`, `drag("#id" or (x, y), (x, y))`, `scroll(rows, target=None)` | Act like a person |
| `wait(seconds)`, `wait_idle()`, `resize(cols, rows)`, `step({...})` | Time, size, any `run_headless` step |
| `snapshot(*, settle=False, png=False) -> Snapshot` | The screen now |
| `query(id) -> dict | None`, `panes()`, `keys()` | Look at widgets, panes and the keys the app answers to |
| `has_quit` | `True` once the app quit (`q`, Ctrl+Space then `q`, the last pane's ×): its shells and add-ons stop, you can still look, `wait` does nothing, and pressing, typing or clicking raises a `StageError` |
| `close()` | Stop (done by `with`): the shells and add-ons the app started stop at once; snapshots taken before still work |

### pytest: `stage_snapshot` fixture
`stage_snapshot.assert_match(snap, name=None, *, png=False, tolerance=0.002)` — compare with
the saved copy in `__snapshots__/` (created on the first run). Refresh with `pytest --stage-update`.

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
| `StageChat(messages)` | list of `{"role", "text"}` | `height` | `on_copy(text)` |
| `StageMarkdown(text)` | Markdown | `height` | — |
| `StagePrompt(text)` | starting text | `placeholder`, `title`, `commands`, `mentions`, `max_lines` | `on_submit(text)`, `on_change(text)` |
| `StageCode(source)` | code | `language`, `path`, `line_numbers`, `height` | — |
| `StageDiff((old, new))` | a pair of texts, or `None` | `path`, `height` | `on_decide({"hunk", "decision"})` |
| `StageToolCall(name)` | tool name | `args`, `status`, `output`, `expanded` | — |
| `StagePermission(question)` | text | `tool`, `args`, `border` | `on_decide(choice)` |
| `StageTerminal(command)` | command (`""` = your shell) or list | `cwd`, `title`, `height` | — |
| `StageLog(text)` | starting text | `max_lines`, `filter`, `height` | — |
| `StagePane(type, **options)` | a pane type | the type's options | `on_change(type)` |
| `StagePlugin(name, **options)` | an add-on's name | its options; `on_<event>=` for its events | the add-on's events |
| `StageWorkspaces(workspaces)` | `{"name": layout, …}` or a list of names | `active` | `on_change(index)` |

### Methods on every widget
`update(content=None, **options)` · `load(content)` · `show()` · `hide()` ·
`focus()` · `split(widget, direction="horizontal")` · `remove()` · `value` ·
`detach()` / `attach()` (open the pane holding the widget in its own window, and
bring it back — [Panes](panes.md#open-a-pane-in-its-own-window)). Containers also have `add(widget)`.

### Extra methods
| Widget | Methods |
|---|---|
| `StageChat` | `add(role, text="", *, name=None) -> ChatMessage`, `add_tool(name, args=None) -> ChatMessage`, `clear()`, `messages` |
| `ChatMessage` | `append(text)`, `update(text)`, `finish(output="", *, ok=True)` (tool calls) |
| `StageMarkdown` | `append(text)`, `clear()` |
| `StagePrompt` | `clear()` |
| `StageDiff` | `accept_all()`, `reject_all()`, `decide(hunk, "accept"/"reject")`, `result` |
| `StageToolCall` | `finish(output="", *, ok=True)`, `append(text)` |
| `StageTerminal` | `send(text)`, `restart()`, `screen` |
| `StageLog` | `append(text_or_lines)`, `clear()`, `filter(text)` |
| `StagePane` | `set_type(type, **options)`, `switch(type)`, `type`, `content`, `detached` |
| `StageCode` | `StageCode.from_file(path)` |
| `StagePlugin` | `call(method, args=None)`, `restart()` |

### `@ts.pane_type(name, *, title=None, icon="cube")`
Register a function returning a widget as a pane type (offered in every pane's menu).
Built-in types: `chat`, `terminal`, `files`, `diff`, `code`, `markdown`, `log`, `empty`.

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

`canvas.rgba()` gives the raw pixels (what an add-on sends as a frame).

## Errors

### `StageError`
Raised for every problem stagetui can explain. Attributes: `what`, `why`, `fix`.
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
