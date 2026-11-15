# BUG: Report a bug as text an agent can act on

- **Status:** blocked
- **Depends on:**
  - LOOK1: a recording names targets by their place in the view tree, which must be stable
  - P0: the app wiring adds an overlay and actions in the split files
- **Unblocks:** LINUX
- **Layer:** L1 · **Lane:** third builder (`report/`)

## Outcome
Record a stretch of use or take a snapshot, fill the short form, press Copy report, and paste the result into any agent.

## Exit criterion
The I9 list in OVERVIEW passes: flows that record, snapshot, fill the form and copy each kind of report; a saved recording replays as a flow to the same end state; scrubbing is proven against seeded secrets; the report folder is checked as a snapshot. `humata report record|stop|snapshot` works.

## Read these (pointers, nothing else)
- SPEC: none in SPEC; the requirement list is OVERVIEW.md › "I9: Bug reports"
- Frames: none; the recording dot sits in the title bar of frame 01
- Decisions: D31, D47 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `report/src/recorder.rs`, `report/src/replay.rs`, `report/src/scrub.rs`, `report/src/prompt.rs`, `report/tests/flows.rs`, `ui/src/record.rs`, `cli/tests/report.rs`, `testing/flow`
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Videos and pictures (never). Any change to the shell beyond the dot, the menu item and the form.

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
