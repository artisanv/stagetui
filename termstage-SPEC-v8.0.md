# Project Specification: `termstage` v8.0 — Rust-First TUI Engine for Agent Harnesses & Humans

> **Tagline:** A cross-platform, Rust-powered terminal UI engine with a thin, typed Python API — built so coding agents can build, see, test and drive rich TUIs (agent harnesses first), and so humans get a terminal that feels like a modern app, in any language including Arabic/RTL.

---

## 0. What Changed from v7.1 (and Why)

| Area | v7.1 | v8.0 | Why |
|---|---|---|---|
| Architecture | Rust helpers + Python/Textual UI | **All engine logic in Rust**; Python is a thin PyO3 layer | Security, stability, speed; one engine usable from Rust and Python |
| TUI framework | Textual (implicit, undeclared) | Own Rust engine: `crossterm` + `taffy` + `ratatui-core` buffer | No Python UI runtime, no hidden dependency, no GIL stalls |
| Python runtime deps | Textual, Playwright, PyAV, sounddevice… | **Zero** (`dependencies = []`) | Installs in seconds, nothing to break |
| Web / video / 3D | Core features | **Optional plugins (Milestone 6)** behind a Rust `PaneSource` trait | Chromium is huge and adds attack surface; a harness doesn't need it in core |
| Focus | General media studio | **Agent-harness kit first** (chat, streaming Markdown, diff, prompt, tool cards, embedded terminal) | The library's reason to exist |
| Arabic/RTL | cosmic-text pixels only | **Dual path**: Rust-shaped presentation forms in cells + pixel runs where graphics exist | Works on every terminal, not just graphics ones |
| Shared memory | Reusable ring buffer | Per-frame named segments (terminal unlinks after read) | Matches the actual Kitty `t=s` contract |
| Sessions | Single process/terminal | **Session/View split** → pane detach/reattach to another terminal window | New feature (§7) |
| CLI alias | `ts` | **Removed** (`ts` collides with moreutils) | Avoid clobbering a common tool |

---

## 1. Vision & Philosophy

1. **Rust owns everything that matters.** Terminal I/O, capability detection, layout, widgets, text shaping, graphics, compositing, headless rendering, IPC, process cleanup. Python never contains engine logic — only ergonomics.
2. **Agents are first-class users.** Every app can be rendered headlessly to **text grid + PNG + JSON widget tree**, driven by scripted input, and inspected over a stable protocol (JSON-RPC and MCP). An agent can build a TUI, *see* it, and test it without a human.
3. **Humans should not fear the terminal.** Mouse-first, discoverable shortcuts, themes, smooth motion, friendly errors, crisp images, and flawless Arabic/RTL.
4. **"Learn Once, Guess Everywhere."** One grammar across Python, Rust and CLI (§2).
5. **Cross-platform by construction.** Linux, macOS, Windows are equal citizens; CI proves it on every commit.
6. **Small, secure core; heavy things are plugins.** Core wheel ≤ 10 MB, zero Python deps, no network access.

---

## 2. The "Learn Once, Guess Everywhere" Grammar

The coding agent MUST enforce these 5 rules across Python, Rust and the CLI.

### Rule 1 — Verbs for scripts & CLI, `Stage*` nouns for apps
* **Scripts (`import termstage as ts`) & CLI (`termstage <verb>`):** `show`, `draw`, `ask`, `notify`, `render`, `inspect`, `drive`, `attach`.
* **Apps:** `StageApp`, `Stage`, `StageSplit`, `StageCard`, `StageButton`, … + `@ts.component` for custom widgets.

### Rule 2 — Identical positional order
`Thing(primary_content, *, id=..., title=..., icon=..., style=..., on_<verb>=...)`
* First argument is always the primary content: path, text, question, callback, or **list of children**.
* Layout is plain nested expressions — no context-manager or generator magic (easier for agents to generate correctly).

