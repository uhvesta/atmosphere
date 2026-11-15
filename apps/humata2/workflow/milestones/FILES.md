# FILES: Move files and keep disks tidy

- **Status:** blocked
- **Depends on:**
  - MACH: transfers ride the machine link; Storage is per machine
  - FRAMES2: the file manager, transfers tray and Storage have no frame yet
- **Unblocks:** SAFE
- **Layer:** L3 · **Lane:** daemon + UI

## Outcome
Use a two-pane file manager, run a transfer that survives a dropped link, see Storage per machine, restore from trash, and turn on retention rules.

## Exit criterion
Flows §26.7 and §26.15 pass against real daemons. A transfer cut at any chunk resumes or undoes (PER-27 fault points, here). Low disk shows only where it blocks and on Hosts.

## Read these (pointers, nothing else)
- SPEC: FM-1–FM-15, LD-5–LD-7, LD-13–LD-22, TR-11, TR-13, RV-9–RV-14; flows §26.7, §26.15. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 06, and the file manager and Storage frames from FRAMES2 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D56, D60, D65, D107, D12 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `chunk`, `daemon/humatad/src/files.rs`, `daemon/humatad/src/compression.rs`, `daemon/humatad/tests/files.rs`, `cli/src/commands/files.rs`, `cli/tests/files.rs`, `features/files`, `clients/src/files.rs`
- API: `api/proto/humata/v1/files.proto` (FileService, TransferService, TrashService)
- Also: OVERVIEW.md › Known gaps › "Compression"
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Moving a whole session (FORK). The Files panel's look (LOOK1).

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
