# ASK: Agents ask, I decide

- **Status:** blocked
- **Depends on:**
  - FL-D: an agent's MCP identity is its real session
  - FL (to close, not to open): flow §26.4 in the app needs a real session on screen
  - LOOK1 (to close, not to open): the session Inbox panel is where the card is drawn
  - FRAMES1 (to close, not to open): the channel's Inbox and Agent access tabs have no frame yet
  - W1 (to close, not to open): the card in frames 03, 04, 26, 27 is redrawn; match it before this closes
- **Unblocks:** BOARD, FORK, AWAY, VIEWS, PR, LINUX
- **Layer:** L1 · **Lane:** daemon, then UI

## Outcome
An agent calls a Humata tool. A card with full context appears. Allow, Deny or Reply…, or Always allow exactly this. The agent continues. Rules are set per channel.

## Exit criterion
Daemon first: the capability matrix runs as a boundary suite over MCP and the CLI; Ask becomes an Inbox request that waits and can be collected; grants are scoped, revocable and survive a restart; an added agent's calls are refused (SE-29). Then the app: flow §26.4 against a real daemon, the card carries every part of IN-9, and the Agent access tab edits rules.

## Read these (pointers, nothing else)
- SPEC: IN-1–IN-26, TP-1–TP-17, SEC-3, SEC-7–SEC-12, SEC-15, SE-20, SE-29, ST-4; flow §26.4. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 03, 04, 26, 27 (as redrawn by W1), and the channel Inbox and Agent access frames from FRAMES1 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D105, D103, D32, D33, D37, D38, D41, D48, D90, D91, D93, D96 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `daemon/access`, `daemon/mcp/src/tools.rs`, `daemon/humatad/src/inbox.rs`, `daemon/humatad/src/access_service.rs`, `daemon/humatad/src/mcp.rs`, `daemon/humatad/tests/inbox.rs`, `daemon/humatad/tests/mcp.rs`, `cli/src/commands/inbox.rs`, `features/inbox`, `ui/src/panels.rs`
- API: `api/MCP.md`, `api/CLI.md`, `api/BOUNDARIES.md`; `api/proto/humata/v1/inbox.proto`, `access.proto`, `capabilities.proto`
- Also: OVERVIEW.md › Known gaps › "MCP, access and other open items"
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
The While I'm away rule and journals (AWAY). Board requests answered on the item (BOARD). Contain agents (SEC-6) and script tokens (SEC-14): not in 1.0.

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
