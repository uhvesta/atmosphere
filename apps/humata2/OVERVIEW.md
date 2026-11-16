# Humata 2: implementation overview

This is the order Humata 2 gets built in. Tick a box when its step is done and tested, and tick a milestone only when all its steps are ticked.

**Reference documents:**
- `SPEC.md`: what Humata 2 does, as behaviour.
- `REVIEW.md`: the review of the spec.
- `api/`: the boundaries: `BOUNDARIES.md`, `GRPC.md` + `proto/`, `MCP.md`, `CLI.md`.
- `prototype/`: the first clickable sketch. It is no longer a reference for looks or behaviour (SPEC §28); the design goldens and SPEC replaced it.
- **Design goldens:** `desktop/tests/__snapshots__/design/*.png` are what the UI should look like (D76). They are drawn by `desktop/tests/design.rs` and `design/*.rs` (`bazel test //apps/humata2/desktop:design_test`; record with `bazel run //apps/humata2/desktop:design_test_record`). They are still mocks, not the app. Every screen you build or restyle must match its frame: layout, spacing, type, colour and wording. If a design needs to change, change the frame first, and only with the owner's OK, then build to it. Don't invent layouts the frames don't show; ask instead. When the docs disagree, the owner's decisions (`DECISIONS.md`), newest first, win; then the goldens; then SPEC. Frames that a newer decision has left out of date, and screens with no frame yet, are listed in D113.

**Rules that hold for every milestone:**
- **Tests prove behaviour, never how the code is written.**
  - Boundary tests drive gRPC, MCP and the CLI from outside.
  - Flow tests drive the app the way a person does.
  - Reducer actions, state and stores are fair to assert on, because they are the abstract model.
  - Private functions, call order and message wording are not.
- **macOS, Linux and web; no other platforms (owner, 2026-11-15).** Every test suite runs on macOS and Linux, and the app outside `desktop/` runs in a browser.
- **The app is only a client** of the host's daemon (AR-2, AR-3, D34). Any number of clients, desktop or web, can be open at once. App code outside `desktop/` builds for wasm, and CI checks it.
- **Infrastructure and the UI on fakes are built side by side (owner, 2026-11-14).** The infrastructure milestones (I1–I9) and the desktop UI on fake clients with made-up data (N1) run in parallel. Gate G is where real clients replace the fakes. An A box is ticked only when its step works against a real daemon, so the A list stays unticked while N1 shows what exists on fakes.
- **Each app milestone ships as a signed `humata2` release.** That's a big behavioural step you can install and try.

---

## I0: Agree the foundations
- [x] Fold the spec review (`REVIEW.md`) into `SPEC.md`: fix contradictions, undefined terms and untestable requirements.
- [x] Agree the boundary specs in `api/`: every requirement maps to an RPC, MCP tool or CLI command, and the capability matrix leaves no gaps. See `api/TRACE.md`; the gaps it still lists are logged as decisions (D20, D21).
- [x] The earlier H2-0/H2-1 code (`apps/humata2/app` (Swift), `bindgen`, `core`, `rehearsal`) was removed under D9 (minion TIDY); git history keeps it.
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
Choose by measurement, not reputation. **Decided (owner, 2026-11-14): gpui-kit on GPUI (D8), with one crate picking the engine (D57).** The owner dropped the Linux pixel-snapshot condition. The measurements still owed are under Known gaps › Testing infrastructure.
- [x] Criteria written down (`research/GUI.md`):
  - near-zero CPU when idle;
  - memory;
  - cold start time;
  - smooth scrolling of a 5k-row tree and a 20k-line diff with wrap on;
  - text shaping (CJK, RTL, emoji), IME and accessibility;
  - macOS and Linux (Wayland + X11), multiple windows;
  - custom drawing for a terminal grid;
  - headless rendering for tests;
  - licence and how actively it's maintained.
