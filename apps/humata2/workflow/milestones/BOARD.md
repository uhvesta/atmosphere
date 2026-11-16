# BOARD: Plan the work on boards

- **Status:** blocked
- **Depends on:**
  - ASK: agent board edits are access-checked and become requests answered on the item
  - LOOK2: the board, table, dashboard and item drawer are drawn there
- **Unblocks:** V1
- **Layer:** L2 · **Lane:** UI + daemon

## Outcome
Keep items on session, channel and global boards, as a board, a table or a dashboard. Start a session from an item. Link items with Depends on. An agent moves its own item and asks for others'.

## Exit criterion
Board flows pass against a real daemon, including the approval drawn in frame 22; agent edits through MCP; concurrent edits resolve by revision (SL-7); every dashboard figure opens the items it counts.

## Read these (pointers, nothing else)
- SPEC: BD-1–BD-16, SE-3, IN-10, TP-7, SL-7, NV-7. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 07, 12, 21, 22, 23, 24, 37 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D106 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `features/board`, `ui/src/board.rs`, `desktop/tests/design/board.rs`, `cli/src/commands/boards.rs`, `daemon/humatad/src/not_built.rs`, `daemon/store`
- API: `api/proto/humata/v1/boards.proto`; `api/MCP.md`
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
The Inbox card itself (ASK). A board view prepared by an agent (VIEWS).

## Lane (written by the manager)
- Design notes: D155 (Fable picks). Stage A is the daemon, CLI and MCP; stage B is the app. Seams are runnable scripts with `--check` in the manager's scratchpad `board-lane/seams/` (copied into the report to main).
- Claims: see the WORKLOG row "BOARD (early, daemon first)".
- Tasks (stage A):
  - [x] T1 store: `daemon/store/src/boards.rs`, tests `daemon/store/tests/pending/boards.rs` · worker (Sonnet, medium) + manager · written, not compiled
  - [x] T2 seams: proto (S1), error kind 58 (S2), GLOBAL_BOARD_WRITE (S3), MCP tool inputs (S4), CLI flags (S5), store migration/lib/error (store.md) · manager · all pass `--check`
  - [ ] T3 humatad BoardService: `daemon/humatad/src/board_service.rs` + `board_service/`, tests `tests/pending/boards.rs`, seam `board_service.py` · worker (Sonnet, medium) · written, not compiled
  - [ ] T4 MCP board tools: `daemon/humatad/src/mcp/boards.rs`, tests `tests/pending/board_tools.rs`, seam `mcp_dispatch.py` · worker (Sonnet, medium) · written, not compiled
  - [x] T5 Inbox board asks: `inbox/deferred.rs` (defer_board, pending_board_asks, withdraw_about_item) · manager
  - [x] T6 CLI: `cli/src/commands/boards.rs` (session and Depends-on lists), `cli/tests/boards.rs` · manager
  - [ ] T7 access tests: `daemon/access/tests/pending/global_board.rs` (moves up with access.py) · manager · written
- Stage A state (2026-11-15, after FREEZE 1): the daemon half is wired and committed in 22df2b44 (store, BoardService, MCP tools, CLI, GLOBAL_BOARD_WRITE; refusals.rs expects 17 Ask me capabilities).
- Stage B plan (manager minion BOARD, 2026-11-15; picks in D177):
  - Design: the app reads items from the daemon in `LiveDaemon::load` (ListItems Global-with-channels plus each channel with its sessions, deduped) and follows `board_item` events as `Event::Item`/`Event::ItemGone`. A new Inbox request whose preview is a board change re-reads that item (GetItem), so `BoardItem.pending` reaches the drawer (D155: items aren't re-sent when an ask comes). Pending asks map to `ItemDetails.{request, request_summary, requested_status, requested_change}`. History is read when a drawer opens (`Daemon::item_history`). Writes: MoveItem (no revision), CreateItem (column), UpdateItem `depends_on_add/remove`, `sessions_add/remove`. SE-3: after a session starts for an item, the app adds it to the item and moves Backlog/To do to In progress.
  - B1 [x] model + trait + fake (manager): `ItemChange`/`FieldChange`, `ItemDetails.requested_change`, `Event::{Item, ItemGone}`, `Daemon::{remove_dependency, set_item_session, item_history}`; fake impls; app applies the two events. Built: clients, clients/live, features/app, features/board.
  - B2 [ ] failing tests (Opus test writer): `clients/live/tests/boards.rs` (real daemon: load, create, move, link/unlink with loop refusal, sessions add/remove, history) and `desktop/tests/live_boards.rs` (frame 22: an agent's `humata board move` waits; the card shows Review; the drawer's Move to Done answers it; the agent's call returns; the item is Done; plus Keep in progress denies). BUILD blocks in `clients/live/BUILD.bazel` and `desktop/BUILD.bazel`.
  - B3 [ ] reducer tests (manager): `features/board/tests/drawer.rs`: ask wording per change, history on open, sessions add/remove, dependency add/remove via the picker (loop and self left out), picker refresh on refusal; `features/app/tests/board_start.rs`: SE-3.
  - B4 [ ] live client (Sonnet worker): new `clients/live/src/boards.rs`; additive lines in `clients/live/src/{lib,daemon,events}.rs`.
  - B5 [ ] reducer + app (Sonnet worker): `features/board/src/lib.rs` (+ new `src/picker.rs`), `features/app/src/new_session.rs` (SE-3 after start).
  - B6 [ ] views (Sonnet worker): `ui/src/board.rs` (ask wording, hover/focus + and ×, picker popover), keeping frames 21–24 unchanged (`//apps/humata2/desktop:design_test`).
  - Exit check: `//apps/humata2/clients/live:boards_test`, `//apps/humata2/desktop:live_boards_test`, `//apps/humata2/features/board:*`, `//apps/humata2/features/app:*`, daemon `boards_test`, `board_tools_test`, design goldens 07/12/21–24/37.
- Known not done: hiding items of archived or trashed sessions and channels (the store has no flags yet); forks in `session_with_forks`; the unreadable-target test.
- Waiting on: the owner for the IN-17 golden row (D158.12 settled it).
- Build gates: none yet.
- Ready to commit: no.

### Stage B status (2026-11-15)
- B1–B6 done. B1 is the app contract (Event::Item/ItemGone, remove_dependency, set_item_session, item_history, ItemChange). B4 is the live client (`clients/live/src/boards.rs`). B5 is the reducer: ask, choices, picker, history, Refresh, and SE-3 in the fake. B6 is the drawer views.
- The tests for B4 were written in parallel with it. They passed when they first compiled, so they never ran red. Review B4's diff (`clients/live/src/boards.rs`) closely.
- **GAPS_REVIEW-ready:** BOARD stage B. Review `clients/live/src/{boards,daemon,events,connection}.rs`, `features/board/src/lib.rs`, `ui/src/board.rs`, `features/app/src/{lib,events}.rs` (Refresh arm, Item events), `clients/src/{model,daemon}.rs`, `clients/src/daemon/fake/{boards,sessions}.rs`, and the tests `features/board/tests/drawer.rs`, `features/app/tests/board_start.rs`, `clients/live/tests/boards.rs`, `desktop/tests/live_boards.rs`. Also a one-wait fix in `desktop/tests/live_flows.rs` (the test raced the first load).