### Rule 3 — Universal properties on every `Stage*` widget
`id`, `theme`, `accent`, `rounded`, `padding`, `icon`, `dir`, `animate` (+ `style=Style(...)`).
* `theme`: `"aurora-dark"` | `"nordic-light"` | `"velvet-dusk"` | `"paper-soft"` | `"hacker-mint"`
* `accent`: hex or friendly name (`"blue"`, `"mint"`, `"amber"`, `"rose"`, `"violet"`)
* `padding`: `"compact"` | `"normal"` | `"spacious"` | int cells
* `dir`: `"auto"` | `"ltr"` | `"rtl"` (auto-detects script and mirrors layout + directional icons)
* `animate`: `"none"` | `"fade"` | `"slide-up"` | `"slide-left"` | `"pulse"` | `"pop"`
* Media-specific props (`quality`, `volume`, …) live only on media widgets.

### Rule 4 — Identical lifecycle verbs on every widget
`.show()` / `.hide()` · `.load(content)` · `.update(**props)` · `.focus()` · `.split(widget, direction=...)` · **`.detach()` / `.attach()`** (§7)

### Rule 5 — Predictable events
`on_click`, `on_change`, `on_submit`, `on_action`, `on_frame`, `on_stream`, `on_tool_call`. Handlers may be `def` or `async def`.

---

## 3. Developer Experience

### A. One-liners
```python
import termstage as ts

ts.show("diagram.svg")                          # image/SVG inline, best protocol available
ts.show("مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج", dir="rtl")  # shaped, joined, right-to-left
choice = ts.ask("Deploy now?", choices=["Yes", "No"])
ts.notify("Build finished", icon="check")
```

### B. An agent harness in ~30 lines
```python
from termstage import (StageApp, StageSplit, StageChat, StagePrompt,
                       StageDiff, StageTerminal, StageBar)

class Harness(StageApp):
    theme = "aurora-dark"
    accent = "violet"
    BINDINGS = [
        ("ctrl+backslash", "split_right", "Split right"),
        ("ctrl+shift+d", "detach_focused", "Pop out pane"),
    ]

    def compose(self):
        return [
            StageSplit([
                StageChat([], id="chat", title="Agent", icon="sparkles"),
                StageSplit([
                    StageDiff(None, id="diff", title="Changes"),
                    StageTerminal("bash", id="shell", title="Shell"),
                ], direction="vertical"),
            ], direction="horizontal", ratio=[0.6, 0.4]),
            StagePrompt("", placeholder="Ask anything…  (/ for commands, @ for files)",
                        on_submit=self.ask),
            StageBar(),
        ]

    async def ask(self, text: str):
        msg = self.query("#chat").add("assistant")
        async for chunk in my_llm.stream(text):
            msg.append(chunk)                     # incremental streaming Markdown

Harness().run()
```

### C. Custom widgets
```python
import termstage as ts
from termstage import StageCard, Style

@ts.component
def MetricTile(label: str, value: str, icon: str = "sparkles", accent: str = "mint"):
    return StageCard(f"**{value}**\n{label}", icon=icon, accent=accent,
                     animate="pop", style=Style(padding="normal", shadow=True))
```

### D. CLI (implemented in Rust, `clap`)
```bash
termstage show hero.svg
termstage render app.py --size 120x40 --out ui.png --text ui.txt --tree ui.json
termstage render "مَرْحَباً بِكُمْ" --dir rtl --out card.png
termstage drive app.py            # JSON-RPC 2.0 over stdio: snapshot / press / type / click / wait_idle
termstage mcp app.py              # same surface as MCP tools for coding agents
termstage sessions                # list running sessions
termstage attach <session>[:pane] # attach a terminal to a session or a single pane
termstage inspect --json          # terminal caps, graphics protocol, fonts, platform
termstage doctor                  # human-friendly diagnosis + fixes
```
Headless mode activates on `--headless`, `--out/--text/--tree`, `TERMSTAGE_HEADLESS=1`, or when neither stdin nor stdout is a TTY.

---

## 4. Architecture

### 4.1 Layers
```text
Python (thin)      termstage/__init__.py  — typed facade, @component, asyncio bridge
                   ──────────── PyO3 (termstage._core) ────────────
Rust core          Session ── widget tree ── taffy layout ── paint
                      │                                       │
                      └── View(s) ── per-view encoder ── terminal | headless | IPC client
```
* **Session** owns app state, the retained widget tree, workers, timers, and child processes.
* **View** owns one output target: its terminal capabilities, size, input stream, and encoder. MVP has one in-process View; Milestone 5 adds IPC Views (§7). This split is mandatory from Milestone 1.

