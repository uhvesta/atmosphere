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
- [x] The session panel strip, header toggle, client-backed Changes/Inbox/Board/Journal content and panel full screen are rendered and exercised by `//apps/humata2/desktop:panels_test`; panel frames are checked in light/dark appearances. Files/Info and unsupported Inbox controls are split out under Known gaps.
- [ ] Finish the indented tree, channel Sessions tab, Home/global Inbox lists, terminal tabs and the two-group split, New session with any agent TUI or a plain terminal, sidebar material and vibrancy, system font sizes and weights, the unified title bar with toolbar items, native list selection, sheets and popovers, light/dark following the system across the shell, and terminal full screen (minion C: this pass builds the session panels only; the rest remains the N1 shell work).
- [ ] Restyle the remaining shell (sidebar, session header, terminal tabs) and each screen until the whole app matches the goldens (minion C: the new panel/strip use D76 tokens; cropped panel references do not claim the earlier shell is already restyled).
- [ ] Draw the goldens on Linux (in the Linux VM batch, after the I milestones) and in a browser. Find out whether gpui's web target can draw `design.rs` headless; if not, check the frames another way. Never edit `ui-prototype` for this.
- [ ] Follow platform behaviour: standard shortcuts, ⌘, for Settings, the native menu bar, traffic-light spacing, scroll and selection behaviour.

## Known gaps (2026-11-15)

**Keep this file current.** Everyone working on Humata 2, agents included, takes work from this file, and in the same commit as the code: ticks what is done and verified (with the Bazel target that proves it), splits partly done items to show what's left, and adds newly found gaps. Each commit also gets an entry in `GAPS_REVIEW.md` so it can be reviewed.

Work that was started but isn't finished, grouped by area. Each item is open until its tests pass and a real run works. Run builds and tests only through Bazel (`export PATH=$PWD/.tools/bin:$PATH`), never cargo.

### Desktop UI (I2, A10; D72, D74)
- [x] ~~Render the pane tree~~: built in a7c6cf3c, then dropped on the owner's direction (2026-11-15, D74 revised). The window is a left sidebar, the screen in the middle, and the open session's side panel on the right.
- [x] Drag the sidebar's and the side panel's dividers to resize them (past the narrowest hides them; a double-click resets them); ⌥⌘0 and the palette's "Toggle side panel" hide or show the panel. Each session keeps its own panel when you switch screens and come back. Test: `desktop:shell_test` (`a_sessions_side_panel_hides_shows_and_survives_switching_screens`).
- [x] Changes, Inbox, Board and Journal show the existing clients' data in the D76 panel framing, in light/dark appearances. Changes selects a repo/file per session and sends its queued drafts; Inbox includes descendants and answers the chosen request; Board scopes to the channel and expands into lanes; Journal scopes to the session. The labelled right-edge strip, header toggle and panel full screen work. Tests: `//apps/humata2/desktop:panels_test` (9 tests, 12 image references inspected), `//apps/humata2/desktop:shell_test`, `//apps/humata2/desktop:design_test`.
- [ ] Files and Info content (minion C: no approved recorded Files/Info content frames exist among design 01–12; mock functions alone are not approved PNG references. Their strip buttons stay disabled pending owner-approved frames).
- [ ] Session-only Inbox grants and the Reply editor (minion C: the client supports only the broader Always rule, which must not stand in for a session grant; no approved reply-editor frame is recorded. The checkbox/Reply button remain disabled, and questions never offer Allow).
- [ ] The Changes base picker, Open in editor and comment composer still need their clients/views (minion C: this pass renders the current base and existing comments/drafts only; the excluded review_diff wiring and a selected editor/comment layout are needed before those controls can be made live).
- [x] A native headless mouse drag checks the panel divider with its 64px strip outside the width: a 40px drag gives 380px, instead of jumping to the 420px maximum. Test: `//apps/humata2/desktop:panels_test` (`resizing_the_panel_keeps_the_strip_outside_its_width`).
- [ ] Cover the sidebar's drag too, and check both dividers in a real window (minion C: the assigned run is headless only).
- [x] Removed `features/layout`, `AppState.panes`, the saved `panes` field (old saved files still load; serde ignores it) and the Reset toast's Undo button, on the owner's call (2026-11-15). Test: `desktop:shell_test`.
- [ ] Settings › Layout, e2e flows, contact sheets, record/replay, FLOWS.md, and the D72 entry.
- [x] Snapshot fixes: the Inbox badge overlaps the rail icon, and Home's row times stop short of the right edge.

