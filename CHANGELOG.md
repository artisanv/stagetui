# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added — Milestone 2: text, Arabic/RTL, fonts, pictures
- Arabic/Persian/Urdu contextual shaping into presentation forms (tables generated from Unicode data), lam-alef ligatures, harakat kept on their letters.
- UAX #9 BiDi reordering per line, by grapheme cluster, with bracket mirroring; layout measured on shaped widths.
- BiDi-aware terminals (VTE) are switched to explicit mode (`CSI 8 l`) so text is never reversed twice.
- Bundled fonts: Noto Sans, Noto Sans Arabic, Noto Sans Devanagari, DejaVu Sans Mono; system fonts as fallback; `ts.fonts()` coverage report.
- Pixel text via cosmic-text: complex-script runs drawn as images on Kitty/Ghostty (`TERMSTAGE_TEXT_MODE`).
- Images: PNG, JPEG, GIF, WebP, BMP, SVG (crisp at any size). Encoders: Kitty (Unicode placeholders + direct), Sixel (exact palette or NeuQuant), iTerm2, half-blocks.
- `Canvas`: rectangles, rounded rectangles, circles, lines, polylines, linear/radial gradients, shaped text, images, PNG export.
- Widgets `StageImage` and `StageCanvas`; frames now carry reading-order text runs and image placements.
- Headless PNG rasterizer with geometric box drawing; `Snapshot.png`, `Snapshot.save()`; snapshot text in reading order.
- Verbs `ts.show`, `ts.draw`; CLI `termstage show`, `termstage render` (text, images, `.py` apps).
- Beginner-friendly documentation in `docs/`, with every example executed by the test suite.

### Added — Milestone 1: terminal core
- Cargo workspace: `termstage-core` (engine), `termstage-cli` (CLI), `termstage-py` (PyO3 bindings); maturin `abi3-py311` packaging with zero Python runtime dependencies.
- Capability probing by querying the terminal (XTVERSION, Kitty graphics, Kitty keyboard, DECRQM 2026, cell/window pixel size, DA1 end marker), with tmux passthrough, Windows console support, and a pure, unit-tested resolver.
- Graphics protocol choice: Kitty (+ Unicode placeholders) → iTerm2 (WezTerm/iTerm2) → Sixel (Windows Terminal, foot, xterm) → half-blocks.
- RAII terminal guard with panic hook and SIGTERM/SIGHUP restore; Windows VT output + UTF-8 code pages.
- Session/View split; cell diff renderer with synchronized output and truecolor/256-color modes.
- Headless view: screen as plain text + JSON widget tree with stable ids; golden snapshot test.
- Zero-zombie reaper: process groups, Linux `PDEATHSIG`, Windows kill-on-close Job Object, and a detached sidecar that kills tracked processes and unlinks shared memory even after `kill -9`.
- `StageApp`, `StageCard`, `StageText`, `Style`, `@component`; 5 themes, friendly accents, 22 built-in icons; three-part `TermstageError` (`what` / `why` / `fix`).
- CLI: `termstage inspect`, `termstage doctor`.
- CI (Linux/macOS/Windows × Python 3.11/3.13/3.15/3.15t), release workflow with PyPI trusted publishing, `cargo deny`.
