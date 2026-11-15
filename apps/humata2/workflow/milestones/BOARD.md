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
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
