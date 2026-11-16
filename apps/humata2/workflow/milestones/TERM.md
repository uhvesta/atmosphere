# TERM: Session tabs run on the reusable terminal, the same on desktop and in the browser

- **Status:** open (2026-11-16, the owner: Codex C1 finished the reusable terminal and left a handoff)
- **Depends on:** FL (the app on the real daemon); DESK* and REVIEW* (they share ui/ and look_test)
- **Unblocks:** V1
- **Layer:** L4 · **Lane:** ui + clients + daemon seams

## Outcome
Every terminal in the app (a session's tab, Open a shell, a desk terminal) is the `terminal_replica` component C1 built: copy/paste, Tab, links, find, resize, scroll and memory saver work as in C1's demo, the terminal survives a viewer refresh or app upgrade without losing its holder, and input is never replayed after a reconnect. The app keeps its own chrome; the component draws only the terminal.

## Exit criterion
- Desktop session tabs, Open a shell and desk terminals render through `terminal_replica` over our daemon's attach transport, on fakes and live.
- A viewer reconnect or app restart reattaches the same holder (same pid/UUID), with output caught up and uncertain input disabled until resynced.
- Native, look and design tests stay green (terminal frames 01–09 within budget); the terminal component's own tests stay green.
- No regression in agent recovery (FORK, SAFE) or STOP.

## Read these (pointers, nothing else)
- `terminal/README.md` (component contract), `terminal/client/README.md`, `terminal/lab/HANDOFF.md`, `terminal/host/{README,TRANSPORT,AGENT_RECOVERY}.md`.
- App today: `ui/src/terminals.rs` (uses `terminal/pane` + `terminal/view` directly), `ui/BUILD.bazel`, the desktop attach path in `clients/live/`, `daemon/holder/` and the daemon's attach service.
- Decisions on terminals: grep DECISIONS.md for holder, attach, terminal.

## Out of scope
Changing `terminal/` itself (C1's; ask main to relay to C1 if the component needs a change). Neovim clipboard. Mosh. Linux VM runs (LINUX).

## Lane (written by the manager)

Manager: minion TERM (Opus 5.5), 2026-11-16. Phase 1 plan; no code yet. Picks came from an Opus 5.5 high advisor (Fable is rate-limited); they go to DECISIONS via main.

### Structure (plain words)
- **One source of truth stays our daemon.** humatad keeps the holders, the store, revive, FORK, SAFE and STOP. C1's `terminal_host` is not embedded and not run beside it. C1 owns its own store and resume adapters, so either option would give two owners of the same holders.
- **New: an authoritative screen per running tab inside humatad.** It is a new crate `daemon/vt_authority`, written on `ghostty_vt`, which is already visible to `apps/humata2` and is `Send`. holders.rs already keeps one link per tab, and that link feeds it. It answers VT queries even when nobody is watching. It records a reply as pending before writing it and clears it on the holder's `InputWritten`. It resizes its own screen when it sends the resize to the holder, and broadcasts `Resized` in order. It debounces checkpoint encoding off the attach task. Scrollback is capped, and the screen is dropped once an exited tab's final checkpoint is taken. Attaches subscribe to this one screen, not to the holder.
- **Wire: the same gRPC Attach, with optional fields added.** The fields are a Ghostty-checkpoint snapshot format, Control, Resized, Detached, ResyncRequired, and InputWritten in place of InputAccepted. The remote relay forwards them untouched. The CLI's raw attach keeps working. A peer that does not send the checkpoint format means resync with input disabled; raw bytes are never replayed into the replica.
- **Client: a per-tab terminal link instead of bytes in the TCA store.** `Terminals::link(session, tab)` gives an ordered stream of `terminal_wire::ServerFrame` and a `ClientFrame` sink. A new single-job crate maps gRPC to and from `terminal_wire` and adapts it to `ReplicaSink`. It is wasm-clean (no tokio) so ANYHOST can reuse it. The fake builds a real Ghostty checkpoint from the fixture bytes at the golden's grid size, sends `Control{controller:true}`, and answers each resize with `Resized`.
- **UI:** `ui/src/terminals.rs` holds one `TerminalReplica` per (session, tab), along with its holder generation, and replaces the replica when the generation changes. The app keeps its chrome, notice, aria text and colours. `terminal_bytes` leaves `features/session` and `clients/workspace`; the store keeps only status, exited, input-uncertain, title and cwd.
- **Writers:** keep D14. Every non-read-only attach is a controller, which keeps goldens 01–09 off the pan container. Only the focused, visible replica sends fit requests.
- **Identity:** replica UUID = UUIDv5(tab id, holder generation). `TabRecord.holder_generation` is new, and revive and FORK move bump it.
- **Recovery:** daemon/revive stays the only recovery authority. C1's AGENT_RECOVERY and FAILURE_MATRIX are used only as a test checklist.
- **Memory saver:** on by default, with a 30 s delay for tabs not visible in any pane. It pins tabs with unflushed or uncertain input and gets a Settings switch. If restoring a 10k-line tab takes more than about 50 ms at p95, raise the delay.
- **Scope:** desktop only. The browser comes in ANYHOST on the same mapper crate over gRPC-web.

### What stays
The holder binary and protocol, revive, the store (except one TabRecord field), the remote relay's shape, the CLI's raw attach, the app's chrome, and the goldens.

### Tasks (failing tests first; smart agents write the tests, Haiku/Sonnet workers implement)
1. A tab's screen in the daemon matches what the holder printed, answers DA/DSR queries with nobody attached, and never resends a reply after a restart (crate tests on `vt_authority`, new crate, no seams).
2. An attach gets a Ghostty checkpoint the replica can decode, then ordered Output and Resized, and input is acknowledged as InputWritten (humatad attach tests; proto fields and serve wiring go to main as seams).
3. A revived or moved tab gets a new holder generation, and a stale replica is replaced, never fed the new process's offsets (store and revive tests; schema and records lines via main).
4. The fake and the live client give each tab an ordered frame link, and a reconnect never replays unacknowledged input (clients and clients/live tests, plus a new mapper crate with its own tests).
5. Session tabs, Open a shell and desk terminals draw through `terminal_replica`, and goldens 01–09 stay within budget (desktop native, look and design tests on fakes; ui seams via main).
6. Bytes leave the TCA store, and features/session, desktop native and live_flows tests move to the link (features/session tests updated, not weakened).
7. `humata attach` in the CLI does not double-answer terminal queries: the CLI filters reply sequences out of raw-attach input (cli tests).
8. FORK, SAFE and STOP stay green: the STOP sweep drops screens without waiting on a pending reply, and SAFE's power-cut suite covers the pending-reply record (existing suites plus one new case each).
9. A live run on the real daemon reattaches the same holder after an app restart, catches up the output, and keeps input disabled until resync (live_flows, headless).

### Risks
Daemon memory and CPU with one screen per running tab (capped scrollback; checkpoints off the hot path). Golden pixels: cursor and blink state in checkpoints, and the first fit before Resized. Mixed daemon versions over the remote relay. The CLI's double replies. The pending-reply record under SAFE: checkpoints are not persisted at first, and after a daemon restart the screen is rebuilt from the holder's kept bytes, or ResyncRequired is sent.

### Requests for C1 (via main; none of them block us)
1. Widen the visibility of `terminal_replica` to `//apps/humata2/ui` and the new mapper crate, and of `terminal_wire` to the mapper crate, `clients/...` and `daemon/vt_authority`.
2. Confirm that `controller=true` renders pixel-identical to a bare `TerminalPane`.
3. Make the replica send fit requests only when it has focus, or make that an option.
4. Say whether a new UUID under the same key needs a new entity or can use a `rebind`.
5. Let ReplicaConfig take default terminal colours that are reapplied on each Snapshot install.
6. Optional: extract the reply-watermark state machine into a pure library.

### Claims (asked of main)
New `daemon/vt_authority/`, new `clients/terminal_link/` (the mapper crate), `daemon/humatad/src/attach.rs`, `daemon/humatad/src/holders.rs` (REVIEW batch 2 held it; it needs main's OK), `clients/live/src/terminals.rs`, `clients/src/daemon/fake/terminals.rs`, `ui/src/terminals.rs` (LOOK1 held it), `features/session/tests/terminal.rs`, and this Lane. Seams go to main: `terminal.proto`, `serve.rs`, `store/{schema,records}.rs`, revive, `clients/src/{lib,daemon,model,workspace}.rs`, `features/session/src/lib.rs`, `ui/src/{lib,shell,session_tabs}.rs`, `desktop/BUILD.bazel`, and the CLI attach.

### Batch 1 (2026-11-16; main approved the plan, picks logged as D271)
- Same-version peers (owner, via main): the mixed-version fallback is dropped; UPGRADE force-upgrades peers. A peer whose snapshot has no checkpoint is refused cleanly (`LinkError::PeerVersionUnsupported`, "being upgraded"), never replayed.
- Task 1: new `daemon/vt_authority` (crate `humata2_vt_authority`, private), with `tests/authority.rs` written first. It is fed by output offsets and refuses gaps. It answers DSR/DA with each reply numbered by sequence. `ReplyRecord{answered_through, pending, uncertain}` is what SAFE must save before a reply is written. `resume` answers only queries that were never answered. It also covers resize-at-offset, the changed flag and Ghostty checkpoints.
- Task 4a: new `clients/terminal_link` (crate `humata2_terminal_link`, private and `manual` until C1's wire visibility lands), with `tests/link.rs` written first. It maps the attach to `terminal_wire` frames. State that comes before the screen is held back until the screen is installed. TerminalState's size becomes Resized, Writable becomes Control/Exited, InputAccepted becomes InputWritten, SnapshotRequest means reopen from 0, and Detach closes. It is wasm-clean (prost + wire only).
- Seam S1 (main): additive proto fields, with no new oneof arms. They are `AttachOpen.checkpoint_engine=9`, `TerminalResize.cell_width/height=3/4`, `TerminalSnapshot.engine=9, terminal_uuid=10` and `TerminalState.cols/rows/cell_width/cell_height=5..8`. It also fixes the 10 struct literals that have no `..`.
- Scripts: scratchpad `term/b1/b1_new_crates.py` and `term/b1/s1_proto_seam.py` (both `--check`).
- Fallbacks if C1 requests 3 and 5 don't land: the ui's sink adapter forwards Resize frames only from the focused, visible replica, keeping the latest. The ui reapplies `set_default_colors` on the pane whenever the replica reaches Live after a Snapshot.
- Next, batch 2: humatad wiring of the authority into holders.rs and attach.rs (task 2, with SAFE/STOP seams via main), and `Terminals::link` plus the fake that builds checkpoints (task 4b).
