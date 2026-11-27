# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0a3] — 2026-11-27

### Changed
- README and PyPI page: a calm "Alpha" note instead of the red work-in-progress warning.

## [0.1.0a2] — 2026-11-27

Milestones 5 and 6a–6b: sessions, workspaces, panes in their own windows, add-ons, video.
The video add-on is its own package: `pip install stagetui-video`.

### Added — Milestone 6b: video
- **`stagetui-video`**, the video add-on (its own package): `StagePane("video", path=… or
  url=…)` plays files and links with sound. Space pause, ← → seek (Shift: 30 s), ↑ ↓ volume,
  M mute, Home start over; click pauses, wheel changes the volume; a status line
  (`▶ 01:23 / 04:56 · volume 80%`). Options `autoplay`, `loop`, `volume`, `muted`, `start`;
  calls `play`, `pause`, `toggle`, `seek`, `volume`, `mute`, `state`; events `ended`, `state`.
- ffmpeg comes from `imageio-ffmpeg` (a dependency of the add-on); sound plays through `cpal`
  (ALSA on Linux — the system's own library, so PipeWire/PulseAudio work — CoreAudio, WASAPI).
  The sound is the clock; pictures follow it, decoded at the pane's size (up to 1280×720).
- Release workflow `release-video.yml` (tags `video-v*`), CI job for the add-on on all three
  systems, example `11_video.py`, docs: Add-ons → Video.
- Fixed: a pane that only said "not installed" was moved to a picture terminal.

### Added — Milestone 6a: add-ons
- **Add-ons (plugins)**: pane types served by their own program, `stagetui-plugin-<name>`,
  found next to the app's Python, in `STAGETUI_PLUGIN_PATH` or on the `PATH`. They show
  frames (pictures) with each terminal's best graphics, get keys/mouse/paste, send events
  (`on_<name>`), answer calls (`pane.content.call(...)`), and crash on their own: the pane
  says so and Enter restarts. Processes are tracked by the reaper.
- New crate `stagetui-plugin-api` (protocol + `PaneSource` trait + `serve()`), and
  `stagetui.plugin` to write add-ons in Python; `Canvas.rgba()`; `StagePlugin`;
  `ts.plugins()`; `stagetui plugins [install <name>]`.
- **Install on first use**: picking an add-on that isn't installed asks "Install …?" and
  installs it into the app's Python (pip, or uv) inside the pane. Official add-ons (video,
  web, 3D) show "coming soon" until we publish them (never installed by name before then).
- **Real pictures**: in a terminal that draws pictures with blocks, a picture add-on opens
  by itself in Ghostty, Kitty, WezTerm or foot if one is installed — or stagetui offers to
  install one (pacman / apt / dnf / zypper, Homebrew, winget), running in the pane.
- The "Leave?" prompt became a general dialog; `Key::name()`; example `10_plugins.py` with
  a sample add-on in `examples/plugins/`; docs: Add-ons.

### Added — Milestone 5b
- **"Leave?" prompt**: q, Ctrl+C or Esc in an attached window asks *Detach* (default),
  *Quit app* or *Cancel*, so a background app is never stopped by accident. Apps run with
  `python app.py` still quit with q.
- **Panes pop out into their own terminal window**: the ↗ header button, the pane menu,
  Ctrl+Space then o, the `pop_out` action, or `widget.detach()` / `attach()`. The window
  runs `stagetui attach <app>:<pane>`; closing it snaps the pane back, with everything
  still running. Its spot in the app shows "↙ Bring it back".
- Terminal launcher: Kitty, Ghostty, WezTerm, foot, Alacritty, Konsole, GNOME Terminal,
  xterm, mintty, Windows Terminal, Terminal.app / iTerm2 (AppleScript), tmux; over SSH it
  shows the command instead. `LAUNCHER = "..."` on the app or `STAGETUI_LAUNCHER` override it.
- `stagetui attach app:pane` shows one pane in any terminal; session protocol 2.
- `09_detach.py` has panes to pop out.

### Added — Milestone 5a: sessions and workspaces
- **Sessions**: every interactive app is a session other terminals can attach to, over a private
  local socket (user-only folder, `0600`; named pipes on Windows) with a per-session secret token.
- `stagetui start app.py [--name] [--detached] [-- args]` runs an app in the background (output to a
  private log; a crash shows its last lines), `stagetui attach [name]`, `stagetui sessions [--json]`,
  `stagetui stop [name]`. **Ctrl+Space then d** detaches; the app keeps running.
