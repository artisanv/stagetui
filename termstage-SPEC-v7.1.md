# Project Specification: `termstage` v7.1 — Production Blueprint (Native 2D/3D Graphics Engine, Dynamic Multi-Pane Splitting, Full-Quality Audio, RTL/Global Scripts & Headless CLI)

> **Tagline:** A cross-platform, zero-copy Python graphics engine, native 2D/3D pixel canvas, prestigious TUI suite, and scriptable Headless CLI—powered by a Rust core—empowering Python developers and AI agents to generate rich 2D/3D visuals, split terminal panes dynamically, stream full-resolution media/web with lossless stereo audio, and render flawless RTL/global typography.

---

## 1. Vision & Philosophy: "Understand Once, Build Anything"

`termstage` is an open-source, PyPI-ready library and CLI tool designed to break the psychological barrier of the stark black terminal screen—giving end users a warm, intuitive, mouse-and-keyboard-friendly experience while giving Python learners, pros, CI/CD pipelines, and AI agent harnesses a rock-solid, predictable foundation.

### Core Design Pillars
1. **Python Generates Its Own 2D & 3D Graphics (`StageCanvas` & `Stage3D`):** Just as JavaScript uses HTML5 `<canvas>` and WebGL to create rich visuals without external assets, `termstage` gives Python a hardware-accelerated 2D vector/raster canvas (powered by Rust `tiny-skia`) and a GPU-accelerated 3D engine (powered by Rust `wgpu` with automatic software fallback)—enabling custom charts, particle systems, games, interactive widgets, and `.glb`/`.gltf` 3D scenes in 100% pure Python.
2. **100% Pure-Python Developer Experience (Zero HTML/CSS/JS Required):** Developers customize everything—colors, borders, spacing, layouts, custom widgets (`@ts.component`), and smooth UI animations (`animate=...`)—using clean, readable Python objects (`Style(...)`). *(Full web browsing/HTML rendering remains supported via `ts.browse()` / `Stage("https://...")`, never required for building UIs.)*
3. **Dynamic Multi-Pane Splitting & Sub-App Compositing (`StageSplit`):** Bind keyboard shortcuts (e.g., `Ctrl+\`, `Ctrl+-`) or call `.split()` programmatically to divide any terminal workspace on the fly—running a weather dashboard, a 60 FPS anime/video stream, a live interactive website, a 3D viewport, and nested `termstage` sub-applications simultaneously side by side with independent placement IDs (`i=`) and zero pixel collisions.
4. **Full-Quality Lossless Audio Pipeline (`48 kHz` Stereo):** Both video playback (`ts.play()`) and live web pages (`ts.browse()`) route full-fidelity stereo audio directly to native OS audio backends (`CoreAudio`, `WASAPI`, `PipeWire`/`ALSA` via Rust `cpal` / `sounddevice`) with per-pane volume, mute, and auto-focus audio ducking so split panes never clash.
5. **Understandable Grammar ("Learn Once, Guess Everywhere"):** Every function, widget, CLI subcommand, property, animation, and event obeys 5 universal rules so engineers and newcomers alike can guess commands by intuition rather than memorizing docs.
6. **First-Class Interactive + Headless CLI (`termstage` / `ts`):** Works both as an embeddable Python library AND a standalone CLI capable of running interactively in a terminal OR headlessly in CI/CD, Docker, and AI agent sandboxes (rendering TUI apps, 2D/3D scenes, web pages, videos, or Arabic/RTL cards straight to PNG/WebP/SVG snapshots without an X11/Wayland display or TTY).
7. **Flawless Global & RTL Typography Out of the Box:** Built for Python 3.15's UTF-8-by-default runtime (`PEP 686`) with bundled open-source **Noto Sans** (Latin, Arabic, CJK, Devanagari) fonts and Rust's `cosmic-text` + `unicode-bidi` shaping engine—guaranteeing connected Arabic cursive ligatures, proper diacritics (tashkeel), and automatic RTL/LTR layout mirroring on every machine with zero system font configuration.
8. **Harness & Agent Ready:** Includes structured event streams (`on_stream`, `on_tool_call`) and programmatic UI/visual state inspection (`capture_state()`, `capture_pil()`) so you can build full AI coding agent harnesses directly on top of `termstage`.

---

## 2. The "Learn Once, Guess Everywhere" Unified Grammar

The coding agent MUST enforce these **5 Universal Rules** across the entire public Python API and CLI:

### Rule 1: Action Verbs for Scripts & CLI, `Stage*` Nouns for Apps
* **Standalone Scripts (`import termstage as ts`) & CLI (`termstage <verb>`):** Plain-English verbs (`show`, `view`, `play`, `browse`, `draw`, `ask`, `notify`, `render`, `inspect`).
* **Full TUI Apps (`from termstage import ...`):** Nouns prefixed with `Stage` (`StageApp`, `Stage`, `StageCanvas`, `Stage3D`, `StageSplit`, `StageCard`, `StageButton`, `StageSidebar`, `StageTabs`, `StageModal`, `StageToast`, `StageGuide`, `StageBar`) + `@ts.component` for custom pure-Python widgets.

### Rule 2: Identical Positional Order Everywhere
`Thing(what_to_show, title=..., icon=..., dir=..., animate=..., style=...)`
* First argument is **always the primary content** (file path, URL, text label, question, 2D/3D scene callback, or child widgets).
* Everything else is an optional keyword argument with sensible defaults.

### Rule 3: Universal Properties Across ALL `Stage*` Widgets
Every widget accepts the exact same **8 universal properties** (plus `Style(...)` for deep customization):
* `theme`: `"aurora-dark"` | `"nordic-light"` | `"velvet-dusk"` | `"paper-soft"` | `"hacker-mint"`
* `accent`: Hex string (`"#7aa2f7"`) or friendly name (`"blue"`, `"mint"`, `"amber"`, `"rose"`, `"violet"`)
* `rounded`: `True` | `False`
* `padding`: `"compact"` | `"normal"` | `"spacious"` (or integer cell count)
* `icon`: Any built-in SVG icon name (`"play"`, `"globe"`, `"sparkles"`, `"book"`, `"check"`, `"cube"`, etc.) or `.svg` path
* `quality`: `"low"` | `"medium"` | `"high"` | `"native"` (or `0.1` to `1.0` float)
* `dir`: `"auto"` | `"ltr"` | `"rtl"` (auto-detects script direction and mirrors layouts, sidebars, and directional icons)
* `animate`: `"none"` | `"fade"` | `"slide-up"` | `"slide-left"` | `"pulse"` | `"pop"` (smooth 60 FPS UI entrance & state transitions)

### Rule 4: Identical Lifecycle Verbs on Every Widget
Every `Stage*` instance responds to the exact same methods:
* `.show()` / `.hide()` — visibility (respecting `animate=...`)
* `.load(new_content)` — hot-swap media, 3D model, URL, text, or nested sub-app without rebuilding the layout
* `.update(...)` — update title, icon, quality, `dir`, `animate`, or `style` live
* `.focus()` — move keyboard, visual focus ring, and active audio focus to this element
* `.split(new_widget, direction="horizontal" | "vertical")` — dynamically split any pane into multi-pane workspaces

### Rule 5: Predictable Event Hooks (`on_<verb>`)
* `on_click=...`, `on_change=...`, `on_submit=...`, `on_action=...`, `on_frame=...` (for 60 FPS `StageCanvas` / `Stage3D` render loops)

---

## 3. 100% Pure-Python Graphics, Splitting & Headless CLI Developer Experience

### A. Native 2D & 3D Graphics in Pure Python (`StageCanvas` & `Stage3D`)
```python
from termstage import StageApp, StageCanvas, Stage3D, StageSplit, StageCard, StageBar

class GraphicsStudio(StageApp):
    theme = "aurora-dark"
    accent = "violet"

    def draw_2d(self, ctx, t: float):
        # Hardware-fast 2D drawing via Rust tiny-skia at 60 FPS
        ctx.clear("#0f172a")
        ctx.gradient_circle(x=240 + 80 * ctx.sin(t), y=180, radius=90, colors=["#7aa2f7", "#bb9af7"])
        ctx.rounded_rect(x=60, y=60, width=200, height=50, radius=14, fill="#1e293b", stroke="#7dcfff")
        ctx.text("60 FPS Native Python Canvas", x=80, y=90, size=18, color="#ffffff")

    def compose(self):
        with StageSplit(direction="horizontal", shortcuts=True):
            yield StageCanvas(on_frame=self.draw_2d, title="2D Vector Engine", icon="sparkles")
            yield Stage3D("robot_model.glb", orbit_controls=True, lighting="studio", title="3D GPU Viewport", icon="cube")
        yield StageBar()
```

### B. Dynamic Multi-Pane Splitting (Weather + Anime Video + Live Website + Sub-App)
```python
from termstage import StageApp, Stage, StageCard, StageSplit, StageButton, StageBar

class MultiWorkspace(StageApp):
    BINDINGS = [
        ("ctrl+backslash", "split_right", "Split Right"),
        ("ctrl+minus", "split_down", "Split Down"),
    ]

    def compose(self):
        with StageSplit(direction="horizontal", id="root_split") as main:
            self.main_split = main
            with StageSplit(direction="vertical"):
                yield StageCard("Kuwait City • 26°C Clear", title="Weather", icon="sun", accent="amber")
                yield Stage(SubAgentApp(), title="Nested Sub-App", icon="terminal")
            with StageSplit(direction="vertical"):
                yield Stage("anime_episode_01.mp4", audio=True, quality="native", title="Anime Stream", icon="play")
                yield Stage("https://example.com", audio=True, title="Live Web", icon="globe")
        yield StageBar()
```

### C. Building Custom Pure-Python Widgets (`@ts.component`)
```python
import termstage as ts
from termstage import StageCard, Style

@ts.component
def MetricTile(label: str, value: str, icon: str = "sparkles", accent: str = "mint"):
    return StageCard(
        f"[bold]{value}[/bold]\n{label}",
        icon=icon,
        accent=accent,
        rounded=True,
        animate="pop",
        style=Style(padding="normal", shadow=True),
    )
```

### D. First-Class Interactive & Headless CLI (`termstage` / `ts` entrypoints)
Uses the exact same Action Verbs on the command line, automatically entering **Headless Mode** when `--headless`, `--out`, `!isatty()`, or `TERMSTAGE_HEADLESS=1` is active:
```bash
# Interactive terminal rendering (auto-detects image, SVG, Lottie, 3D GLB, video, URL, or RTL text)
termstage show hero.svg --quality native
termstage show model.glb --quality high
termstage play anime.mp4 --quality native --volume 80
termstage browse https://example.com

# Headless CLI rendering (ideal for CI/CD, Docker, automated visual regression & AI agents)
termstage render my_app.py --headless --size 1280x800 --theme nordic-light --out preview.png
termstage render "مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج" --dir rtl --headless --out arabic_card.png
termstage browse https://example.com --headless --wait 1.5 --out page.png --dump-state state.json
termstage inspect  # Prints JSON diagnostics: terminal protocol, GPU/wgpu status, audio device, fonts, extras
```

---

## 4. Production Guardrails & System Architecture

### 8 Mandatory Production Guardrails
1. **Cross-Platform Shared Memory (`Linux` + `macOS` + `Windows`):** Rust uses the cross-platform `shared_memory` crate (abstracting POSIX `shm_open` with macOS 31-character identifier limits and Windows `CreateFileMapping`) with multi-slot ring buffers so simultaneous split panes (`StageSplit`) stream at 60+ FPS without lock contention, falling back seamlessly to chunked SIMD Base64 (`t=d`) over SSH.
2. **Deterministic Cleanup & Zero-Zombie Process Reaper:**
   * Rust manages a background **Process & Memory Watchdog** (`job object` on Windows, `PR_SET_PDEATHSIG` / process groups on Unix) that automatically terminates any spawned Chromium or FFmpeg worker processes and unlinks all shared memory segments on normal exit, exception, `SIGINT`/`SIGTERM`, or parent crash.
3. **Zero-Config Bundled Multilingual Fonts & Perfect Arabic Shaping:**
   * Embed compressed subsets of open-source **Noto Sans**, **Noto Sans Arabic**, **Noto Sans CJK**, and **Noto Sans Devanagari** directly inside the Rust binary/wheel (`include_bytes!`).
   * Rust's `cosmic-text` + `unicode-bidi` shapes connected Arabic cursive letters, ligatures, harakat/diacritics, and mixed LTR/RTL paragraphs into high-DPI pixel layers—ensuring 100% identical, unbroken Arabic & global script rendering across every OS and terminal emulator, even inside minimal Docker containers with zero system fonts.
4. **GPU + Software-Fallback 2D/3D Pipeline (`tiny-skia` & `wgpu`):**
   * `StageCanvas` uses Rust's `tiny-skia` for anti-aliased 2D paths, gradients, bezier curves, and text at native terminal DPI.
   * `Stage3D` uses Rust's `wgpu` offscreen render targets (Vulkan/Metal/DX12) to render `.glb`/`.gltf` models, procedural meshes, PBR lighting, and custom shaders directly into shared memory—automatically falling back to software rasterization when running on headless CI servers without a hardware GPU.
5. **Full-Quality `48 kHz` Stereo Audio Mixer (`cpal` / `sounddevice`):**
   * Routes high-fidelity stereo audio from video streams (`ts.play()`) and headless Chromium web audio (`ts.browse()`) with per-pane volume, mute toggles, and optional focus-follows-audio mixing when multiple media panes are open in `StageSplit`.
6. **Headless Virtual Framebuffer & Scripted Automation Engine:**
   * When `--headless`, `TERMSTAGE_HEADLESS=1`, or `!isatty()` (e.g., `pytest`, GitHub Actions CI, Docker, or AI agent tool calls) is detected, `TerminalDetector` automatically activates Rust's in-memory **Virtual Offscreen Framebuffer** (`1280x800` default, configurable via `--size`).
   * Supports deterministic headless scripting (`--script actions.json` or Python `app.run_headless(steps=[...])`) to simulate clicks, split shortcuts, keystrokes, theme switches, and capture composited PNG/WebP screenshots + structured JSON widget trees (`state.json`).
7. **Smart SSH & Low-Bandwidth Auto-Throttling:**
   * Detects `SSH_TTY` / `SSH_CONNECTION` or slow terminal response latency and automatically caps target FPS (e.g., 24 FPS) and adjusts default `quality="medium"` with Zlib compression (`o=z`), keeping remote sessions snappy.
8. **Modular Optional Extras (`pip install termstage`):**
   * Core `termstage` installs in seconds (~8–12 MB prebuilt wheel with Rust core, CLI, `tiny-skia` 2D canvas, images, SVGs, Lottie animations, RTL text engine, and TUI components).
   * Heavy runtimes are clean optional extras with friendly install prompts if invoked without them:
     * `pip install "termstage[3d]"` (`wgpu` / `gltf` 3D engine assets & helpers)
     * `pip install "termstage[video]"` (`av`, `sounddevice` / `cpal` audio)
     * `pip install "termstage[web]"` (`playwright` with web audio capture)
     * `pip install "termstage[all]"`

### Repository & Open-Source Packaging Layout

```text
termstage/
├── LICENSE                      # MIT / Apache-2.0 dual open-source license
├── pyproject.toml               # Maturin + PyPI metadata, [project.scripts] (`termstage`, `ts`), optional extras
├── Cargo.toml                   # Rust workspace configuration
├── README.md                    # PyPI-ready README with visual showcase, 2D/3D, Python & CLI quickstarts
├── CHANGELOG.md                 # Semantic versioning release notes
├── .github/
│   └── workflows/
│       ├── ci.yml               # Linux/macOS/Windows Rust + Python 3.11–3.15 headless CLI & test matrix
│       └── release.yml          # Maturin cross-platform wheel builds -> PyPI trusted publishing
├── assets/
│   └── fonts/                   # Bundled Noto Sans (Latin, Arabic, CJK, Devanagari) font files
├── rust/
│   └── src/
│       ├── lib.rs               # PyO3 bindings (`termstage._core`)
│       ├── detector.rs          # Terminal pixel probing, TTY/Headless virtual buffer & SSH detection
│       ├── shm_ring.rs          # Cross-platform multi-pane shared memory buffers + Watchdog reaper
│       ├── kitty.rs             # Kitty Graphics Protocol encoder (SHM + compressed Base64 + multi-ID)
│       ├── fallback.rs          # SIMD 24-bit Unicode half-block & Sixel encoder
│       ├── raster.rs            # SIMD image resizer, quality scaler, `resvg` SVG & Lottie/APNG player
│       ├── canvas2d.rs          # `tiny-skia` pure-Python 60 FPS 2D vector/raster drawing engine
│       ├── render3d.rs          # `wgpu` offscreen 3D engine (.glb/.gltf, meshes, lighting, software fallback)
│       ├── audio.rs             # Cross-platform 48 kHz stereo audio mixer (`cpal`) with per-pane volume
│       └── bidi.rs              # `unicode-bidi` + `cosmic-text` RTL/Arabic cursive & global text shaper
├── python/
│   └── termstage/
│       ├── __init__.py          # Action Verbs (`show`, `view`, `play`, `browse`, `draw`, `ask`, `notify`), `Stage*`, `Style`, `@component`
│       ├── cli.py               # Unified interactive & headless CLI (`termstage` / `ts` entrypoints)
│       ├── headless.py          # Offscreen TUI + multi-pane pixel compositor, scripted input runner & snapshot exporter
│       ├── router.py            # Smart source (.png/.svg/.json/.glb/.mp4/URL/RTL) auto-detection
│       ├── errors.py            # 3-part plain-English `TermstageError` exceptions
│       ├── core.py              # Typed wrapper over `termstage._core`
│       ├── harness.py           # AI Agent Harness hooks (`on_stream`, `on_tool_call`, `capture_state()`)
│       ├── engines/
│       │   ├── browser.py       # Optional [web]: Async Playwright + CDP screencast, web audio & input bridge
│       │   └── video.py         # Optional [video]: PyAV decoder thread + 48 kHz A/V sync clock
│       └── ui/
│           ├── base.py          # Unified Grammar mixin (8 universal props, lifecycle verbs & events)
│           ├── style.py         # Pure-Python `Style(...)` class -> Textual & Rust style compiler
│           ├── motion.py        # 60 FPS UI transition & keyframe animation engine (`animate=`)
│           ├── component.py     # `@ts.component` decorator for custom pure-Python widgets
│           ├── app.py           # `StageApp` zero-config application shell (interactive + `run_headless()`)
│           ├── stage.py         # Universal `Stage` widget, `StageCanvas`, `Stage3D` & semantic aliases
│           ├── split.py         # `StageSplit` dynamic pane splitter, draggable dividers & nested sub-app host
│           ├── components.py    # `StageCard`, `StageButton`, `StageSidebar`, `StageTabs`, `StageModal`, `StageToast`, `StageGuide`, `StageBar`
│           ├── compositor.py    # Multi-pane layout bounds, resize & Z-index modal occlusion sync
│           ├── themes.py        # 5 curated themes + friendly color names
│           └── icons.py         # Built-in crisp SVG icon registry + RTL directional mirroring
├── tests/
│   ├── rust/                    # `cargo test` unit tests (SHM, Kitty, 2D/3D render, BiDi/Arabic shaping, headless buffer)
│   └── python/                  # `pytest` async, multi-pane split, CLI headless snapshot, RTL/Arabic layout, motion & Pilot tests
└── examples/
    ├── 01_hello_pixels.py       # Cross-platform Rust SHM 60 FPS gradient blitter test
    ├── 02_one_line_show.py      # Instinctive `ts.show(...)` across SVG, PNG, Lottie, 3D GLB & URL
    ├── 03_video_player.py       # Hardware-decoded video + quality toggle + 48 kHz audio sync
    ├── 04_mini_browser.py       # Interactive Chromium web browser + audio + two-way DOM events
    ├── 05_prestigious_ui.py     # Pure-Python `Style`, `@ts.component`, UI animations & themes
    ├── 06_rtl_arabic_global.py  # Connected Arabic cursive shaping, RTL mirrored layout & CJK/Hindi demo
    ├── 07_canvas_2d_3d.py       # Pure-Python 60 FPS 2D `StageCanvas` + interactive 3D `Stage3D` showcase
    ├── 08_multi_split.py        # Dynamic `StageSplit` workspace (Weather + Anime Video + Live Website + Sub-App)
    └── 09_agent_harness.py      # AI Coding Agent Harness starter template using `harness.py`
```

---

## 5. Coding Agent Execution Rules

1. **Strict Milestone Gatekeeping:** Build **one milestone at a time**. Do not start Milestone `N + 1` until:
   * All Rust tests (`cargo test`) and Python tests (`pytest`) for Milestone `N` pass with zero errors.
   * You output clear **Manual Verification Instructions** and pause for user confirmation.
2. **100% Pure-Python API Surface:** Never expose raw CSS strings, JavaScript snippets, or terminal escape sequences in public widget APIs. Compile `Style(...)` and `animate=...` cleanly behind the scenes.
3. **Zero-Zombie & Cross-Platform Guarantee:** Every shared memory allocation and background worker MUST be tracked by Rust's watchdog/RAII `Drop` guards and verified on `Linux`, `macOS`, and `Windows` (plus headless CI mode).
4. **Arabic & Global Script Perfection:** Every text rendering test suite MUST include connected Arabic words with diacritics (e.g., `"مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج"`), mixed Arabic-English BiDi sentences, Japanese/Chinese, and Devanagari strings to verify ligature shaping and RTL layout mirroring.
5. **CLI & Library Parity:** Every core verb (`show`, `play`, `browse`, `draw`, `render`, `inspect`) must work identically from Python and from the `termstage` CLI—including `--headless` image/JSON export.

---

## 6. Development Milestones & Verification Plan

### Milestone 1: Cross-Platform Rust Core, Multi-Pane SHM, Watchdog Reaper, Headless Virtual Buffer & CLI
**Goal:** Scaffold the `maturin` + `PyO3` project (with GitHub CI/PyPI release workflows and `termstage` CLI entrypoint), implement cross-platform multi-slot shared memory (`Linux`/`macOS`/`Windows`), headless virtual framebuffer mode, and 60+ FPS multi-ID Kitty blitting with SSH auto-throttling.

* **Features to Build:**
  * Initialize `Cargo.toml`, `pyproject.toml` (with `[project.scripts]` for `termstage`, optional extras `[3d]`, `[video]`, `[web]`, `[all]`), `LICENSE`, and `.github/workflows/ci.yml`.
  * Implement `TerminalDetector` in Rust (`TIOCGWINSZ` / Windows Console API + non-blocking Kitty probe + `$TMUX` passthrough + `SSH_TTY` latency throttling + `!isatty()` / `--headless` virtual framebuffer).
  * Implement `ShmRingBuffer` supporting simultaneous independent placement IDs (`i=`) for multi-pane workspaces + Rust RAII/Watchdog cleanup guarantees.
  * Implement `KittyBlitter` with z-index (`z=`), cell placement (`c=`, `r=`), compressed SIMD Base64 (`t=d,o=z`) over SSH, and `termstage inspect --json` CLI diagnostics.
* **Automated Tests:**
  * `cargo test`: Verify multi-ID Kitty escape formatting, SSH compression, headless virtual framebuffer export, and shared memory cleanup.
  * `pytest`: Verify zero-copy `memoryview` transfer (< 0.5 ms per 1080p frame) and `termstage inspect --json` CLI execution.
* **Hand-Testing Guide (Agent must prompt user to run):**
  * Run `maturin develop --release && termstage inspect && python examples/01_hello_pixels.py`.
  * **Expected Result:** `termstage inspect` prints clean terminal/framebuffer capabilities, and `01_hello_pixels.py` renders a silky-smooth 60 FPS animated gradient exiting cleanly on `q` or `Esc`.

---

### Milestone 2: Native 2D Canvas (`tiny-skia`), 3D Engine (`wgpu`), Media/Lottie, Bundled Noto Fonts & Arabic/RTL Shaper
**Goal:** Give Python its own native 2D (`StageCanvas`) and 3D (`Stage3D`) graphics engines in Rust alongside image/SVG/Lottie decoding, bundled Noto Sans multilingual fonts, `unicode-bidi` + `cosmic-text` Arabic/RTL shaping, and `ts.show()` / `ts.draw()` / `termstage render --headless`.

* **Features to Build:**
  * Implement `canvas2d.rs` (`tiny-skia`) exposing anti-aliased 2D shapes, gradients, bezier curves, charts, and particle primitives to Python at 60 FPS.
  * Implement `render3d.rs` (`wgpu` with automatic software fallback) supporting `.glb`/`.gltf` model loading, procedural 3D meshes, studio lighting, and orbit camera controls.
  * Bundle compressed **Noto Sans** (Latin, Arabic, CJK, Devanagari) in `assets/fonts/` via `include_bytes!` and build `bidi.rs` (`unicode-bidi` + `cosmic-text`) for flawless Arabic cursive ligatures and RTL/LTR layout mirroring.
  * Build `router.py`, `cli.py` (`show`, `draw`, `render`), and 3-part plain-English `errors.py`.
* **Automated Tests:**
  * `cargo test`: Verify `tiny-skia` 2D rendering, `wgpu` 3D offscreen frame output, SVG/Lottie rasterization, and Arabic cursive ligature shaping & BiDi ordering.
  * `pytest`: Test `ts.show()`, `StageCanvas`, `Stage3D`, and `termstage render --headless --out ...` across PNG, SVG, Lottie, 3D GLB, and Arabic/Japanese/Hindi strings.
* **Hand-Testing Guide (Agent must prompt user to run):**
  * Run `python examples/02_one_line_show.py`, `python examples/06_rtl_arabic_global.py`, `python examples/07_canvas_2d_3d.py`, and `termstage render "مَرْحَباً بِكُمْ" --dir rtl --headless --out temp_rtl.png`.
  * **Expected Result:** Smooth 60 FPS native 2D/3D Python graphics, razor-sharp connected Arabic cursive text, live `1`–`4` quality switching, and a valid `temp_rtl.png` exported headlessly.

---

### Milestone 3: Optional `[video]` Engine with `48 kHz` Stereo Audio Mixer & Process Watchdog
**Goal:** Stream local anime/video files and network streams via `ts.play()` and `termstage play` at native frame rates with full-quality `48 kHz` stereo audio (`cpal` / `sounddevice`), per-pane volume/mute controls, live `quality` toggling, and guaranteed worker cleanup.

* **Features to Build:**
  * Build `audio.rs` (`cpal`) + `VideoEngine` (`PyAV`) writing raw frames into Rust's `ShmRingBuffer` synced to a `48 kHz` audio master clock, with per-pane volume and focus-ducking support.
  * Support `termstage play anime.mp4` (interactive) and `termstage play anime.mp4 --headless --at 2.0 --out frame.png` (headless frame snapshot).
  * Raise friendly `TermstageError` with `pip install "termstage[video]"` instructions if optional video dependencies are missing.
* **Automated Tests:**
  * `pytest`: Generate a synthetic 3-second MP4 + stereo audio fixture via `PyAV`, verify playback throughput, seek accuracy, headless `--at` frame export, and zero zombie threads/processes on termination.
* **Hand-Testing Guide (Agent must prompt user to run):**
  * Run `termstage play sample.mp4` (or `python examples/03_video_player.py sample.mp4`).
  * **Expected Result:** Smooth 30/60 FPS video playback with crisp full-quality stereo audio and responsive keyboard/mouse controls (`Space`, `Left`/`Right`, `Up`/`Down`, `m`).

---

### Milestone 4: Optional `[web]` Headless Chromium Engine, Web Audio & Two-Way Interactive Bridge
**Goal:** Render fully loaded live websites via `ts.browse()` and `termstage browse` with sub-cell mouse/keyboard translation, full-quality web audio routing, two-way Python event callbacks, headless screenshot/DOM export, and Rust watchdog protection against orphaned Chromium processes.

* **Features to Build:**
  * Build `BrowserEngine` wrapping async `playwright` (Chromium) registered under Rust's **Process Watchdog** so Chromium is guaranteed to terminate even on abrupt parent exit.
  * Stream CDP `Page.startScreencast` frames directly into Rust's `ShmRingBuffer` at the pane's exact pixel dimensions and `quality` scale, with web audio routed cleanly to the OS audio mixer.
  * Translate terminal `(col, row)` mouse events and keyboard events via CDP, plus `on_action` event hooks and `termstage browse <url> --headless --out page.png`.
* **Automated Tests:**
  * `pytest`: Launch local test page, simulate translated mouse click and keyboard typing, test headless `--out` capture, and verify zero lingering Chromium processes after test teardown.
* **Hand-Testing Guide (Agent must prompt user to run):**
  * Run `termstage browse https://example.com` (or `python examples/04_mini_browser.py https://example.com`).
  * **Expected Result:** Full-resolution interactive website inside the terminal with smooth trackpad scrolling, hover states, clicking, typing, and audio.

---

### Milestone 5: Dynamic `StageSplit`, Pure-Python TUI Component Suite, Motion Engine, Headless Compositor, Harness Hooks & PyPI Release
**Goal:** Deliver dynamic multi-pane splitting (`StageSplit` with live shortcuts and nested sub-app embedding), the complete 100% pure-Python component suite (`StageApp`, `Stage`, `StageCanvas`, `Stage3D`, `StageCard`, `StageButton`, `StageSidebar`, `StageTabs`, `StageModal`, `StageToast`, `StageGuide`, `StageBar`), `Style(...)`, `@ts.component`, 60 FPS UI animations (`animate=`), `headless.py` multi-pane offscreen compositor, AI agent harness hooks (`harness.py`), and PyPI release automation.

* **Features to Build:**
  * Implement `ui/split.py` (`StageSplit` with keyboard split/unsplit shortcuts, draggable dividers, simultaneous multi-pane pixel IDs, and nested `StageApp` sub-application hosting).
  * Implement `ui/base.py`, `ui/style.py` (`Style(...)` pure-Python compiler), `ui/motion.py` (`animate=` transitions), and `ui/component.py` (`@ts.component`).
  * Build all `Stage*` widgets with automatic RTL layout mirroring (`StageSidebar` right-docking, directional SVG icon flipping) and `StageModal` Z-index pixel occlusion.
  * Build `headless.py` and `StageApp.run_headless(steps=[...])` so any multi-pane `StageApp` can run in CI/CD or CLI (`termstage render app.py --headless --out ui.png --dump-state state.json`) producing a pixel-accurate composite screenshot of both Textual widgets and all embedded 2D/3D/video/web pixel panes.
  * Build `themes.py` (5 curated themes + live hot-switching), `icons.py` (25+ bundled crisp SVG icons), and `harness.py` (`on_stream`, `on_tool_call`, `capture_state()` / `capture_pil()`).
  * Implement Rust `fallback.rs` (SIMD 24-bit Unicode half-blocks & Sixel) for legacy terminals and finalize `.github/workflows/release.yml` for multi-platform PyPI wheel publishing.
* **Automated Tests:**
  * `pytest`: Verify dynamic `StageSplit` pane creation/reflow without pixel overlap, nested sub-app mounting, Unified Grammar across all `Stage*` widgets, pure-Python `Style` & `@ts.component`, UI motion transitions, RTL mirroring, `StageModal` Z-index occlusion, `termstage render app.py --headless` composite image/JSON export, and `harness.py` state capture.
* **Hand-Testing Guide (Agent must prompt user to run):**
  * Run `python examples/05_prestigious_ui.py`, `python examples/08_multi_split.py`, `python examples/09_agent_harness.py`, and `termstage render examples/08_multi_split.py --headless --out multisplit_preview.png`.
  * **Expected Result:** A luxurious, 100% pure-Python GUI-grade TUI studio where pressing split shortcuts dynamically divides panes to show weather cards, 60 FPS anime video with stereo audio, live websites, 2D/3D graphics, and nested sub-apps side by side—with flawless Arabic/RTL support and pixel-accurate headless CLI exports.

---

## 7. Starter Prompt for Your Coding Agent

Save this file as **`SPEC.md`** in a new directory (`mkdir termstage && cd termstage`), open your coding agent inside a Kitty-compatible terminal (**Kitty**, **Ghostty**, or **WezTerm**), and send this prompt:

> *"Read `SPEC.md` carefully. We are building `termstage` v7.1 as a cross-platform, open-source, PyPI-ready library and Headless CLI with a Rust (`PyO3` + `maturin`) core and a 100% pure-Python frontend built around the 'Learn Once, Guess Everywhere' Unified Grammar, native 2D (`tiny-skia`) and 3D (`wgpu`) Python graphics engines, dynamic multi-pane splitting (`StageSplit`) with full-quality `48 kHz` stereo audio, smooth UI animations, zero-config Noto Arabic/global RTL typography, deterministic process/memory cleanup, headless offscreen rendering, and AI agent harness hooks. Start with Milestone 1 only: scaffold the Cargo, Python, CLI (`termstage`), and GitHub Actions project structure, implement the cross-platform Rust terminal detector (with headless virtual framebuffer fallback), multi-slot shared-memory ring buffer with watchdog cleanup, and multi-ID Kitty blitter, run both `cargo test` and `pytest`, and then stop and give me the exact CLI command to test Milestone 1 by hand."*
