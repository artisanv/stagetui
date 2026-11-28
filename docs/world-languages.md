# Arabic, Persian, Hindi, Chinese… — world languages

stagetui is built so that every language looks right, in every terminal,
without any setup. This page explains what works and the few things worth knowing.

## It just works

```python
import stagetui as ts

ts.show("مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج", title="العربية")
ts.show("سلام دنیا، گل‌های زیبا", title="فارسی")
ts.show("नमस्ते दुनिया", title="हिन्दी")
ts.show("こんにちは世界 · 你好世界 · 안녕하세요", title="CJK")
```

What stagetui does for you:

- **Letters join.** Arabic letters change shape depending on their neighbours
  (ب at the start of a word looks different from ب at the end). stagetui picks
  the right shape for every letter, including the special لا ligature and the
  Persian letters پ چ ژ گ ک ی.
- **Right-to-left order.** Arabic and Persian lines are written from the right,
  while English words and numbers inside them stay left-to-right.
- **Mirrored layout.** In an RTL card the title moves to the right edge and the
  text is right-aligned. Brackets are mirrored, so `(مرحبا)` looks correct.
- **Harakat (tashkeel)** such as َ ُ ِ ّ stay on top of the right letter.

## Whole apps flip, too

When an app's text starts in Arabic (or Persian, Hebrew, Urdu), the **layout**
mirrors as well: rows start on the right, the first pane of a split sits on the
right, scrollbars and pop-up messages move to the left, and arrows point the
other way. Typing in a `StageInput` works right-to-left, and ← / → follow the
text.

```python
from stagetui import Stage, StageApp, StageButton, StageCard, StageInput


class Arabic(StageApp):
    def compose(self):
        return [
            StageCard(
                "مَرْحَباً! هذا التطبيق كله من اليمين إلى اليسار.", title="أهلاً", icon="sparkles"
            ),
            StageInput(placeholder="اكتب رسالتك…", title="رسالة"),
            Stage(
                [StageButton("إرسال", variant="primary"), StageButton("إلغاء")],
                direction="horizontal",
            ),
        ]


Arabic().run()
```

Set `dir = "rtl"` or `dir = "ltr"` on the app to choose yourself, or `dir=` on a
single container (for example, one tab page).

## Choosing the direction

By default stagetui looks at the first real letter: Arabic → right-to-left,
English → left-to-right. You can force it with `dir`:

```python
from stagetui import StageCard

StageCard("123 مرحبا", dir="rtl")
StageCard("Version 2 — الإصدار الثاني", dir="ltr")
```

## How it looks in different terminals

| Terminal | How Arabic is drawn |
|---|---|
| **Any terminal that shows pictures** (Kitty, Ghostty, WezTerm, iTerm2, foot, Windows Terminal, xterm) | As small pictures of text, drawn with stagetui's bundled Noto fonts. Perfect whatever font your terminal uses. |
| **GNOME Terminal, Ptyxis, Tilix** (VTE) | As shaped letters in cells. stagetui switches off VTE's own reordering so text is never reversed twice. |
| **Everything else** | As shaped letters in cells. Looks best with a monospace font that includes Arabic "presentation forms", such as **DejaVu Sans Mono**. |

You can choose the mode yourself with an environment variable:

```bash
STAGETUI_TEXT_MODE=cells python my_app.py    # always letters in cells
STAGETUI_TEXT_MODE=auto python my_app.py     # default: pictures wherever the terminal can show them
```

> **Copying text.** Where Arabic is drawn as a picture, selecting it
> with the mouse may not copy it. App-level copy support is *coming soon*.

## Fonts

**In your terminal, every language uses your terminal's own fonts** — Chinese,
Japanese and Korean included — so apps look right with no setup.

**Pictures of text** (Arabic and Hindi in terminals that show pictures, and PNG
snapshots) are drawn by stagetui itself. For those it carries fonts inside the
package, so they look the same on every computer — even in a minimal Docker container:

- **Noto Sans** — Latin (including Vietnamese), Greek, Cyrillic
- **Noto Sans Arabic** — Arabic, Persian, Urdu, Pashto, Kurdish, Sindhi, Uyghur
- **Noto Sans Devanagari** — Hindi, Marathi, Nepali, Sanskrit
- **DejaVu Sans Mono** — the monospace font for terminal-style snapshots (also Armenian and Georgian)

Every other script — Chinese, Japanese, Korean, Hebrew, Thai, Bengali, Tamil,
emoji… — uses the fonts already on your computer (macOS and Windows have
Chinese, Japanese and Korean fonts built in; many Linux setups don't). A character
no font has shows as an empty box in pictures, and stagetui says so once on the
terminal's error output, for example:

```text
stagetui: no font for Chinese/Japanese text on this computer — it shows as empty boxes in pictures. Install Noto Sans CJK (Linux: fonts-noto-cjk / noto-fonts-cjk).
```

To check which scripts have a font:

```python
import stagetui as ts

report = ts.fonts()
for script in report["scripts"]:
    print(script["script"], "✓" if script["covered"] else "missing")
```

`report["missing_in_pictures"]` lists the scripts that have already been drawn as
empty boxes in this program, and `stagetui doctor` mentions missing fonts too.

If Chinese, Japanese or Korean shows as missing on Linux, install Noto CJK — for
example `sudo apt install fonts-noto-cjk` or `sudo pacman -S noto-fonts-cjk`.