### Markdown (`apps/humata2/markdown`)
- [ ] Fix `headless:flows` (1 of 7 still fails, `bazel test //apps/humata2/markdown/headless:flows`):
  - [x] Toggling Source and back with the keyboard: `toggle_mode` moves focus to the new text view.
  - [x] The selection toolbar's buttons act on press, using the selection cached when the toolbar was drawn, because pressing clears the text's selection first. Copy now reaches the clipboard.
  - [x] Copy as Markdown: flow targets can aim at a control's start (`Target::at_start`). A selection starting at a block's first visible character takes in its opening markup, and one ending at its last takes in the closing markup (`with_block_markup`), so it copies as `# Gallery`.
  - [ ] Comment on selection: dragging paragraph 2 → heading 3 gives no toolbar, because gpui-base's `SourceRangeSelection::merge` turns the whole selection Unmapped when one block can't map (`text/node.rs` in gpui-base 0.9.1). Tried 2026-11-15 and reverted: a patch making `source_range_for_segments` bridge rendered gaps didn't bring the toolbar back, so the Unmapped comes from somewhere else (a block-level Unmapped, the `first.rendered.start` path, or the paragraph's `selected_source_range`). The repin it needed also changed code and inline-code rendering in `headless:snapshots`. Next: log which block returns Unmapped before patching again.
- [x] Inspected both Markdown image goldens and the current render, recorded `headless:timing` figures and limitations in D78, and ran Bazel-managed buildifier. Timing target: `//apps/humata2/markdown/headless:timing` (`-c opt --test_arg=--nocapture`).
- [ ] The two Markdown pixel checks differ from their stored goldens by about 19,700 pixels each: current fenced/inline code is monospace, stored references use proportional text. Semantic snapshots and streamed/one-shot pixels pass (minion C: left references intact for owner review; no rendering change was made here).
- [ ] Margin comments clip long text horizontally in the annotated gallery (minion C: observed in the stored golden; needs a layout choice before restyling).
- [x] Flow scroll actions dispatch pixel deltas at an accessible target in refreshed and natural frames, and the long Markdown scroll flow uses them. `secondary-` parsing already uses the platform OS; new tests prove dispatch on both engines. Tests: `//apps/humata2/testing/flow:scroll_test`, `:scroll_fast_test`, and `//apps/humata2/markdown/headless:flows --test_arg=the_scroll_wheel_moves_a_long_document_and_only_visible_blocks_are_drawn`.
- [ ] Add syntect to the gpui-fast lock and Markdown Fast GUI suites (minion C: dependency repinning is excluded; flow infrastructure itself is tested on both engines).

### Review pane (`apps/humata2/review`; D53)
- [x] `review:large` dispatches one scroll event per measured frame, instead of counting the scroll helper's two refreshes plus a third draw as one. Every one of 200 frames stays within 120 mounts; no production overdraw change was needed. Test: `//apps/humata2/review:large`.
- [x] Review goldens show Saved after advancing test time, and an empty send queue no longer increments the revision after saving. The saved revision equals the displayed review's revision. Tests: `//apps/humata2/review:snapshots`, `//apps/humata2/review:flows`; all four image goldens inspected.
- [ ] Embed `humata2_diff_view` and a MarkdownRenderer; today the pane has stubs for both.
- [ ] The daemon remembers Type keys durably, so a send never repeats after a restart.
- [ ] MCP relays `review_focus`, `review_note` and `review_prepare` to the pane.

### review_diff wiring (A2, A8; PR-22–PR-26, D66)
- [x] humatad serves DiffService and ReviewChatService (`daemon/humatad/src/review.rs`) on the socket. MarkReviewed snapshots per repo the `shown_tree` the client names, else the last GetDiff's new tree, else the working tree; review chat changes reach WatchReviewChanges; the agent may read its own diff, `review_context` and `review_focus`, never MarkReviewed, Undo or the switches (user-only). Test: `//apps/humata2/daemon/humatad:review_test`.
- [ ] humatad serves CommentService (at least the `note` destination: review_note, review_note_edit, review_note_drop) and ViewService (PrepareView; in a review chat `review_prepare` applies through `ReviewChat::prepare`).
- [x] MCP dispatches `review_context` and `review_focus` to ReviewChatService for the caller's own session. Test: `//apps/humata2/daemon/humatad:mcp_test` (`the_review_chats_agent_shows_the_user_a_place_and_reads_it_back`).
- [ ] MCP dispatches `review_note`, `review_note_edit`, `review_note_drop` and `review_prepare` once CommentService and ViewService are served (they answer not built today).
- [ ] The review state in `src/review.rs` is memory only: the store persists UserNote, ReviewViewState, the switches and the last shown trees, so a restart keeps the review chat and Mark reviewed still snapshots what was shown.
- [ ] The CLI's empty `shown_tree` is what the proto asks of it ("Empty (the CLI): the last GetDiff's trees"), and the daemon now fills it in; drop this item unless the CLI gets its own diff view.
- [ ] DiffService on the peer link (a session on another machine) is still not built; the socket's daemon serves it.
- [ ] `GetDiff` with `other_session` is refused (wire leaves it to the daemon); resolve it to that session's snapshot.
- [ ] Measure gix end to end.

