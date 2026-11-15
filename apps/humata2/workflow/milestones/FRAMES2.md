# FRAMES2: The screens for machines, files, forks, PR reviews and views are drawn and signed off

- **Status:** blocked
- **Depends on:**
  - FRAMES1: same files and the same lane; the earlier frames set the patterns
- **Unblocks:** MACH, FORK, FILES, VIEWS, PR
- **Layer:** L1 · **Lane:** docs

## Outcome
The owner can look at a frame for every remaining screen SPEC defines, and sign it off.

## Exit criterion
New goldens for: Add machine and the interactive login; the machine page; Storage; the file manager and transfers tray; Fork (settle D102); Move; Archive with unpushed work; an archived session; the PR review dialog, review session and Submit review; a prepared view's tab and its Showing bar; Delete channel (only if it stays in 1.0). Recorded at a gate, signed off by the owner.

## Read these (pointers, nothing else)
- SPEC: HO-1–HO-4, MP-12–MP-17, LD-13–LD-15, FM-1, FM-7, FK-2–FK-4, MV-4, MV-8, MV-9, LD-2, PR-1–PR-5, VW-6, VW-8, CH-24. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: existing ones to match in style: 28 (Hosts), 31 (dialog), 32 (toast), 11 (diff) (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D76, D102, D113 (its "Frames still missing" list), D11 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `desktop/tests/design.rs`, `desktop/tests/design/kit.rs`, `desktop/tests/design/pages.rs`, `desktop/tests/design/overlays.rs`, `desktop/tests/__snapshots__/design`
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
App code. Frames FRAMES1 or W1 own.

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
