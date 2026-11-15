# Humata 2: implementation overview

This is the order Humata 2 gets built in. Tick a box when its step is done and tested, and tick a milestone only when all its steps are ticked.

**Reference documents:**
- `SPEC.md`: what Humata 2 does, as behaviour.
- `REVIEW.md`: the review of the spec.
- `api/`: the boundaries: `BOUNDARIES.md`, `GRPC.md` + `proto/`, `MCP.md`, `CLI.md`.
- `prototype/`: how things roughly behave. For how things look, the design goldens below win over it.
- **Design goldens:** `desktop/tests/__snapshots__/design/*.png` are what the UI should look like (D76). They are drawn by `desktop/tests/design.rs` and `design/*.rs` (`bazel test //apps/humata2/desktop:design_test`; record with `bazel run //apps/humata2/desktop:design_test_record`). They are still mocks, not the app. Every screen you build or restyle must match its frame: layout, spacing, type, colour and wording. If a design needs to change, change the frame first, and only with the owner's OK, then build to it. Don't invent layouts the frames don't show; ask instead.

**Rules that hold for every milestone:**
- **Tests prove behaviour, never how the code is written.**
  - Boundary tests drive gRPC, MCP and the CLI from outside.
  - Flow tests drive the app the way a person does.
  - Reducer actions, state and stores are fair to assert on, because they are the abstract model.
  - Private functions, call order and message wording are not.
- **macOS, Linux and web; no other platforms (owner, 2026-11-15).** Every test suite runs on macOS and Linux, and the app outside `desktop/` runs in a browser.
- **The app is only a client** of the Mac's daemon (AR-2, AR-3, D34). Any number of clients, desktop or web, can be open at once. App code outside `desktop/` builds for wasm, and CI checks it.
- **The infrastructure milestones (I1–I9) come first.** The desktop app (A1 onward) starts only after gate G is passed.
- **Each app milestone ships as a signed `humata2` release.** That's a big behavioural step you can install and try.

---

## I0: Agree the foundations
- [x] Fold the spec review (`REVIEW.md`) into `SPEC.md`: fix contradictions, undefined terms and untestable requirements.
- [x] Agree the boundary specs in `api/`: every requirement maps to an RPC, MCP tool or CLI command, and the capability matrix leaves no gaps. See `api/TRACE.md`; the gaps it still lists are logged as decisions (D20, D21).
- [ ] Decide what happens to the earlier H2-0/H2-1 code (`apps/humata2/app` (Swift), `bindgen`, `core`, `rehearsal`): keep, fold in, or remove.
- [x] Lay out the crates and Bazel targets for Humata 2, with the narrowest visibility (see AGENTS.md). See `LAYOUT.md`.

## I1: State management (tca-rust)
Expand `libs/tca-rust` (Store, LocalStore, TestStore, effects, `for_each`, `if_let`, IdentifiedVec, TestClock) until it can carry a desktop app.
- [x] Navigation: stacks, sheets and alerts as state, with tests for presenting and dismissing.
- [x] Shared state: one value (settings, pinned, the machine list) read by many features without copying, with change tests.
- [x] Dependencies: a standard way to swap clients (daemon, clock, clipboard, file system) for fakes in tests.
- [x] Persistence: save and restore chosen state (layout, pane sizes, fold state, open tabs) across restarts.
- [x] Fine-grained observation: a view re-renders only when the part it reads changes, measured with a 10k-row list.
- [x] Streams as effects: a gRPC stream feeds actions in, is cancelled with its feature, and resumes from a cursor.
- [x] TestStore covers all of the above: exhaustive and non-exhaustive modes, readable diffs on failure.
- [x] README updated. Publishing to crates.io only if you ask (not published).
- Linux: pending the next batched Linux run (D18).

## I2: GUI framework
Choose by measurement, not reputation.
- [ ] Criteria written down:
  - near-zero CPU when idle;
  - memory;
  - cold start time;
  - smooth scrolling of a 5k-row tree and a 20k-line diff with wrap on;
  - text shaping (CJK, RTL, emoji), IME and accessibility;
  - macOS and Linux (Wayland + X11), multiple windows;
  - custom drawing for a terminal grid;
  - headless rendering for tests;
  - licence and how actively it's maintained.
- [ ] Shortlist at most three, from GPUI, Iced, Slint, Floem, Makepad, Freya and egui.
- [ ] Spike the same three screens in each finalist: the sidebar tree, the wrapped multi-repo diff, and a terminal grid. Wire each to tca-rust.
- [ ] Measure on macOS and Linux, record the numbers, and write a decision you sign off. Fork or patch only if the numbers demand it.

