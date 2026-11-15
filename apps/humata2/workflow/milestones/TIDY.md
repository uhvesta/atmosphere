# TIDY: No leftovers from dropped designs in the code or API

- **Status:** blocked
- **Depends on:**
  - W1: deleting an API and folders can't be undone; it waits for the owner to confirm D104 and D9
- **Unblocks:** V1
- **Layer:** L1 · **Lane:** daemon

## Outcome
An agent or CLI user is never offered spec mode, and a reader of the tree finds no dead Swift-era app.

## Exit criterion
`SetSpecMode` and `spec_mode` are gone from `sessions.proto`; the MCP tool `session_spec_mode` and `humata session spec` are gone; `api/MCP.md`, `api/CLI.md` and the SM-1, SM-2, SM-5 rows of `api/TRACE.md` are updated; boundary suites pass. If the owner says so under D9, `app/`, `bindgen/`, `core/` and `rehearsal/` are deleted in their own commit. A grep for wording the tidy retired ("Always let", "Widen", "Approve" as a button) in `api/` and user-facing strings comes back clean.

## Read these (pointers, nothing else)
- SPEC: SM-3, SM-4, SM-6 (what replaced spec mode); the dropped IDs are SM-1, SM-2, SM-5. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: none
- Decisions: D104, D9 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `api/proto/humata/v1/sessions.proto`, `daemon/mcp/src/tools.rs`, `cli/src/commands/mod.rs`, `cli/src/commands/sessions.rs`, `cli/src/daemon.rs`, `api/MCP.md`, `api/CLI.md`, `api/TRACE.md`, `app`, `bindgen`, `core`, `rehearsal`
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Markdown review itself (REVIEW). `prototype/` and `ui-prototype/` (never touched).

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
