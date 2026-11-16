# BUG: Report a bug as text an agent can act on

- **Status:** in progress (lane BUG)
- **Depends on:**
  - LOOK1: a recording names targets by their place in the view tree, which must be stable
  - P0: the app wiring adds an overlay and actions in the split files
- **Unblocks:** LINUX
- **Layer:** L1 · **Lane:** third builder (`report/`)

## Outcome
Record a stretch of use or take a snapshot, fill the short form, press Copy report, and paste the result into any agent.

## Exit criterion
The I9 list in OVERVIEW passes: flows that record, snapshot, fill the form and copy each kind of report; a saved recording replays as a flow to the same end state; scrubbing is proven against seeded secrets; the report folder is checked as a snapshot. `humata report record|stop|snapshot` works.

## Read these (pointers, nothing else)
- SPEC: none in SPEC; the requirement list is OVERVIEW.md › "I9: Bug reports"
- Frames: none; the recording dot sits in the title bar of frame 01
- Decisions: D31, D47 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `report/src/recorder.rs`, `report/src/replay.rs`, `report/src/scrub.rs`, `report/src/prompt.rs`, `report/tests/flows.rs`, `ui/src/record.rs`, `cli/tests/report.rs`, `testing/flow`
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Videos and pictures (never). Any change to the shell beyond the dot, the menu item and the form.

## Lane (written by the manager)
- **Goal / exit:** as above. The `report/` library already records, snapshots, scrubs, writes, fills forms, makes the prompt, replays and has folder goldens (its tests are the scrubbing and folder-snapshot proof). What is missing is the app and the CLI: the dot, menu, shortcuts, the form, Copy report, gestures and frames fed in, the reports folder, `humata report` reaching the app, and app-level flows that record, snapshot, fill, copy and replay.
- **Design notes (picks in D133):**
  - `features/app/src/report.rs`: `ReportState { recording, form }`, `ReportForm` (folder, kind, happened, expected, elements to pin from the saved view tree, query, pinned comments, copied, error), `ReportAction` (Started, Saved{form, open}, Failed{message, recording}, Happened, Expected, Query, Pin, CommentText, Unpin, Copied, Close). Pure state; the I/O (save, fill_form, copy_prompt, clipboard) is in the ui because it needs the window. A CLI save shows a toast whose action (`ToastAction::OpenReport`) opens the form. Escape closes the form.
  - `ui/src/record.rs`: the recorder gains a reports folder and seeded secrets (D133.6), element names with their tree path, start/stop/snapshot from a window, fill-and-copy, and the gesture/keystroke hooks.
  - `ui/src/report_view.rs`: one entity drawn on top of the shell: the title-bar dot ("Stop recording"), the form sheet, and the mouse hook; `actions()` adds ⇧⌘R/⇧⌘S handlers to the shell's root; `serve()` answers the app socket.
  - `report/src/remote.rs`: the app socket (`~/.humata2/app.sock`, 0600, `HUMATA_APP_SOCKET`): JSON lines, `Listener::bind` refusing a live socket and replacing a stale one, `ask()` for the CLI.
  - Daemon RPCs on fakes: no RPC layer to hook yet; the library records them (`Recording::rpc`), the app feeds them at gate G.
- **Claims:** see WORKLOG Claims row BUG; also `features/app/src/toast.rs`, `ui/src/overlay_toasts.rs` (toast button label for OpenReport).
- **Tasks:**
  - [ ] T1 report state and reducer · worker feat (Sonnet, medium) · `features/app/src/{report,overlay,toast}.rs`, `features/app/tests/report.rs` · written, reviewed
  - [ ] T2 app socket · worker remote (Sonnet, medium) · `report/src/{remote,lib,error}.rs`, `report/BUILD.bazel`, `report/tests/remote.rs` · written, reviewed
  - [ ] T3 app wiring · worker wire (Sonnet, medium) · `ui/src/{record,report_view,keys,bridge,overlay_toasts}.rs`, `desktop/src/main.rs` · written, reviewed (compile unverified: TextareaState::set_value, on_next_frame, async window context)
  - [ ] T4 app flows · worker flows (Sonnet, medium) · `desktop/tests/report.rs` · written, reviewed (compile unverified)
- **Seam lines for the main agent:** listed in the batch report; CLI seam in scratchpad `bug-cli-seam.md`, desktop target in `bug-desktop-build.md`.
- **Deferred:** D133.7 terminal mark, report palette commands, RPC capture on fakes, drag gestures, pin pick mode.
- **Waiting on:** main applies seam lines and builds batch 1 (targets in the batch report).
- **Build gates:** none yet.
- **Ready to commit:** no.