### 4.2 Threading model (no GIL stalls)
* Rust runs input, layout/render, and worker threads; `app.run()` releases the GIL.
* Python handlers run on one dispatcher thread that takes the GIL briefly per event. `async def` handlers run on an asyncio loop (`await app.run_async()` supported).
* Widget handles are thread-safe: every Python mutation is a message on a Rust channel.
* Render is **dirty-driven**: 0% CPU when idle; up to 60 FPS only while something animates/streams.
* Supports free-threaded CPython (3.14t / 3.15t).

### 4.3 Rendering pipeline
1. Layout (`taffy` flex/grid) → cell rects.
2. Paint into layers: **cell layer** (text, borders, colors) + **pixel layers** (images, canvas, complex-script text runs, icons).
3. Per-View encode:
   * Cells → `ratatui-core` buffer diff → minimal ANSI, wrapped in **synchronized output** (DEC mode 2026) to prevent tearing.
   * Pixels → best available protocol (§4.4).
   * Headless → raster compositor (cells drawn with bundled mono font + pixel layers) → PNG, plus text grid and JSON tree.

### 4.4 Graphics protocol matrix (detected by querying the terminal, not by env vars)
| Protocol | Terminals | Notes |
|---|---|---|
| Kitty + **Unicode placeholders** (`U=1`, U+10EEEE) | Kitty, Ghostty | Images live in cells → diffing, scrolling, clipping and tmux just work. Preferred. |
| Kitty direct placement | WezTerm, Konsole | Compositor handles clipping/occlusion. |
| Sixel | Windows Terminal ≥ 1.22, iTerm2, foot, xterm, WezTerm | Primary path on Windows. |
| iTerm2 OSC 1337 | iTerm2 | |
| Half-block / quadrant truecolor | Everything else (macOS Terminal.app, conhost) | Always available. |

* Transport for Kitty: local → per-frame POSIX shm (`t=s`, names `/tsg-<pid>-<n>` ≤ 31 chars for macOS; the terminal unlinks after reading) or temp file (`t=t`); remote/SSH → zlib-compressed direct (`t=d,o=z`).
* Cell pixel size via `CSI 16 t` (fallback `CSI 14 t` ÷ grid size; final fallback 10×20).
* SSH/slow link detected → cap FPS (24), lower default image quality, compress.