- Several windows can show one app at the same time, each drawn with its own terminal's graphics
  (Kitty pictures in one, half blocks in another); the app takes the size of the window used last.
- `ts.sessions()`, `app.session`, and `app.background(coro)` for long-running async work.
- **Workspaces**: `StageWorkspaces` (Blender-style tabs; Ctrl+Space then 1–9),
  `app.workspace()`, `app.save_workspace()`, `app.load_workspace()`, `app.workspaces()`.
  Layouts are plain JSON; loading re-uses panes by id, so shells and chats keep running.
  Panes remember their options so saved layouts can rebuild them.
- Agent tools (MCP and `drive`): `workspace`, `load_workspace`, `save_workspace`.
- Example `09_detach.py` (a pretend agent that keeps working), workspace tabs and saved layouts in
  `08_workspace.py`; docs: Sessions, Panes → Workspaces.
- New dependency: `interprocess` (local sockets / named pipes).

### Fixed
- `pane.content` found nothing in an app that had not been drawn yet (background sessions).

## [0.1.0a1] — first alpha

### Changed
- **Renamed to `stagetui`** (the name `termstage` was already taken on PyPI and crates.io):
  `pip install stagetui`, `import stagetui`, the `stagetui` command, crates `stagetui-core` /
  `stagetui-cli`, environment variables `STAGETUI_*`, and the error class `StageError`.

### Added — Milestone 4b: agent interface
- `stagetui drive app.py` (JSON-RPC 2.0 over stdio) and `stagetui mcp app.py` (MCP tools: snapshot with optional PNG, press, type, click, scroll, resize, wait, query, list_panes, set_pane_type); app output is kept off the protocol channel.
- `StageApp.drive()` → `Driver` for step-by-step headless control; `run_headless` is built on it.
- `capture_text()`, `capture_tree()`, `capture_png()` on running apps.
- pytest plugin (`stage_snapshot` fixture, `--stage-update`), golden text and PNG comparison with a tolerance (`_core.png_diff`).
- Docs: Let AI agents see and use your app.

### Added — Milestone 4a: harness kit and Blender-style panes
- `StageChat` (user / assistant / system / tool messages, streaming via `msg.append`, follow mode, select + copy with OSC 52), `StageMarkdown` (streaming Markdown with a cache of finished blocks; headings, emphasis, lists, task lists, quotes, tables, syntax-highlighted code blocks; RTL paragraphs), `StagePrompt` (multi-line, history, `/` commands and `@` file completion, paste).
- `StageCode` (syntect, 150+ languages, line numbers), `StageDiff` (similar; hunks with accept/reject, `result`), `StageToolCall` (collapsible, spinner/✓/✗), `StagePermission` and `await app.ask_permission(...)`, `StageLog` (ring buffer, level colors, filter).
- `StageTerminal`: a real program in a pseudo-terminal (portable-pty, ConPTY on Windows) rendered with vt100; keys, paste, mouse and scrollback; tracked by the reaper.
- Blender-style panes: `StagePane(type, **options)`, header with type menu and split/close buttons, stash-and-restore when switching, `@ts.pane_type`, `app.panes()`, prefix key (`Ctrl+Space`) for pane commands from anywhere.
- Widget methods over one channel (`call`), outside wake-ups for shell output, clipboard (OSC 52).
- Examples `07_harness.py` (pretend agent) and `08_workspace.py`; docs: Build an AI agent harness, Panes.
- New dependencies: pulldown-cmark, syntect (pure-Rust regex), similar, portable-pty, vt100.

