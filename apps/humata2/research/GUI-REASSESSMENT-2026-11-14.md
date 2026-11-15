# Humata UI framework reassessment

Research date: 2026-11-14, America/Los_Angeles.

This is a source and documentation investigation by three specialist agents plus a synthesis review. No comparative applications were built, dependencies installed, or benchmarks run for this investigation. Existing repository code and published cached dependencies were inspected read-only. This report does not change D8 or select a new production framework.

## Recommendation and confidence

**Keep GPUI Kit as the macOS desktop baseline. Treat GPUI Fast as a performance candidate that must pass additional correctness tests. Put egui alongside Kit in the final shortlist when production browser support and testing effort receive equal weight. Keep Iced as a serious alternative, especially for its state architecture.**

This is an engineering judgment about likely total implementation and maintenance cost, not a measured performance ranking. No source establishes that current Fast, Iced or egui wins on Humata's complete workload. The existing prototype's application bugs do not establish a framework defect.

For desktop-first priorities: Kit ordinary/Fast, then egui, then Iced is the proposed validation order. For equally important production browser/native clients and automation: validate egui first, Kit second, Iced third. This is a priority-dependent order for gathering evidence, not a claim that the first entry has already passed the requirements.

The earlier comparison was too confident about the overall winner. Kit's specialized workspace components are a strong desktop advantage, but production web support is a significant unresolved condition. egui has more usable workspace extensions than the earlier answer credited; Iced already has panes and substantive Markdown support.

The lowest-maintenance engine is not necessarily the lowest-maintenance application. We need to count framework upgrades, custom widgets, text editing, input integration, browser differences and test infrastructure together.

## Requirements actually found in the repository

- `DECISIONS.md` D8 selects gpui-kit and says only macOS image snapshots are required; Linux pixel snapshots are not a blocking requirement.
- `OVERVIEW.md` I2 asks for low idle CPU, memory/startup measurements, a 5k-row session tree, a 20k-line wrapped diff, custom terminal drawing, text/IME/accessibility, and Mac/Linux behavior.
- D34 and the current overview require a meaningful web client, not just a component preview: UI/features/terminal view build for Wasm, multiple clients can connect to the same daemon, and platform services sit behind clients.
- `LAYOUT.md` keeps features and reducers framework-free. `terminal/grid` is already independent of the renderer/VT engine.
- `testing/snapshot` already distinguishes state, semantic view, layout and pixels. `testing/flow` has a driver boundary for user actions. Existing GPUI investment reduces switching cost only partly: another framework needs new views, custom renderer integration and a driver.

These documents contain some historical contradictions. D8 removes the Linux image gate while old research retains it; D34 increases the importance of web beyond the older desktop-led comparison. The recommendation above uses macOS pixels plus meaningful production web support as the current interpretation.

## Version and evidence discipline

| Stack | Investigated baseline | Important qualification |
|---|---|---|
| GPUI Kit | 0.9.1 | Published cached source identifies commit `39ab849a93b5f15507b969164d49891db8d5b470` |
| Ordinary Kit engine | gpui-pre/platform 0.3.12 | Republished matched GPUI snapshot; not interchangeable with arbitrary Zed main |
| GPUI Fast | 0.1.14 | Experimental; distinguish published behavior from newer main documentation |
| Iced | 0.14.0 | Moving docs.iced.rs often describe 0.15.0-dev; released source was checked separately |
| egui/eframe | 0.37.0 | Extensions need their own version compatibility checks |
| Slint | 1.18.1 | Stable public 1.x does not make internal testing/custom GPU interfaces stable |

Evidence falls into four categories: source-confirmed capability, maintainer-reported measurement, previously demonstrated local behavior, and inference. A CI workflow's presence is not an independently observed passing run. A Wasm compile check is not a browser runtime test. A renderer trait is not a complete integration test.

## Comparative assessment

