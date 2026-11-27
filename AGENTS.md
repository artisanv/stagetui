# AGENTS.md — working on and with stagetui

Read `termstage-SPEC-v8.0.md` first. It is the source of truth.

## Architecture in one breath
- `crates/stagetui-core` — the engine (pure Rust, no PyO3). All logic lives here.
  - `text/` shaping (Arabic presentation forms), BiDi, layout, fonts (bundled in `assets/fonts/`), pixel text
  - `gfx/` images, Canvas, Kitty/Sixel/iTerm2/half-block encoders, headless rasterizer
  - `tree.rs` app description → retained widget arena (ids, keys, update/split/remove)
  - `layout.rs` taffy flex/grid → cell rects, scroll extents, split dividers, modals
  - `session.rs` state machine: focus, keys, mouse, scroll, drag, modals, toasts, motion, events
  - `input.rs` keys/mouse (crossterm + `"ctrl+s"` parsing); `motion.rs` effects on a clock
  - `app.rs` terminal runtime: input thread + dirty-driven render thread + event queue;
    local and remote windows (each its own `TerminalView`), accept loop for attaching windows
  - `ipc.rs` sessions: private runtime folder, token, framing, registry, `attach` client
  - `workspace.rs` layouts of splits/panes: capture, apply (re-using nodes by id), JSON files
  - `launch.rs` opens a new terminal window (per terminal family) for popped-out panes; finds or installs a picture terminal
  - `plugins.rs` add-on discovery (`stagetui-plugin-*`), the official catalog, installer (pip / uv); `widgets/plugin.rs` runs one
- `crates/stagetui-plugin-api` — the add-on protocol, `PaneSource` trait, `serve()` (no stagetui dependency)
- `crates/stagetui-video` — the video add-on (`stagetui-plugin-video`; its own PyPI package, ffmpeg via
  imageio-ffmpeg, sound via cpal). Not a default workspace member (needs ALSA headers on Linux):
  `cargo build -p stagetui-video`; test with `STAGETUI_PLUGIN_PATH=$PWD/target/debug pytest tests/python/test_video.py`
- `crates/stagetui-web` — the web add-on (`stagetui-plugin-web`; its own PyPI package): headless Chrome via
  chromiumoxide + tokio, screencast JPEG frames, CDP input. `cargo build -p stagetui-web`; test with
  `STAGETUI_PLUGIN_PATH=$PWD/target/debug pytest tests/python/test_web.py`
- `crates/stagetui-3d` — the 3D add-on (`stagetui-plugin-3d`; its own PyPI package): its own glTF/OBJ/STL
  readers and a CPU renderer (bands on every core, deferred shading, 2×2 smoothing when still). No GPU,
  no new libraries. `cargo build -p stagetui-3d`; test with `STAGETUI_PLUGIN_PATH=$PWD/target/debug pytest tests/python/test_3d.py`
  - `widgets/` card, text, image, button, input, checkbox, select, progress, spinner, table, tree, bar,
    markdown (streaming renderer), code, diff, log, chat, prompt, tool_call (+ permission), terminal (PTY + vt100)
  - `panes.rs` Blender-style pane types, menu, built-in content; `text/highlight.rs` syntect
  - `frame.rs` one painted screen: cells + reading-order text runs + image placements (blit/clip/occlude)
  - `view.rs` per-terminal graphics state; `inline.rs` `show`; `headless.rs` snapshots
- `crates/stagetui-cli` — `stagetui` CLI (clap). Also used by Python's console script.
- `crates/stagetui-py` — PyO3 bindings → `stagetui._core`. Glue only.
- `python/stagetui` — typed facade (`StageApp`, `StageCard`, …), handler dispatch, asyncio bridge. Glue only.