### Diff view (`apps/humata2/diff`; D67)
- [x] D67 records the existing GPUI/Fast figures, their old-view/loaded-machine limits, no open-all-folds action, and the separate Full file measurement gate (no benches rerun).
- [x] Embedded DiffView runs in a Pane harness on both engines: SpecChanged retains the pending comparison, CommentRequested carries code/base/side, ContextNeeded accepts missing text, the counter handles replacement/empty reviews, and Viewed folds until content changes. Tests: `//apps/humata2/diff:view_test`, `//apps/humata2/diff:view_fast_test`.
- [ ] Change navigation on a review that fits the viewport leaves the counter above the first change after layout (reproduced on both engines); the dependency navigates from the clamped viewport rather than the chosen change (minion C: fixing gpui-component requires the prohibited dependency patch/repin; deferred).
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
- [ ] Investigate `Click("board-card-0")`, which once hit the 12-frame settle limit (minion C: that control and parity journey occur only in the excluded, concurrently edited `ui-prototype`; no matching control exists in the permitted desktop/testing code).
- [x] Removed the stale `apps/humata2/testing/Cargo.lock` and corrected its workspace manifest comment; Bazel targets remain the build/test path. Tests: `//apps/humata2/testing/flow:scroll_test`, `:scroll_fast_test`, `:natural_frames_test`, `:natural_frames_fast_test`. The ui-prototype documentation remains the other session's work.
- [x] Proposed a minimal root `rustfmt.toml` (edition 2024, width 100) in D79; checked the two new test files with Bazel-managed rustfmt. No tree reformat.
- [ ] Decide on enforcing/migrating rustfmt across the existing tree (minion C: D79 is Provisional; awaiting owner choice).

### Daemon and persistence (I7; PER-25–PER-31, D71)
- [x] The last good copy no longer waits on file events (Bazel's darwin sandbox delivered none): the store calls `Store::on_commit` hooks after each committed write, and humatad's keeper debounces those. Tests: `daemon/humatad:humatad2_unit_test` (`a_copy_is_saved_once_writes_go_quiet`, no longer ignored), `daemon/store:records_test` (`each_committed_write_is_told_and_a_refused_one_is_not`).
- [x] The background reconcile, `ServerInfo.last_start` and the last good copy's debounce are D77 and GRPC.md §2.2 (and its §2.1 table).
- [x] humatad2's own log lines go in restart reports: `logs/humatad2.log`, kept as `humatad2.previous.log` at each start, its last 50 lines in `daemon.log` and `restart.json`'s `daemon_log` (D77). Tests: `daemon/humatad:restart_test` (`the_restart_report_has_what_the_daemon_logged_before_it_stopped`), `daemon/revive:kill_matrix_test` (`a_failed_resume_is_reported_with_what_it_takes_to_diagnose_it`).
- [ ] Write the restart doctor skill (`humata doctor restarts`).
- [ ] Push `last_start` to the app when it's set (EventService) instead of the app asking again.
- [ ] MV-13/MV-14 move export and import, including the store's MV-13 export.
- [x] Wire humatad to the store's restart work: `Store::open_or_restore` with `Restored` passed to reconcile, `save_last_good` after writes go quiet (30 s, at most 10 min) on a blocking thread, `ReviveConfig.reports` filled, and one `NotBroughtBack` line per agent or shell that didn't come back in `ServerInfo.last_start`. Test: `daemon/humatad:restart_test` (`a_shell_that_cant_come_back_is_one_line_and_the_others_still_do`, `a_damaged_state_comes_back_from_its_last_good_copy_and_the_start_says_so`).
- [ ] Show `last_start`'s lines (and a restore's losses) in the app, as the goldens draw it.
- [ ] A resume redone after a crash skips the "you were interrupted" note when the conversation already holds it, and then only checks that the agent came up, stayed up and opened no other conversation (D71). Find a stronger check if one exists.
- [x] Rechecked `terminal_test`: 2097152 is `SNAPSHOT_MAX`, a whole 2 MiB snapshot sent as one chunk, past the decoder's 1 MiB `CHUNK_MAX`; snapshots now go in pieces (5c795563), covered by `a_big_snapshot_comes_in_pieces_to_a_client_that_decodes`. Passed 65 runs (`--runs_per_test=40 --local_test_jobs=16` plus 25).
- [x] Fixed the flaky `humatad2_unit_test`: its idle-daemon test counted wakes while the start's background reconcile (and, before 71c9071b, file events for the last good copy) could still land in either window. It now waits for the restart report first. Passed 40 runs at 16 jobs.
- [ ] `restart_test`'s one Unauthenticated wasn't reproduced in 65 runs. Its daemon runs `--admin`, where a call with no credential is the user, so that call must have reached a daemon without it (or carried a stale token). Watch for it.

### Machines (I7, A3; D61)
- [x] Restart the service only when a file changes or it isn't running the way setup starts it: the check reports `running` (systemd: our unit, lingering, enabled and active, no boot entry; setsid: the boot entry, the daemon, no unit), and setup skips the service step when nothing is pending and that matches. Tests: `daemon/machines:fake_ssh_test` (`reconnecting_with_the_same_build_installs_nothing`, `reconnecting_starts_the_service_again_only_when_it_stopped_or_a_file_changed`), `daemon/machines:machines_unit_test` (`the_check_sees_how_the_daemon_runs_after_each_start_and_after_removal`).
- [ ] `daemon/machines:takeover_test` `a_bridge_that_never_syncs_times_out_with_the_end_of_its_output` failed once under a full `daemon/machines/...` run (its 3 s sync timeout on a loaded machine) and passed 5 runs alone. Watch for it.
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
