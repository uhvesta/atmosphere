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
- Tasks (stage B, after LOOK2 and FL release their files): live client board calls; drawer approve/deny from `pending`; sessions add/remove and unlink; stale re-ask; dashboard drills; `ui/src/board.rs` lines; flows in new `desktop/tests/live_boards.rs` (frame 22 approval against a real daemon).
- Known not done: hiding items of archived or trashed sessions and channels (the store has no flags yet); forks in `session_with_forks`; the unreadable-target test.
- Waiting on: main to apply the seams and compile; LOOK2 and FL for stage B; the owner for the IN-17 golden row.
- Build gates: none yet.
- Ready to commit: no.
