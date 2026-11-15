# FL-D: The daemon holds a real local session that survives restarts

- **Status:** done (9ec5b5ed)
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
  - New library crate `daemon/sessions` (`humata2_sessions`, visible to humatad only) does the work on disk and in git; humatad's new `channel_service.rs` and `session_service.rs` only translate gRPC, access checks (Guard) and operations. No gRPC types in the crate.
  - Store: one additive migration with plain-column tables (no new store protos: shared protos are a seam): `channels`, `channel_repos` (main clone state: cloning, ready, failed), `channel_machines`, `session_repos` (branch, path), `session_starts` (start step, model, first message, revision). `SessionRecord` stays as it is (many tests build it as a literal). Module `store/src/channels.rs`.
  - Folders (FC-1): `<humata root>/<channel folder>/{main/<repo>, sessions/<session folder>/<repo>, shared/}`; the root is a new serve flag `--humata-root` (default `~/Humata`). `SessionRecord.folder` is the session folder, so files.rs finds each clone at `folder/<repo name>`. Session folder name = the branch slug without the prefix.
  - Clones go to `<dest>.partial` and are renamed into place, so a crash leaves a partial to remove and never a half clone in the real path. Main clone = `git clone <remote>`; session clone = clone of the main clone, origin set back to the real remote, `checkout -b <branch>` from the default branch (FC-3).
  - Branch names (FC-11..FC-14): prefix + slug of the title (lowercase a-z0-9, single hyphens, cut at a word boundary to 40 chars); `-2`, `-3` when taken locally (main clone branches), on the remote (`git ls-remote --heads`) or by another session of the channel.
  - Agents: built in `claude` (`--session-id <uuid>`, resume `--resume <uuid>`); tests pass definitions with an admin-only `--agents <json file>` and file remotes with admin-only `--git-remote <template>` (production `https://github.com/{owner}/{name}.git`).
  - StartSession steps (GRPC.md section 7): `write_record`, `clone:<repo>`..., `launch`, `verify`; launch goes through `humata2_revive::launch_agent`, so from `launch` on revive's reconcile owns it. CreateChannel's setup operation: kind ADD_REPO, steps `record`, `clone_main:<owner/name>`...
  - PER-27: fault points `sessions.start.*` and `sessions.channel.*` at every step. After a restart, a start that hadn't reached `launch` is finished forward; if finishing fails its clones and record are removed and the restart log says so. A main clone left `cloning` is cloned again. Recovery runs after revive's reconcile in the same background task, so the two never launch one agent twice.
  - Session status comes from the AgentRecord state (Launching/Resuming = STARTING, Running = WORKING, Exited = EXITED, Stopped = IDLE, ResumeFailed = RESUME_FAILED).