| Option | Why it fits | What we would maintain | Testing position | Main uncertainty |
|---|---|---|---|---|
| Plain GPUI | Low-level custom rendering and editor-oriented architecture | More of the workspace controls ourselves, plus upstream changes | Test executor, event simulation, offscreen rendering | Little reason to discard Kit's components while retaining GPUI's integration cost |
| Kit + ordinary GPUI | Virtualized diff, editor, lists/tree infrastructure, docking, Markdown | Version coordination, app adapters, terminal renderer, browser/platform integration | Credible semantic/layout/input tests and offscreen pixels | Browser maturity; actual streaming/scroll CPU |
| Kit + Fast | Same component surface; retained subtrees and scroll layers can reduce repeated work | Ordinary Kit obligations plus experimental engine/invalidation diagnosis | Regular flow suite plus separate natural-frame parity tests | Correctness/performance under our actual update graph |
| Iced | Elm-style state/messages/update, async integration, PaneGrid, Markdown | Specialized diff/tree virtualization, terminal/custom widgets, richer inspection | First-party Simulator/Emulator, screenshots and experimental recorded flows | Custom GPU snapshot fidelity, upstream accessibility, browser/native edge cases |
| egui/eframe | Direct state-driven UI, first-class web path, strong inspection/test ecosystem | Advanced editor/diff interactions and compatible extension stack | Particularly convenient semantic and pixel regression tools | Full bidi/RTL; total custom workspace effort |
| Slint | Stable public API, declarative tooling, headless options | DSL/model bridge and custom terminal/editor integrations | Preliminary testing APIs and multiple renderer paths | Stability of precisely the test/custom-GPU surfaces we need |

These are comparative judgments, not numerical scores. None of these custom-drawn stacks becomes an AppKit-widget application merely because it compiles natively. Native appearance, correct input and native runtime performance are separate properties.

## GPUI, Kit and Fast: detailed findings

### Component savings are real

Kit's virtualized Diff has split/unified modes, annotations, gutter markers and wrapping controls. Editor, docks, list/tree infrastructure, Markdown and command surfaces cover a substantial part of Humata. Plain GPUI removes these savings without removing upstream churn. The custom terminal remains our responsibility, behind `Grid`. [Diff API](https://gpui-kit.com/component/diff/), [GPUI architecture and stability warning](https://raw.githubusercontent.com/zed-industries/zed/main/crates/gpui/README.md).

### Fast performance claims need a matched comparison

Maintainer benchmarks show substantial reductions in active CPU for unchanged surrounding content. But the headline gallery comparator is an older ordinary GPUI snapshot, not Humata's matched current baseline. The comparison is not a benchmark against Iced or egui. Treat its result as a reason to investigate Fast, not as our measured speedup. [Benchmark setup and caveats](https://github.com/longbridge/gpui-fast#performance-vs-gpui).

Fast can reuse unchanged views/layout, so partitioning the terminal, sidebar and diff into independent views matters. A giant root invalidated for every stream event loses much of that benefit. Parsing Markdown, rendering Mermaid and preparing diffs inside frame rendering is expensive regardless of toolkit. Cache these separately.

### Critical testing issue: ordinary helpers bypass the retained path

