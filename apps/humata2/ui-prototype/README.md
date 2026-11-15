# Humata 2 — isolated UI prototype

This package is **Humata 2 prototype UI work only**, with simulated
data and no connection to Humata daemons, agents, SSH, Git, or real files.
It lives in the main Cargo workspace and has a Bazel desktop target. The
original HTML prototype and GUI performance spike are untouched.

One Rust view tree runs on macOS and in the browser. The browser's HTML file
only loads WebAssembly; it does not reimplement the interface.

## Run locally

```sh
apps/humata2/ui-prototype/run.sh
```

## Build for the browser

```sh
apps/humata2/ui-prototype/web.sh
```

Open <http://127.0.0.1:8740>. After editing Rust, run `web.sh build` and
reload. Stop the local server with Ctrl+C. The web dependency `wasm_thread`
requires nightly; native builds use the workspace's Rust 1.100 toolchain.
The nightly wasm target and matching wasm-bindgen 0.2.127 CLI must be
installed; the CLI is kept locally in `.tools/bin/`.

## Headless control and snapshots

```sh
apps/humata2/ui-prototype/capture.sh 'channel:0;session:1;panel:Diff;expand;restore'
```

This drives the **real Rust controls**, off screen, without computer-use
automation. Every step produces a PNG, a semantic view tree and a measured
layout under `target/ui-captures/manual/`. Pass a second argument to choose
another output directory. The named steps are `channel:<index>`,
`session:<index>`, `panel:Diff|Files|Board|Inbox|Journal|Info`, `expand`,
`restore`, `click:<element-id>`, `type:<text>` and `key:<keystroke>`.
Indices are zero-based. `drag:G-1->blocked` drags an item to a column; status
keys are `backlog`, `todo`, `in-progress`, `blocked`, `in-review`, `done`, and
`wont-do`. It uses the application's own fonts and icon assets.

The expand/restore regression actually clicks each panel's controls,
resizes the terminal, repeats expansion/restoration three times for Diff,
Board and Inbox, and compares the **drawn pane widths** within one pixel.
Run it with:

```sh
cargo +1.100.0 test -p humata2-ui-prototype --lib expand_restore_preserves_user_split_sizes -- --nocapture
```

PNG capture needs access to the Mac's Metal device. These headless tests
reuse the repository's `humata2-snapshot` tooling for semantic and layout
snapshots; they open no desktop window.

## First pass

- Channel/session tree and pinned session, with expandable channels.
- The sidebar is the only session switcher; no duplicate top session tabs.
- New-session and code/context fork dialogs.
- Resizable sidebar and terminal/review split; pane visibility controls.
- Agent and shell tabs, simulated messages, multi-repository diff and Wrap.
- Draft review comments sent to the simulated terminal.
- Files inside the session pane; no misleading global Files entry.
- Scoped Inbox requests with Allow once / Deny, and scoped boards.
- Draggable board cards, including Blocked and Won’t do columns. Blocking
  opens a prerequisite picker with optional external context; cycles and
  self-dependencies are refused.
- Click a card to edit its title and description, add comments, inspect
  blockers and read its ordered history of edits, comments and status moves.
- Disconnected machines wait for an explicit Reconnect action, in either
  the session or the Machines page; no automatic reconnect is shown.
- Home, machine overview, and a minimal appearance page.

This is an iteration surface, not full behavioural parity yet. PR reviews,
the two-pane cross-machine file manager, board creation/deletion, access-rule
editing and persistence still need porting from the original prototype.
State lives only in memory. Restart or reload to reset. Mock terminal text
includes example test output; those are illustrative data, not results of
tests run by this prototype. This is a desktop-sized design study.

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
