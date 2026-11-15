# Rust GUI framework for the native agent-directing app

Research as of 2026-11-14. Web evidence only: nothing was built or run.
Wherever the evidence is one person's test, a third-party summary, or
missing, the text says so. "Thin" means don't rely on it without the spike.

## 1. What "gpui-kit" is

"gpui-kit" is **Longbridge's GPUI Kit**: the project that used to be called
**gpui-component**, renamed and repackaged.

- The GitHub repo is now `longbridge/gpui-kit`. The `gpui-component` crate
  on crates.io points its repository field there:
  https://crates.io/api/v1/crates/gpui-component
- It is three layers. `gpui-base` holds unstyled behaviour and infrastructure.
  `gpui-component` is the styled component library (75+ components).
  `gpui-kit` is the single crate an app depends on: it pins a matching GPUI
  and re-exports GPUI, base, component and assets.
  https://raw.githubusercontent.com/longbridge/gpui-kit/main/README.md ,
  https://docs.rs/gpui-kit/latest/gpui_kit/
- **How it gets GPUI from crates.io:** Zed's own `gpui` crate stopped at
  0.2.2 (2025-10-22) and hasn't been published since
  (https://crates.io/api/v1/crates/gpui). gpui-kit 0.9.1 depends on
  `gpui-pre =0.3.12`, `gpui-pre-platform`, `gpui-pre-macros` and
  `gpui-pre-web` (https://crates.io/api/v1/crates/gpui-kit/0.9.1/dependencies).
  `gpui-pre` describes itself as a "snapshot of zed@d2a4fb6". One Longbridge
  maintainer, Jason Lee (huacnlee), publishes it about once a week
  (https://crates.io/api/v1/crates/gpui-pre). So gpui-kit is on crates.io
  only because Longbridge republishes Zed's GPUI under another name.
- **Optional `gpui-fast` core:** `longbridge/gpui-fast` is a "performance-focused
  fork of Zed's GPUI with incremental rendering and native window
  composition". It is experimental and at v0.1.x. On a Linux benchmark it
  reports about 85% less main-thread CPU per frame than upstream: scrolling
  a sidebar takes 5.95 ms (86% CPU) on upstream and 0.79 ms (12% CPU) on
  gpui-fast. It notes that "results depend on the workload" and gives no
  idle numbers. Its retained mode needs `cx.notify()` whenever render state
  outside tracked entities changes. gpui-kit switches to it with a feature
  flag. https://raw.githubusercontent.com/longbridge/gpui-fast/main/README.md ,
  https://github.com/longbridge/gpui-fast
- **Other things with similar names, which are not it:** iamnbutler's
  `gpuikit`, a small "upcoming" UI toolkit (https://tangled.org/nate.rip/gpuikit,
  https://docs.rs/crate/gpuikit/0.1.0), and `gpui-ui-kit`
  (https://docs.rs/crate/gpui-ui-kit/0.1.4). Both are small and
  single-maintainer.

### Health of gpui-kit

| Signal | Evidence |
|---|---|
| Cadence | 14 releases from 2026-09-03 to 2026-11-09, roughly weekly. Latest is 0.9.1 (2026-11-09). Every minor version can break: 0.6, 0.7, 0.8 and 0.9 came out within 10 weeks. https://crates.io/api/v1/crates/gpui-kit |
| Adoption | About 378k downloads of gpui-kit since September, 592k of gpui-component. 81 dependent crates (libraries.io). https://libraries.io/cargo/gpui-kit |
| Repo | 20.7k stars, 1.3k forks, 2,908 commits, 105 open issues, 18 open PRs. https://github.com/longbridge/gpui-kit |
| Backing | Longbridge, a brokerage. Its Longbridge Pro desktop trading app is built on it "from day one" (README). |
| Licence | Apache-2.0. The docs prose is also under CC BY 4.0. |
| Docs | https://gpui-kit.com has versioned docs, from main back to v0.6.0. The README lists Apache licences, a story gallery and agent "skills". |
| Linux | Supported. The open issues are mostly X11 polish: a hover that stays stuck on a maximized window's title bar (#3987), keyboard focus following the pointer in a webview (#3860), a dialog backdrop spilling into the client shadow (#3744), focus-ring rendering (#1851), and an inspector that can't inspect any element in the window (#3056). https://github.com/longbridge/gpui-kit/issues?q=is%3Aissue+is%3Aopen+linux |
| Accessibility | It has an `accesskit` module. Open issues: disabled controls are announced as enabled (#3660), and there is no aria-live region (#3659). |

### gpui-kit components against our screens

The module list is from https://docs.rs/gpui-component/latest/gpui_component/

| Need | gpui-kit | Note |
|---|---|---|
| ~5k-row sidebar tree | `tree`, `sidebar`, `VirtualList` (with variable row heights) | The virtual list renders only the visible range: https://docs.rs/crate/gpui-component/0.5.1/source/src/virtual_list.rs |
| Multi-repo diff, 20k lines | `diff`: a "readonly, virtualized patch display" with split and unified modes, `DiffDecoration` line backgrounds, and `DiffAnnotation` / `DiffGutterMarker` for inline comments. https://docs.rs/gpui-component/latest/gpui_component/diff/index.html | Soft wrap is not mentioned, and neither is behaviour across many files in one scroll. Spike both. |
| Editor / text input | `input`: a code editor built on a rope, tree-sitter and LSP, "up to 200K lines". https://github.com/longbridge/gpui-component | Issue #3702: highlighting is truncated past an injection budget in long documents. |
| Terminal grid | None. Draw it ourselves with GPUI's `canvas`/paint quads, as Zed draws alacritty_terminal in its terminal view. | This uses GPUI's primitives. gpui-kit adds nothing here. |
| Markdown | `text::markdown()`, `TextView`, plugins, HTML, selection. https://docs.rs/gpui-component/latest/gpui_component/text/index.html | Streaming and incremental parsing aren't documented. |
| Fuzzy palette | `command`, `combobox`, `searchable_list` | The fuzzy matcher's quality is unverified. |
| Split panes | `resizable`, `dock` (dock area, tab groups, nested splits) | |
| Drag and drop | GPUI's own `on_drag`/`on_drop` (Zed uses them). gpui-kit has no DnD module. | File drops from Finder or the Linux file manager are a platform feature. Spike it on Wayland. |
| Multiple windows | GPUI `open_window` (Zed is multi-window). | |
| Tests | `#[gpui::test]`, `TestAppContext`, `VisualTestContext` and a `test-support` feature with a fluent test-observation extension that reads accessibility properties. `HeadlessAtlas` and `NoopTextSystem` let tests paint without a GPU. https://docs.rs/gpui-kit/latest/gpui_kit/ | Pixel snapshots: see GPUI below. |

## 2. Plain GPUI (Zed)

- **Standalone use:** It works outside Zed: gpui-kit, Longbridge Pro and many
  forks use it. But Zed doesn't sell or support it as a product, and
  commenters describe a fragmented community and a rough developer
  experience (https://news.ycombinator.com/item?id=50390881).
- **crates.io:** `gpui` 0.2.2 is from 2025-10. Zed main has since split it into
  `gpui`, `gpui_platform`, `gpui_web` and `gpui_macros`, and none of those
  are published
  (https://baeseokjae.github.io/posts/native-app-for-coding-agents-rust-gpui/ ,
  a secondary source that matches the crates.io API). You either use git,
  or a republished snapshot (`gpui-pre`, `gpui-box`, `gpui-unofficial`, and
  others).
- **API churn:** High, and unversioned. One port from the 2025-10 snapshot
  needed four one-line edits, such as `Application::new()` becoming
  `gpui_platform::application()` (same blog). Smaller changes are the usual
  case, but nothing promises it.
- **Linux:** Supported on Wayland and X11. The Linux renderer moved from Blade
  to wgpu (zed PR #46758, merged 2026-02-13), which fixed Wayland freezes on
  NVIDIA (https://news.hada.io/topic?id=26669). IME fixes keep landing, for
  example duplicate IME input on Wayland, Jan 2026:
  https://git.secluded.site/zed/commit/d92f47746f3eb83e90de8319f0e945a7b2fb8ed0
- **Text:** It uses CoreText on macOS and cosmic-text on Linux. CJK and emoji
  work in Zed every day. **RTL and bidi: no evidence of support.** I found no
  source either way, so treat it as missing until the spike tests it.
- **Accessibility:** AccessKit work exists in upstream branches and in forks
  (https://docs.rs/crate/rgpui/0.2.2/source/src/_accessibility.rs). Shipping
  status in upstream is unclear. Thin.
- **Headless snapshots:** Zed added screenshot tests (PR #45259, merged
  2026-01-05) that render to an offscreen Metal texture. The PR is
  **macOS only** (https://github.com/zed-industries/zed/pull/45259). PR #51415
  (2026-03-12) added a "platform-neutral headless renderer interface" and
  `render_to_image`, but only implemented it for Metal
  (https://github.com/zed-industries/zed/pull/51415). **I found no
  implementation for Linux on wgpu.** A Linux CI snapshot would need us to
  implement `render_to_image` for wgpu on lavapipe, or run Xvfb or a
  headless Wayland compositor and capture. Logic and flow tests, with no
  pixels, work today on every platform with the deterministic test executor.
- **Performance evidence:**
  - Bake-off on an M4 Max, 100k rows, by one user, macOS only: first frame
    152 ms (fastest of five), peak RSS 125.5 MiB, but 4.06 s of user CPU
    against Slint's 0.94 s, and best p99 scroll. Idle CPU wasn't measured.
    https://github.com/zed-industries/zed/discussions/63832
  - A ported js-framework-benchmark ("vibe coded", in a VM, CPU only): GPUI
    was the slowest at row updates. Selecting a row took 10.7 ms against
    Iced's 0.37 ms. https://news.ycombinator.com/item?id=50390881
  - gpui-fast's Linux numbers above show that upstream re-renders the whole
    frame on every animation or scroll. That is the cause of the CPU cost,
    and gpui-fast is the fix in progress.
  - Idle CPU: GPUI redraws only when a view is notified or input arrives, so
    idle should be near zero, but **I found no published measurement**.
- **Fit with tca-rust:** Reasonable. GPUI owns state in `Entity<T>` and
  re-renders a view on `cx.notify()`. Wrap a `LocalStore` in one entity per
  feature scope (GPUI's main thread is single-threaded, which matches
  `LocalStore`). Subscribe to scoped changes and `notify` only the views
  that watch them. Re-render granularity is per view, not per signal.
  Friction: GPUI runs its own executors, not tokio. Effects either run on
  `Store::on` with a tokio runtime on a background thread and get bridged
  back with `cx.spawn`, or drive `LocalStore::run` from a GPUI foreground
  task. The spike must prove this bridge.

## 3. Comparison table

Legend: Y = yes, P = partial or fork-only, N = no, ? = no evidence found.

| | gpui-kit (on GPUI) | Iced | Slint | egui | Freya | Xilem/Masonry | Floem | Makepad |
|---|---|---|---|---|---|---|---|---|
| Last release | 0.9.1, 2026-11-09 [1] | 0.14.0, 2025-12-07 [2] | 1.18.1, 2026-09-21 [3] | 0.37.0, 2026-10-15 [4] | 0.5.2, 2026-11-11 [5] | 0.4.0, 2025-10-29 [6] | 0.2.0, 2024-11 [7] | widgets 1.0.0, 2025-05 [8] |
| Cadence | weekly, breaking | ~yearly, breaking | ~bimonthly, semver 1.x | ~monthly, breaking | fast, breaking | ~6 months | crates stalled | crates stalled |
| Backing | Longbridge (rides on Zed's GPUI) | hecrj; Kraken sponsor; System76 uses a fork (COSMIC) | SixtyFPS GmbH, a company | Rerun.io | one maintainer + sponsors | Linebender (community) | Lapce team | Makepad founders |
| Stars / open issues | 20.7k / 105 | 32k / 370 [9] | 24.6k / 592 [10] | ~28k / ? | 3.4k / 33 [11] | 5.6k / 106 [12] | 4.3k / 74 [13] | 7.3k / 38 [14] |
| Notable apps | Longbridge Pro; Zed uses GPUI | COSMIC desktop, Halloy | embedded HMIs; few desktop apps | Rerun viewer | few | none at scale | Lapce | Robrix (thin) |
| Licence | Apache-2.0 | MIT | GPLv3 / royalty-free with attribution / commercial [15] | MIT/Apache | MIT | Apache-2.0 | MIT | MIT/Apache |
| Wayland + X11 | Y (X11 polish bugs) | Y (feature flags) [2] | Y (winit) | Y (winit) | Y (winit), ? detail | Y (winit) | Y (winit) | Y (own backends) [14] |
| CJK / emoji | Y | Y (cosmic-text) | Y | P (needs fonts) | Y (Skia) | Y (Parley) | Y (cosmic-text) | ? |
| RTL / bidi | ? | Y via cosmic-text [16] | Y | N, as far as I know (thin) | Y (Skia shaping) | Y (Parley + ICU4X) [17] | Y (cosmic-text) | ? |
| IME | Y | Y since 0.14 [2] | Y | Y | ? | P | P | ? |
| Accessibility | P (AccessKit, gaps) | N upstream; forks only [18] | Y (AccessKit) | Y (AccessKit) | Y [11] | Y (AccessKit) [12] | ? | ? |
| Custom GPU draw (terminal) | Y canvas/paint (Zed terminal) | Y shader/canvas widgets; COSMIC Terminal uses alacritty_terminal | P (custom rendering is limited; mostly the DSL) | Y paint callbacks; egui_term exists | Y Skia canvas; a terminal component exists [11] | P | Y | Y (shaders are first-class) |
| Virtual lists | Y | P (lazy; table/grid in 0.14) | Y (ListView model) | Y (show_rows) | Y (VirtualScrollView) | P | Y [13] | Y (PortalList) |
| Big soft-wrapped text | Y (editor to 200k lines) | P | P | P | P | ? | Y (Lapce editor) | Y (thin) |
| Multi-window | Y | Y (since 0.12) | Y | Y (viewports) | ? | N/? | Y | Y |
| Headless pixel snapshots | Metal only; flow tests everywhere | **Y: iced_test Simulator + PNG snapshot** [19] | P: testing backend + element queries, preliminary [20] | **Y: egui_kittest + wgpu snapshots** [21] | Y: freya-testing renders to file [11] | Y: masonry_testing screenshot tests [22] | ? | ? |
| Fits a TCA store | good: entity per store scope, notify per view | **very good**: Elm update/view; full view rebuild per change | awkward: state lives in .slint properties and models; Rust pushes into them | good: render from &State every frame; no diffing to fight | OK: own signals core | OK: Xilem is Elm-like, but alpha | awkward: signals own state | awkward: own live DSL |
| Idle / memory evidence | 125 MiB RSS for 100k rows (macOS, one test) [23]; idle ? | reactive rendering default in 0.14 [24]; numbers thin | 117.6 MiB RSS, lowest CPU [23] | 121.7 MiB [23]; idle repaint only on input | ? | ? | ? | ? |

Sources: [1] https://crates.io/api/v1/crates/gpui-kit
[2] https://github.com/iced-rs/iced/releases
[3] https://crates.io/api/v1/crates/slint
[4] https://crates.io/api/v1/crates/egui
[5] https://crates.io/api/v1/crates/freya
[6] https://crates.io/api/v1/crates/xilem
[7] https://crates.io/api/v1/crates/floem
[8] https://crates.io/api/v1/crates/makepad-widgets
[9] https://github.com/iced-rs/iced
[10] https://github.com/slint-ui/slint
[11] https://github.com/marc2332/freya
[12] https://github.com/linebender/xilem
[13] https://github.com/lapce/floem
[14] https://github.com/makepad/makepad
[15] https://slint.dev/terms-and-conditions , https://unpkg.com/slint-ui@1.12.0/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md
[16] https://docs.rs/crate/cosmic-text/latest
[17] https://github.com/linebender/parley
[18] https://www.goodfirstissue.org/iced-rs/iced/issues/552 , https://docs.rs/crate/toddy-iced/0.6.2
[19] https://docs.rs/iced_test/0.14.0 , https://docs.iced.rs/iced_test/simulator/struct.Simulator.html
[20] https://docs.rs/i-slint-backend-testing
[21] https://docs.rs/egui_kittest
[22] https://docs.rs/crate/masonry_testing/0.4.0
[23] https://github.com/zed-industries/zed/discussions/63832
[24] https://book.iced.rs/faq.html

Notes on the table:

- **Freya is no longer Dioxus + Skia.** Since 0.4 it uses its own reactive core
  and still renders with Skia (https://github.com/marc2332/freya). Skia adds
  a large native dependency and binary size, and it has one main maintainer.
- **Iced:** the last stable release is 11 months old, and 0.15 is still in
  development (https://www.mintlify.com/iced-rs/iced/installation). The
  README still calls it "experimental". Upstream has no accessibility, an
  open request since 2020.
- **Floem and Makepad:** their crates haven't been published in 12 to 24
  months, so they would have to be used from git. Not contenders here.
- **Xilem:** says "experimental" in its own README. It has good bones:
  Parley text, AccessKit and a snapshot harness. Too early.
- **Others considered:** Dioxus Native (Blitz) is still alpha, at 0.8.0-alpha,
  with incomplete CSS (https://github.com/DioxusLabs/dioxus/releases). Not a
  contender yet. libcosmic is an Iced fork with System76's widgets, so it
  belongs under Iced. Qt through cxx-qt is serious but heavy: Qt 6 had
  214 MiB RSS in the bake-off [23], and it is C++, not "easy to extend in
  Rust".
- **Throughput:** egui (immediate mode) was cheapest on CPU in the ported
  js-framework-benchmark, and Iced was fastest at fine-grained row updates
  (https://news.ycombinator.com/item?id=50390881). That run is CPU-only,
  not peer reviewed, and in a VM.

## 4. Ranked shortlist

### 1. gpui-kit (GPUI, with gpui-fast as an option)

It is the only option that already has most of our hard screens as
components: a virtualized diff with line decorations and annotations, a
200k-line code editor, a tree, a dock with splits, markdown, a command
surface and multiple windows. GPUI itself powers a shipping, multi-window
editor with an alacritty-backed terminal on macOS and Linux. It is the
cheapest path to a working app, and it already has the most momentum.

### 2. Iced (0.14, tracking 0.15)

It fits tca-rust best: Elm update/view is TCA without effects-as-values,
and `Task` maps onto `Effect`. It has the best Linux headless pixel testing
we found (iced_test Simulator, PNG snapshots, a tiny-skia CPU backend). A
real desktop runs on it (COSMIC, with COSMIC Terminal on alacritty_terminal).
Weak points: no upstream accessibility, yearly breaking releases, few
ready-made widgets for a diff or a dock (we'd build them), and it rebuilds
the whole view tree on each change (mitigated with `lazy`).

### 3. egui (fallback, or for prototypes and dev tools)

It is the most mature snapshot tooling (egui_kittest, which queries through
AccessKit), it releases monthly, it's backed by a company (Rerun), and it
uses little CPU and memory. Immediate mode makes "state in, actions out"
trivial. Against it: the look and feel isn't native; text layout is weak on
RTL and complex scripts (thin evidence); and polished large-text and diff
views are DIY.

Slint is fourth: the lowest memory and CPU in the bake-off, a company behind
it, and semver 1.x. But its royalty-free licence requires attribution, its
DSL-owned state fits an external store poorly, and custom GPU drawing for
the terminal is its weakest area.

## 5. The spike (the same for each finalist)

Build the three screens on tca-rust with the same reducers. Only the view
layer differs:

1. **Sidebar tree**: 5,000 rows from an `IdentifiedVec`, expand and
   collapse, selection, and 50 row updates a second from a fake effect.
2. **Multi-repo diff**: 3 repos and 20k lines, soft wrap, red and green line
   backgrounds, two inline comment widgets, and scrolling top to bottom.
3. **Terminal grid**: alacritty_terminal (or libghostty-vt) fed by `yes`
   and `cat bigfile`. A custom-drawn 200×60 cell grid at 120 Hz.

Measure on an M-series Mac and on Linux (Wayland on GNOME, and X11):

- idle CPU over 60 s with the window open and unfocused, and with it focused
  (target ≤0.1% averaged);
- RSS after start, and after the diff has been fully scrolled;
- cold start to first frame, and to interactive;
- scroll frame times p50/p95/p99 and dropped frames, for the diff and the
  tree;
- terminal throughput (MB/s) at a steady 120 fps;
- a **headless snapshot on Linux CI** with no GPU (lavapipe or a CPU backend):
  render screen 2 to PNG and diff it against a baseline;
- a flow test: send actions to the store, simulate a click on a tree row,
  and assert both the state and the snapshot.

Per framework:

- **gpui-kit:** run once on upstream and once with `gpui-fast`. Wire
  `LocalStore` in an entity, bridge tokio effects with `cx.spawn`. Use the
  `Diff` component as is, then check soft wrap and multi-file scrolling.
  Prove the headless pixel path on Linux. Without one, either implement
  `render_to_image` for gpui_wgpu, or fall back to Xvfb/headless Weston
  capture. Budget this explicitly. Also test: Persian or Arabic text in
  markdown and in the diff (RTL), fcitx5 or IBus on Wayland, and a file
  dropped from Nautilus.
- **Iced:** wire the store in through a `Subscription` that emits state
  versions. `update` forwards to `store.send`. `view` reads a snapshot.
  Build the diff with `lazy` rows inside a virtualized scrollable, and see
  how much code it takes. The terminal is a `shader` widget. Snapshots come
  from iced_test with the tiny-skia backend.
- **egui:** render straight from `&State` each frame and collect actions
  into a Vec that is sent after the frame. Use `show_rows` for the tree and
  the diff, galley caching for wrapped lines, and a paint callback for the
  terminal. Snapshots come from egui_kittest with the wgpu snapshot feature
  on lavapipe.

The spike takes about 1–1.5 weeks per finalist for one engineer. Do
gpui-kit first, and Iced only if gpui-kit misses a gate.

## 6. Recommendation

**Yes to gpui-kit, with conditions.** Build on it provisionally, and drop it
if the spike misses any of these gates:

1. idle CPU is near zero on Linux Wayland and on macOS;
2. the 20k-line soft-wrapped diff scrolls at p99 ≤ 16 ms on Linux, on
   upstream or on gpui-fast;
3. we can produce a headless pixel snapshot on Linux CI within about 3 days
   of extra work;
4. RTL text renders correctly. If it doesn't, decide explicitly whether that
   is acceptable for now.

The lean is supported on components, momentum and the real app behind it.
It is not supported on two points the brief cares about: Linux headless
snapshots, and evidence on CPU and idle.

### What would make gpui-kit fail (watch list)

1. **Supply chain:** our GPUI would come from `gpui-pre`, which one Longbridge
   maintainer republishes from a Zed commit. If Longbridge stops, we're on
   Zed git with no versions. Zed has no stated commitment to GPUI as a
   product.
2. **Churn:** gpui-kit ships a breaking 0.x every one to three weeks, and the
   GPUI snapshot moves underneath it. Plan for a pinned version and a
   monthly bump.
3. **Linux pixel tests don't exist upstream.** Visual tests are Metal-only,
   and the headless renderer trait has only a Metal implementation. This
   was the "matters a lot" requirement.
4. **CPU per frame:** upstream GPUI re-renders the whole frame on animation
   and scroll (gpui-fast measured 5.9 ms and 86% CPU on Linux for a spinner)
   and was slowest in the row-update benchmark. gpui-fast fixes it, but it
   is experimental, at v0.1, and changes when you must call `notify`.
5. **Linux polish:** open X11 bugs in hover and focus, the inspector broken on
   Linux, and IME fixes still landing in 2026.
6. **Text:** no evidence of RTL or bidi support. Accessibility has gaps
   (disabled state, live regions).
7. **The executor isn't tokio.** If bridging tca effects proves awkward or
   costly, that friction is with us for good.
8. **Bundle size:** a 70 MB+ app bundle was reported for gpui-shell (#3906).
   Check it for our binary.

### Risks and how to reverse

- **Keep the reducers framework-free**, which tca-rust already enforces. All
  behaviour lives in reducers and TestStore tests, and views are thin
  adapters. Reversing then means rewriting the views, not the app.
- **Put the terminal grid behind our own trait** (cells in, quads out) so it
  can move to an Iced `shader` or an egui paint callback.
- **Keep the store bridge in one crate** (`ui-bridge-gpui`). Moving to Iced is
  a new bridge plus new views.
- **Lean on gpui-kit's components as little as practical in the first
  milestone.** The `Diff` component is the biggest lock-in and the biggest
  win; isolate it behind our own diff view model.
- **Reverse if:** gpui-pre goes 6 weeks without a release, or a gate above
  fails, or Linux snapshot testing is still manual after the first
  milestone. Iced is the reversal target.
