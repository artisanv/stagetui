# Arabic, Persian, Hindi, Chinese… — world languages

termstage is built so that every language looks right, in every terminal,
without any setup. This page explains what works and the few things worth knowing.

## It just works

```python
import termstage as ts

ts.show("مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج", title="العربية")
ts.show("سلام دنیا، گل‌های زیبا", title="فارسی")
ts.show("नमस्ते दुनिया", title="हिन्दी")
ts.show("こんにちは世界 · 你好世界 · 안녕하세요", title="CJK")
```

What termstage does for you:

- **Letters join.** Arabic letters change shape depending on their neighbours
  (ب at the start of a word looks different from ب at the end). termstage picks
  the right shape for every letter, including the special لا ligature and the
  Persian letters پ چ ژ گ ک ی.
- **Right-to-left order.** Arabic and Persian lines are written from the right,
  while English words and numbers inside them stay left-to-right.
- **Mirrored layout.** In an RTL card the title moves to the right edge and the
  text is right-aligned. Brackets are mirrored, so `(مرحبا)` looks correct.
- **Harakat (tashkeel)** such as َ ُ ِ ّ stay on top of the right letter.

## Choosing the direction

By default termstage looks at the first real letter: Arabic → right-to-left,
English → left-to-right. You can force it with `dir`:

```python
from termstage import StageCard

StageCard("123 مرحبا", dir="rtl")
StageCard("Version 2 — الإصدار الثاني", dir="ltr")
```

## How it looks in different terminals

| Terminal | How Arabic is drawn |
|---|---|
| **Any terminal that shows pictures** (Kitty, Ghostty, WezTerm, iTerm2, foot, Windows Terminal, xterm) | As small pictures of text, drawn with termstage's bundled Noto fonts. Perfect whatever font your terminal uses. |
| **GNOME Terminal, Ptyxis, Tilix** (VTE) | As shaped letters in cells. termstage switches off VTE's own reordering so text is never reversed twice. |
| **Everything else** | As shaped letters in cells. Looks best with a monospace font that includes Arabic "presentation forms", such as **DejaVu Sans Mono**. |

You can choose the mode yourself with an environment variable:

```bash
TERMSTAGE_TEXT_MODE=cells python my_app.py    # always letters in cells
TERMSTAGE_TEXT_MODE=auto python my_app.py     # default: pictures wherever the terminal can show them
```

> **Copying text.** Where Arabic is drawn as a picture, selecting it
> with the mouse may not copy it. App-level copy support is *coming soon*.

## Fonts

termstage carries its own fonts, so pictures and PNG snapshots look the same on
every computer — even inside a minimal Docker container:

- **Noto Sans** — Latin, Greek, Cyrillic
- **Noto Sans Arabic** — Arabic, Persian, Urdu
- **Noto Sans Devanagari** — Hindi, Marathi, Nepali
- **DejaVu Sans Mono** — the monospace font for terminal-style snapshots

Chinese, Japanese and Korean fonts are very large, so termstage uses the ones
already on your computer (macOS and Windows always have them). To check:

```python
import termstage as ts

report = ts.fonts()
for script in report["scripts"]:
    print(script["script"], "✓" if script["covered"] else "missing")
```

If CJK shows as missing on Linux, install Noto CJK — for example
`sudo apt install fonts-noto-cjk` or `sudo pacman -S noto-fonts-cjk`.
