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
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
