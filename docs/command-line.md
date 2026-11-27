# Command line

Installing stagetui also installs a `stagetui` command. It does the same
things as the Python library — handy in shell scripts, CI and AI agents.

`python -m stagetui …` works too, and is useful when `stagetui` isn't on your PATH.

## `stagetui show` — show something now

```bash
stagetui show logo.png
stagetui show chart.svg --width 40
stagetui show "Build finished" --title CI --icon check --accent mint
stagetui show "مرحبا بكم" --title "عربي"
```

| Flag | Meaning |
|---|---|
| `--title`, `--icon`, `--accent`, `--theme`, `--dir` | Card look (see [Apps & widgets](apps.md)) |
| `--width`, `--height` | Size in cells |
| `--fit contain\|cover\|stretch` | How a picture fills its box |

## `stagetui render` — make files, no terminal needed

`render` accepts three kinds of things:

```bash
# 1. Text → a card
stagetui render "مرحبا بكم" --dir rtl --out card.png

# 2. An app file → its screen
stagetui render my_app.py --size 100x30 --out app.png --text app.txt --tree app.json

# 3. An image → PNG (great for converting SVG)
stagetui render logo.svg --out logo.png
```

| Flag | Meaning |
|---|---|
| `--out FILE.png` | Write a PNG picture |
| `--text FILE.txt` | Write the screen as plain text |
| `--tree FILE.json` | Write the widget tree |
| `--size 100x30` | Screen size in cells (for images: pixels). Default: fit the content |
| `--cell 16x32` | Pixel size of one cell in PNGs (default; bigger = sharper) |
| `--title`, `--icon`, `--accent`, `--theme`, `--dir` | Card look (text only) |

Files are saved in the folder you run the command from (or wherever the path
points), and `render` tells you exactly where: `✓ saved /home/you/project/app.png`.
With none of `--out`, `--text`, `--tree`, the screen text is printed instead.

## `stagetui mcp` and `stagetui drive` — let agents use an app

```bash
stagetui mcp my_app.py                  # MCP tools for coding agents (stdin/stdout)
stagetui drive my_app.py --size 120x40  # JSON-RPC 2.0, one message per line
```

See [Let AI agents see and use your app](agents.md).

## `stagetui start`, `attach`, `sessions`, `stop` — apps that keep running

```bash
stagetui start my_app.py               # start it in the background and open it here
stagetui start my_app.py -d            # start it in the background only
stagetui sessions                      # what is running
stagetui attach my-app                 # open it (again) in this terminal
stagetui attach my-app:chat            # just one pane (it pops out into this terminal)
stagetui stop my-app                   # quit it
```

While attached, **Ctrl+Space** then **d** detaches; **q** asks whether to detach or
quit the app. See [Sessions](sessions.md).

## `stagetui doctor` — what can my terminal do?

```bash
stagetui doctor
```

A plain-English report: pictures, colors, keyboard, Arabic support, SSH, and
suggestions to make things better.

## `stagetui inspect` — the same, as JSON

```bash
stagetui inspect             # pretty JSON
stagetui inspect --compact   # one line
```

Useful for bug reports and for programs that need to know about the terminal.

## Flags shared by `show`, `doctor`, `inspect`

| Flag | Meaning |
|---|---|
| `--headless` | Pretend there is no terminal |
| `--no-probe` | Don't ask the terminal questions (use environment variables only) |

## Exit codes

`0` success · `1` something went wrong (the message explains how to fix it) · `2` the command was typed incorrectly.
