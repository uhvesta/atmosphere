# FL: First light: a real session in the app that survives a reboot (0.1)

- **Status:** blocked
- **Depends on:**
  - FL-D: the app needs real Session and Channel services and holders
  - LOOK1: a real terminal needs the session page, tabs and sidebar
  - P0: the real client follows the per-service client traits P0 splits out
  - FRAMES1: New channel, clone progress and "Couldn't resume" have no frame yet
- **Unblocks:** ASK (its exit), MACH, REVIEW (its exit)
- **Layer:** L1 · **Lane:** daemon + UI seam

## Outcome
Install the signed app. It installs its daemon. Make a channel with one repo on this Mac, start an agent, type. Quit the app, reboot; the session is back and resumed.

## Exit criterion
Flows §26.2 (local) and §26.8 pass against a real `humatad2` on macOS. A new `clients/live` package implements the client traits over gRPC, and one parity suite runs the same cases against the fake and the real client. The signed release is installed in /Applications (0.1).

## Read these (pointers, nothing else)
- SPEC: AR-1–AR-5, CH-1–CH-5 (local), FC-1–FC-5, SE-1–SE-16, TS-0, PER-3, PER-4, PER-13–PER-17, PER-20, NV-16; flows §26.2, §26.8. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 01, 02, 05, 31, and the New channel, clone progress and "Couldn't resume" frames from FRAMES1 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D34, D51, D57, D73, D77 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `clients/src/lib.rs`, `clients/src/daemon.rs`, `clients/src/workspace.rs`, `clients/tests/fake_daemon.rs`, `features/app/src/lib.rs`, `ui/src/terminals.rs`, `terminal/pane`, `desktop/src/main.rs`, `testing/flow`, `cli/src/live.rs`
- API: `api/GRPC.md`; `api/proto/humata/v1/events.proto`, `sessions.proto`, `terminal.proto`
- Also: OVERVIEW.md › "A1: First light" and "G: Gate before the app"
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Other machines (MACH). Diff and comments against real clones (REVIEW). Inbox (ASK). Keep it to This Mac.

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
