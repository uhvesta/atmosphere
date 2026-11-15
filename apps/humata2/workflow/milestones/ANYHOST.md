# ANYHOST: Any client, any host

- **Status:** blocked
- **Depends on:**
  - LINUX: a self-hosted host is a Linux host; its suites run in the VM
- **Unblocks:** V1
- **Layer:** L6 · **Lane:** daemon

## Outcome
Open the app in a browser after pairing. Run the host on a Linux VM and connect the desktop app and the CLI to it.

## Exit criterion
The same suites pass against a host in a VM. Boundary tests cover unpaired, revoked and cross-origin callers. The wasm build is checked at every gate. Two desktop clients and a CLI stay in sync.

## Read these (pointers, nothing else)
- SPEC: AR-5–AR-9, TR-1, SL-1. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: none new; the browser draws the same frames
- Decisions: D34, D57 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `engine`, `daemon/humatad/src/serve.rs`, `daemon/humatad/src/guard.rs`, `daemon/humatad/tests/auth.rs`, `desktop/src/main.rs`
- Also: The web half (wasm build, pairing) needs only FL. If the daemon lane is idle before LINUX, the main agent may start that half early.
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
The VM tier (PER-8–PER-10): not in 1.0.

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