### Added — Milestone 3: layout, widgets, interaction, motion
- `taffy` flex/grid layout: `Stage` (vertical, horizontal, grid), `width`/`height` in cells or `%`, `grow`; content centers when it fits and **scrolls** when it doesn't (scrollbars, wheel, PageUp/PageDown).
- `StageSplit` with ratios and mouse-draggable dividers; `widget.split()` / `widget.remove()` at runtime (nested splits collapse back).
- `StageTabs`, `StageModal` (dimmed backdrop, focus trap, Esc/click-outside, pictures underneath occluded), `StageBar` (status + shortcut hints), toasts via `app.notify()`.
- Controls: `StageButton` (3 variants), `StageInput` (RTL-aware editing, paste, password), `StageCheckbox`, `StageSelect` (dropdown), `StageProgress` (smooth eighth blocks, indeterminate), `StageSpinner`, `StageTable`, `StageTree`.
- Focus and input: Tab order, hover highlights, click-to-focus, mouse wheel, Kitty keyboard + legacy key normalization, `BINDINGS` with built-in actions, live theme switching.
- Motion: `fade`, `slide-up`, `slide-left`, `pop`, `pulse` — deterministic under a virtual clock; `reduce_motion` / `STAGETUI_REDUCE_MOTION=1`.
- Live `StageCanvas(on_frame=..., fps=...)` with back-pressure.
- Threaded runtime (input + dirty-driven render threads, GIL released); Python handlers may be `async`; `await app.run_async()`.
- Headless `run_headless(steps=[...])`: press, type, click, scroll, wait, resize; `settle=`.
- Lifecycle verbs on every widget: `update`, `load`, `show`, `hide`, `focus`, `split`, `remove`, `value`.
- `@ts.component` accepts functions returning a widget or a list; `Style` gains `shadow`, `width`, `height`, `grow`.
- Sixel/iTerm2/Kitty-direct views re-send only pictures that changed (live canvases stay cheap).
- Examples `05_ui_gallery.py`, `06_splits.py`; docs: Layouts, Making apps interactive, Motion.

### Changed
- Overflowing content now scrolls instead of being hidden behind a "more below" hint.
- `_core.run_app` / `run_headless` / `render_png` replaced by the `_core.App` class.

### Added — Milestone 2: text, Arabic/RTL, fonts, pictures
- Arabic/Persian/Urdu contextual shaping into presentation forms (tables generated from Unicode data), lam-alef ligatures, harakat kept on their letters.
- UAX #9 BiDi reordering per line, by grapheme cluster, with bracket mirroring; layout measured on shaped widths.
- BiDi-aware terminals (VTE) are switched to explicit mode (`CSI 8 l`) so text is never reversed twice.
- Bundled fonts: Noto Sans, Noto Sans Arabic, Noto Sans Devanagari, DejaVu Sans Mono; system fonts as fallback; `ts.fonts()` coverage report.
- Pixel text via cosmic-text: complex-script runs drawn as images on Kitty/Ghostty (`STAGETUI_TEXT_MODE`).
- Images: PNG, JPEG, GIF, WebP, BMP, SVG (crisp at any size). Encoders: Kitty (Unicode placeholders + direct), Sixel (exact palette or NeuQuant), iTerm2, half-blocks.
- `Canvas`: rectangles, rounded rectangles, circles, lines, polylines, linear/radial gradients, shaped text, images, PNG export.
- Widgets `StageImage` and `StageCanvas`; frames now carry reading-order text runs and image placements.
- Headless PNG rasterizer with geometric box drawing; `Snapshot.png`, `Snapshot.save()`; snapshot text in reading order.
- Verbs `ts.show`, `ts.draw`; CLI `stagetui show`, `stagetui render` (text, images, `.py` apps).
- Beginner-friendly documentation in `docs/`, with every example executed by the test suite.

### Added — Milestone 1: terminal core
- Cargo workspace: `stagetui-core` (engine), `stagetui-cli` (CLI), `stagetui-py` (PyO3 bindings); maturin `abi3-py311` packaging with zero Python runtime dependencies.
- Capability probing by querying the terminal (XTVERSION, Kitty graphics, Kitty keyboard, DECRQM 2026, cell/window pixel size, DA1 end marker), with tmux passthrough, Windows console support, and a pure, unit-tested resolver.
- Graphics protocol choice: Kitty (+ Unicode placeholders) → iTerm2 (WezTerm/iTerm2) → Sixel (Windows Terminal, foot, xterm) → half-blocks.
- RAII terminal guard with panic hook and SIGTERM/SIGHUP restore; Windows VT output + UTF-8 code pages.
- Session/View split; cell diff renderer with synchronized output and truecolor/256-color modes.
- Headless view: screen as plain text + JSON widget tree with stable ids; golden snapshot test.
- Zero-zombie reaper: process groups, Linux `PDEATHSIG`, Windows kill-on-close Job Object, and a detached sidecar that kills tracked processes and unlinks shared memory even after `kill -9`.
- `StageApp`, `StageCard`, `StageText`, `Style`, `@component`; 5 themes, friendly accents, 22 built-in icons; three-part `StageError` (`what` / `why` / `fix`).
- CLI: `stagetui inspect`, `stagetui doctor`.
- CI (Linux/macOS/Windows × Python 3.11/3.13/3.15/3.15t), release workflow with PyPI trusted publishing, `cargo deny`.
