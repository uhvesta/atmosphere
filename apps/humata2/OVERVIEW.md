# Humata 2: implementation overview

This is the order Humata 2 gets built in. Tick a box when its step is done and tested, and tick a milestone only when all its steps are ticked.

**Reference documents:**
- `SPEC.md`: what Humata 2 does, as behaviour.
- `REVIEW.md`: the review of the spec.
- `api/`: the boundaries: `BOUNDARIES.md`, `GRPC.md` + `proto/`, `MCP.md`, `CLI.md`.
- `prototype/`: how things look and roughly behave. Where the prototype and the spec differ, the spec wins.

**Rules that hold for every milestone:**
- **Tests prove behaviour, never how the code is written.**
  - Boundary tests drive gRPC, MCP and the CLI from outside.
  - Flow tests drive the app the way a person does.
  - Reducer actions, state and stores are fair to assert on, because they are the abstract model.
  - Private functions, call order and message wording are not.
- **macOS and Linux from the start.** Every test suite runs on both.
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
- [ ] Layout snapshots: the whole drawn tree as text, containers included, with rounded bounds, layout, padding, gap and colours, so a diff shows what moved, resized or restyled (owner request 2026-11-14).
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
- [ ] Where state lives:
  - the Mac is the source of truth;
  - machines hold only clones and agent context, and report pointers (repo, branch, sha, pushed, dirty files);
  - writes made while the Mac is away are journalled and replayed in order;
  - Rebuild on another machine works.
- [ ] Machine providers: `create`, `connect` with `{cmd}`, `identity`, interactive login with takeover, install humatad, the systemd user service with an admin fallback. The Lima provider config makes local VMs for the tests.
- [ ] Channels across machines and repos: main clones, per-session clones, branch prefix + slug, add and remove repos.
- [ ] Moves, forks across machines, file transfers that pause and resume, trash, archive and restore, Storage, retention.
- [ ] Skills sync.
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
- [ ] Paste a PR link, then clone it on any machine with a read-only agent.
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