## Rules
1. **Rust-first.** New behaviour goes in `stagetui-core`. Python files hold only typing, facades, decorators and asyncio glue.
2. **Unsafe only in `stagetui_core::os::*`**, each block with a `SAFETY:` comment.
3. **Errors are three-part** (`Error::friendly(what, why, fix)`); the fix must be something a user can actually do.
4. **Every text test includes** Arabic with harakat, mixed Arabic–English, Japanese/Chinese and Devanagari.
5. **CLI ↔ library parity**: anything the Python API can do, the CLI can do, with the same output.
6. **No new dependency** without asking the project owner.
7. **Never probe a real terminal in tests**: set `STAGETUI_HEADLESS=1`, or drive a PTY and answer queries yourself (see `tests/python/test_pty.py`).
8. One milestone at a time; stop for manual verification between milestones.
9. **Document everything you build** in `docs/` in plain language a new Python learner can follow.
   Every ```python block in `README.md` and `docs/*.md` is executed by `tests/python/test_docs.py`.
10. Regenerate Unicode tables with `python scripts/gen_tables.py`; never edit `*_table.rs` / `diacritics.rs` by hand.

## Commands
```bash
source .venv/bin/activate                      # uv venv && uv pip install maturin pytest
maturin develop --release                      # build the extension into the venv
cargo test -p stagetui-core -p stagetui-cli  # Rust tests
python -m pytest                               # Python tests
cargo clippy --all-targets -- -D warnings && cargo fmt --all
cargo clippy --target x86_64-pc-windows-msvc -p stagetui-core -p stagetui-cli --all-targets -- -D warnings
STAGETUI_UPDATE_SNAPSHOTS=1 cargo test -p stagetui-core headless   # refresh golden snapshots (review the diff!)
python scripts/screenshots.py                  # README gallery pictures (docs/images/) from the real examples
```

## Operating an app as an agent
`stagetui mcp my_app.py` exposes the app as MCP tools (snapshot, press, type, click, …);
`stagetui drive my_app.py` is the same over plain JSON-RPC. In Python: `with App().drive((100, 30)) as d: ...`.

## Sessions
`stagetui start app.py -d` runs an app in the background (`STAGETUI_SERVE=1`); `stagetui sessions`,
`stagetui attach <name>`, `stagetui stop <name>`. Tests use a private `XDG_RUNTIME_DIR`
(see `tests/python/test_sessions.py`) and `ipc::Registry::at(dir)` in Rust.

## Seeing what you built (headless)
```python
snap = MyApp().run_headless((100, 30), steps=[{"press": "tab"}, {"type": "hi"}, {"click": "#send"}])
print(snap.text)  # the screen as plain text
snap.query("#chat")  # widget JSON: id, type, bounds, text, dir, theme, focused…
```
`stagetui inspect` prints capabilities as JSON; `stagetui doctor` explains them.

## Environment variables
| Variable | Effect |
|---|---|
| `STAGETUI_HEADLESS=1` | Force headless mode |
| `STAGETUI_SIZE=120x40` | Headless grid size |
| `STAGETUI_NO_PROBE=1` | Do not send terminal queries |
| `STAGETUI_PROBE_TIMEOUT_MS` | Probe timeout (default 250) |
| `STAGETUI_TEXT_MODE=auto\|cells\|pixels` | How complex scripts are drawn |
| `STAGETUI_SYSTEM_FONTS=0` | Bundled fonts only (deterministic PNGs) |
| `STAGETUI_REDUCE_MOTION=1` | Disable animations |
| `STAGETUI_UPDATE_SNAPSHOTS=1` | Rewrite golden snapshots in tests |
| `STAGETUI_SERVE=1` | Run in the background with no terminal of its own (set by `stagetui start`) |
| `STAGETUI_SESSION=name` | Session name |
| `STAGETUI_NO_SESSION=1` | Interactive apps don't accept attaching windows |
| `STAGETUI_LOG=path` | Log file shown by `stagetui sessions --json` (set by `stagetui start`) |
| `STAGETUI_CHROME=path` | Browser for the web add-on |
| `STAGETUI_WEB_NO_SANDBOX=1` | Chrome without its sandbox (CI containers only) |
| `STAGETUI_PLUGIN_PATH=dirs` | Extra folders with `stagetui-plugin-*` add-ons |
| `STAGETUI_NO_PLUGINS=1` | Don't list add-ons in pane menus |
| `STAGETUI_LAUNCHER="foot {cmd}"` | How pop-out pane windows are opened (default: same terminal family) |
