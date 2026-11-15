# FRAMES1: The screens the next layers need are drawn and signed off

- **Status:** open
- **Depends on:**
  - D0: frames are drawn from the tidied rules
- **Unblocks:** FRAMES2, FL, DESK, ASK (its exit), REVIEW
- **Layer:** L0 · **Lane:** docs

## Outcome
The owner can look at a frame for each screen FL, ASK, REVIEW and DESK build that has none today, and sign it off.

## Exit criterion
New goldens, numbered from 44, for: New channel and clone progress; "Couldn't resume" and the restart report line; the channel's Inbox tab and Agent access tab; comment states (addressed with before/after and Resolve, resolved by the agent, archived, the Archived list in Info); the comment box in a diff; Clean up…. Draw New channel and "Couldn't resume" first (FL needs them). Each is built from the same kit as frames 01–43, recorded at a gate by the main agent, and signed off by the owner. Draw only what SPEC says; where SPEC is silent, ask.

## Read these (pointers, nothing else)
- SPEC: CH-1–CH-5, PER-15, PER-29, CH-11, IN-1, IN-17–IN-19, SM-17–SM-21, RV-4, RV-25, RV-26, DK-11. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: existing ones to match in style: 18–20 (channel tabs), 26 (Inbox), 31 (dialog), 05 (exited session), 11 (diff), 36 (Desk) (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D76, D101, D113 (its "Frames still missing" list) (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `desktop/tests/design.rs`, `desktop/tests/design/kit.rs`, `desktop/tests/design/channel.rs`, `desktop/tests/design/overlays.rs`, `desktop/tests/design/session.rs`, `desktop/tests/__snapshots__/design`
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Changing frames 01–43 (W1). The frames for machines, files, forks, PR reviews and views (FRAMES2). App code.

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
