# Bundled fonts

These fonts are compiled into stagetui so text (including Arabic and Hindi)
looks the same on every machine, even in a bare Docker container.

| File | Font | License |
|---|---|---|
| `NotoSans-Regular.ttf`, `NotoSans-Bold.ttf` | Noto Sans (Latin, Greek, Cyrillic) | SIL OFL 1.1 — `LICENSE-OFL-Noto.txt` |
| `NotoSansArabic-Regular.ttf`, `NotoSansArabic-Bold.ttf` | Noto Sans Arabic | SIL OFL 1.1 — `LICENSE-OFL-Noto.txt` |
| `NotoSansDevanagari-Regular.ttf` | Noto Sans Devanagari | SIL OFL 1.1 — `LICENSE-OFL-Noto.txt` |
| `DejaVuSansMono.ttf`, `DejaVuSansMono-Bold.ttf` | DejaVu Sans Mono 2.37 (monospace, includes all Arabic presentation forms) | Bitstream Vera / public domain — `LICENSE-DejaVu.txt` |

Chinese, Japanese and Korean fonts are large, so stagetui uses the ones already
installed on your system (macOS and Windows always have them).
