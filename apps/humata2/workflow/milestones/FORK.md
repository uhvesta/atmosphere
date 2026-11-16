# FORK: Fork, move, archive and rebuild a session

- **Status:** blocked
- **Depends on:**
  - MACH: forks and moves across machines need the link
  - ASK: an agent's fork is an Inbox request
  - FRAMES2: Fork, Move, Archive with unpushed work and an archived session have no frame yet
- **Unblocks:** SAFE
- **Layer:** L3 · **Lane:** daemon + UI

## Outcome
Fork a session three ways, onto another machine. Move one. Archive with the unpushed-work check and Undo. Restore. Forget a dead machine and rebuild its sessions elsewhere.

## Exit criterion
Flows §26.5, §26.6, §26.11, §26.18, §26.19 and §26.23 pass against real daemons. A move or fork cut at any step finishes or undoes itself (PER-27 fault points, in this milestone). Undo after Archive is the 10-second delayed delete.
- Delete channel (CH-24, frame 71, D11 (a)) passes against real daemons with its PER-27 fault points; restoring from trash within 30 days brings the channel back, and its rules row goes with it (D127.6). (D128)

## Read these (pointers, nothing else)
- SPEC: FK-1–FK-13, MV-1–MV-15, LD-1–LD-12, SL-3, SL-4, FC-11–FC-17, TP-6, PER-27; flows §26.5, §26.6, §26.11, §26.18, §26.19, §26.23. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 16, 32, and the Fork, Move and Archive frames from FRAMES2 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D102, D110, D63 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `features/session/src/lib.rs`, `features/session/tests/fork.rs`, `features/session/tests/archive.rs`, `daemon/humatad/src/operations.rs`, `daemon/humatad/src/not_built.rs`, `daemon/store/src/journal.rs`, `cli/src/commands/sessions.rs`
- API: `api/proto/humata/v1/sessions.proto`, `operations.proto`
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Trash, Storage and retention (FILES). 

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