- [x] Shortlist at most three, from GPUI, Iced, Slint, Floem, Makepad, Freya and egui. Closed by D8: gpui-kit, with Iced and then egui as fallbacks.
- [x] Spike the same three screens in each finalist: the sidebar tree, the wrapped multi-repo diff, and a terminal grid. Wire each to tca-rust. Closed by D8: the owner chose before the other finalists were spiked; `spike/gui` holds the gpui-kit spike.
- [x] Measure on macOS and Linux, record the numbers, and write a decision you sign off. Fork or patch only if the numbers demand it. Closed by D8 on the macOS numbers in `research/`; Linux numbers come with the Linux batch.

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
- [x] Choose the terminal engine. Decided 2026-11-14: libghostty-vt, drawn by our own GPUI terminal view (D10, D22). The question as first asked: Ghostty is the lead candidate (libghostty's VT core in a Rust GPU view, or GhosttyKit on macOS as in Humata 1). Compare alacritty_terminal and drawing the grid ourselves. Decide on correctness (wide characters, emoji, scripts), speed and cross-platform fit.
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
  - the host is the source of truth;
  - machines hold only clones and agent context, and report pointers (repo, branch, sha, pushed, dirty files);
  - writes made while the host is away are journalled and replayed in order;
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
The UI on fake clients did not wait for this gate (owner, 2026-11-14; see N1). The gate now guards swapping the fakes for real clients and ticking A boxes.
- [ ] I1–I9 are done and their suites pass on macOS and Linux.
- [ ] Flow and snapshot tooling has proven itself on a throwaway screen.
- [ ] `features/`, `ui/` and `terminal/` build for `wasm32-unknown-unknown`, and a throwaway screen runs in a browser against a local daemon while the desktop client is open (D34).
- [ ] You agree to move the app onto real clients.

---

## A1: First light (release humata2 0.1)
- [ ] The app on the chosen GUI and tca-rust, signed and notarized on macOS, with a Linux build.
- [ ] Channels and sessions in the sidebar, with Expand all / Collapse all.
- [ ] A local session with an embedded agent terminal (Claude, Codex, Copilot, OpenCode or shell), with the default agent setting (ST-15; Humata picks no model, SE-2).
- [ ] The session survives an app restart and a reboot.
- [ ] The sidebar and the side panel resize and collapse, and their sizes persist (NV-12–NV-17).

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
- [ ] Session, channel and global boards in board, table and dashboard views (BD-11–BD-13), with several sessions per item, dependency edges and requests answered on the item (BD-14–BD-16), and agent edits through MCP.

## A7: Files and storage (0.7)
- [ ] The two-pane file manager, transfers, trash, Storage per machine, and retention rules.

## A8: PR reviews (0.8)
- [ ] Paste a PR link, then clone it on any machine with a review chat: an agent that answers your questions and works the view (focus, highlight, notes for you, order), never a verdict of its own.
- [ ] Drafted comments that post only after you approve them.
- [ ] Update when the PR gets new commits, and archive when it's merged.

## A9: Markdown review (0.9)
- [ ] Markdown tabs with Preview, Source and Side by side (SPEC §11: SM-3, SM-4, DK-7, DK-8). There is no separate spec mode and no contents list (D104). The same work on fakes is N1's markdown review item.
- [ ] Comments on source lines, with draft → queued → sent → addressed → resolved, sent one at a time or as a batch (SM-6–SM-21).

## A10: Views, palette and polish (1.0)
- [ ] Views that agents prepare, which open only if you choose to (Open it / Later / Not now), and the Journal.
- [ ] The ⌘K palette covering every action.
- [ ] Every spec flow passes as a flow test on macOS and Linux.
- [ ] Humata 2 can replace Humata 1 day to day.

## N1: A native macOS look and feel (next, owner 2026-11-15)
The owner dislikes how the UI looks and how it's styled. Redo the UI and UX so it looks and feels as Apple/macOS-native as we can get, while staying a chat-workspace app with channels and sessions.
- [x] Owner's pick (2026-11-15): looks A (light) and B (dark), following the system. The middle of a session is its embedded terminal, not a chat. Sidebar, terminal and right panel are three columns resized by dragging; the terminal or the right panel can each go full screen.
- [x] Owner's picks (2026-11-15, D76), now design goldens 01–33 (43 frames since D100) in `desktop/tests/__snapshots__/design/`:
  - The right panel is switched from a labelled icon strip down the window's right edge; the session header keeps a panel toggle.
  - A session's terminals and open files are tabs in one row (`claude | zsh | client.rs | +`). The only split is two tab groups side by side in one session, at most one terminal per agent on the same code (14). Anything more is a fork: code + context, context only, or code only. No general pane workspace (revises D74).
  - The sidebar tree indents forks with a fork glyph; channels fold to one row.
  - Clicking a channel opens its Sessions tree table; Activity and Repos and machines are its other tabs.
  - Board: kanban, item drawer, table and dashboard. The Board panel expanded to full screen shows the kanban lanes side by side (12).
  - Home and Inbox are lists, top to bottom, with what needs you first; the Inbox is split (list and the open request).
  - New session runs any agent's own TUI (Claude Code, Codex, Copilot, OpenCode) or a plain terminal (31).
- [x] The session panel strip, header toggle, client-backed Diff/Inbox/Files/Board/Journal/Info content and panel full screen are rendered and exercised by `//apps/humata2/desktop:panels_test`; panel frames are checked in light/dark appearances. Unsupported file/permission actions are split out under Known gaps.
- [x] Native shell foundation: the rail is gone; the sidebar, window controls, session breadcrumb/header and terminal type use D76 colors and sizes in both appearances. Sidebar navigation/folding and lateral Back behavior are exercised by `//apps/humata2/desktop:native_test` and `//apps/humata2/features/app:navigation_test`. Full matching remains split into the unchecked items below.
- [x] Session lifecycle and presentation foundation: terminal/panel full screen persists per session and returns to the same columns; exited Resume, offline notices, selected-strip close/reopen and tab-scoped output are covered by `//apps/humata2/features/app:presentation_test` and `//apps/humata2/desktop:native_test`. A 720px -> 1280px resize cycle restores the saved widths; the remaining session visuals and two-group split stay open.
- [x] Shared session tab row: terminal, file and accepted-view tabs, client-backed plus menu, at most two groups, attach existing agent, join and saved group selection. `//apps/humata2/features/session:tabs_test`, `//apps/humata2/features/app:presentation_test` and `//apps/humata2/desktop:native_test` cover it. Offers wait for Open it, Later returns next visit, Not now stays dismissed (`//apps/humata2/features/views:offers_test`). Interactive TerminalPane/PTY wiring and exact visual matching remain open.
- [x] Home/wide Inbox foundation: grouped needs-you-first session rows (including exited Resume), the selected request beside the list, scoped Allow/Deny, Waiting/Answered/All and first-visible selection. `//apps/humata2/desktop:native_test` and `//apps/humata2/features/inbox:selection_test` prove the flows; frames 25–27 have inspected real-app captures. Fixture counts/order, preview styling and reply/grant/bulk/log controls remain open.
- [x] Channel Sessions/Activity/Repos foundation: saved tab/filter/history/folds, tree ancestors and rollup Inbox counts, typed activity navigation and derived clone grid filtering. `//apps/humata2/features/app:channel_test` and `//apps/humata2/desktop:native_test` verify the flows; frames 18–20 have inspected real-app captures. Pin/menu, add repo/machine, host selection and Access body remain open; Board body now uses the live Board presentation.
- [x] Board presentation foundation: global scope includes channel items; kanban/table/dashboard and item drawer, scope/text/drill filters, new item with status, status moves, item-specific approval, dependencies/session links and dashboard Resume. `//apps/humata2/features/board:presentation_test`, app presentation and native tests verify state/flows; inspected 21–24. Full session-board scope, drawer edits/assignment/start-session, table folding and pointer-drag coverage remain open.
- [x] Hosts/Settings presentation foundation: selected host table/health/bridge, reconnect/provider start and shell launcher; native settings navigation, appearance, font, pinned visibility, wrap, default agent and bounded group presentation. `//apps/humata2/features/hosts:selection_test`, `//apps/humata2/features/settings:preferences_test`, app presentation and native tests verify state/flows; inspected frames 28/29. Add-machine/free-space, fuller host/security/notification/key/model management and robust code wrapping remain open.
- [x] Palette/New session/Undo/first-run foundations: grouped context and clone file search; launch model/repo/first-message/item options and valid host defaults; success-based archive Undo; empty-workspace channel/SSH actions. Verified by native26 flows, new launch/search/archive TestStore tests and fake-client tests, all18 Bazel targets passed. Frames30–33 inspected; picker/input styling, real terminal/launch bridge, richer SSH setup and full audit remain (minion C1).
- [x] Embed TerminalPane in the real session UI through fake raw byte/input/resize clients; retain per-terminal history across tab/group switches and send sizes on half/full resize. Verified by native27, Session terminal replay TestStore, fake raw-input test and terminal/pane:pane_test; all20 gate targets passed. Model selection removed from New session per direct owner direction; launch uses agent defaults. Real daemon/PTy launch and remaining terminal presentation/transport actions remain (minion C1).
- [x] Fork dialog foundation: header/palette opens title, three kinds, now/parent preview and guarded Create/Cancel; failure retains the draft for retry. Verified by Session Fork TestStore2, native28 create/cancel flow, fake kind/files/activity/archived-parent metadata test and final21-target gate. Missing frame layout is Provisional D102; FK-3 selectors/live read-only clone ownership/own-copy remain (minion C1).
- [x] Terminal keyboard focus follows selected/new tabs and newly created/forked sessions; closing a modal restores focus only if the route/tab/full-screen target is unchanged. Clicking a split grid focuses its group. Native30 actual typing/group flows and Session tabs TestStore prove it; final21-target gate passed (minion C1).
- [x] Changes Wrap keeps number/change mark on the first row, continuation indentation and Unicode grapheme boundaries; Scroll keeps one horizontally scrolling row and font settings scale its gutter. Pure UI tests and native31 width/font/wrap regression plus full21-target gate passed. Rewrap scroll anchoring and paired side-by-side remain open (minion C1).
- [x] Session waiting-message bar/list, Show/Hide, per-message Send now/Remove, explicit fallback/shell sends, acknowledged-payload retention and failed retry; selected-terminal writer label. Verified by Session queue TestStore4, native34 (3 new queue/presence flows) and full22-test gate. Review/approval/comment producers and real paste/presence transport remain open (minion C1; D114 Provisional list layout).
- [x] Review comment outbox foundation: retained structured batches, repo/base/quoted new-line context, failed retry and removal back to drafts; App messages9, native36 and final23-target Bazel gate passed. Latest owner direction is documented, but individual-comment Send and explicit delivery without mandatory turn-end hold are the first remaining task (minion C1; see C1_HANDOFF.md and D126).
- [ ] Finish the indented tree, channel Sessions tab, Home/global Inbox lists, terminal tabs and the two-group split, New session with any agent TUI or a plain terminal, sidebar material and vibrancy, system font sizes and weights, the unified title bar with toolbar items, native list selection, sheets and popovers, light/dark following the system across the shell, and terminal full screen (minion C: this pass builds the session panels only; the rest remains the N1 shell work).
- [ ] Restyle the remaining shell (sidebar, session header, terminal tabs) and each screen until the whole app matches the goldens (minion C: the new panel/strip use D76 tokens; cropped panel references do not claim the earlier shell is already restyled).
- [ ] Check the goldens in a browser (D129): pixel goldens are recorded on macOS and checked on web; Linux gets no pixel goldens, only the launch smoke test. Find out whether gpui's web target can draw `design.rs` headless; if not, check the frames another way. Never edit `ui-prototype` for this.
- [x] Owner signed off frames 34–43 on 2026-11-15 (D100): empty states, the Desk and markdown review. The Desk row is now in the sidebar of every golden (01–43), so native frames 01–33 differ from the goldens by that row until the app has it.
- [ ] Build the Desk in the app (SPEC DK-1–DK-5, DK-10–DK-14; D108): the sidebar Desk row and its sessions, the Desk page, New desk session, scratch or picked folders, Close / Reopen / Keep / Clean up…, moving a Desk session to another machine (DK-3) and copying a document to a channel (DK-13). Frames 36–40.
- [ ] Build the empty states for Home and a new channel (frames 34, 35; DK-6). The first-launch welcome (33, NV-11) is retired (D128).
- [ ] Build markdown review (SPEC §11: SM-3–SM-16, DK-7, DK-8; D104): Preview/Source/Side by side, linked scroll, comments on source lines sent to the session's agent. Frames 39, 41–43.
- [ ] Record and show last used per session (SPEC SE-26, D99) wherever sessions are listed.
- [ ] Add the MCP tool `comment_reply` (SPEC DK-9), after DK-1–DK-8.
- [ ] Draw frames for comment states (addressed with before/after and Resolve, resolved by the agent, archived, the Archived list in Info) for the owner's review (SPEC SM-17–SM-21, D101).
- [ ] Build resolve/archive (SM-17–SM-21): resolved never comes back, Resolve N addressed, `comment_resolve` MCP tool with a required reply and its own agent access rule, user-only Archive with Undo and the Archived list.
- [ ] Write the TP-17 delegation rule into Humata's skills and MCP tool descriptions: mechanical calls go to a cheap subagent (Haiku / Luna) with the smallest context window and medium reasoning effort where the CLI allows.
- [ ] Follow platform behaviour: standard shortcuts, ⌘, for Settings, the native menu bar, traffic-light spacing, scroll and selection behaviour.
- [ ] Build what the docs tidy (minion J1, 2026-11-15) wrote into SPEC from the goldens and the owner's decisions: tab groups and a second agent (SE-27–SE-29, D103); the Inbox card's full context, Allow / Deny / Reply… and the narrow grant checkbox (IN-9, IN-14, D105); the board's Table, Dashboard, Depends on and requests answered on the item (BD-11–BD-16, D106); Hosts' disk column and the 5 GB clone floor (HO-3, LD-19, D107); Undo after Archive as a delayed delete (MV-15, D110); the Settings sections (ST-18–ST-20, D111). Each is Provisional until the owner confirms D103–D112.
- [ ] Get the owner's OK for the frame changes in D113 (29 without Panes, 31 without the model picker, the Inbox card in 03/04/26/27, 33, and the smaller ones), then redraw them; draw the missing frames D113 lists before their screens are built.

### Pick up here (minion FL)
- **FL (2026-11-15, manager minion FL; plan in workflow/milestones/FL.md Lane; picks D134-D137):** first light: a real session in the app that survives a reboot. Batch 1 passed (clients/live with parity 7/7 on fake and real). Batch 2 written, not built: desktop/install sha256 read loop; Cargo.toml for clients/live, desktop/install, daemon/app_peer (main repins); `DaemonError::NotServedYet` (D134); humatad publishes Session events (start, retitle, stop, resume) and Channel events (create, clone done, rename), filtered by SessionRead/ChannelRead, tests in daemon/humatad/tests/session_events.rs; clients/live synthesizes only after an attach reports a status (D136); new crate `daemon/app_peer` (audit token + code-signing requirement for the signed app, D135; FFI constants unverified until its test runs); humatad BUILD gets app_peer and HUMATA2_TEAM_ID/HUMATA2_BUNDLE_ID from @lifr_identity; `desktop/src/live.rs` (pick live or fakes, install on launch, wait for the socket, login-shell PATH check, D137).
- **Batch 3 (written, not built):** serve.rs serves through `humata2_app_peer::incoming`; identity.rs takes the signed app as the user outside every tab's tree (D135, fails closed); desktop main.rs runs live from a bundle or with `--live`/`--socket` (fakes unchanged); `session_service/watch.rs` hears agent exits and tells the feed; `desktop/tests/live_flows.rs` runs §26.2 (local) and §26.8 headless on a real humatad2_admin.
- **Committed ed5882ed** (with BUG and LOOK1). live_flows_test is `manual` until §26.2 and §26.8 pass.
- **Gate g26 fixes (written, not built):** a resumed holder numbers output from past a gap and the app dropped everything after a gap, so clients/live now maps holder offsets onto gapless app offsets (`Placed`); live_flows prints app, client and daemon state on any wait timeout (26.2's "never listed" cause comes from that print).
- **Gate g33:** 26.8 and the lost-conversation flow pass. 26.2's typed text never reached the sheet, so the session started under its default title. Written, not built: a short bounded retry for a tab that isn't listed yet, and a wait in the flow so a failure names the typing step.
- **Gate g37:** 26.2 still fails at typing. The cause is the harness: `frame::type_text` drew a frame before GPUI delivered the input's Change, so the sheet's sync reset the text. The fix is in the seam `fl-g37-typing-seam.md` (engine `type_character` and the driver drawing in its own update).
- **T9 (written, not built):** all four workers are done and their diffs are checked. The full seam list for main is section 8 of scratchpad `fl-t9-seams.md`. D143 reuses ResumeSession MODE_START_FRESH_WITH_SUMMARY.
- **T10 (written, not built; D149):** `app/` (rule `humata2_app`, Info.plist), `daemon/humatad:tools_mac`, and `tools/release/{release,check,install}.sh`. Seam: desktop:humata2 visible to //apps/humata2/app.
- **Gate g39:** all green, T10 builds. live_flows' "no reactor" panics are fixed in clients/live; the tca `SystemClock::sleep` seam is with main.
- **Exact next:** main applies the clock seam and re-gates live_flows. Then main applies the T9 seams and gates them (section 8), FL fixes compile errors, and T10 follows (bundle, signed release, install in /Applications).
- **Open:** turn ends aren't tracked by the daemon; "Couldn't resume", the restart toast and New channel progress UI after LOOK1 (T9); app bundle and release script (T10); 0.1 assumed Developer ID + hardened runtime (main confirms at release); the install and socket wait block the main thread up to 10 s before the window opens.

### Pick up here (minion BUG)
- **BUG (2026-11-15, manager minion BUG; plan in workflow/milestones/BUG.md Lane; picks in D133):** bug reports in the app and the CLI (I9). Batch 1 written, not built: `features/app/src/report.rs` (report state and reducer: recording, the form, pins, Copy outcome; a CLI save toasts "Add details"); `report/src/remote.rs` (the app socket `~/.humata2/app.sock`, 0600, `HUMATA_APP_SOCKET` override, JSON lines, stale sockets replaced); `ui/src/record.rs` (saving to `<data>/reports`, seeded env secrets, begin/finish/snapshot/copy, coalesced typing and scrolls, ancestors on targets, pins list); `ui/src/report_view.rs` (the dot, the form sheet, mouse and key hooks, ⇧⌘R/⇧⌘S, `serve` for the socket); desktop main wires the folder, the socket and Help › Report a Bug; `desktop/tests/report.rs` app flows; `cli/tests/report.rs` refusals.
- **Exact next:** main applies the seam lines (features/app lib.rs, ui lib.rs and shell.rs, the CLI seam in scratchpad bug-cli-seam.md, the desktop `report_test` target) and builds; fix compile errors in report_view.rs first (TextareaState::set_value, on_next_frame, the async window context were guesses).
- **Left open:** D133.7 "Keep This Terminal's Text in Reports" not built; no palette commands for reports; daemon RPCs aren't captured (fakes have no RPC layer); drags aren't recorded; a pick mode for pins comes later (D133.4).

### Pick up here (minion REVIEW2)
- **REVIEW, early lane (2026-11-15, manager minion REVIEW; plan in workflow/milestones/REVIEW.md Lane; picks in D138):** the daemon side of comments for the session's agent. Batch 1 (applied, passing): `daemon/review_diff/src/{comments,comment_message,passage}.rs` (the one comment model, the SM-9 message, re-anchoring and addressed), `daemon/store/src/comments.rs` (comments in tables of their own, one row per comment, only changed rows written), `daemon/humatad/src/review/{comments,sink}.rs` (CommentService for the terminal destination: create, edit, drop, re-attach, send, take back, resolve, reopen, archive; write-ahead batches; restart recovery), tests under `tests/pending/` in `daemon/review_diff`, `daemon/store` and `daemon/humatad`.
- **Batch 2 (applied, g37):** exactly once for real. The holder types a keyed input once and remembers its key, and each holder process has an instance id; a batch records its tab and holder instance before the write-ahead save; after a restart the daemon asks that holder and types again only on a certain "never arrived". Crash tests stop the admin test daemon with `HUMATA2_STOP_AT`.
- **Batch 3 (applied, gate g39 pending):** `comment_reply` and `comment_resolve` over MCP under `CommentReply` and `CommentResolve` (Default Allow, own session only), and `comments_list` served.
- **Exact next (manager minion REVIEW2):** main gates batch 3 and the crash-test fix in `daemon/humatad/tests/comments.rs`, then applies and gates scratchpad `review-lane/batch4-resolve-addressed-paging.patch` (T9: `ResolveAddressedComments`, ListComments pages). Targets are in the Lane's Build gates. Then T7: the app on the daemon's comment model, planned in the Lane as T7a–T7e (`features/review` is claimed; new files under `clients/` need main's OK; ui lines go to LOOK2), then T8.
- **Left open:** a batch typed into a holder from an older build, or whose holder died with the daemon, is Unconfirmed after a restart and the user chooses; ReplyToComment from the user answers not built (only the agent replies today); the agent's `comments_list` shows comments Humata is still holding, and over gRPC an agent can still read its session's drafts (ListComments is a read of its own session); every change in the clone counts as the agent's for "addressed"; the app's fake still leads diff comments with the repo (SM-9 says only with several repos); `features/review`, the live client and the flows wait for main's OK on app files, DESK and FL.

### Pick up here (minion MACH)
- **State (2026-11-15):** early lane. Stages A–D and H are live: the skills engine and scripted rule config in the machines crate (D140), provider adds and ProviderService (config, CreateMachine, start/stop/status/delete, DeleteWhatWasMade), skills over the link with SkillsService and sync on link-up, the app's `Machines` client with the Add machine and machine page reducers, and the interactive login with make-unattended. All gated green except `//apps/humata2/daemon/humatad:login_test`'s happy path (g38: the takeover went silent; now bounded and shown, cause not yet proven). Plan, tasks, gates and enable stages: `workflow/milestones/MACH.md` › Lane.
- **Written, not switched on:** views for frames 56–59 (`ui/src/{overlay_add_machine,machine_page}.rs`, stage E, waits for the ui files); the relay on the machine (`daemon/humatad/src/{relay,relay_auth}.rs`, stage F); the host's link registry and report sink (`{host_relay,peer_reports}.rs`, stage G); Their tests wait in `tests/pending/`. Exact enable lines for F, G, H: session scratchpad `mach-wiring/RELAY.md`; if that file is gone, the workers' enable lists must be rebuilt from the code's module docs.
- **Decided:** D139 (the peer link relays an allowlist of client calls to the machine's socket daemon; the host is seen there as the user) and D144 (the relay proves itself once per socket connection with the host's own ProveHost proof; the readable enrollment key is accepted until SAFE moves to an Ed25519 pair).
- **Next:** fix `login_test` from its next gate's output; then E (when the ui files are free), F, G, then stage I: the remote channels, sessions and terminals written in `daemon/humatad/src/{remote_work,remote_terminal}.rs` (D150), whose seams for `daemon/sessions`, `channel_service.rs`, `session_service.rs`, `terminal.rs`, `identity.rs`, `serve.rs` and a store table are in the scratchpad's `mach-wiring/REMOTE.md`; the flows §26.1, §26.2, §26.9, §26.21 (`daemon/humatad/tests/pending/remote.rs`) pass only after that; T10 (host variables, TrustNewHostKey, storage calls from FILES, launchd for macOS machines, a watcher for host skill changes, scripted rules over the link); CLI boundary tests.
- **Traps:** BUILD globs turn every `tests/*.rs` into a target and `features/hosts` feeds `ui`, so new code stays in unreferenced files and new tests in `tests/pending/` until gated. Before stage F, a link started with a temp `--state` and no `--socket` must not reach the real `~/.humata2/humatad.sock` (pick in the wiring note). The daemon gives provider commands no host variables yet. The proto's skills manifest has no executable bit or set id; first-sync clashes can't be previewed without a new RPC.

### Pick up here (minion DESK)
- **DESK (2026-11-15, early lane, manager minion DESK; plan in workflow/milestones/DESK.md Lane; picks D141, D148):** the Desk without changing how any existing view draws until LOOK2 releases its files. Built and passing (gate g37): the client contract, fake and fixtures (`clients/src/desk.rs`, `daemon/fake/desk.rs`, `fixture/desk.rs`), the `features/desk` crate, the app wiring (`features/app/src/desk.rs`, `Route::Desk`, `Clients.desk`), the sidebar model's Desk rows, the last-used hooks and the reserved `desk` channel name.
- **Written since g37, not built:** D148 in the model (a scratch folder is named from the title the session starts with and the dialog shows the path it will get; the sidebar lists every open Desk session in start order); pop-up state for the views (`Menu`, `sheet_open`); a toast for a failed change off the Desk page; and T7's views as inert new files: `ui/src/desk.rs` (frame 36), `ui/src/desk_bar.rs` (frames 37–39), `ui/src/overlay_desk{,_new,_clean_up,_copy,_add_repo}.rs` (frames 40, 55, Copy, Add repo), `ui/src/sidebar_desk.rs`.
- **Exact next:** main gates the D148 edits, then applies the T7 seam lines in DESK.md's Lane once LOOK2 releases `ui/src/{sidebar,pages,session_tabs}.rs`, builds, and records the native frames (each gains the Desk row). Then T7e: native frames 36–40 and 55 from flows, the Desk in `Clients::demo`, Home's New desk session and Use the Desk (frame 34), Board panel on All channels, Info switches, the Inbox's Desk group, palette entries, the keyboard cursor over the Desk rows; T8: W1's path check and the release on fakes.
- **Open, for the owner (main shows before/after):** frame 40's caption `plan-q4` → `plan-q4-board`; frames 36–43 and 55 gain "Tidy infra items" under Desk (D148).

### Pick up here (minion C1 terminal experiment)
- **Owner redirected assignment (2026-11-15):** build a separate reusable real-terminal experiment; LOOK1 now owns the previous app/frame work below. New terminal/session is the UI-independent local/SSH holder client and disk journal; terminal/embed is the GPUI component; terminal/lab is its standalone host. Local PTY tests4 passed (two shells/environment isolation, resize, persisted group movement, detach/reopen, dead-holder history, lock and torn-tail handling). Real SSH test passed against a new dedicated Lima VM h2-terminal-c1 (1CPU/768MiB/4GiB); existing VMs untouched. Standalone app/headless interaction build is in progress. First SSH launch hung on inherited pipes; replaced it with bounded detached startup and independent nonmultiplexed tunnels, then SSH test passed. No main-app UI integration or process resurrection after reboot is claimed. Exact next: finish headless app flow/captures, harden failure/replay cases, document/publish experiment. Changed only new experiment packages plus narrowly scoped visibility in holder/pane/view BUILD files; do not stage concurrent pane.rs or other agents' app work.

### Pick up here (minion LOOK1)
- **Batch 6 (T15, 2026-11-15, re-audit of 01–17):** written, not built. Rail pitch 51 pt, diff gutter gap, CircleX registered (05 exit icon), frame 06 laid out before the link drops, frame 12 asking card with Review and no Done fade on the full board, 14 draft, 15 queue strip, 16 opens the fork, selected rows semibold, terminal scrollbar hidden at the newest output (L16). Decisions L16–L17; non-blocking polish moved to workflow/milestones/LOOK2.md. Exit claim pending these captures.
- **LOOK1 (2026-11-15, manager minion LOOK1; plan and decisions L1–L9 in workflow/milestones/LOOK1.md):** Batch 1 (D126) written: comments are an explicit send. A Comments message goes as soon as the session takes input, whatever the turn. Composer text waits for a hook-backed turn end. Review has SendDraft(i) (Send on each draft card) and SendComment (send the comment being written); batch Send stays. Removing or editing the last waiting message clears the failure pause. Batch 2 (frame audit 01–17) written: VIP-only sidebar inbox count with forks rolled up; fork rows 2 pt left; pinned row 36 pt, selected highlight; diff comment chips, Open in editor (disabled), 1-cell sign column, Diff rail count = sent + drafts + waiting; Info agent name, kv spacing, clone order + chips; Files repo order, A/M letters, doc/code icons; terminal font 11.5/18 at the default setting; fake TUI orange lines and input box with hint; offline notice in flow; exit notice/offer banner/queue bar restyled; no "others can type" when split. Fixture data for 01–06/15–17 in progress. Batches 1–2 committed (e07e8653). Batch 3 (T12, T10) written, not built: TUI input box pinned at the bottom, notices in flow under the output, offer banner 40 pt, peers only while Working, panel metrics, board attention card, 07/08 native captures, per-draft Send in goldens 01/11, fake agent redraws on resize, "⎿ " keeps its space, diffs clip by default; D131: a sidebar dot is the reported run status; forks' requests only feed the badge. Batch 3 committed (13121e1d). Batch 4 (T13, exit audit) written, not built: compact board order and golden 07 data (L14), Inbox 7 with Ledger Working, Diff rail 2 in the lifecycle frames, panes lay out early output at their drawn size (L15). Batch 5 (T14) written, not built: model fields Machine.lost_at, Session.model/restart_detail/permissions, PreparedView.note/prepared, wired into the offline notice, Info and a VW-10 Journal of prepared views. **Exact next:** main builds and records batch 4 (targets in the Lane), then fix failures and re-audit 01–17; model fields done in T14; open items in the Lane T12 need claims or ASK model fields. Left open: Files filter height (inside gpui_kit Input); exact 7.2 pt terminal cells (terminal/view rounds cell width); offline notice time needs a real daemon source for Machine.lost_at.
- **Done:** extracted the panels' D76 light/dark palette into `ui/src/theme.rs::Look`; removed the icon rail and Humata title; added the 236px sidebar, traffic-light close/minimize/zoom controls, search/navigation, indented forks, Hosts/Settings rows, session breadcrumb/header, terminal typography and existing tab selection. The demo now uses `clients::fixture::design()`; existing behavioral fixtures remain available as `standard()`. `bazel build //apps/humata2/ui //apps/humata2/desktop:humata2` and both `desktop:native_test_record` tests passed.
- **Current verification:** App messages9, Session queue4 and fake-client atomic ambiguous-path test passed. Full native35/panels12/shell2 records passed after Waiting to send wording, and the added native focus-isolation regression passed (36 native tests now). Final required23-target gate passed (52 total targets), including native36. Verified checkpoint9e185a5b is paired with its hash-fill log for shared main; no C1 test process remains. Latest light/dark waiting-comment and expanded waiting-message captures inspected. Earlier missing design44 reference is resolved; do not record or stage any design sources/PNGs from C1. Published previous C1 pair491a1cdd/7c5de8fe; review checkpoint9e185a5b is recorded in the handoff/log. Session wrapped at owner request; individual/direct send is next. No real daemon edits.
- **Frame audit:** 01 partial (file/plus tabs and per-terminal writer label now present; fake terminal prompt/tones and panel counts/actions differ; input/VT bridge now works); 02 partial (same, dark); 03 partial (offer/tabs and working queue bar present; terminal tones and Inbox preview styling/extra fixture request differ; Not now shown per VW-2); 04 partial (same, dark); 05 partial (tabs/Resume/exit notice present; header truncation and Info metadata differ); 06 partial (tabs/disconnected notice present; Files paths/order and fixture host status differ); 07 partial (existing Board panel; attention/sample detail differs); 08 partial (existing Journal; sample events differ); 09 partial (file/plus row, per-terminal presence and no-panel layout recorded; fake prompt/tones differ; VT input works); 10 partial (full-screen behavior/tab row done; terminal gaps remain); 11 partial (full-screen behavior done; draft/count/toolbar differ); 12 partial (expanded lanes/PAY items present; attention/created-by/lane detail differs); 13 partial (restore strips/tab row present; terminal gaps remain); 14 partial (two groups/plus/attach/join recorded; divider sizing, 12px vs 11.5px terminal type, padding, tones and extra prompt/tail differ); 15 partial (offer/tabs/Pinned 40px summary present; PR row and icon work in 15/25; summary truncation/spacing differ); 16 partial (indented selection and tabs present; real navigation opens child while mock keeps parent); 17 partial (folding/rollups/tabs present; sample attention state differs); 18 partial (tree/filter/fold/count actions verified; filter height and sample counts differ, header pin/menu absent); 19 partial (typed feed + Review/Diff verified; header pin/menu absent and button border/width differ); 20 partial (derived clone grid/filter links verified; add repo/machine disabled, header weight/alignment/counts differ, host link opens general Hosts); 21 partial (kanban/new/status/approval flows verified; toolbar arrows/input height/button borders and tiny spacing differ); 22 partial (drawer/status/dependency/approval verified; chip/toolbar spacing differ, owner/assignment/edit/start-session gaps); 23 partial (grouped table inspected, row/drawer flows verified; toolbar and Review borders differ, group folding not wired); 24 partial (dark dashboard counts/chart links/Resume verified; only actual 9-item counts/history shown versus inconsistent mock totals, session order/extra completed session differ); 25 partial (grouped rows/toolbar, Review/Resume, Pinned summary and PR row implemented; working count/order, fork wording and recent action details differ); 26 partial (split list/details/history and scoped answers work; two extra fixture requests, preview styling, scoped grant/reply/bulk/log controls remain); 27 partial (same, dark); 28 partial (table/health/bridge/selection/reconnect/provider Start/shell launcher verified and captured; fake SSH add now works; free-space and richer machine management missing, actual session counts differ); 29 partial (native sections/theme/font/pinned/wrap/default agent and group display verified/captured; font uses stepper, pane text follows bounded D76, access/storage/notifications/key/model management incomplete); 30 partial (grouped palette/context commands/scoped files verified/captured; focused input outline, extra actual matches and shortcut chip styling differ; recent rank within groups); 31 owner-revised (agent-only dialog and no model override verified/captured per direct chat; repo toggle vs popup, leading machine arrow and focus outline differ; real process launch outside this fake-client pass); 32 partial (success-based archive/Undo preserves output/pin/tree and failure stays open; inspected capture; no invented freed-size claim, underlying Home/sidebar fixture differences remain); 33 partial (layout/copy/actions verified/captured; button widths/border differ slightly; SSH add is a fake host, discovery/install progress remains).
- **Files/helpers:** Session ShowQueue/SendQueued and failed_queues support the visible message list. Saved queue payloads remain until acknowledgement, edits/removal are guarded while pending, errors pause automatic retry; only Claude/Copilot hook Ended flushes. Workspace terminal_peers reports other input-enabled attachments, excluding this app/read-only; tab rows show only the selected terminal count and hide offline. Native queue tests own fake failure injection; widgets error banner constrains text and keeps Dismiss reachable/readable.  `ui/src/code_wrap.rs` expands four-cell tabs and wraps whole graphemes into display/source-range fragments, caps deep indentation at half the viewport, and retains original bytes for semantic text. Panels measures Menlo cells with WindowTextSystem and top-aligns gutter/sign, scaling rows with preferences; Scroll has a longest-line minimum width. Existing pinned Unicode dependencies added to UI only.  `terminals::focus_target` identifies route/selected tab/full-screen; Shell focuses a newly selected terminal (retrying if its snapshot is not here yet) or root, while leaving same-target text editing alone. Overlays restores prior focus only for the same target. Each pane has a deferred focus observer that dispatches Session FocusTerminal, selecting the active split group without changing its tabs.  Overlays renders Session fork state with its own title input, kind buttons, preview, pending/error state and focus trap; bridge observes fork/pending/error. Session forking guards duplicate actions and retains failed drafts. Fake forks use the existing serialized kinds (Worktree=Code+context, Conversation=Context only, Shared=Code only), clone listed file text, keep/clear conversation by kind, reset extra terminals/archive/times and emit typed activity.  `ui/src/terminals.rs` caches a TerminalPane per session/tab, hides rather than destroys it, updates theme/font only as needed, forwards raw keys/pastes/resizes through one client queue, and exposes grid text for flows/accessibility. `Terminals::input/resize` and fake ANSI byte history/events supply the transport; Session terminal_bytes deduplicates replay/overlap and rejects gaps. Fake initial output covers original/new/forked programs. UI no longer has the transcript renderer or drawn prompt. New-session model input/state/action/default override removed by owner direction.  Palette context/file commands in features/palette and root; overlays owns model/textarea inputs. NewSession options persist included repos/model/branch/first message/item in fake Workspace; channel_machines unifies valid-host defaults/validation. Fake restore retains newer output and tree metadata; root toast action retries on error. Hosts add-address state/form and first-run actions use fake add_machine only. Native30–33 record real views.  `ui/src/hosts.rs` and `settings.rs` render 28/29. Hosts Selected persists in SavedUi; MachineHealth reports disk/tools/system/link data. Settings adds Section, System theme, show_pinned and split_groups with old-save defaults; Shell applies appearance preferences and session_tabs hides the second group without destroying it.  `ui/src/board.rs` renders kanban/table/dashboard/drawer and owns GPUI input handles; framework-free Board View/Mode/Filter saves through SavedUi. `Workspace.item_details` supplies fake descriptions/history/ownership/approval/dates; fake answer applies only the linked requested status.  `features/app/src/channel.rs` holds per-channel saved presentation and ancestor-preserving tree rows; `ui/src/channel.rs` draws Sessions/Activity/Repos, scoped Inbox and temporary Board/Access links; Shell owns the filter input. `Workspace.activity` supplies typed fake feed rows.  `ui/src/lists.rs` draws Home and wide Inbox with route toolbars; existing request card is reused. `features/inbox` has History and first-visible selection; Session.activity and design fixture request/pull metadata drive rows.  `features/session/src/tabs.rs` owns the saved one/two-group model; `ui/src/session_tabs.rs` draws the shared row, plus menu, file/view content and offer. Fake-client `Terminals::open` deduplicates agents, independent shells; `Daemon::read_file` reads this session/repo/path, with explicit errors. `Workspace::file_contents` is fake host text.  `ui/src/chrome.rs` provides actionable title-bar controls, icons and status dots; `ui/src/theme.rs::Look` shares the approved panel palette; `clients/src/fixture.rs::design()` supplies the approved sample workspace; `desktop/tests/native.rs` drives the real shell, navigation and folding and records full-window comparisons; `desktop:native_test`/`:native_test_record` are its verify/record targets. Existing `ui/src/panels.rs` remains the panel implementation; its palette was extracted, not redesigned.
- **Commands:** `export PATH=$PWD/.tools/bin:$PATH`; `bazel run //apps/humata2/desktop:native_test_record`; compare `desktop/tests/__snapshots__/native/NN-*.png` with approved `desktop/tests/__snapshots__/design/NN-*.png` using image inspection. Also record shell and panels with the targets above. Use Bazel-managed rustfmt on touched files only. Never record design_test.
- **Traps:** parent child reductions are synchronous: expect the incoming delegate and inspect resulting state rather than wait for a child action that was never emitted. A concurrent missing44 design reference caused an earlier gate failure and is now resolved. Never record design_test from C1.  headless windows start inactive and suppress GPUI focus observers. Activate the headless window for focus tests only; this does not open/focus a real window. Use a scoped terminal canvas inside the group for unambiguous split clicks.  constrain Pinned summaries to one line and non-shrinking 40px rows; otherwise the 2m suffix wraps over the next row. `git diff --check` across the whole worktree may report minion F daemon whitespace; never fix/stage that area.  snapshot record normally retains pixels within its tolerance, which hid small tab changes; native record now uses EXACT while verify keeps normal tolerance. The old panel Files flow assumed Open big widened the pane; updated it to select/open a file tab and return to terminal.  headless `window.resize()` also needs `window.bounds_changed(cx)` to update the GPUI viewport. Resume changes the fake session to Idle, waiting for input. Format only touched files with `--config skip_children=true` to avoid recursive module formatting.  the initial `App::appearance` probe failed; use `window_appearance` and observe window appearance changes. The tree initially shared spare height with the bottom spacer and clipped rows after infra; it now has a shrinkable content-height allocation and retains virtualization. Uniform rows need an explicit width for a right selection inset. Pinned rows belong before navigation. Another agent's shared-doc rewrite removed this uncommitted pickup block once: keep a current copy in `/tmp/c1-pickup.md`, reinsert as needed, and stage shared docs from HEAD plus only C1 changes.
- **Boundaries:** other agents' daemon, ui-prototype and .claude edits are untouched. Headless only. No dependency repins, lockfiles, rustfmt.toml, design sources or approved design images changed. Git commands use `-c lfs.storage=/tmp/humata2-minion-c-lfs`. The shared Bazel server may be busy; wait without stopping it.
- **Open owner questions:** D114 is the Provisional expanded queue list, extending03/04 inline with per-message Send now/Remove and a200px cap; captures sent, no approval stop.  D102 is a Provisional Fork-dialog layout using31 framing because no approved Fork frame exists; inspected light/dark captures are in native/. Advanced point/agent/host controls remain absent rather than pretending to work.  frame29 still offers four/unbounded panes despite D76 and this task explicitly limiting the session to two tab groups. Built Fixed/Simple for one/two groups and omitted Free, retaining the approved section layout; no general workspace added.  frames 01/02 show Working in the header but Needs you for the same session in the sidebar; the app uses the reported status consistently (SPEC SE-2). Frame 16 keeps the Ledger header when a child is selected; real navigation opens the child (SE-6). Preserve behavior while recording these mock-data differences. No blocking owner decision or new Provisional pick yet.

- **Terminal audit status:** TerminalPane/ghostty_vt now renders the fake ANSI stream and sends real encoded input/resize through clients, retaining history across tab/group switches. Fake prompt is inline and tones/type/banner differ from approved stills; no real PTY or agent process is spawned. Raw history in the fake Workspace/Session state is currently retained in memory; real snapshot/ring transport, richer fake TUI editing, tab rename/title/pwd/clipboard-read/paste-file/image actions and direct offline-input embedding coverage remain. Native key/find/resize flow and standalone pane tests cover the wired path. Frames34–43 are newly approved/not started in the real app.

## Known gaps (2026-11-15)

**Keep this file current.** Everyone working on Humata 2, agents included, takes work from this file, and in the same commit as the code: ticks what is done and verified (with the Bazel target that proves it), splits partly done items to show what's left, and adds newly found gaps. Each commit also gets an entry in `GAPS_REVIEW.md` so it can be reviewed.

### Pick up here (minion LOOK2)
- **LOOK2 (2026-11-15, manager minion LOOK2; plan, claims and tasks in workflow/milestones/LOOK2.md Lane; picks and data rulings in D147):** every other screen looks like goldens 18–35. Batch 1 and the splits passed gate g34: the pages in `ui/src/{channel,board,lists,pages,hosts,settings,overlay_palette,overlay_new_session}.rs` follow their frames' drawing code; `ui/src/panels.rs` is a root plus `panel_*.rs`; `desktop/tests/native.rs` is the harness plus `desktop/tests/native/*.rs` by screen.
- **Batch 2, written and checked, not built:** no Panes switch (D111: `split_groups` gone); empty states (sidebar for an empty workspace and an empty channel, captures 34 and 35, 33 retired); design fixture data per the advisor's rulings; Pin, "…" and Add a GitHub repo… drawn disabled; ⌘, for Settings and a Shortcuts sheet that lists only bound keys; Home rows name their machine (HM-1); FL's restart views wired into the session page and Home.
- **Exact next:** main applies the seams in the scratchpad `look2/seams/seams_for_main.md` (the last-used script, the ⌘, handler, the Settings… menu item), gates batch 2 (targets in the Lane), deletes `native/33-first-run.*` after recording 34 and 35, and takes `look2/seams/golden_wave.md` to the owner. Then T14 (one shared button helper, Files filter height), T17 (Edit and Window menus), T18 (re-audit 18–35 on fresh captures; fakes release).
- **Open:** frame changes waiting on the owner (D147: Home 25, channel 18–20, Hosts 28, palette 30); the toast's "Frees 1.4 GB" needs the archive reply to carry the size (`look2/seams/seam_toast_frees.md`); Desk pieces wait for DESK; the Inbox card's inside is ASK's; Free space… is MACH's.

Work that was started but isn't finished, grouped by area. Each item is open until its tests pass and a real run works. Run builds and tests only through Bazel (`export PATH=$PWD/.tools/bin:$PATH`), never cargo.

### Pick up here (minion F)
- Done: review chat Undo and idempotency keys across a restart (D86).
- Done: draft views in ViewService, the store and MCP (D87); open: their push to the app, include_forks and a CLI.
- Done: a note going outdated or moving is a WatchReviewChanges change (D88).
- Done: views on the event feed (D89).
- Done: Inbox requests on the event feed (D90), grants made and revoked (D91), and review chat notes (D92).
- Done: MCP `inbox_request` (D93).
- Done: operations on the event feed (D94). Sessions and channels have nothing to tell: SessionService and ChannelService aren't built in humatad2.
- Done: `by` on each event (D95).
- Done: the Inbox and grants kept across a restart (D96).
- Done: `humata views` and `humata view` (D97). Not doing include_forks: nothing records a fork's parent until forking is built.
- Done: an agent's notes feed follows ListComments (D98).
- Nothing to tell yet for grant suspensions or session-ended grants: no RPC sets a channel rule (`Policy::set_rule` has no caller in humatad2) and DeleteSession isn't served. Tell them on the feed when either is built.
- Not a gap: the views Journal is ListViews (VW-10). `PreparedView.journal` is the machine-journal position (SL-5), unset because nothing is journaled yet.
- On (minion F): reporting back. Waiting on an owner or proto call: Ask→Inbox (D38.6; no RPC can answer "pending" with a request id yet, and approval must run the held call, MCP.md TP-13); `last_start` push (no Event payload).
- Trap: `//apps/humata2/daemon/humatad:machines_test` (`the_feed_tells_each_machine_change_...`) can fail under the full parallel run; it passes alone.
- Traps: OVERVIEW, GAPS_REVIEW and DECISIONS may hold minion C1's uncommitted edits; stage only my hunks from `git show HEAD:<file>`.

### Desktop UI (I2, A10; D74 as revised, D76)
- [x] ~~Render the pane tree~~: built in a7c6cf3c, then dropped on the owner's direction (2026-11-15, D74 revised). The window is a left sidebar, the screen in the middle, and the open session's side panel on the right.
- [x] Drag the sidebar's and the side panel's dividers to resize them (past the narrowest hides them; a double-click resets them); ⌥⌘0 and the palette's "Toggle side panel" hide or show the panel. Each session keeps its own panel when you switch screens and come back. Test: `desktop:shell_test` (`a_sessions_side_panel_hides_shows_and_survives_switching_screens`).
- [x] Diff, Inbox, Files, Board, Journal and Info show fake-client data in the D76 panel framing, in light/dark appearances. Diff selects a repo/file per session and sends queued drafts; Inbox includes descendants and answers the chosen request; Files filters/folds/selects the session's clone paths; Board scopes to the channel and expands into lanes; Journal scopes to the session; Info shows its agent, host, start, forks and clones. The labelled strip, header toggle and panel full screen work. Tests: `//apps/humata2/desktop:panels_test` (12 tests, 16 inspected image references), `//apps/humata2/desktop:shell_test`, `//apps/humata2/desktop:design_test`.
- [x] Files and Info follow approved frames 06 and 05, respectively; the earlier claim that these frames were absent was incorrect. Files uses a per-session clone listing in the existing fake Workspace, including unchanged files, rather than leaking host-wide paths. View state survives switching and older saved sessions still load. New fake forks inherit their parent's listing. Tests: `//apps/humata2/desktop:panels_test`, `//apps/humata2/clients:fake_daemon_test`.
- [ ] Files Open in editor/Upload and Info Channel rules/restart details (minion C: the existing clients lack editor/upload operations and permission/resume-command data; unsupported controls stay disabled and missing data says it has not been reported).
- [ ] Session-only Inbox grants and the Reply editor (minion C: the client supports only the broader Always rule, which must not stand in for a session grant; no approved reply-editor frame is recorded. The checkbox/Reply button remain disabled, and questions never offer Allow).
- [ ] The Diff base picker, Open in editor and comment composer still need their clients/views (minion C: this pass renders the current base and existing comments/drafts only; the excluded review_diff wiring and a selected editor/comment layout are needed before those controls can be made live).
- [x] A native headless mouse drag checks the panel divider with its 64px strip outside the width: a 40px drag gives 380px, instead of jumping to the 420px maximum. Test: `//apps/humata2/desktop:panels_test` (`resizing_the_panel_keeps_the_strip_outside_its_width`).
- [ ] Cover the sidebar's drag too, and check both dividers in a real window (minion C: the assigned run is headless only).
- [x] Removed `features/layout`, `AppState.panes`, the saved `panes` field (old saved files still load; serde ignores it) and the Reset toast's Undo button, on the owner's call (2026-11-15). Test: `desktop:shell_test`.
- [ ] E2e flows, contact sheets, record/replay and FLOWS.md. Settings › Layout keeps Appearance, Terminal font, Sidebar shows Pinned and Long lines, and loses the Panes choice once the owner OKs redrawing frame 29 (ST-18, D111, D113). There is no D72 entry and none is owed: D74 (as revised) and D76 cover the desktop UI.
- [x] Snapshot fixes, from before the rail was removed: the Inbox badge overlapped the rail icon, and Home's row times stop short of the right edge.

### Markdown (`apps/humata2/markdown`)
- [ ] Fix `headless:flows` (1 of 7 still fails, `bazel test //apps/humata2/markdown/headless:flows`):
  - [x] Toggling Source and back with the keyboard: `toggle_mode` moves focus to the new text view.
  - [x] The selection toolbar's buttons act on press, using the selection cached when the toolbar was drawn, because pressing clears the text's selection first. Copy now reaches the clipboard.
  - [x] Copy as Markdown: flow targets can aim at a control's start (`Target::at_start`). A selection starting at a block's first visible character takes in its opening markup, and one ending at its last takes in the closing markup (`with_block_markup`), so it copies as `# Gallery`.
  - [ ] Comment on selection (the app no longer needs this: comments are made on source lines, SPEC SM-6, D104; it stays open only for the markdown crate's own flow): dragging paragraph 2 → heading 3 gives no toolbar, because gpui-base's `SourceRangeSelection::merge` turns the whole selection Unmapped when one block can't map (`text/node.rs` in gpui-base 0.9.1). Tried 2026-11-15 and reverted: a patch making `source_range_for_segments` bridge rendered gaps didn't bring the toolbar back, so the Unmapped comes from somewhere else (a block-level Unmapped, the `first.rendered.start` path, or the paragraph's `selected_source_range`). The repin it needed also changed code and inline-code rendering in `headless:snapshots`. Next: log which block returns Unmapped before patching again.
- [x] Inspected both Markdown image goldens and the current render, recorded `headless:timing` figures and limitations in D78, and ran Bazel-managed buildifier. Timing target: `//apps/humata2/markdown/headless:timing` (`-c opt --test_arg=--nocapture`).
- [ ] The two Markdown pixel checks differ from their stored goldens by about 19,700 pixels each: current fenced/inline code is monospace, stored references use proportional text. Semantic snapshots and streamed/one-shot pixels pass (minion C: left references intact for owner review; no rendering change was made here).
- [ ] Margin comments clip long text horizontally in the annotated gallery (the app shows comments inline under source lines, SPEC SM-8, so this concerns only the markdown crate's gallery; minion C: observed in the stored golden; needs a layout choice before restyling).
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
- [x] humatad serves CommentService's `note` destination (CreateComment, EditComment, DropComment; notes are the agent's own, anchored in the clone, only on files the last diff showed, off when the user turns notes off) and ViewService's PrepareView with a review plan, which in an open review chat (D80) applies through `ReviewChat::prepare`. Test: `//apps/humata2/daemon/humatad:review_test` (`notes_are_the_agents_and_the_user_can_turn_them_off`, `a_plan_in_an_open_review_chat_orders_the_review_and_leaves_its_notes`).
- [x] ListComments lists the review chat's notes for the session named (an agent: its own), each with its session, author, anchor and outdated flag, filtered by file; a status filter lists none, since notes are never sent (minion E, D85). Test: `//apps/humata2/daemon/humatad:review_test` (`list_comments_gives_the_review_chats_notes_by_file`).
- [ ] The terminal and PR comment destinations answer not built.
- [x] Draft views: a session's agent prepares a walkthrough, a board of its channel, pull request drafts or (outside an open review chat) a review plan with PrepareView; it alone sees and revises its drafts (ReviseView, `expected_revision`) until MarkReady offers one as Waiting; then only the user answers with SetViewState. The store keeps each view and its idempotency keys (`views` table), so a restart keeps drafts, offers and answers and a repeated key makes no second view (minion F, D87). MCP `view_prepare`, `view_revise`, `view_mark_ready` and `views_list` call them. Tests: `//apps/humata2/daemon/store:views_test`, `//apps/humata2/daemon/humatad:review_test` (`a_draft_view_is_the_agents_until_offered_then_the_users_and_a_restart_keeps_it`, `a_draft_view_is_checked_before_it_is_kept`), `//apps/humata2/daemon/humatad:mcp_test` (`an_agent_prepares_revises_and_offers_a_view`).
- [x] Views are on the event feed (TOPIC_VIEWS): each change to a view is an Event with the view, which the user gets once it's offered and the session's agent gets from its draft on; a feed narrowed to channels or sessions gets only theirs (minion F, D89). Test: `//apps/humata2/daemon/humatad:review_test` (`the_feed_tells_the_agent_of_its_drafts_and_the_user_of_offers_only`).
- [ ] An offered view's `journal` (its machine-journal position, SL-5) is unset, since nothing is journaled while the host is away yet; ListViews ignores `include_forks` (no session records its forks yet: forking isn't built) and pages; views live on the daemon that runs the session, not the host (VW-12; SPEC keeps views on the host, SL-1, so this is a gap in the code) (D87).
- [x] `humata views [--session s]` and `humata view open|later|not-now|revert|keep <view>`; with no session the user gets every session's offered views, newest first (minion F, D97). Tests: `//apps/humata2/cli:views_test`, `//apps/humata2/daemon/humatad:review_test` (`the_user_lists_every_offered_view_newest_first_without_naming_a_session`).
- [ ] CommentAnchor has no hunk target, so `review_note`'s `hunk_at` becomes a one-line note on that line (D80).
- [x] Each GetDiff marks notes outdated whose quoted lines changed, and moves the ones whose lines only moved; the review is saved, so a restart keeps it (RV-34, PR-26; minion E). Test: `//apps/humata2/daemon/humatad:review_test` (`a_later_diff_marks_notes_whose_lines_changed_outdated_and_moves_the_rest`).
- [x] A note a later GetDiff marks outdated or moves is a change on WatchReviewChanges (tool `diff`, the note as it now is, no undo id), saved with the review (minion F, D88). Tests: `//apps/humata2/daemon/humatad:review_test` (`a_later_diff_marks_notes_whose_lines_changed_outdated_and_moves_the_rest`), `//apps/humata2/daemon/review_diff:review_diff_unit_test` (`a_note_whose_lines_changed_is_marked_outdated_once`).
- [ ] The app's review pane must show a `diff` change without an Undo button (D88).
- [x] MCP dispatches `review_context` and `review_focus` to ReviewChatService for the caller's own session. Test: `//apps/humata2/daemon/humatad:mcp_test` (`the_review_chats_agent_shows_the_user_a_place_and_reads_it_back`).
- [x] MCP dispatches `review_note`, `review_note_edit`, `review_note_drop` and `review_prepare` to CommentService and ViewService. Test: `//apps/humata2/daemon/humatad:mcp_test` (`the_review_chats_agent_leaves_notes_and_orders_the_review`).
- [x] The store keeps each session's review (`reviews` table, a `ReviewState` blob): the chat's view, notes and switches, the last shown diffs, the last change and note numbers and whether the view was reported, so a restart keeps the review chat, never reuses a note id, and Mark reviewed still snapshots what was shown. Tests: `//apps/humata2/daemon/store:reviews_test`, `//apps/humata2/daemon/review_diff:notes_test` (`a_restored_chat_never_reuses_a_note_or_change_id`), `//apps/humata2/daemon/humatad:review_test` (`the_agents_notes_are_kept_through_a_restart_and_never_reuse_an_id`).
- [x] Undo of a review change made before a restart, and the idempotency keys of notes and plans, survive a restart: the review keeps its last 256 changes' undo history and each session's last 256 keyed answers (focus, notes, plans, Mark reviewed) in its saved state (minion F, D86). Tests: `//apps/humata2/daemon/humatad:review_test` (`undo_and_a_repeated_key_act_the_same_after_a_restart`), `//apps/humata2/daemon/review_diff:review_diff_unit_test` (`a_chat_restored_with_its_history_can_still_undo_and_knows_what_was_undone`, `only_the_newest_changes_can_be_undone`), `//apps/humata2/daemon/store:reviews_test`.
- [ ] DiffService on the peer link (a session on another machine) is still not built; the socket's daemon serves it. Not straightforward (minion B): the peer-link process holds neither the store nor the clones, so it needs a relay to the socket's daemon, as TerminalService and FileService do.
- [x] `GetDiff` with `other_session` compares each repo with the commit the other session's clone is on, fetched into this clone as `refs/humata/other/<session>`; the caller must be able to read both sessions' clones (minion E, D85). Tests: `//apps/humata2/daemon/review_diff:snapshots_test` (`another_sessions_commit_is_fetched_to_compare_against_and_its_clone_is_left_alone`), `//apps/humata2/daemon/humatad:review_test` (`a_diff_against_another_session_compares_with_the_commit_its_clone_is_on`).
- [ ] `other_session` across machines, and with the other session's uncommitted changes (D85).
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
- [x] ⌘W is settled: NV-30 (close pane) left SPEC with the pane workspace (D74 as revised), so ⌘W closes the active tab and never stops a program (D73, SPEC NV-37). Nothing to build.

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
- [ ] Show `last_start`'s lines (and a restore's losses) in the app (PER-29, PER-15). No golden draws the restart report line or "Couldn't resume" yet; they are among the frames still to draw (D113).
- [ ] A resume redone after a crash skips the "you were interrupted" note when the conversation already holds it, and then only checks that the agent came up, stayed up and opened no other conversation (D71). Find a stronger check if one exists.
- [x] Rechecked `terminal_test`: 2097152 is `SNAPSHOT_MAX`, a whole 2 MiB snapshot sent as one chunk, past the decoder's 1 MiB `CHUNK_MAX`; snapshots now go in pieces (5c795563), covered by `a_big_snapshot_comes_in_pieces_to_a_client_that_decodes`. Passed 65 runs (`--runs_per_test=40 --local_test_jobs=16` plus 25).
- [x] Fixed the flaky `humatad2_unit_test`: its idle-daemon test counted wakes while the start's background reconcile (and, before 71c9071b, file events for the last good copy) could still land in either window. It now waits for the restart report first. Passed 40 runs at 16 jobs.
- [x] The Unauthenticated seen once in `restart_test` (and under load in `review_test`): the start's background reconcile asked for every saved tab's variables, and doing so revoked the tab's token and gave out a new one, even for a tab whose holder still answered. So after any restart a reattached tab's agent lost its token, and a tab opened just after a start lost its own if the reconcile listed it. Now only a holder the reconcile starts gets new variables (minion B). Test: `//apps/humata2/daemon/humatad:mcp_test` (`a_tab_that_outlives_a_restart_keeps_its_token`). Passed 40 runs each of `daemon_test`, `restart_test`, `review_test`, `mcp_test` and `daemon/machines:takeover_test` together.
- [x] On macOS a killed daemon's socket still takes connections for 3–12 ms after it's reaped, so the tests' `Daemon::restart` could reach the dying daemon (broken pipe) or start one that took it for a live daemon and stopped. `Daemon::kill` now waits until the socket refuses. Seen 2 in 40 `daemon_test` runs before; 0 in 40 after.
- [ ] The same window in the product: a daemon started within milliseconds of the last one's crash stops with "another humatad2 already answers". The service manager's restart delay should cover it; check when launchd or systemd restarts are wired.

### Machines (I7, A3; D61)
- [x] Restart the service only when a file changes or it isn't running the way setup starts it: the check reports `running` (systemd: our unit, lingering, enabled and active, no boot entry; setsid: the boot entry, the daemon, no unit), and setup skips the service step when nothing is pending and that matches. Tests: `daemon/machines:fake_ssh_test` (`reconnecting_with_the_same_build_installs_nothing`, `reconnecting_starts_the_service_again_only_when_it_stopped_or_a_file_changed`), `daemon/machines:machines_unit_test` (`the_check_sees_how_the_daemon_runs_after_each_start_and_after_removal`).
- [ ] `daemon/machines:takeover_test` `a_bridge_that_never_syncs_times_out_with_the_end_of_its_output` failed once under a full `daemon/machines/...` run (its 3 s sync timeout on a loaded machine) and passed 5 runs alone. Not reproduced (minion B): 40 runs alongside 40 each of `daemon_test`, `restart_test`, `review_test` and `mcp_test`, all passed. Watch for it.
- [x] Operation Cancel and Retry (minion B): Cancel stops an add at its next step boundary and undoes it (the operation ends cancelled), a removal answers `not_cancellable`, and Retry runs a failed operation again under the same id (D81). Tests: `//apps/humata2/daemon/machines:fake_ssh_test` (`an_add_cancelled_after_any_step_stops_there_and_is_undone`), `//apps/humata2/daemon/humatad:machines_test` (`an_add_cancelled_while_it_connects_ends_cancelled_and_leaves_nothing`, `a_failed_add_tried_again_runs_under_the_same_id_and_a_removal_cannot_be_cancelled`).
- [ ] Retry from the failed step rather than the first, once some step's work survives a failure (D81); a cancelled add marks `install` undone even when it never ran (the undo always removes humatad2 once the key is pinned).
- [x] `daemon/machines:takeover_test` `the_files_are_installed_and_the_shell_becomes_the_bridge` failed in full runs: the install script asked the computer's real `pgrep` for a running `humata2_holder`, and other tests' daemons (or the user's own) run one, so each file was staged instead of swapped in. The test's shell now has a `pgrep` of its own (minion B). Fails before with any `humata2_holder` running; passes after.
- [x] EventService machine streaming (minion B): each change to a machine as ListMachines shows it (added, changed, removed) goes on an in-memory feed with resume and Resync (D82). Test: `//apps/humata2/daemon/humatad:machines_test` (`the_feed_tells_each_machine_change_and_a_resume_starts_after_the_last_seen`).
- [x] Inbox requests on the feed (minion F): each new request, answer and withdrawal is an Event with the InboxRequest, Created then Updated; the user sees every request, an agent its own session's; topics, channels and sessions narrow it (D90). Test: `//apps/humata2/daemon/humatad:inbox_test` (`the_feed_tells_of_each_request_and_its_answer`).
- [x] Grants on the feed (minion F): the grant an approval makes is a Created Event before the answered request, and a revoked grant a Deleted one with its last state; seen and narrowed as ListGrants lists them, by TOPIC_ACCESS (D91). Test: `//apps/humata2/daemon/humatad:inbox_test` (`the_feed_tells_of_the_grant_an_approval_makes_and_its_revoking`).
- [x] Review chat notes on the feed (minion F): each note made, changed (edited, gone outdated, moved, restored), dropped or taken away by an Undo is a Comment Event, Created, Updated or Deleted with its last state; a restart doesn't tell the kept notes again; TOPIC_COMMENTS narrows it (D92). Test: `//apps/humata2/daemon/humatad:review_test` (`the_feed_tells_of_each_note_made_changed_dropped_and_undone_but_not_again_after_a_restart`).
- [x] An agent hears of notes on every session it may read, as ListComments lists them: its own, and another as its grants and channel rules allow when the note changes (minion F, D98). Test: `//apps/humata2/daemon/humatad:review_test` (`an_agent_hears_of_notes_on_every_session_it_may_list_them_for`).
- [x] Operations are on the event feed (TOPIC_OPERATIONS): a Created Event when one starts and an Updated one for each change, in order; the user hears of all, an agent of its own session's (minion F, D94). Test: `//apps/humata2/daemon/humatad:machines_test` (`the_feed_tells_an_operation_from_its_start_to_how_it_ended`).
- [x] `by` on each event: the caller that made the change (an agent with its session, or the user), unset when the daemon did (minion F, D95). Tests: `//apps/humata2/daemon/humatad:inbox_test` (`the_feed_tells_of_each_request_and_its_answer`), `//apps/humata2/daemon/humatad:machines_test` (`the_feed_tells_an_operation_from_its_start_to_how_it_ended`).
- [ ] The rest of the feed: sessions and channels (once their services are built) and the other topics, the machine socket's filtered relay, and a feed that outlives a restart (D82).
- [ ] launchd for macOS remotes.
- [x] A host-side peer-link client with keepalive (minion D): the host dials `humatad2 serve --stdio` through the machine's link command, proves the enrollment key both ways (Hello, ProveHost), opens the Uplink, pings after 30 s idle and ends the link after 20 s unanswered, and dials again after 5–30 s; each link up or down is the machine's reachability on the feed (D83). Test: `//apps/humata2/daemon/humatad:humatad2_unit_test` (`peer_client::tests`).
- [x] Enrollment (minion E): an add makes a fresh 32-byte key, writes it on the machine as `~/.humata2/state/enrollment-key` (sent on stdin, 0600, renamed into place) during the install step, and keeps the host's copy in `<state>/peer-keys/<machine>` (0600, written atomically, never logged or sent to clients). A Reconnect enrolls a machine the host has no key for; removal and an undone add delete the host's copy. The link starts after a good add or Reconnect and for ready machines at daemon start (D84). Tests: `//apps/humata2/daemon/machines:fake_ssh_test` (`an_add_writes_the_same_private_enrollment_key_on_the_machine_and_the_host`, `reconnecting_keeps_the_key_and_writes_one_only_when_the_host_has_none`, the undo tests), `//apps/humata2/daemon/humatad:machines_test` (`an_added_machine_holds_the_hosts_key_and_its_link_comes_up_again_after_a_restart`, over a real `serve --stdio` on the fake machine).
- [ ] Keep the host's copy in the login keychain instead of a file (D83, D84).
- [ ] Run the peer link client over real SSH (Linux batch); a link command that needs host variables can't dial yet.
- [x] A wrong machine proof, or the machine refusing the host's, is `REACHABILITY_IDENTITY_MISMATCH` with no retry (LD-12); `daemon_version` comes from Hello, `last_seen_at` is when the link was last known up, and `next_retry_at` is the next dial (minion E, D84). Tests: `//apps/humata2/daemon/humatad:humatad2_unit_test` (`peer_client::tests`: `the_feed_tells_when_the_link_comes_up_and_when_it_is_lost`, `a_machine_whose_proof_is_wrong_shows_an_identity_mismatch_and_no_retry`).
- [ ] The app's machine view: say "add it again" for IDENTITY_MISMATCH and show `last_seen_at` / `next_retry_at` (LD-12).
- [ ] The Uplink carries nothing yet: the journal mirror, acks, and chunk codecs in Hello.
- [ ] The remove plan covers sessions and channels, and runs the dispose, leave_channels, provider_delete and find_agents steps.
- [ ] A real Lima/Linux run, including the four-platform tools bundle (after all I milestones, per the Linux batch rule).

### Compression (D65)
- [ ] Put FileService on the peer link so the remote throughput estimate fills in. Test that Auto picks zstd beyond the unit tests.
- [ ] TransferService.Upload, diff chunks and bug-report chunks, the compression settings page, and FileService roots beyond session clones.
- [ ] Bench numbers on a quiet machine.

### MCP, access and other open items (I8, A5)
- [ ] ReportService, script tokens (SEC-14), and the web listener.
- [x] MCP `inbox_request` asks ahead through RequestAccess and answers `{decision, grant_id}` (D93): `//apps/humata2/daemon/humatad:mcp_test`.
- [ ] Ask→Inbox (D38.6).
- [x] Spec mode removed under D104 (minion TIDY): `SetSpecMode`, `spec_mode` and `spec_files` are reserved in `sessions.proto`; the MCP tool `session_spec_mode`, `humata session spec` and the SM-1, SM-2 and SM-5 TRACE rows are gone.
- [x] The Inbox and grants kept across a restart: each request, its answer and its grant saved before anyone hears of them; grants rebuilt exactly as narrow, revoked ones stay revoked, new grant ids never reuse old ones (minion F, D96). Tests: `//apps/humata2/daemon/humatad:inbox_test` (`a_restart_keeps_the_inbox_and_the_grants_still_standing`), `//apps/humata2/daemon/store:inbox_test`.
- [ ] Decisions waiting on the owner, each with a recommendation in DECISIONS.md: D51, D56, D62 items 7–10, D63's synthetic FS, D67, I5 sign-off, and whether to keep the peer-link pipe-fd change (D64). From the docs tidy (minion J1): D103–D112, the frame changes in D113, and D9 (the Swift-era folders).