- Claims: `daemon/sessions/` (new), `daemon/store/src/{schema,lib,records,channels,error}.rs`, `daemon/store/tests/channels.rs` (new), `daemon/{store,holder,revive,durable}/BUILD.bazel` (visibility lines for sessions only), `daemon/humatad/src/{channel_service,session_service}.rs` (new), `daemon/humatad/src/{not_built,restart,main,operations,error}.rs`, `daemon/revive/src/{lib,reconcile}.rs` (a public resume_agent), `daemon/humatad/BUILD.bazel`, `daemon/humatad/tests/{channels,sessions}.rs` (new), `daemon/humatad/tests/{bridge,daemon}.rs` (not-built example moved to ActivityService), `daemon/humatad/tests/common/mod.rs`, `daemon/humatad/tests/bin/` (new agent script only)
- Tasks:
  - [x] T1 store: channel and session-start tables, records, API, tests · worker stor (Sonnet, medium) · files store/src/{schema,lib,records,channels,error}.rs, store/tests/channels.rs · done, reviewed (error.rs: NoChannel, NoChannelRepo)
  - [x] T2 sessions crate, pure parts: names, layout, git, agents, errors · worker names (Sonnet, medium) · files daemon/sessions/{BUILD.bazel,src/{lib,names,layout,git,agents,error}.rs} · done, reviewed
  - [x] T3 sessions crate: channel setup, session start, recovery, faults build, kill matrix · worker starts (Sonnet, medium) · files daemon/sessions/src/{setup,start,recover}.rs, sessions/BUILD.bazel, sessions/tests/kill_matrix.rs, {store,holder,revive,durable}/BUILD.bazel visibility · done, reviewed (store fault points are store.<op>.committing/committed)
  - [x] T4 humatad ChannelService + SessionService, off not_built, recovery in restart · worker serve (Sonnet, medium) · files humatad/src/{channel_service,session_service,not_built,restart,main}.rs, humatad/BUILD.bazel · done, reviewed
  - [x] T4b fix-ups: Operations dynamic steps, humatad error variants, revive resume_agent, store rename_channel · worker fixup (Sonnet, medium) · files humatad/src/{operations,error,session_service,channel_service}.rs, revive/src/{lib,reconcile}.rs, store/src/{channels,error}.rs, store/tests/channels.rs · done, reviewed
  - [x] T5 boundary tests over gRPC and the CLI, kill matrix and reboot · worker bounds (Sonnet, medium) · files humatad/tests/{channels,sessions}.rs, humatad/tests/common/mod.rs, humatad/tests/bin/ · done, reviewed (common/mod.rs append only; no BUILD change)
  - [x] G1 fixes (manager) · not-built tests use ActivityService; recovery only touches the channels and sessions the store held before serving (`Leftovers::read`), so it never races a live CreateChannel or StartSession; a resume that failed (PER-15) is a LastStart `not_brought_back` line (PER-29) · files humatad/tests/{bridge,daemon,channels}.rs, humatad/src/restart.rs, sessions/src/{recover,lib}.rs, sessions/tests/kill_matrix.rs
- Decisions for the main agent (reversible picks; give them D numbers):
  - a. Plain-column store tables, no new protos; `SessionRecord` unchanged.
  - b. `--channels` root defaults to `<root>/Humata`; a session folder is `sessions/<slug without prefix>`; clones go through `<dest>.partial` plus a rename.
  - c. CreateChannel's op is kind ADD_REPO with steps `record`, then `clone_main:<owner/name>`... (GRPC.md section 7 says `clone_main:<machine>..., record`). A failed clone fails the op and keeps the channel with that repo marked Failed.
  - d. On restart, a start that hadn't reached launch is finished forward, or undone with a LastStart line; from launch on revive owns it. Recovery runs after reconcile in the same task, and only over what the store held before the daemon served.
  - e. The branch prefix is empty until Settings or gh exist (ST-13). The default agent comes from `--default-agent`. Tests use the hidden `--agents` and `--remote-template` flags.
  - f. Machines other than this one answer UNIMPLEMENTED (comes with MACH); an empty list means this machine.
  - g. StartSession answers once the record is written; the op targets the channel. Pre-checks run before the idempotency key is used.
  - h. RenameChannel changes only the name; the folder stays.
  - i. StopSession acts only on Running; Launching and Resuming answer busy. Resume uses kind START_SESSION with steps `launch` and `verify`.
  - j. Status map: Launching/Resuming = STARTING, Running = WORKING, Exited = EXITED, Stopped = IDLE, ResumeFailed = RESUME_FAILED; an unfinished start shows STARTING.
- Waiting on: main agent: gate 2 (rebuild and rerun the failing targets); client traits seam (proposed in the first report)
- Build gates: G1 2026-11-15 build pass, tests 39/43 (bridge_test, daemon_test, channels_test half-clone, sessions_test LastStart); fixed, unbuilt
- Ready to commit: committed 9ec5b5ed (gate 2: 43/43 pass); decisions a–j are D115–D124, the client traits D125
