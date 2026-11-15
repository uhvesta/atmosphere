# MACH: Work on another machine

- **Status:** blocked
- **Depends on:**
  - FL: a remote session is a local session plus a link
  - FRAMES2: Add machine, the login dialog and the machine page have no frame yet
- **Unblocks:** FORK, FILES, AWAY, FIND, SAFE
- **Layer:** L2 · **Lane:** daemon

## Outcome
Add a machine over SSH or a provider (with a login prompt), make a channel across two machines, start a session there. Pull the network; it catches up.

## Exit criterion
Flows §26.1, §26.2, §26.9, §26.16 and §26.21 pass on macOS with a local stdio or fake-SSH peer (the real-SSH run is LINUX). Skills and tools sync on connect. Links push and never poll. Add machine, install and enroll each carry their PER-27 fault points here.

## Read these (pointers, nothing else)
- SPEC: CH-1–CH-10, CH-18–CH-23, FC-6–FC-10, HO-1–HO-13, SK-1–SK-11, TS-1–TS-4 (scripted syncs: owner asked, D49), MP-1–MP-24, SE-16, PER-18–PER-21, TR-2, TR-5–TR-10, SL-1–SL-3, LD-19, LD-20; flows §26.1, §26.2, §26.9, §26.16, §26.21. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 06, 20, 28, and the Add machine, login and machine page frames from FRAMES2 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D39, D42, D43, D45, D46, D54, D61, D81, D83, D84, D107 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `daemon/machines`, `daemon/machines/providers/lima.toml`, `daemon/humatad/src/machine_service.rs`, `daemon/humatad/src/peer_link.rs`, `daemon/humatad/src/bridge.rs`, `daemon/humatad/src/sync.rs`, `daemon/humatad/tests/machines.rs`, `daemon/humatad/tests/peer_stdio.rs`, `cli/src/commands/hosts.rs`, `features/hosts`, `ui/src/hosts.rs`
- API: `api/proto/humata/v1/hosts.proto`, `peer.proto`; `api/GRPC.md`
- Also: OVERVIEW.md › Known gaps › "Machines"
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Forks and moves across machines (FORK). Transfers and Storage (FILES). Host-closed work (AWAY). The VM tier (PER-8–PER-10): not in 1.0. Real SSH and VMs (LINUX).

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
