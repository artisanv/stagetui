# Command line

Installing termstage also installs a `termstage` command. It does the same
things as the Python library — handy in shell scripts, CI and AI agents.

`python -m termstage …` works too, and is useful when `termstage` isn't on your PATH.

## `termstage show` — show something now

```bash
termstage show logo.png
termstage show chart.svg --width 40
termstage show "Build finished" --title CI --icon check --accent mint
termstage show "مرحبا بكم" --title "عربي"
```

| Flag | Meaning |
|---|---|
| `--title`, `--icon`, `--accent`, `--theme`, `--dir` | Card look (see [Apps & widgets](apps.md)) |
| `--width`, `--height` | Size in cells |
| `--fit contain\|cover\|stretch` | How a picture fills its box |

## `termstage render` — make files, no terminal needed

`render` accepts three kinds of things:

```bash
# 1. Text → a card
termstage render "مرحبا بكم" --dir rtl --out card.png

# 2. An app file → its screen
termstage render my_app.py --size 100x30 --out app.png --text app.txt --tree app.json

# 3. An image → PNG (great for converting SVG)
termstage render logo.svg --out logo.png
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

## `termstage doctor` — what can my terminal do?

```bash
termstage doctor
```

A plain-English report: pictures, colors, keyboard, Arabic support, SSH, and
suggestions to make things better.

## `termstage inspect` — the same, as JSON

```bash
termstage inspect             # pretty JSON
termstage inspect --compact   # one line
```

Useful for bug reports and for programs that need to know about the terminal.

## Flags shared by `show`, `doctor`, `inspect`

| Flag | Meaning |
|---|---|
| `--headless` | Pretend there is no terminal |
| `--no-probe` | Don't ask the terminal questions (use environment variables only) |

## Exit codes

`0` success · `1` something went wrong (the message explains how to fix it) · `2` the command was typed incorrectly.
