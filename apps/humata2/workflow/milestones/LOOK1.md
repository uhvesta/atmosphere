# LOOK1: The session page and sidebar look like the goldens

- **Status:** in progress (minion C1, outside this workflow; do not hand out)
- **Depends on:**
  - D0: the frames and rules it matches are the tidied ones
- **Unblocks:** LOOK2, FL, BUG, ASK (its exit)
- **Layer:** L0 · **Lane:** UI

## Outcome
On made-up data, the owner clicks through the session page, its panels, tab groups and the sidebar, in light and dark, and they look like frames 01–17.

## Exit criterion
For each of frames 01–17 the native capture matches the design frame within tolerance and one flow reaches it (`desktop:native_test`, `shell_test`, `panels_test`). Regions D113 disputes (the Inbox card in 03/04, "· Opus" in 05, the made-up data in 01, 02, 16) match the current frames until W1. C1 commits its own work; when its claim in WORKLOG is released, the main agent marks this done or opens a manager for what is left.

## Read these (pointers, nothing else)
- SPEC: SE-6–SE-16, SE-18–SE-20, SE-27–SE-29, CH-16, CH-17, NV-2–NV-6, NV-12–NV-19, NV-32–NV-34, NV-36, NV-37, ST-7–ST-12; flow §26.13. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 01–17 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D76, D74 (as revised), D103, D73 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `ui/src/shell.rs`, `ui/src/sidebar.rs`, `ui/src/session_tabs.rs`, `ui/src/terminals.rs`, `ui/src/panels.rs`, `ui/src/theme.rs`, `features/session/src/lib.rs`, `features/sidebar`, `desktop/tests/native.rs`, `desktop/tests/__snapshots__/native`
- Also: OVERVIEW.md › "N1: A native macOS look and feel" (C1's items and its Pick up here block are C1's, not yours)
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Channel, board, Home, Inbox, Hosts, Settings and overlays (LOOK2). The Desk row in the sidebar (DESK). The Inbox card's content (ASK). Real terminals (FL).

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
