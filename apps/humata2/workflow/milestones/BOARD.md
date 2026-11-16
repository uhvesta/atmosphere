# BOARD: Plan the work on boards

- **Status:** done (this commit, 2026-11-16): stage C1 and C2 green at main's gate
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

### Stage C (2026-11-16, manager minion BOARD; picks in D216)
- **Where stage A and B stand:** both are committed and green. No file is uncompiled, nothing of BOARD is under any `tests/pending/`, and no stage A seam is left. Evidence: every BOARD file is tracked and unmodified since freeze 2; WORKLOG events of 2026-11-15 (all 13 BOARD targets pass, the live-client mutation check); and gate g80 on `//apps/humata2/...` (376/379, none of them BOARD's). The milestone's exit criterion holds. What's left is the outcome's "session boards" and D155's archived hiding, which became buildable once FORK landed `fork_origins()` and `archives()` in the store.
- **C1 daemon [green at main's gate 2026-11-16; commits with the next batch]:** hidden items (an archived session's board) and SessionWithForks at any depth. `daemon/humatad/src/board_service.rs` (+ `board_service/`). Failing tests written by the manager at the end of `daemon/humatad/tests/boards.rs`: `a_session_and_its_forks_lists_every_forks_board_and_nothing_else`, `an_archived_sessions_items_are_hidden_and_come_back_on_restore`, `an_archived_fork_drops_out_of_its_parents_family` (helpers `start_with_forks`, `start_seeded`, `listed`, `set_archived`). Worker: Haiku (implementation). Gap closed after the gate: `an_archived_sessions_board_takes_no_new_items_until_restored`; `create` refuses an archived session's board as not found (manager).
- **C2 app [tests written, Haiku workers implementing]:** session boards in the app (BD-5–BD-8).
  - Seam (main): `seam_scope_session.py` adds `Scope::Session { channel, session }` and `Scope::channel()` to `clients/src/model.rs`. Apply it with the C2 batch, because it breaks the exhaustive matches until they get their arms.
  - `clients/live/src/boards.rs`: `scope()` reads `BoardItem.board`, and `board_ref` gains the session arm.
  - `features/board/src/lib.rs`: a persisted `ChannelView` in `View`, so `matches` follows BD-5 and BD-7 (Global leaves session items out). The scope menu offers one entry per session with forks, then one per session, and New item follows BD-6. The Sessions picker uses `scope.channel()`.
  - `ui/src/{board,panel_board}.rs`: the session panel shows its channel's board with the session's own items first (BD-8 as reworded, `session_panel`). The ui labels take the new arm; the scope menu lists `channel_views()`; the dashboard uses `in_scope`.
  - Seam (main): `seam_app_arms.py` gives the new arm to `ui/src/desk_board.rs` (DESK's), `features/app/tests/every_action.rs` and `features/app/src/new_session.rs` (an item session for a session item starts in its channel). `seam_spec_bd8.py` rewords SPEC BD-8 and SE-11.
  - `desktop/tests/native/session.rs`: no change; 07/12 keep the shared channel fixtures.
  - Tests first:
    - `features/board/tests/scope.rs`: Global leaves out session items; each ChannelView shows its set; a gone or archived session reads as Channel; the picker offers forks only for sessions with forks; New item in OneSession goes to that session; the session panel lists own items first, then the channel's.
    - `clients/live/tests/boards.rs`: a session item maps to `Scope::Session`.
- **07/12 vs BD-7/BD-8 (settled in D216 by a Fable pick):** BD-8 is reworded so the panel shows the channel's board plus the session's own items; no golden is owed a redraw. Left for the owner's next wave, blocking nothing: how a session-board item is marked inside the panel (no frame draws one; interim frame 37's accent outline).
- **Targets:**
  - C1: `//apps/humata2/daemon/humatad:boards_test`, `:board_tools_test`, `:humatad2_unit_test` (rerun boards_test for the create gap).
  - C2: adds `//apps/humata2/features/board:all`, `//apps/humata2/clients/live:boards_test`, `//apps/humata2/features/app:all`, `//apps/humata2/ui:ui_unit_test`, `//apps/humata2/desktop:{design_test,look_test,live_boards_test}`, and the native session/board tests (07/12 must still match).