### 4.5 Text, fonts & Arabic/RTL (dual path)
* **Cell path (every terminal):** Rust shapes Arabic into **presentation forms** (U+FB50–FDFF, U+FE70–FEFF, incl. lam-alef ligatures), applies UAX #9 BiDi to produce visual order per line, then emits it. Before emitting, send **BDSM explicit mode (`CSI 8 l`)** so BiDi-aware terminals (VTE/GNOME Terminal, mlterm, mintty) don't reorder twice. Layout measures width **after** shaping (lam-alef = 1 cell for 2 letters). Proven approach: the author's working Omarchy setup (VTE presentation forms + DejaVu Sans Mono, which has all 140 forms at mono width). `termstage doctor` recommends such a font.
* **Pixel path (graphics terminals):** only complex-script runs (Arabic, Devanagari, …) are rasterized via `cosmic-text` into cell-aligned pixel runs; Latin stays in cells. Selection/copy for pixel runs is handled by the app (mouse select → OSC 52 clipboard).
* `text_mode="auto" | "cells" | "pixels"` per app/widget; `auto` = pixels when Kitty placeholders are available, else cells.
* **Bundled fonts** (`include_bytes!`; the wheel's zip compression already shrinks them, so no extra codec is needed): Noto Sans, Noto Sans Arabic, Noto Sans Devanagari, DejaVu Sans Mono (headless cell raster, includes Arabic presentation forms).
* **CJK:** system fonts first via `fontdb` (always present on macOS/Windows); optional `termstage-fonts-cjk` package (`pip install "termstage[cjk]"`). `termstage inspect` reports missing scripts.
* Python 3.15 UTF-8 mode (PEP 686) assumed; Windows console forced to UTF-8 code page on start and restored on exit.

### 4.6 Input
* `crossterm` events; **Kitty keyboard protocol** when available (unambiguous `ctrl+shift+…`), legacy fallback.
* SGR mouse (1006), pixel mouse (1016) where supported for canvases; hover, drag (split dividers), wheel.
* Bracketed paste; image paste into `StagePrompt` where the terminal allows.

### 4.7 Process & resource cleanup (zero zombies)
* A tiny Rust **reaper sidecar** (`termstage-reaper`, shipped in the wheel) is the parent of child processes (PTY shells, plugins). Python/session holds a pipe to it; on pipe EOF (normal exit, exception, `SIGKILL`, crash) it kills process groups and unlinks shm names.
* Per-OS hardening on top: Linux `PR_SET_PDEATHSIG` + pidfd; macOS `kqueue EVFILT_PROC/NOTE_EXIT`; Windows Job Object `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
* Terminal state (raw mode, alt screen, mouse modes, keyboard protocol, BDSM) restored by RAII guard + panic hook + signal handlers.

### 4.8 Security
* `#![forbid(unsafe_code)]` in all crates except listed FFI modules (`py`, `shm`, `os/*`), each `unsafe` block documented.
* Core makes **no network calls**. IPC sockets live in the user runtime dir with `0600` perms (Unix) / current-user ACL named pipes (Windows) + per-session random token.
* CI runs `cargo deny`, `cargo audit`, `clippy -D warnings`.
* Plugins run out-of-process where they spawn heavy runtimes (browser, decoders) → crash isolation.

---

## 5. Widget Suite

**Framework:** `StageApp`, `Stage` (universal: routes by content type), `StageSplit` (dynamic split/unsplit, draggable dividers, ratios), `StageTabs`, `StageModal` (z-order + pixel occlusion), `StageToast`, `StageSidebar` (auto right-dock in RTL), `StageBar` (status + shortcut hints), `StageGuide` (onboarding tour).

**Basics:** `StageCard`, `StageButton`, `StageInput`, `StageSelect`, `StageCheckbox`, `StageTable`, `StageTree`, `StageProgress`, `StageSpinner`, `StageImage`, `StageCanvas` (`tiny-skia`, `on_frame`).

**Harness kit (the priority):**
| Widget | What it does | Built on |
|---|---|---|
| `StageMarkdown` | Streaming Markdown; `append()` re-parses only the last block | `pulldown-cmark` |
| `StageCode` | Syntax highlighting, line numbers, folding | `syntect` (pure-Rust regex) |
| `StageDiff` | Unified/side-by-side diff, per-hunk accept/reject | `similar` |
| `StagePrompt` | Multiline editor, history, `/` commands, `@` file mentions, paste | own |
| `StageChat` | Message list (user/assistant/tool), auto-scroll, copy | `StageMarkdown` |
| `StageToolCall` | Collapsible card: name, args, status spinner, output | own |
| `StagePermission` | Approve / deny / always-allow prompt for tool calls | `StageModal` |
| `StageTerminal` | Real embedded shell/PTY pane | `portable-pty` (ConPTY on Windows) + `alacritty_terminal` |
| `StageLog` | High-throughput log tail with filter | own |

**Styling & motion:** `Style(...)` compiles to a Rust style struct (no CSS strings). 5 curated themes with live hot-switch. Built-in icon set (SVG → pixels when graphics exist, Unicode symbol fallback; no Nerd Font required). `animate=` runs cell/pixel transitions only while active; honors `reduce_motion`.

---

## 6. Agent Interface (how agents build, see and test)

* **Snapshots:** `app.capture_text()` (cell grid as plain text — best for LLMs), `capture_png()` (bytes, no PIL needed), `capture_tree()` (JSON: id, type, bounds, text, state, focus).
* **Scripted runs:** `app.run_headless(steps=[{"press": "ctrl+backslash"}, {"type": "hello"}, {"click": "#send"}, {"wait_idle": True}])` with a **virtual clock** so animations and timers are deterministic.
* **Query API:** `app.query("#id")`, `app.query_all("StageButton")`.
* **Driver protocol:** `termstage drive app.py` (JSON-RPC 2.0 over stdio) and `termstage mcp app.py` (MCP adapter via `rmcp`) — methods `snapshot`, `press`, `type`, `click`, `resize`, `wait_idle`, `query`.
* **Testing:** `termstage.testing` pytest plugin with golden snapshots (text exact-match, PNG perceptual diff).
* **Harness hooks:** `on_stream`, `on_tool_call`, `capture_state()`.
* **Docs for agents:** `AGENTS.md`, `llms.txt`, full `.pyi` stubs (generated by `pyo3-stub-gen`), every public symbol with a runnable example. Errors are 3-part: *what happened · why · exact fix*.

---

## 7. Sessions, Pane Detach & Reattach

**Goal:** any pane (video, agent log, shell, chat) can pop out into its own terminal window and snap back, without interrupting it.

* **How:** the Session keeps all state; a View is just a window onto it. `pane.detach()` (or a keybinding) launches a new terminal window running `termstage attach <session>:<pane>`; that View renders only the pane's subtree. The original slot shows a placeholder card *"Detached → Reattach"*. `pane.attach()`, the reattach button, or closing the popped-out window returns it. Media, streams and shells keep running throughout.
* **Whole-session detach:** close the terminal, keep the agent running; `termstage attach <session>` later (tmux-style). Ideal for long agent runs.
* **IPC:** `interprocess` local sockets (Unix domain sockets / Windows named pipes), `postcard`-framed messages. Each View renders with **its own** terminal's capabilities (a Kitty window and a Windows Terminal window can view the same session).
* **Terminal launcher** (auto-detected, overridable via `detach_launcher=`): Kitty (`kitty @ launch --type=os-window`, else `kitty -e`), WezTerm (`wezterm cli spawn --new-window`), Ghostty, Windows Terminal (`wt.exe -w new`), iTerm2/Terminal.app (AppleScript), GNOME Terminal, foot, Alacritty, `$TERMINAL -e`. Inside tmux → new tmux window. Over SSH → print the `termstage attach` command.

---

## 8. Plugins (later, optional)

Rust trait `PaneSource` (frames + optional audio + input handling + lifecycle), in-process or out-of-process.
* `termstage-video` — FFmpeg decode, `cpal` 48 kHz stereo, per-pane volume/mute/focus ducking, A/V sync.
* `termstage-web` — Rust CDP client (`chromiumoxide`) driving the user's installed Chrome/Chromium (no Node/Playwright), audio left native (unmuted) with per-pane gain via injected script; `blitz` (Rust HTML/CSS, no JS) for lightweight HTML/Markdown previews.
* `termstage-3d` — `wgpu` offscreen (Vulkan/Metal/DX12/WARP; Linux software path needs lavapipe/llvmpipe).

---

## 9. Packaging & Cross-Platform

* **Wheels:** maturin, `abi3-py311` (one wheel per OS/arch covers 3.11–3.15) + free-threaded wheels (cp314t, cp315t).
* **Targets:** manylinux_2_28 + musllinux (x86_64, aarch64), macOS (x86_64, arm64), Windows (x64, arm64).
* **CLI:** `[project.scripts] termstage = "termstage._core:cli_main"` (Rust `clap` behind a 1-line trampoline); standalone binary also via `cargo install termstage-cli`.
* **Size:** core wheel ≤ 10 MB.
* **CI matrix:** ubuntu / macos / windows × Python 3.11, 3.13, 3.15, 3.15t; `cargo test`, `pytest`, headless snapshot suite, clippy, rustfmt, ruff, pyright (strict on stubs), `cargo deny`.
* **Release:** PyPI trusted publishing + crates.io; first public alpha after Milestone 4.

### Repository layout
```text
termstage/
├── Cargo.toml                    # workspace
├── pyproject.toml                # maturin, python-source = "python", dependencies = []
├── LICENSE-MIT, LICENSE-APACHE
├── README.md, CHANGELOG.md, AGENTS.md, llms.txt
├── .github/workflows/{ci.yml, release.yml}
├── crates/
│   ├── termstage-core/           # pure Rust engine (no PyO3)
│   │   ├── assets/fonts/         # bundled fonts (inside the crate so crates.io/sdist builds include them)
│   │   └── src/
│   │       ├── caps.rs           # capability probing (graphics, cell px, kbd, sync, bidi, SSH)
│   │       ├── term.rs           # crossterm I/O + RAII terminal guard
│   │       ├── session.rs        # Session: state, tree, workers, timers
│   │       ├── view.rs           # View trait: terminal / headless / ipc
│   │       ├── tree.rs           # retained widget tree, ids, query
│   │       ├── layout.rs         # taffy integration
│   │       ├── style.rs, theme.rs, motion.rs, icons.rs
│   │       ├── text/             # shaping, presentation forms, bidi, fonts, measure
│   │       ├── gfx/              # kitty.rs, sixel.rs, iterm2.rs, blocks.rs, shm.rs, raster.rs, canvas.rs
│   │       ├── widgets/          # all Stage* widgets
│   │       ├── headless.rs       # raster compositor, text grid, JSON tree, virtual clock
│   │       ├── driver.rs         # JSON-RPC + MCP surface
│   │       ├── ipc.rs            # local sockets, framing, auth token
│   │       ├── launcher.rs       # per-terminal "open new window" strategies
│   │       └── reaper.rs         # sidecar protocol + per-OS hardening
│   ├── termstage-py/             # PyO3 bindings → termstage._core
│   ├── termstage-cli/            # clap CLI (also reaper sidecar entry)
│   └── termstage-plugin-api/     # PaneSource trait (Milestone 6)
├── python/termstage/
│   ├── __init__.py, __init__.pyi, _core.pyi, py.typed
│   ├── component.py              # @ts.component
│   ├── aio.py                    # asyncio bridge
│   └── testing.py                # pytest plugin
├── tests/{rust/, python/, snapshots/}
└── examples/
    ├── 01_hello.py               # styled card, quit on q/Esc
    ├── 02_show.py                # ts.show across PNG/SVG/text
    ├── 03_arabic_rtl.py          # cell + pixel Arabic, mixed BiDi, mirrored layout
    ├── 04_canvas.py              # StageCanvas animation
    ├── 05_ui_gallery.py          # themes, Style, @component, motion
    ├── 06_splits.py              # dynamic StageSplit
    ├── 07_harness.py             # full agent harness template
    ├── 08_headless_agent.py      # render/drive/snapshot loop an agent would use
    └── 09_detach.py              # pop a pane out and back
```

---

## 10. Coding Agent Execution Rules

1. **Milestone gatekeeping:** one milestone at a time. Before moving on: `cargo test`, `pytest`, clippy, and the headless snapshot suite pass on Linux, macOS and Windows CI; then output **Manual Verification Instructions** and pause for user confirmation.
2. **Rust-first:** engine logic goes in `termstage-core`. Python files contain only facade, typing, decorators and asyncio glue.
3. **Pure-Python surface:** no CSS strings, JS, or escape sequences in public APIs.
4. **No new dependency** (Rust or Python) outside this spec without asking the user.
5. **Arabic & global text in every text test suite:** `"مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج"`, mixed Arabic-English BiDi, lam-alef ligatures, Japanese/Chinese, Devanagari — in both cell and pixel paths.
6. **CLI ↔ library parity:** every verb behaves identically from Python, Rust and CLI, including headless export.
7. **Zero zombies:** every child process and shm segment is tracked by RAII + reaper; tests assert none remain.
8. **Performance budgets (tested):** idle CPU < 1%; compose + diff of a 200×60 grid < 4 ms; input-to-paint < 16 ms; streaming Markdown append O(chunk), < 1 ms per typical chunk.
9. **Determinism:** headless runs with the virtual clock produce byte-identical text snapshots.

---

## 11. Milestones

### Milestone 1 — Workspace, Terminal Core, Session/View, Headless Text
* Scaffold workspace, `pyproject.toml`, licenses, CI + release workflows; check `termstage` name availability on PyPI and crates.io.
* `caps.rs` capability probing (graphics protocol queries, cell px size, Kitty keyboard, sync output, BiDi, SSH, TTY/headless).
* `term.rs` RAII guard + panic/signal restore; `session.rs` / `view.rs` split; cell buffer diff renderer with synchronized output.
* Headless View: text grid + JSON tree. Reaper sidecar skeleton. PyO3 skeleton + generated stubs. CLI: `inspect`, `doctor`.
* **Tests:** caps parsing from recorded terminal replies, diff correctness, guard restore, headless text determinism, reaper kills children on parent `SIGKILL`.
* **Hand test:** `maturin develop --release && termstage inspect && python examples/01_hello.py` → clean caps report; styled card; `q`/`Esc` exits and the terminal is fully restored.

### Milestone 2 — Text, Arabic/RTL, Fonts, Images & Graphics Protocols
* `text/`: shaping, presentation forms, UAX #9 BiDi, BDSM handling, post-shape measurement, cosmic-text pixel runs, bundled fonts, system-font fallback.
* `gfx/`: Kitty (placeholders, shm/temp/direct), Sixel, iTerm2, half-blocks; `image` + `resvg` decode; `tiny-skia` `StageCanvas`; headless PNG raster.
* Verbs: `ts.show`, `ts.draw`, `termstage show`, `termstage render`.
* **Tests:** golden shaping/order for the Arabic set, encoder byte-exact outputs, PNG snapshot diffs, CJK fallback reporting.
* **Hand test:** `python examples/03_arabic_rtl.py` in Kitty/Ghostty **and** GNOME Terminal; `termstage render "مَرْحَباً بِكُمْ" --dir rtl --out rtl.png`.

### Milestone 3 — Layout, Widget Framework & Python API
* `taffy` layout, retained tree, query API, focus/keyboard/mouse, `StageApp`, `StageSplit` (dynamic split/unsplit/drag), basics widgets, `Style`, themes, icons, motion, `@ts.component`, asyncio bridge, RTL mirroring, modal occlusion.
* **Tests:** split reflow without overlap, grammar conformance across all widgets, motion under virtual clock, RTL mirroring snapshots.
* **Hand test:** `python examples/05_ui_gallery.py`, `python examples/06_splits.py`.

### Milestone 4 — Harness Kit & Agent Interface → first alpha release
* `StageMarkdown`, `StageCode`, `StageDiff`, `StagePrompt`, `StageChat`, `StageToolCall`, `StagePermission`, `StageTerminal`, `StageLog`.
* `run_headless(steps)`, `capture_*`, `termstage drive`, `termstage mcp`, pytest plugin, `AGENTS.md`, `llms.txt`.
* **Tests:** streaming throughput, PTY round-trip on all OSes (ConPTY on Windows), driver protocol conformance, MCP tool listing/calls.
* **Hand test:** `python examples/07_harness.py`; then have a coding agent run `termstage mcp examples/07_harness.py` and operate it.

### Milestone 5 — Sessions, Detach & Reattach
* IPC Views, auth token, `termstage sessions`, `termstage attach`, pane detach/reattach, whole-session detach, launcher matrix, per-View capabilities.
* **Tests:** detach/reattach keeps PTY and stream state, two Views with different protocols on one session, socket permission checks, no orphan processes after window close.
* **Hand test:** `python examples/09_detach.py` → `ctrl+shift+d` pops the pane into a new window; closing it snaps the pane back.

### Milestone 6 — Plugins & 1.0
* `PaneSource` API; `termstage-video`, `termstage-web`, `termstage-3d` as separate optional packages; 1.0 release.

---

## 12. Starter Prompt for the Coding Agent

> *"Read `termstage-SPEC-v8.0.md` carefully. We are building `termstage` v8.0: a Rust-first TUI engine (`termstage-core`) with a thin typed Python API (PyO3 + maturin) and a Rust CLI, designed for coding agents to build and test agent harnesses and for humans to build friendly terminal apps — cross-platform (Linux, macOS, Windows), with flawless Arabic/RTL. Start with Milestone 1 only: scaffold the workspace, CI, capability probing, RAII terminal guard, Session/View split, cell diff renderer with synchronized output, headless text/JSON View, reaper sidecar skeleton, PyO3 skeleton with stubs, and `termstage inspect` / `doctor`. Run `cargo test` and `pytest`, then stop and give me the exact commands to verify Milestone 1 by hand."*