Published Kit 0.9.1 `TestWindowExt::render_frame` calls `refresh()` before drawing. Its click/type/pointer helpers repeatedly use this path. Published Fast 0.1.14 rejects retained reuse while the window is refreshing. Therefore the regular interaction tests can pass while a missing invalidation leaves stale content in normal retained execution. They remain useful tests of input, callbacks and layout; they are insufficient evidence for retention correctness. [Pinned Kit helper](https://github.com/longbridge/gpui-kit/blob/39ab849a93b5f15507b969164d49891db8d5b470/crates/kit/src/test.rs#L348-L351), [retention behavior](https://github.com/longbridge/gpui-fast/blob/main/docs/retained-mode.md).

Add a separate matched-state test using public direct event dispatch and `Window::draw` without `refresh`, retention on/off, and real image capture. Assert `window.layout_stats().views_reused > 0`; layout-node reuse alone does not prove retained-view replay. Otherwise a parity test can pass while testing two full-redraw paths. Shipped test-support APIs include retention/scroll-layer switches and reuse statistics. Test scroll layers separately and settle after changing the switch, which itself refreshes the window. Upstream has an internal incremental-versus-fresh oracle, although some of its helpers are private. [Verification design](https://github.com/longbridge/gpui-fast/blob/main/docs/retained-mode.md#how-it-is-verified).

### Linux pixels and accessibility: correct stale statements

Both cached platform 0.3.12 and Fast platform 0.1.14 select a WGPU headless renderer on Linux and Metal on macOS. The claim that we must implement Linux pixel rendering ourselves is outdated. Our own snapshot wrapper currently limits images to Mac by policy/implementation; that is separate from upstream capability. Windows headless renderer parity is not established. [Current rendering guide](https://gpui-kit.com/docs/test/#verify-rendering-independently).

Fast 0.1.14 already records/replays accessibility data during retained rendering. Active accessibility does not universally turn retention off; an initial unrecorded accessibility frame requires a fresh accessibility-tree build, not an application call to `Window::refresh`. This does not prove full VoiceOver correctness or complete Kit widget semantics. Test real assistive technology and ordinary apps that activate accessibility. [Retained accessibility](https://github.com/longbridge/gpui-fast/blob/main/docs/retained-mode.md#accessibility).

### Dependency and support burden

Kit's coordinated feature switches the engine/platform/components together, which is substantially easier than maintaining an application fork. But Fast and ordinary GPUI types must not be mixed, and selecting Fast does not prevent upstream packages from compiling. Pin the whole resolved graph, not just an approximate frontend version. Fast's issue tracker is currently disabled according to its contribution policy, so diagnosis/contribution may require more effort. [Backend integration](https://gpui-kit.com/docs/getting-started/#use-gpui-fast), [contribution policy](https://github.com/longbridge/gpui-fast/blob/main/CONTRIBUTING.md).

### Browser is a release gate

Kit explicitly labels its web support a showcase, not a validated full-application distribution path. Our desired shared UI compiles to a browser canvas; that does not automatically provide DOM locators, native browser text interaction, accessibility, reliable clipboard/IME or good startup size. Nightly/Wasm toolchain matching is another maintenance surface. [Web guide](https://gpui-kit.com/docs/webassembly/).

## Iced: detailed findings

### More capable than the earlier comparison implied

Released PaneGrid already supports nested splits, resizing, drag/drop and programmatic management. Markdown has incremental parsing, tables, tasks, links/images and a customizable code-block renderer. Mermaid can integrate at that boundary; it is not itself built in. These would not all need implementing from zero. [PaneGrid source](https://raw.githubusercontent.com/iced-rs/iced/0.14.0/widget/src/pane_grid.rs), [Markdown source](https://raw.githubusercontent.com/iced-rs/iced/0.14.0/widget/src/markdown.rs).

Its state/message/update/view design fits tca-rust naturally. Reactive rendering and event-loop waiting support low idle CPU, but do not constitute a Humata measurement. Custom widgets still must request redraws properly. `Lazy` caches a dependency-based subtree; it is not virtualization. The released Table constructs/traverses cells, so its existence is not proof of a 20k-line variable-height diff solution. [Reactive rendering contract](https://github.com/iced-rs/iced/pull/2662), [Lazy](https://raw.githubusercontent.com/iced-rs/iced/0.14.0/widget/src/lazy.rs), [Table](https://raw.githubusercontent.com/iced-rs/iced/0.14.0/widget/src/table.rs).

### First-party tests have distinct fidelity levels

Simulator drives widgets and returns messages. Emulator runs a Program with tasks/subscriptions and can reach controlled daemon clients. Neither is an arbitrary installed-app OS controller. The released Emulator also has TODOs around OS/runtime services and limited window handling. Treat it as a useful in-process harness, not a complete reproduction of native runtime behavior. [Stable API](https://docs.rs/iced_test/0.14.0/iced_test/), [Emulator implementation](https://docs.rs/iced_test/0.14.0/src/iced_test/emulator.rs.html).

The `.ice` flow syntax is explicitly experimental. Its named-key enum and assertion/target vocabulary are much narrower than Rust raw-event tests. It should not become our sole persistent flow format. Selectors can use IDs/text/points/custom functions, but do not establish an AccessKit role/name/value tree. [Flow format](https://raw.githubusercontent.com/iced-rs/iced/0.14.0/test/src/ice.rs), [instructions](https://docs.rs/iced_test/0.14.0/src/iced_test/instruction.rs.html), [selectors](https://raw.githubusercontent.com/iced-rs/iced/0.14.0/selector/src/lib.rs).

### Critical renderer trap

The software fallback deliberately skips custom shader primitives. A CPU snapshot can therefore cover the surrounding UI while omitting a shader-based terminal. If we use custom WGPU rendering, our pixel gate must also use WGPU. A canvas/ordinary text implementation has different tradeoffs. [Exact fallback branch](https://raw.githubusercontent.com/iced-rs/iced/0.14.0/renderer/src/fallback.rs).

The built-in snapshot path uses exact pixels and accepts a missing baseline by writing one. We should retain our reviewed-record/verify policy and failure artifacts. Stable upstream CI commonly uses tiny-skia; that is not proof of custom GPU correctness. [Snapshot implementation](https://raw.githubusercontent.com/iced-rs/iced/0.14.0/test/src/simulator.rs), [test workflow](https://raw.githubusercontent.com/iced-rs/iced/0.14.0/.github/workflows/test.yml).

### Platform and maintenance qualification

Iced's modern web path is a canvas Wasm build, not the archived `iced_web` DOM backend. Compile CI exists, but browser input/rendering requires runtime checks. Upstream accessibility remains an open issue. COSMIC Terminal is evidence for an Iced-family terminal architecture, but its libcosmic fork and Alacritty integration are not a turnkey upstream widget or proof of upstream accessibility. [Wasm CI](https://raw.githubusercontent.com/iced-rs/iced/0.14.0/.github/workflows/check.yml), [accessibility issue](https://github.com/iced-rs/iced/issues/552), [COSMIC dependencies](https://raw.githubusercontent.com/pop-os/cosmic-term/master/Cargo.toml).

## egui/eframe: detailed findings

egui's immediate-mode UI does not mean continuous idle repainting. Repaint is driven by input/animation/requested updates. Virtualized lists and cached expensive content still matter under terminal streaming. No matched measurement here establishes whether its per-update costs are better than Fast. [Architecture and performance guidance](https://github.com/emilk/egui/tree/0.37.0).

`egui_kittest` combines AccessKit-derived semantic queries, interactions, assertions and WGPU image comparisons. The inspection ecosystem also offers live programmatic control. This is a substantial advantage for agent-driven reproduction and regression investigation. Native offscreen snapshots are still not browser screenshots or complete OS integration tests. Its WGPU harness explicitly excludes the browser backend because screenshot capture blocks. [Testing harness](https://github.com/emilk/egui/tree/0.37.0/crates/egui_kittest), [renderer implementation](https://raw.githubusercontent.com/emilk/egui/0.37.0/crates/egui_kittest/src/wgpu.rs), [inspection](https://github.com/emilk/egui/tree/0.37.0/crates/egui_inspection).

Do not assume egui means writing every workspace control ourselves: Rerun maintains `egui_tiles`, `egui_table` and inspection tools; other projects provide docks, trees and cached Markdown. These reduce the component gap, but each extension's version compatibility and maintenance belong in our dependency assessment. A terminal extension using Alacritty is not a replacement for our chosen Ghostty integration. [Rerun extensions](https://github.com/rerun-io/egui_crates).

Text claims also need precision. Current egui has OpenType shaping work and eframe native system-font support, so old blanket claims that it cannot shape or discover fonts are stale. Full bidirectional/RTL editing is a separate problem: the tracked RTL work was still open at this research date. This is a real gate for mixed-direction documents, selection and caret movement. [Pinned eframe defaults](https://raw.githubusercontent.com/emilk/egui/0.37.0/crates/eframe/Cargo.toml), [RTL work](https://github.com/emilk/egui/pull/8836).

The native-plus-browser story is more established than Kit's explicit showcase scope. Nevertheless the inspection feature is native-only and custom GPU/text/clipboard behavior needs real browser verification. A browser demo alone does not prove our production app.

The test harness runs until immediate repaint requests settle, with a step limit; continuously animated content needs explicit stepping. Some GPU content needs `render_every_step`, rather than relying on an end-state render. The live inspection path and windowless WGPU test renderer are separate: live screenshots require a rendered window, while the offscreen harness does not. These details affect how we reproduce a terminal update bug. [Pinned harness implementation](https://raw.githubusercontent.com/emilk/egui/0.37.0/crates/egui_kittest/src/lib.rs), [live inspection contract](https://raw.githubusercontent.com/emilk/egui/0.37.0/crates/egui_inspection/README.md).

egui remains pre-1.0. Its 0.37 upgrade includes public API changes, and docking/tree/Markdown extensions have to track compatible versions. A direct state-driven view removes some notification bookkeeping; it does not eliminate framework or plugin upgrade work. [Release history](https://github.com/emilk/egui/releases).

## Other credible alternatives

| Stack | What deserves credit | Why it is outside the immediate shortlist |
|---|---|---|
| Slint | Stable public application API, native/Wasm, preview/designer tools, headless interaction and rendering paths | Additional UI-language/model boundary; testing and custom WGPU integration have their own weaker stability contracts |
| Qt + CXX-Qt | Mature UI/test platform; real Rust integration and Wasm support | Adds C++/QML, Qt SDK and build-toolchain integration; bridge is still early-development API |
| Xilem/Masonry | Explicit headless event/accessibility/animation/snapshot harness, strong text foundations | Still experimental; DOM web views and native Masonry views are distinct implementations |
| Floem | Reactive, virtual-list and editor-oriented foundations | Production browser support and automation/upgrade contracts remain insufficiently established in this investigation |
| Freya | Actual windowless interaction/image testing and custom drawing | Additional rendering/reactive stack; production web and a performance advantage were not established |

Slint's public stability claim is meaningful. However its testing API explicitly warns that even patch releases may break it. Custom WGPU textures/underlays are supported through an explicitly unstable module; saying it cannot draw a terminal would be wrong. Its software renderer also has text/feature differences, so the production renderer matters to snapshots. [Public API policy](https://github.com/slint-ui/slint), [tagged testing contract](https://raw.githubusercontent.com/slint-ui/slint/v1.18.1/internal/backends/testing/README.md), [WGPU integration](https://docs.slint.dev/latest/docs/rust/slint/wgpu_30/), [renderer differences](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/).

CXX-Qt supports/tests Wasm; it should not be dismissed as having no web path. Qt Quick Test has key/mouse/drag and image operations. The drawback for us is the number of integration surfaces, not an inability to build the UI. [CXX-Qt](https://github.com/KDAB/cxx-qt), [Qt Quick Test](https://doc.qt.io/qt-6/qtquicktest-index.html), [test operations](https://doc.qt.io/qt-6/qml-qttest-testcase.html).

Masonry's test harness includes controlled animation, access-tree inspection and golden images. Xilem web uses DOM views rather than simply compiling Masonry widgets into the browser, so sharing business state does not mean sharing every view implementation. [Masonry harness](https://docs.rs/masonry_testing/0.4.0/masonry_testing/struct.TestHarness.html), [native Xilem](https://github.com/linebender/xilem/tree/main/xilem), [web Xilem](https://github.com/linebender/xilem/tree/main/xilem_web).

If a DOM/WebView architecture becomes acceptable, Tauri/Electron deserve a separate comparison rather than dismissal. They can reuse browser document/editor/terminal ecosystems and browser automation. Tauri officially supports a Leptos Rust/Wasm frontend, so it is not excluded by a Rust UI requirement. It introduces WebView/IPC integration rather than the native GPU UI architecture primarily evaluated here; browser editor/terminal integrations may additionally use JavaScript. Tauri's mock runtime does not run its actual webview, and its desktop driver story differs by platform. Playwright's Electron support is explicitly experimental and native dialogs require separate handling. Neither alternative guarantees a smaller runtime footprint or simpler whole application without measurement. [Tauri Rust frontend](https://v2.tauri.app/start/frontend/leptos/), [Tauri testing](https://v2.tauri.app/develop/tests/), [Playwright Electron automation](https://playwright.dev/docs/api/class-electron).

## Testing strategy independent of framework

| Layer | What it proves | What it does not prove |
|---|---|---|
| Reducer/state tests | Work hierarchy, queue ownership, recovery transitions, permissions and effects | Pixels, focus, actual terminal delivery |
| Semantic interaction flows | Actual UI event-to-state wiring, labels, enabled states, navigation and user actions | Correct colors/glyphs or full OS services |
| Layout snapshots | Restored pane widths, clipping, nested geometry, wrapping bounds | Correct image assets and real glyph rasterization |
| Production-renderer pixel tests | Icons, text visibility, colors, custom terminal/diagram drawing | Daemon ownership, persistence or OS IME correctness |
| Natural-frame cache tests | Incremental rendering agrees with full rendering and actually reuses work | Complete application behavior across services |
| Real-client integration/e2e | Feedback arrives at the right agent, streams recover and state survives restart | Every native window-system/browser interaction |
| Packaged/platform/browser smoke tests | Packaging, real text input, clipboard, focus, display scaling, window lifecycle | Exhaustive business-state coverage |

Keep the scenario fixtures and expected outcomes independent of the toolkit. A flow should have a stable semantic target and assert the user-visible result plus relevant domain state. Framework-specific drivers handle the input/render boundary. Custom canvas widgets need their own observable semantics and real pixel tests; a test harness cannot automatically understand terminal cells or GPU callbacks.

Use fixed bundled fonts, viewport, scale, theme, focus, controlled clocks/services and settled animations for images. Record and verify separately; review newly created baselines. Prefer semantic assertions for behavior and small targeted pixel snapshots where pixels add evidence. Cross-platform equality is not automatically expected when different text systems/rasterizers are involved.

## Decisive measurements still required

No large framework rewrite is justified by the research alone. Compare at most Kit ordinary/Fast and egui first; retain Iced if the comparison exposes a gap it plausibly solves. This is a proposed validation plan, not work executed in this investigation.

Four matched release workloads:

1. A 5k-row deep session tree: expand/collapse, selection and background status updates. Measure construction/layout work, memory and scroll p95/p99, not only painted rows.
2. A 20k-line, three-repo diff: wrap on/off, long and non-ASCII lines, variable-height inline comments, resize and search. Test streaming terminal output at the same time.
3. A 200-by-60 terminal grid: fixed seeded cell updates, selection, scrolling, cursor, paste and dirty-row caching. Separate renderer throughput from PTY/VT parsing and backpressure.
4. A rich specification: Markdown tables/code/tasks plus Mermaid, edit/preview, cached rendering and invalid diagram recovery. Measure cold parsing separately from normal scrolling.

For each: same machine, fonts, display scale, resolution, refresh cap, data/update rates and release profile. Alternate runs, retain raw results and report variability. Measure idle focused/unfocused, app CPU separately from GPU, startup, RSS, active CPU per frame, dropped frames and input latency. A 16.7 ms frame budget targets 60 Hz; 120 Hz has 8.3 ms, so report both the target and headroom.

Eight matched behavior flows:

- Add file/line feedback, switch sessions, and deliver only to the owning agent.
- Queue while busy, disconnected or exited; deliver after the correct explicit/turn boundary.
- Close/reopen terminal view without stopping the process.
- Stop/resume an agent; show a recoverable fallback when exact resume is unavailable.
- Archive/delete/restore while preserving nested forks and unpushed work.
- Drag work across statuses, select blockers and reject cycles.
- Expand/restore panes repeatedly and after window resize without changing saved sizes.
- Navigate deep forks and retain their independent panel/editor/draft state.

Run native flows on the selected engine, plus Fast natural-frame parity when applicable. Run real browser tests for keyboard/IME, clipboard/selection, DPR/resizing, background tabs, reconnect and graphics startup. Add platform IME/accessibility checks rather than treating simulated typing as equivalent.

## Decision boundaries

- **Promote Fast** only when it improves matched release measurements, passes natural-frame parity and does not require recurring application-owned engine patches.
- **Prefer egui** if browser/test tooling advantages outweigh the remaining editor/diff integration and text gaps after an equivalent slice.
- **Prefer Iced** if its state/runtime integration produces less total application complexity and our specialized views, GPU tests and accessibility requirements are satisfied.
- **Keep ordinary Kit** if it meets the real budgets and Fast's benefit is small; fewer engine-specific concerns can be worth more than a synthetic speedup.
- **Reconsider the whole shortlist** if production web or required multilingual/accessibility behavior fails. Do not silently lower those requirements to preserve an earlier framework choice.

The output of the next measurement should be a small decision record containing raw results, observed failures, custom code owned, dependency upgrade experience and the chosen tradeoff. Framework reputation, component counts and screenshot elegance are useful leads, not acceptance evidence.

## Remaining unknowns

- No matched Humata release measurements exist in this investigation for idle CPU, terminal/diff concurrency, input latency or memory across finalists.
- Production browser parity has not been demonstrated here for any complete Humata implementation.
- Required bidi/IME/accessibility behavior has not passed the same native/browser corpus across the shortlist.
- Extension-version compatibility must be resolved for an actual egui alternative; listing available crates is not proof that one compatible set meets the full spec.
- Fast's natural-frame reuse and application-level correctness need explicit proof beyond full-refresh semantic tests.
- Maintenance estimates have not been tested by a trial upgrade or equivalent vertical slice. Existing GPUI code is a switching cost, not evidence that continuing must be cheapest.

The independent reviews agree on these uncertainties. They support Kit's desktop fit and egui's stronger position as a web/testing challenger; they do not establish a universal winner or justify a migration merely to obtain a different-looking prototype.
