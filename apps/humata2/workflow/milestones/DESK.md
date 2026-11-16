# DESK: A place to work without a channel

- **Status:** blocked
- **Depends on:**
  - LOOK2: the Desk page reuses the channel page, lists and empty states
  - FRAMES1: Clean up… has no frame yet
  - W1 (to close, not to open): frame 33 may be retired and frames 37–40 print a new path; match them before this closes
- **Unblocks:** REVIEW
- **Layer:** L2 · **Lane:** UI

## Outcome
A fresh install shows Get set up. Open the Desk, start a scratch session or pick a folder, read a document beside the terminal, close and reopen it (on fakes).

## Exit criterion
Frames 34–40 match. Flows: empty Home to a new desk session; Close, Reopen, Keep and Clean up…; copy a document to a channel; last used sorts the Desk. The Desk row is in the sidebar, so native frames 01–33 match the goldens again. A release on fakes is installed.

## Read these (pointers, nothing else)
- SPEC: DK-1–DK-6, DK-10–DK-14, SE-26, NV-11, FC-1, MV-1. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 33 (until W1), 34–40 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D100, D108, D109, D99 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `desktop/tests/design/desk.rs`, `ui/src/sidebar.rs`, `ui/src/pages.rs`, `ui/src/channel.rs`, `features/sidebar`, `features/app/src/channel.rs`, `clients/src/fixture.rs`
- Also: OVERVIEW.md › N1 items "Build the Desk in the app" and "Build the empty states"
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Markdown tabs and comments, frames 39 and 41–43 (REVIEW). Real desk sessions on a daemon (after FL; the daemon lane adds them as a task of MACH or FORK when DK-3's move is built).

## Lane (written by the manager)
- **State:** early lane (LOOK2 open). Manager minion DESK (Fable 5.2). Closes only after LOOK2 is done, frames 34–40 match and the release on fakes is installed. Until main says "LOOK1 committed", this lane writes new files only.
- **Goal / exit:** as above. Early part: the Desk's model, client and fake, its reducer and flows as TestStore tests, the app wiring file and the sidebar's Desk row in the model. No existing view draws differently until LOOK2 is done.
- **Design notes (D141):**
  - *A Desk session is a `Session`.* Its channel is the reserved id `desk` (`humata2_clients::desk::channel()`; DK-2 reserves the folder name, so no channel can have it). The session page, terminals, panels, pin, VIP and search work unchanged; channel trees and channel pages never list it because no channel has that id. Closed (DK-11) is `archived = true` plus a `Closed` record, so every list that hides archived sessions hides closed Desk sessions.
  - *What only the Desk knows* (folder scratch/kept or picked, repos, the two access switches, last used, closed and days left, unpushed work) is a `DeskSession` per session in `Workspace.desk` (`DeskData`), next to the documents list. It rides on the workspace like `session_options` and `session_files`, so load and Resync carry it and views read it from the `Arc<Workspace>` they already get. `Session` itself is not touched (model.rs is LOOK1's and main's).
  - *Client:* a separate trait `DeskClient` (`clients/src/desk.rs`): start, close, reopen, keep, clean_up_list, clean_up (acts only on the list it showed, NV-27), copy_document, add_repo, remove_repo, move_to, set_access. The fake daemon implements it in `clients/src/daemon/fake/desk.rs` over the same in-memory workspace and announces changes with `Event::Resync`, as the board and channel fakes do. Scripted failures use `Operation::Desk(desk::Operation)`. Reducers take it as an `Arc<dyn DeskClient>` (as MACH's `Machines`), not as a `Clients` field, until the live client serves the Desk.
  - *Last used (SE-26)* is an order number plus a label on the Desk record; the fake bumps it on start and reopen and through `FakeDaemon::use_desk_session`. Batch 2 hooks typing, opening and answering into it. It moves onto `Session` when the other lists show it.
  - *The scratch folder's name* is sent by the dialog (`FolderChoice::Scratch { name }`), as New session sends its branch; the daemon only adds `-2`, `-3` when taken. The form derives it from the title by FC-13 unless set by hand.
  - *Feature crate `features/desk` (`humata2_desk`):* `Desk` reducer, `DeskState` (workspace, sort, the New desk session form, the Clean up… dialog, the Copy to a channel… dialog, pending, error), lists `open()`, `closed()`, `documents()`. It delegates `OpenSession` and `Toast` to the app.
  - *App wiring (batch 2):* new `features/app/src/desk.rs` (`AppAction::Desk`, `AppState.desk`, `Route::Desk`, Show fan-out, delegates, Escape closes a Desk dialog, the saved sort); the sidebar model gets a Desk row under Board with the open Desk sessions under it (DK-1).
  - *Views (batch 3, after LOOK2):* frames 34–40 and 55 ported from `desktop/tests/design/{desk,cleanup}.rs`; the folder bar; "Desk ›" crumb and no Fork on a Desk session (DK-12); Board panel scope All channels (DK-4); the Desk group in the Inbox (DK-12).
- **Claims:** new files `clients/src/desk.rs`, `clients/src/fixture/desk.rs`, `clients/src/daemon/fake/desk.rs`, `clients/tests/pending/desk.rs` (not globbed; moves up to `clients/tests/desk.rs` with the seam); new crate `features/desk/` (targets `manual` until the seam); batch 2: new `features/app/src/desk.rs`, `features/app/tests/desk.rs`; `features/sidebar/` only after main says "LOOK1 committed"; this Lane; OVERVIEW block "Pick up here (minion DESK)"; DECISIONS D141. Seam lines go to main: `clients/src/{lib,workspace,daemon,fixture}.rs`, `clients/src/daemon/fake.rs`, root `Cargo.toml` + repin, later `features/app/src/{lib,route,overlay,events}.rs`, `features/app/{BUILD.bazel,Cargo.toml}`, `ui/src/{lib,shell}.rs`.
- **Tasks:**
  - [x] T1 the client contract and model · manager · `clients/src/desk.rs` · written
  - [ ] T2 fake daemon's Desk, fixtures (frames 34, 36, 55), client tests · worker desk-fake (Sonnet, medium) · `clients/src/fixture/desk.rs`, `clients/src/daemon/fake/desk.rs`, `clients/tests/pending/desk.rs` · running
  - [ ] T3 `features/desk` crate: reducer and the flows (start, close/reopen/keep, clean up, copy, sort) as TestStore tests · worker desk-feature (Sonnet, medium) · `features/desk/**` · running
  - [ ] T4 app wiring: `features/app/src/desk.rs`, flows through the app (empty Home → New desk session → its page; Close returns to the Desk; Escape) · `features/app/tests/desk.rs` · batch 2, after gate 1
  - [ ] T5 sidebar model: Desk row under Board, open Desk sessions under it, selection and keyboard · `features/sidebar/` · batch 2, after "LOOK1 committed"
  - [ ] T6 fake hooks: last used bumped by mark_read, send, terminal input and answers; `create_channel` and the New channel form refuse the name `desk` (DK-2); palette finds Desk sessions (DK-12) · existing fake files, after "LOOK1 committed" and via main for LOOK2's `features/palette`
  - [ ] T7 views: frames 34–40 and 55, folder bar, Desk crumb, no Fork, Board panel All channels, Info switches, Inbox Desk group; `fixture::desk` joins `fixture::design()` so native frames 01–33 get the Desk row · batch 3, after LOOK2
  - [ ] T8 W1 check: frames 37–40 print `~/Humata/desk/…`; release on fakes installed (main)
- **Open questions (for a Fable advisor before T7):** frame 40 previews `~/Humata/desk/plan-q4` for the title "Plan Q4 board", but FC-13's slug is `plan-q4-board` (and `brin-rfc` vs "BRIN rollout RFC" in 37–39). The model sends whatever name the dialog shows, so either answer fits; the pick is what the dialog shows by default.
- **Waiting on:** main for the batch 1 seam lines and gate; "LOOK1 committed" for `features/sidebar` and the fixture files; LOOK2 for the views.
- **Build gates:** none yet
- **Ready to commit:** no
