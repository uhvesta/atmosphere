# FIND: Find and reach anything

- **Status:** blocked
- **Depends on:**
  - MACH: search runs over the conversation mirror from every machine
  - LOOK2: the palette and the Pinned section are drawn there
- **Unblocks:** V1
- **Layer:** L3 · **Lane:** UI + daemon

## Outcome
⌘K runs every action and searches every session on every machine. Pin things and mark them VIP.

## Exit criterion
Flow §26.22 passes against a real daemon. A search hit opens the turn. An audit test shows every action in the app is in the palette. Pins and VIP marks live on the host and show in two clients at once.

## Read these (pointers, nothing else)
- SPEC: KP-1–KP-6, LD-8, LD-21–LD-25, NV-20–NV-26; flow §26.22. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 15, 30 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D63, D62 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `features/palette`, `features/palette/tests/search.rs`, `ui/src/overlays.rs`, `ui/src/sidebar.rs`, `daemon/humatad/src/not_built.rs`, `cli/src/commands/looking.rs`
- API: `api/proto/humata/v1/settings.proto` (MarkService); `api/MCP.md` (`sessions_search`)
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
The palette's look (LOOK2). Review this PR from a pasted link (PR).

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
