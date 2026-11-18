# Humata 2 — isolated UI prototype

This package is **Humata 2 prototype UI work only**, with simulated
data and no connection to Humata daemons, agents, SSH, Git, or real files.
It lives in the main Cargo workspace and has a Bazel desktop target. The
original HTML prototype and GUI performance spike are untouched.

GPUI Fast is selected by both launch scripts, using optimized release builds.
One Rust view tree runs on macOS and in the browser. The browser's HTML file
only loads WebAssembly; it does not reimplement the interface.

## Run locally

```sh
apps/humata2/ui-prototype/run.sh
```

## Browser status

The previously compiled browser bundle is available at <http://127.0.0.1:8740>.
The current movable-pane iteration is native-first: the repository does not
have a Bazel Wasm target yet, so that bundle has not been rebuilt for the pane
changes. `web.sh` is the legacy Cargo build path and is not part of the current
Bazel-only workflow.

## Headless control and snapshots

```sh
apps/humata2/ui-prototype/capture.sh 'channel:0;session:1;panel:Diff;expand;restore'
```

This drives the **real Rust controls**, off screen, without computer-use
automation. Every step produces a PNG, registered control roles/labels and measured
control bounds under `target/ui-captures/manual/`. Pass a second argument to choose
another output directory. The named steps are `channel:<index>`,
`session:<index>`, `panel:Diff|Files|Document|Board|Inbox|Journal|Info`, `expand`,
`restore`, `click:<element-id>`, `type:<text>` and `key:<keystroke>`.
Indices are zero-based. `drag:G-1->blocked` drags an item to a column; status
keys are `backlog`, `todo`, `in-progress`, `blocked`, `in-review`, `done`, and
`wont-do`. It uses the application's own fonts and icon assets.

The expand/restore regression opens each pane menu,
resizes the terminal, repeats expansion/restoration three times for Diff,
Board and Inbox, and compares the **drawn pane widths** within one pixel.
Run it with:

```sh
.tools/bin/bazel test -c opt //apps/humata2/ui-prototype:ui_test \
  --test_filter=expand_restore_preserves_user_split_sizes --test_output=errors
```

Bazel tests save captures in their undeclared test outputs. `capture.sh` writes
to the chosen prototype capture directory. PNG capture needs access to the Mac's Metal device. These headless tests
use the selected GPUI backend’s renderer and test APIs; they open no desktop window.

## Browser diagnostics without computer use

The optional `browser_capture.py` launches an isolated headless Chrome,
connects through CDP, captures JavaScript diagnostics and saves a screenshot.
Its profile and Python environment stay under the ignored prototype tools
and target directories. It never attaches to your browsing profile.

```sh
.tools/browser-qa/bin/python browser_capture.py --software \
  --url 'http://127.0.0.1:8740/?scene=content' \
  --output target/ui-captures/browser-mermaid.png
```

Run this from the prototype folder. It requires `websocket-client==1.9.0`
in `.tools/browser-qa`. Scene URLs support `content`, `work`, `scenarios`,
`offline`, `exited` and `closed`. Use **Scenarios** for all other entry states.

The Mermaid dependency is pinned to upstream PR #125’s immutable revision:
its published 0.3.1 layout profiler calls an unsupported browser clock.
The pin fixes that runtime panic without vendoring a library copy here.

## Prototype coverage

Open **Scenarios** in the title bar for connected workspace examples and
15 additional flow studies. [UX_FLOWS.md](UX_FLOWS.md) maps the existing
spec’s 23 user journeys to their prototype surfaces and distinguishes
connected interactions from staged designs.

The workspace includes:

- Session-bound movable panes: Agent, independent Shells, Docs, Diff, Files,
  Work, Inbox, Journal and Info. Drag pane titles or sidebar sessions onto an
  edge; use the pane menu to split right/below or maximize/restore. The right
  rail opens panes. Closing a pane keeps its terminal and unsent feedback;
  Reopen recovers it. Cmd+W closes a pane; Cmd+\ splits right.

- Work lists and boards with milestones, standalone tasks, child steps,
  Global/channel scope, blockers, details, Markdown comments and history.
- A real embedded Humata terminal renderer over a dummy grid, with input,
  copy/selection, paste, scrolling and per-session state.
- Multi-repository diff feedback with selectable added, deleted and mixed
  line ranges, routed to its owning agent terminal, including
  draft, queued, waiting, sent and explicit-send states.
- Closed-tab recovery, stop/resume, unavailable-resume fallback, archive,
  Trash, restore, manual reconnect and four nested fork levels.
- A generic Docs pane for specifications, documents and notes, with Preview,
  Edit and Changes views. Selected passages in preview or source accept
  inline feedback addressed to a chosen agent. Original context is retained
  when an edit makes a comment outdated.
- Syntax-highlighted fenced code and Rust-rendered Mermaid flowchart,
  sequence and state diagrams, with bundled fonts for browser and native.
- Scoped Inbox approvals, files, prepared views, resizable panels and search.

The wider studies cover channels/repos, transfers, providers, moving,
PR review, spec comments, prepared walkthroughs, lost machines, restarts,
storage, access, skills, palette and settings. They use staged hardcoded
screens rather than full specialized feature widgets.

State lives only in memory. Restart or reload to reset. No actual process,
file operation, approval, provider action or GitHub submission is performed.

## Design references

Humata 2's existing prototype supplies the channel/session model and
terminal-first workspace. The supplied GPUI trading-app reference informs
the restrained surfaces, separators and panel density. Codex informs the
quiet navigation. Controls, input, icons and resizing
are real GPUI Kit components.

GPUI Kit web integration follows the [official WebAssembly guide](https://gpui-kit.com/docs/webassembly/).
The guide describes web support as a showcase path; browser rendering and
input need verification. The resolved dependency graph requires
`wasm-bindgen` 0.2.127, so the local CLI matches that version.

Fonts are the compact Inter, IBM Plex Sans, and JetBrains Mono files from
GPUI Kit's `crates/story-web/fonts/`. They are bundled so the browser can
measure text without installed fonts; their upstream license files are
included under `assets/`.
