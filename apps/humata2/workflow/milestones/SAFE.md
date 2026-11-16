# SAFE: Nothing is lost, ever

- **Status:** open (lane SAFE, 2026-11-16)
- **Depends on:**
  - MACH: add machine and install must exist to be cut
  - FORK: moves and forks must exist to be cut
  - FILES: transfers must exist to be cut
- **Unblocks:** LINUX
- **Layer:** L4 · **Lane:** daemon

## Outcome
Power-cut the host, a machine or both at any point. On restart everything is back or named in a restart report, and `humata doctor restarts` repairs what it can.

## Exit criterion
An audit: every multi-step operation has fault points (each daemon milestone added its own); the PER-28 power-cut suite passes on macOS for host, machine and both; every restart writes a report; the restart doctor skill and `humata doctor restarts` work; the last good copy restores. VM hard resets run in LINUX.

## Read these (pointers, nothing else)
- SPEC: PER-23–PER-31, TR-7–TR-10, SL-4. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: the restart report line from FRAMES1 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D52, D71, D77, D86, D96 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `daemon/store/src/fault.rs`, `daemon/store/src/last_good.rs`, `daemon/revive/src/report.rs`, `daemon/revive/src/reconcile.rs`, `daemon/humatad/src/restart.rs`, `daemon/humatad/src/last_good.rs`, `daemon/humatad/tests/restart.rs`, `daemon/durable`
- Also: OVERVIEW.md › Known gaps › "Daemon and persistence"
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Writing each operation's fault points (the milestone that adds the operation). Real VM resets (LINUX).

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
