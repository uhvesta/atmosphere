# AWAY: Keep working with the host closed

- **Status:** blocked
- **Depends on:**
  - MACH: away mode is machines without the host
  - ASK: a machine judges requests against a copy of the rules
- **Unblocks:** V1
- **Layer:** L3 · **Lane:** daemon

## Outcome
Close the host. SSH to a machine, `humata login`, `humata attach`, answer a request with `humata inbox`. Reopen the host and see it all merged.

## Exit criterion
Flow §26.10 passes. The AW-7 table runs as a boundary suite. Journals replay in order and are re-checked against the rules; the first answer wins; conflicts follow SL-6–SL-8.
- Contain agents (SEC-6): a contained agent's writes outside its session folder and `shared/`, its reads of `~/.ssh`, the `gh` config and other sessions, and any network but the daemon socket all fail, as a boundary suite on macOS. (D128)

## Read these (pointers, nothing else)
- SPEC: AW-1–AW-12, SL-5–SL-8, SEC-10–SEC-13, SE-24, SE-25, TP-9, ST-6; flow §26.10. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: none
- Decisions: D3, D50, D14 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `daemon/humatad/src/cli_login.rs`, `daemon/humatad/src/attach.rs`, `daemon/store/src/journal.rs`, `daemon/humatad/tests/cli_login.rs`, `cli/src/login.rs`, `cli/src/commands/inbox.rs`, `cli/tests/login.rs`
- API: `api/CLI.md`, `api/BOUNDARIES.md`
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Script tokens (SEC-14): ASK. The web client (ANYHOST).

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
