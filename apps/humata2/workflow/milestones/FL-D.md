# FL-D: The daemon holds a real local session that survives restarts

- **Status:** open
- **Depends on:**
  - D0: the rules it builds are the tidied ones
- **Unblocks:** FL, ASK
- **Layer:** L0 · **Lane:** daemon

## Outcome
With the CLI alone (`humata`), make a channel with one repo on this machine, start an agent session, type into it, kill the daemon or reboot, and find the session back and resumed.

## Exit criterion
Boundary tests over gRPC and the CLI against a real `humatad2`: ChannelService and SessionService are served (no longer answered by `not_built.rs`); a channel's folders and main clone are made; a session gets its clone and branch name; its holder keeps the PTY; reattach replays; the kill matrix (app, daemon, holder) and a simulated reboot bring it back or say what was lost. Each multi-step operation added here carries its PER-27 fault points in the same milestone. The per-service client traits the app will use are agreed with the main agent (seam).

## Read these (pointers, nothing else)
- SPEC: AR-1–AR-5, CH-1, CH-2, CH-5 (this machine only), FC-1–FC-5, FC-11–FC-16, SE-1, SE-4, SE-13–SE-15, TS-0, PER-1–PER-7, PER-11–PER-17, PER-20, PER-23, PER-24, PER-27, TR-1, TR-3, TR-4. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: none (no UI)
- Decisions: D15, D38, D44, D51, D64, D71, D77 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `daemon/humatad/src/serve.rs`, `daemon/humatad/src/not_built.rs`, `daemon/humatad/src/holders.rs`, `daemon/humatad/src/terminal_session.rs`, `daemon/humatad/src/restart.rs`, `daemon/holder`, `daemon/revive`, `daemon/store`, `daemon/durable`, `daemon/humatad/tests/terminal.rs`, `daemon/humatad/tests/restart.rs`, `cli/src/commands/sessions.rs`, `cli/tests/sessions.rs`
- API: `api/proto/humata/v1/sessions.proto`, `channels.proto`, `terminal.proto`; `api/GRPC.md`, `api/CLI.md`
- Also: OVERVIEW.md › Known gaps › "Daemon and persistence" and "Pick up here (minion F)" (SessionService and ChannelService aren't built)
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
The app (FL). Remote machines and links (MACH). Inbox and access (ASK). `serve.rs` and shared protos are seam files: ask the main agent for exact lines.

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
