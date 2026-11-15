# VIEWS: Agents prepare views; I choose to open them

- **Status:** blocked
- **Depends on:**
  - ASK: preparing a view is rule-gated
  - REVIEW: a prepared review opens in the diff and editor
  - FRAMES2: the prepared view's tab and Showing bar have no frame yet
- **Unblocks:** V1
- **Layer:** L4 · **Lane:** UI + daemon

## Outcome
An offer bar appears. Open it, Later or Not now. While a view shows, Revert puts back exactly what was there. The Journal lists every view.

## Exit criterion
Flow §26.12 passes against a real daemon, for a session on another machine too.

## Read these (pointers, nothing else)
- SPEC: VW-1–VW-14, RV-31–RV-34; flow §26.12. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 03, 08, and the view frames from FRAMES2 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D55, D80, D87, D89, D97, D112 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `features/views`, `features/views/tests/offers.rs`, `daemon/humatad/src/review/views.rs`, `daemon/store/src/views.rs`, `cli/tests/views.rs`, `skills`
- API: `api/proto/humata/v1/review.proto` (ViewService); `api/MCP.md`
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
PR views and drafts (PR). Where views are stored (VW-12 says the host; the code differs: fix it here).

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
