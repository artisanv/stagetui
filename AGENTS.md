# AGENTS.md — working on and with termstage

Read `termstage-SPEC-v8.0.md` first. It is the source of truth.

## Architecture in one breath
- `crates/termstage-core` — the engine (pure Rust, no PyO3). All logic lives here.
  - `text/` shaping (Arabic presentation forms), BiDi, layout, fonts (bundled in `assets/fonts/`), pixel text
  - `gfx/` images, Canvas, Kitty/Sixel/iTerm2/half-block encoders, headless rasterizer
  - `tree.rs` app description → retained widget arena (ids, keys, update/split/remove)
  - `layout.rs` taffy flex/grid → cell rects, scroll extents, split dividers, modals
  - `session.rs` state machine: focus, keys, mouse, scroll, drag, modals, toasts, motion, events
  - `input.rs` keys/mouse (crossterm + `"ctrl+s"` parsing); `motion.rs` effects on a clock
  - `app.rs` terminal runtime: input thread + dirty-driven render thread + event queue
  - `widgets/` card, text, image, button, input, checkbox, select, progress, spinner, table, tree, bar
  - `frame.rs` one painted screen: cells + reading-order text runs + image placements (blit/clip/occlude)
  - `view.rs` per-terminal graphics state; `inline.rs` `show`; `headless.rs` snapshots
- `crates/termstage-cli` — `termstage` CLI (clap). Also used by Python's console script.
- `crates/termstage-py` — PyO3 bindings → `termstage._core`. Glue only.
- `python/termstage` — typed facade (`StageApp`, `StageCard`, …), handler dispatch, asyncio bridge. Glue only.

## Rules
1. **Rust-first.** New behaviour goes in `termstage-core`. Python files hold only typing, facades, decorators and asyncio glue.
2. **Unsafe only in `termstage_core::os::*`**, each block with a `SAFETY:` comment.
3. **Errors are three-part** (`Error::friendly(what, why, fix)`); the fix must be something a user can actually do.
4. **Every text test includes** Arabic with harakat, mixed Arabic–English, Japanese/Chinese and Devanagari.
5. **CLI ↔ library parity**: anything the Python API can do, the CLI can do, with the same output.
6. **No new dependency** without asking the project owner.
7. **Never probe a real terminal in tests**: set `TERMSTAGE_HEADLESS=1`, or drive a PTY and answer queries yourself (see `tests/python/test_pty.py`).
8. One milestone at a time; stop for manual verification between milestones.
9. **Document everything you build** in `docs/` in plain language a new Python learner can follow.
   Every ```python block in `README.md` and `docs/*.md` is executed by `tests/python/test_docs.py`.
10. Regenerate Unicode tables with `python scripts/gen_tables.py`; never edit `*_table.rs` / `diacritics.rs` by hand.

## Commands
```bash
source .venv/bin/activate                      # uv venv && uv pip install maturin pytest
maturin develop --release                      # build the extension into the venv
cargo test -p termstage-core -p termstage-cli  # Rust tests
python -m pytest                               # Python tests
cargo clippy --all-targets -- -D warnings && cargo fmt --all
cargo clippy --target x86_64-pc-windows-msvc -p termstage-core -p termstage-cli --all-targets -- -D warnings
TERMSTAGE_UPDATE_SNAPSHOTS=1 cargo test -p termstage-core headless   # refresh golden snapshots (review the diff!)
```

## Seeing what you built (headless)
```python
snap = MyApp().run_headless((100, 30), steps=[{"press": "tab"}, {"type": "hi"}, {"click": "#send"}])
print(snap.text)  # the screen as plain text
snap.query("#chat")  # widget JSON: id, type, bounds, text, dir, theme, focused…
```
`termstage inspect` prints capabilities as JSON; `termstage doctor` explains them.

## Environment variables
| Variable | Effect |
|---|---|
| `TERMSTAGE_HEADLESS=1` | Force headless mode |
| `TERMSTAGE_SIZE=120x40` | Headless grid size |
| `TERMSTAGE_NO_PROBE=1` | Do not send terminal queries |
| `TERMSTAGE_PROBE_TIMEOUT_MS` | Probe timeout (default 250) |
| `TERMSTAGE_TEXT_MODE=auto\|cells\|pixels` | How complex scripts are drawn |
| `TERMSTAGE_SYSTEM_FONTS=0` | Bundled fonts only (deterministic PNGs) |
| `TERMSTAGE_REDUCE_MOTION=1` | Disable animations |
| `TERMSTAGE_UPDATE_SNAPSHOTS=1` | Rewrite golden snapshots in tests |