## I3: Snapshot testing
- [x] State snapshots: TestStore steps recorded as readable text.
- [x] View snapshots: the rendered view as a semantic text tree (roles, labels, text, focus, enabled). These are stable across fonts and GPUs and are the default.
- [x] Layout snapshots: the whole drawn tree as text, containers included, with rounded bounds, layout, padding, gap and colours, so a diff shows what moved, resized or restyled (owner request 2026-11-14).
- [x] Image snapshots, on macOS only: headless rendering with a tolerance, for the few screens where pixels matter (the diff colours, the terminal, wrap). Linux runs the state, view and flow snapshots.
- [x] One command records and another verifies. Failures write the old image, the new image and their diff, to review side by side.

## I4: Flow testing
- [x] A flow is a script of user actions (click, type, key, drag, paste) run against the real app state and views with fake services. Each step can take a snapshot.
- [x] Flows read like the spec's UX flows, and each is named after the spec flow it proves.
- [ ] The same flows run against real daemons (local machines and local VMs) as end-to-end tests.
- [x] Each flow run produces a reviewable artifact: its steps and snapshots in order.

## I5: Testing guideline
- [x] An Opus 5.5 xhigh agent writes `TESTING.md` covering:
  - **gRPC:** contract tests from the protos, error mapping, resumable streams, idempotency.
  - **MCP:** each tool's schema, its tier (default, needs approval, never for agents), refusal shapes, and the capability matrix as tests.
  - **CLI:** exit codes, `--json` output, which actor it runs as.
  - **User-facing:** flows, view snapshots, image snapshots.
  - **What never to test:** wording, call order, private helpers.
- [x] Each rule has a worked example against a real Humata 2 boundary.
- [ ] You sign it off. Every later milestone follows it.

## I6: Terminals
- [ ] Choose the terminal engine. Ghostty is the lead candidate (libghostty's VT core in a Rust GPU view, or GhosttyKit on macOS as in Humata 1). Compare alacritty_terminal and drawing the grid ourselves. Decide on correctness (wide characters, emoji, scripts), speed and cross-platform fit.
- [ ] Embed it in the chosen GUI: tabs, keys, copy and paste, scrollback, find, and the Wrap setting not applying to terminals.
- [ ] Remote terminals:
  - a PTY held on the machine by a per-session holder;
  - output streamed over gRPC with sequence numbers;
  - reattaching replays from the last sequence received;
  - resizing works.
- [ ] Tests: image snapshots of tricky output, and boundary tests for reattaching after a dropped connection.

## I7: Daemon, persistence and machines
Reuse what holds up from Humata 1's `humatad` (agent, wire, workspace, the SSH e2e harness).
- [ ] humatad 2 serves `api/proto` over a single SSH or stdio stream. The app and the daemons speak only gRPC.
- [ ] Sessions survive:
  - **app quit, SSH drop or Mac sleep:** holders keep running;
  - **daemon upgrade or crash:** holders are untouched;
  - **reboot:** the session record and scrollback are saved to disk and the agent resumes;
  - **crash:** resume is checked and the agent is told it was interrupted.
- [ ] Kill-matrix test: kill each process at each point and show that the session either comes back or says exactly what was lost.
- [ ] Restarts never lose state (PER-25–PER-31): power-cut tests for host, machine and both at every fault point, real VM hard resets, every multi-step operation finished or undone, a restart report for every restart, and the restart doctor skill with `humata doctor restarts`.
- [ ] Links push and never poll (TR-7–TR-10): file notifications inside machines, link work off the UI thread, idle wakeups only for keepalives, and latency benches against a local VM.
- [ ] Where state lives:
  - the Mac is the source of truth;
  - machines hold only clones and agent context, and report pointers (repo, branch, sha, pushed, dirty files);
  - writes made while the Mac is away are journalled and replayed in order;
  - Rebuild on another machine works.
- [ ] Machine providers: `create`, `connect` with `{cmd}`, `identity`, interactive login with takeover, install humatad, the systemd user service with an admin fallback. The Lima provider config makes local VMs for the tests.
- [ ] Channels across machines and repos: main clones, per-session clones, branch prefix + slug, add and remove repos.
- [ ] Moves, forks across machines, file transfers that pause and resume, trash, archive and restore, Storage, retention.
- [ ] Skills sync, installing Humata's embedded CLIs and daemons on the host from the app (TS-0) and on every added machine, and scripted sync rules (TS-1–TS-4).
- [ ] Many clients at once (D34): two desktop clients and a CLI stay in sync, each seeing the others' changes as they happen; per-client view state lives on the host.
- [ ] A self-hosted host (AR-7): humatad runs as the host on Linux, and the desktop client and CLI connect to it over SSH, so the same suites pass against a host in a local VM.
- [ ] Web listener (D34): gRPC-web on loopback, off by default, Origin and Host checks, pairing with a revocable per-device key; boundary tests for unpaired, revoked and cross-origin callers.
- [ ] Boundary tests on macOS and Linux, with local VMs as the remote machines.

## I8: MCP and CLI
- [ ] The MCP server per `api/MCP.md`, with each agent bound to its own session and machine.
- [ ] The capability checks live in the daemon:
  - default tools work for the agent's own session;
  - "needs approval" tools become inbox requests;
  - "never for agents" tools are refused, saying why.
- [ ] Grants that are scoped, revocable and recorded, and that no tool can widen.
- [ ] The `humata` CLI per `api/CLI.md`, running the same operations through the same checks.
- [ ] Boundary suites: every tool and command, every refusal, and the capability matrix run as tests.

## I9: Bug reports
A bug report is text an agent can read and act on: what you did, where, what the app showed, and what changed, with enough detail to replay it. There are no videos and no pictures. There are two kinds, and both are saved as a folder. See D31.
- **Recording:** you start, use the app, stop. It captures every event in between.
- **Snapshot:** one moment, taken with a shortcut.

- [ ] Start and stop a recording, or take a snapshot, from the menu, a shortcut or `humata report record|stop|snapshot`. While recording, a dot shows in the title bar.
- [ ] A recording captures, as text and in order:
  - each interaction (click, key, scroll, drag, paste), naming its target by role, label and path in the view tree (for example `Button "Add" in List "Items" in channel #web`);
  - each reducer action with its state diff;
  - the view and layout trees at the start, then the change after each step;
  - daemon RPCs and errors in the same period;
  - the app, OS, daemon versions, window size and the machines and sessions involved.
- [ ] A snapshot captures the state, the view tree and the layout tree at that moment, plus the same versions and context.
- [ ] After either, a short form:
  - what happened;
  - what you expected;
  - comments pinned to elements you pick (saved as the element's path, not as a picture).
- [ ] Saved under the app's data folder in `reports/<date>-<slug>/`:
  - `report.md`, the summary and form;
  - `events.jsonl`;
  - `state.txt`;
  - `view.txt`;
  - `layout.txt`;
  - `daemon.log`.
- [ ] A recording replays as a flow test (I4) against fake services, so a report can become a regression test.
- [ ] Secrets are scrubbed before saving: tokens, host variable values and paste-folder contents. Terminal text is kept only for the terminal you mark.
- [ ] **Copy report** puts a prompt on the clipboard: the summary plus the absolute path of every file, ready to paste into any LLM.
- [ ] Tests:
  - flows that record, take a snapshot, fill the form and copy each kind of report;
  - a saved recording replays to the same end state;
  - scrubbing proven against seeded secrets;
  - the report folder checked as a snapshot.

## G: Gate before the app
- [ ] I1–I9 are done and their suites pass on macOS and Linux.
- [ ] Flow and snapshot tooling has proven itself on a throwaway screen.
- [ ] `features/`, `ui/` and `terminal/` build for `wasm32-unknown-unknown`, and a throwaway screen runs in a browser against a local daemon while the desktop client is open (D34).
- [ ] You agree to start the app.

---

## A1: First light (release humata2 0.1)
- [ ] The app on the chosen GUI and tca-rust, signed and notarized on macOS, with a Linux build.
- [ ] Channels and sessions in the sidebar, with Expand all / Collapse all.
- [ ] A local session with an embedded agent terminal (Claude, Codex, Copilot, OpenCode or shell), with default agent and model settings.
- [ ] The session survives an app restart and a reboot.
- [ ] Panes resize and collapse, and their sizes persist.

## A2: Review (0.2)
- [ ] Diffs across N repos in a session: inline, side by side, and full file with changes inline.
- [ ] Wrap and Scroll for long lines.
- [ ] Files explorer and the editor's Open big view.
- [ ] Comments on non-PR changes, anchored to repo, file, line range, base and quoted code. Send them one at a time or batched to the terminal.

## A3: Machines (0.3)
- [ ] Hosts, Add machine through providers (with the interactive login), and local VMs.
- [ ] Channels across machines, the repo × machine grid, and skills sync.
- [ ] Sessions on remote machines, with remote terminals that survive dropped connections.

## A4: Forks and moves (0.4)
- [ ] Code, context and code+context forks on any machine in the channel.
- [ ] Move a session, archive with the unpushed-work check, restore, and Rebuild on another machine.

## A5: Inbox and access (0.5)
- [ ] Inbox at global, channel and session scope. Every request carries what it needs to be decided on.
- [ ] Access switches, grants and "Always" with scope, and the "while I'm away" rule.
- [ ] Pinned and VIP.

## A6: Boards (0.6)
- [ ] Session, channel and global boards, with dependency edges and agent edits through MCP.

## A7: Files and storage (0.7)
- [ ] The two-pane file manager, transfers, trash, Storage per machine, and retention rules.

## A8: PR reviews (0.8)
- [ ] Paste a PR link, then clone it on any machine with a review chat: an agent that answers your questions and works the view (focus, highlight, notes for you, order), never a verdict of its own.
- [ ] Drafted comments that post only after you approve them.
- [ ] Update when the PR gets new commits, and archive when it's merged.

## A9: Spec mode (0.9)
- [ ] Rendered markdown with a Source toggle and a contents list.
- [ ] Comments anchored to text, with draft → sent → addressed → resolved, and batch send.

## A10: Views, palette and polish (1.0)
- [ ] Views that agents prepare, which open only if you choose to (Open it / Later / Not now), and the Journal.
- [ ] The ⌘K palette covering every action.
- [ ] Every spec flow passes as a flow test on macOS and Linux.
- [ ] Humata 2 can replace Humata 1 day to day.

## N1: A native macOS look and feel (next, owner 2026-11-15)
The owner dislikes how the UI looks and how it's styled. Redo the UI and UX so it looks and feels as Apple/macOS-native as we can get, while staying a chat-workspace app with channels and sessions.
- [x] Owner's pick (2026-11-15): looks A (light) and B (dark), following the system. The middle of a session is its embedded terminal, not a chat. Sidebar, terminal and right panel are three columns resized by dragging; the terminal or the right panel can each go full screen.
- [x] Owner's picks (2026-11-15, D76), now the 33 design goldens in `desktop/tests/__snapshots__/design/`:
  - The right panel is switched from a labelled icon strip down the window's right edge; the session header keeps a panel toggle.
  - A session's terminals and open files are tabs in one row (`claude | zsh | client.rs | +`). The only split is two tab groups side by side in one session, at most one terminal per agent on the same code (14). Anything more is a fork: code + context, context only, or code only. No general pane workspace (revises D74).
  - The sidebar tree indents forks with a fork glyph; channels fold to one row.
  - Clicking a channel opens its Sessions tree table; Activity and Repos and machines are its other tabs.
  - Board: kanban, item drawer, table and dashboard. The Board panel expanded to full screen shows the kanban lanes side by side (12).
  - Home and Inbox are lists, top to bottom, with what needs you first; the Inbox is split (list and the open request).
  - New session runs any agent's own TUI (Claude Code, Codex, Copilot, OpenCode) or a plain terminal (31).
- [ ] Then build it to match the goldens: the right-edge strip switcher and header toggle, the indented tree, the channel Sessions tab, Home and Inbox lists, terminal tabs and the two-group split, New session with any agent TUI or a plain terminal, sidebar material and vibrancy, system font sizes and weights, the unified title bar with toolbar items, native list selection, sheets and popovers, light and dark following the system, and a terminal/panel full-screen state in `features/app` with tests.
- [ ] Restyle the shell (sidebar, session header, terminal tabs, side panel, strip), then each screen, until the app's snapshots match the goldens.
- [ ] Draw the goldens on Linux (in the Linux VM batch, after the I milestones) and in a browser. Find out whether gpui's web target can draw `design.rs` headless; if not, check the frames another way. Never edit `ui-prototype` for this.
- [ ] Follow platform behaviour: standard shortcuts, ⌘, for Settings, the native menu bar, traffic-light spacing, scroll and selection behaviour.

## Known gaps (2026-11-15)

**Keep this file current.** Everyone working on Humata 2, agents included, takes work from this file, and in the same commit as the code: ticks what is done and verified (with the Bazel target that proves it), splits partly done items to show what's left, and adds newly found gaps. Each commit also gets an entry in `GAPS_REVIEW.md` so it can be reviewed.

Work that was started but isn't finished, grouped by area. Each item is open until its tests pass and a real run works. Run builds and tests only through Bazel (`export PATH=$PWD/.tools/bin:$PATH`), never cargo.

### Desktop UI (I2, A10; D72, D74)
- [x] ~~Render the pane tree~~: built in a7c6cf3c, then dropped on the owner's direction (2026-11-15, D74 revised). The window is a left sidebar, the screen in the middle, and the open session's side panel on the right.
- [x] Drag the sidebar's and the side panel's dividers to resize them (past the narrowest hides them; a double-click resets them); ⌥⌘0 and the palette's "Toggle side panel" hide or show the panel. Each session keeps its own panel when you switch screens and come back. Test: `desktop:shell_test` (`a_sessions_side_panel_hides_shows_and_survives_switching_screens`).
- [ ] Give the side panel real content (Changes, Inbox, Files, Board, Info); today it shows its title and an empty state.
- [ ] A divider drag isn't covered by a test yet (needs mouse events in the headless driver); check it in a real window.
- [x] Removed `features/layout`, `AppState.panes`, the saved `panes` field (old saved files still load; serde ignores it) and the Reset toast's Undo button, on the owner's call (2026-11-15). Test: `desktop:shell_test`.
- [ ] Settings › Layout, e2e flows, contact sheets, record/replay, FLOWS.md, and the D72 entry.
- [x] Snapshot fixes: the Inbox badge overlaps the rail icon, and Home's row times stop short of the right edge.

### Markdown (`apps/humata2/markdown`)
- [ ] Fix `headless:flows` (1 of 7 still fails, `bazel test //apps/humata2/markdown/headless:flows`):
  - [x] Toggling Source and back with the keyboard: `toggle_mode` moves focus to the new text view.
  - [x] The selection toolbar's buttons act on press, using the selection cached when the toolbar was drawn, because pressing clears the text's selection first. Copy now reaches the clipboard.
  - [x] Copy as Markdown: flow targets can aim at a control's start (`Target::at_start`). A selection starting at a block's first visible character takes in its opening markup, and one ending at its last takes in the closing markup (`with_block_markup`), so it copies as `# Gallery`.
  - [ ] Comment on selection: dragging paragraph 2 → heading 3 gives no toolbar, because gpui-base's `SourceRangeSelection::merge` turns the whole selection Unmapped when one block can't map (`text/node.rs` in gpui-base 0.9.1). Tried 2026-11-15 and reverted: a patch making `source_range_for_segments` bridge rendered gaps didn't bring the toolbar back, so the Unmapped comes from somewhere else (a block-level Unmapped, the `first.rendered.start` path, or the paragraph's `selected_source_range`). The repin it needed also changed code and inline-code rendering in `headless:snapshots`. Next: log which block returns Unmapped before patching again.
- [ ] Read the goldens, record `headless:timing` numbers, run buildifier, and give the markdown decision a number (it proposed D75).
- [ ] Add syntect to the gpui-fast lock (code blocks are uncoloured on Fast), add a Flow scroll action, parse `secondary-` keystrokes, and add Fast-engine GUI tests.

### Review pane (`apps/humata2/review`; D53)
- [ ] The `large` test mounts about 207 rows per scroll frame against a bound of 120. Cause unknown.
- [ ] Goldens show "Saving…" because the test clock never passes the 300 ms debounce.
- [ ] Embed `humata2_diff_view` and a MarkdownRenderer; today the pane has stubs for both.
- [ ] The daemon remembers Type keys durably, so a send never repeats after a restart.
- [ ] MCP relays `review_focus`, `review_note` and `review_prepare` to the pane.

### review_diff wiring (A2, A8; PR-22–PR-26, D66)
- [ ] humatad serves DiffService, ReviewChatService, CommentService and ViewService (`src/review.rs`), with a MarkReviewed tree snapshot.
- [ ] MCP dispatches the 6 review tools; add `humatad/tests/review.rs` and access tests.
- [ ] The CLI sends a real `shown_tree` (today it's empty); the store persists UserNote and ReviewViewState.
- [ ] Measure gix end to end.

### Diff view (`apps/humata2/diff`; D67)
- [ ] Write D67 in DECISIONS.md with the GPUI and Fast numbers. Agreed: no "open all folds" action, and Full file mode is gated on its own measurement.
- [ ] Behaviour tests for the new DiffView (Pane harness, SpecChanged, CommentRequested, ContextNeeded, the counter, viewed files folding). It has been compiled, never run.
- [ ] Point the gate at DiffView instead of DiffState, then re-run the gates (GPUI and Fast, incremental patch) on a quiet machine. Last numbers were from a loaded machine, on the old view: scroll with comments p95 6.71 / max 10.07 ms; Fast p95 ≤ 3.64 ms.
- [ ] Drop the unused unicode-segmentation and unicode-width dependencies (needs a repin).

### Terminal (I6; D73)
- [ ] Run `stream_bench` and `stream_fast_bench` for 1, 2 and 4 panes.
- [ ] Test IME in a real window.
- [ ] Settle ⌘W: TerminalTabs and NV-30 (close pane) both claim it.

### Testing infrastructure (I3–I5)
- [ ] Run `engine:frames_bench_fast`, `terminal/view:frame_bench{,_fast}` and the ui-prototype `markdown_frame_costs`. Fast can't become the default until it has numbers.
- [ ] Re-measure plain GPUI on a quiet machine (loaded run: idle p99 30.6 ms; 20k diff p99 22.2 ms).
- [ ] Real-window GPU benchmarks; verify IME, clipboard and VoiceOver.
- [ ] Investigate `Click("board-card-0")`, which once hit the 12-frame settle limit.
- [ ] Remove the stale `apps/humata2/testing/Cargo.lock`. The ui-prototype README/UX_FLOWS still show cargo commands; that's the other session's to fix.
- [ ] Enforce rustfmt (there is no rustfmt.toml yet).

### Daemon and persistence (I7; PER-25–PER-31, D71)
- [x] The last good copy no longer waits on file events (Bazel's darwin sandbox delivered none): the store calls `Store::on_commit` hooks after each committed write, and humatad's keeper debounces those. Tests: `daemon/humatad:humatad2_unit_test` (`a_copy_is_saved_once_writes_go_quiet`, no longer ignored), `daemon/store:records_test` (`each_committed_write_is_told_and_a_refused_one_is_not`).
- [ ] Give the background reconcile, `ServerInfo.last_start` and the debounce a decision number and a GRPC entry.
- [ ] Put humatad's own log lines in restart reports. Write the restart doctor skill (`humata doctor restarts`).
- [ ] MV-13/MV-14 move export and import, including the store's MV-13 export.
- [x] Wire humatad to the store's restart work: `Store::open_or_restore` with `Restored` passed to reconcile, `save_last_good` after writes go quiet (30 s, at most 10 min) on a blocking thread, `ReviveConfig.reports` filled, and one `NotBroughtBack` line per agent or shell that didn't come back in `ServerInfo.last_start`. Test: `daemon/humatad:restart_test` (`a_shell_that_cant_come_back_is_one_line_and_the_others_still_do`, `a_damaged_state_comes_back_from_its_last_good_copy_and_the_start_says_so`).
- [ ] Show `last_start`'s lines (and a restore's losses) in the app, as the goldens draw it.
- [ ] A resume redone after a crash skips the "you were interrupted" note when the conversation already holds it, and then only checks that the agent came up, stayed up and opened no other conversation (D71). Find a stronger check if one exists.
- [ ] Recheck `terminal_test` (once failed with `SizeOutOfRange { claimed: 2097152 }`) and `restart_test` (once Unauthenticated), and fix the flaky `humatad2_unit_test`.

### Machines (I7, A3; D61)
- [ ] Restart the service only when checksums change, not on every connect.
- [ ] Operation Cancel and Retry, EventService machine streaming, and launchd for macOS remotes.
- [ ] A host-side peer-link client with keepalive.
- [ ] The remove plan covers sessions and channels, and runs the dispose, leave_channels, provider_delete and find_agents steps.
- [ ] A real Lima/Linux run, including the four-platform tools bundle (after all I milestones, per the Linux batch rule).

### Compression (D65)
- [ ] Put FileService on the peer link so the remote throughput estimate fills in. Test that Auto picks zstd beyond the unit tests.
- [ ] TransferService.Upload, diff chunks and bug-report chunks, the compression settings page, and FileService roots beyond session clones.
- [ ] Bench numbers on a quiet machine.

### MCP, access and other open items (I8, A5)
- [ ] ReportService, script tokens (SEC-14), and the web listener.
- [ ] MCP `inbox_request` and Ask→Inbox (D38.6); persist the Inbox and grants.
- [ ] Decisions waiting on the owner, each with a recommendation in DECISIONS.md: D51, D56, D62 items 7–10, D63's synthetic FS, D67, I5 sign-off, and whether to keep the peer-link pipe-fd change (D64).
