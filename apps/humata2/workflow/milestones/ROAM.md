# ROAM: Agents propose channel repos, Desk sessions move into channels, each channel has its own Desk, and any agent conversation forks and moves

**Owner asks (2026-11-16):**
1. Agents may propose adding a repo to a channel. IN-17's "Change channel settings or repos" is Never; split out "Add a repo to a channel" as Ask me.
2. An agent proposes moving a Desk session into the channel it belongs to; the user accepts or declines. The session becomes a channel session, carrying its folder, conversation and repos.
3. A channel has its own Desk: ephemeral, managerial sessions for that channel or one of its sessions. They use that channel's rules plus management grants. No branch, fork or board of their own.
4. Our own fork and move for Claude Code, Codex and Copilot conversations, built on each CLI's storage (Claude JSONL, Codex rollout files and SQLite state, Copilot session-state). Fork at a turn, carry a conversation to another machine or channel, and resume it there. Unfinished input and background jobs are not carried. Start from terminal/host/AGENT_RECOVERY.md and its passing real-CLI fixtures, and from D-entries on FK-3 (claude-jsonl only today).

**Exit:** SPEC rules for each ask (new IDs, frames listed for the goldens flow), then behaviour tests on fakes and real-CLI fixtures; fork and move work for all three agents.

**Rules:** AGENTS.md; spec and frames before UI; frames go to the owner before any golden changes; Bazel only.

## Lane
(manager writes here)

### Goal and exit
Exit as above. Spec is done (step 1, 2026-11-16):
- SPEC: CH-26, CH-24 and CH-11 additions, IN-17 rows, DK-15–DK-19, CD-1–CD-6, CV-1–CV-7, FK-3, MV-13, q11.
- DECISIONS: D304–D307 and the "Frames for ROAM" list. Main numbered the frames 87–95; frame 18's tab row rides with 91.
- An Opus xhigh re-judge of the picks (the owner's no-Fable rule) is applied (2026-11-16). Changes:
  - IN-17 splits "Stop or archive another session: Ask me" from "Delete another session: Never";
  - CD-3 has six switches (notes and typing are separate, typing is off);
  - new CV-8, Continue in another channel…;
  - DK-17 offers git checkouts as repos;
  - new frames 96 and 97, plus re-renders of the Agent access and Delete channel frames.

### Research (step 2)
- Proven by real-CLI fixtures (terminal/provider/tests):
  - native fork and exact-id resume for all three: Claude 2.1.325, Codex 0.169.0, Copilot 1.0.112-0;
  - isolation env: HOME+CLAUDE_CONFIG_DIR, HOME+CODEX_HOME, HOME+COPILOT_HOME+COPILOT_CACHE_HOME (+XDG_*);
  - Copilot's `session-state/<uuid>/events.jsonl` with `session.start`, `user.message` and `assistant.message` events.
- Not proven by anything:
  - Codex rollout paths, line types and SQLite db/tables, and whether resume needs a db row;
  - Copilot's other files (workspace.yaml?) and where cwd is kept;
  - whether a hand-truncated or renamed file resumes, for any CLI;
  - whether Claude lines carry `cwd`/`sessionId`, and whether a copy with a stale `sessionId` resumes (fork.rs does this today, untested against the real CLI);
  - folder-trust keys.
- So the build starts with **layout discovery** fixtures. Each adapter's format is written down only from what they record.

### Design notes
- **New single-job crate `daemon/conversation` (`humata2_conversation`)**. It is sync, has no gRPC, and knows only the CLIs' files:
  - `trait Adapter { fn turns(&self, conv: &Conversation) -> Result<Vec<Turn>, ConversationError>; fn fork(&self, conv, before_turn: Option<u32>, new_id, to: &Placement) -> Result<Conversation, _>; fn pack(&self, conv, stage: &Path) -> Result<Packed, _>; fn place(&self, packed, from: &Placement, to: &Placement) -> Result<Conversation, _>; fn check(&self, conv, cli_version) -> Result<(), Unsupported>; }`;
  - `Placement { home, folder, cli_home }` locates the files. `Conversation { id, files }` lists every file that belongs to the conversation, with its role.
  - Adapters: `ClaudeJsonl`, `CodexRollout`, `CopilotSessionState`, each in its own module, and `fn adapter(format) -> Option<&dyn Adapter>`.
  - Unknown format or version gives `Unsupported` (CV-3). There is no byte-copy fallback.
- `humata2_sessions` later swaps `copy_conversation`/`turn_offset` (fork.rs), `project_keyed` and `replacements` (carry.rs, the agent part only) and `ConversationFormat` for calls into the crate. `ConversationFormat` gains `codex-rollout` and `copilot-session-state`, and the built-ins gain codex and copilot kinds. That wiring is batch 3: sessions/src/carry.rs is SAFE's claim, so it goes as a SEAM via main.
- **Fixtures:** `daemon/conversation/fixtures/` holds Python real-CLI tests (sh_test wrappers, as terminal/provider does). They use a temp HOME and the CLIs' home env vars, and a loopback model server copied from terminal/provider/tests (no upstream calls). They are tagged `manual`, `external`, `no-sandbox` and `real-cli`, so they never run in the normal gate. Each writes a layout report (tree, line types, sqlite schema) to TEST_UNDECLARED_OUTPUTS_DIR and asserts the facts the adapter relies on. Recorded, scrubbed sample files go to `daemon/conversation/testdata/<cli>/<version>/`, and the crate's unit tests run on them in the normal gate.
- **Ask 1:** split `Capability::ChannelRepoAdd` out of `ChannelChange` (proto, access, guard, access_service, mcp tools, policy default Ask me, no IN-22 target). AddRepo from an agent gates on it. A channel agent proposing for another channel gets Never, and a Desk agent may propose for any channel. Card kind "repo proposal". `also_to_session` is a second gated step (OwnSessionManage). A card answered when the repo is already in the channel resolves "Already added".
- **Ask 2:** `MoveDeskSessionToChannel` on DeskService, with the user path plus an agent path gated by `DeskMoveToChannel` (AskNoAllow; While-I'm-away never). Carry runs through the existing pack/unpack plus the adapter. Batch 2, after the judge.
- **Ask 3:** channel Desk = `SessionKind::ChannelDesk { channel, target, switches }`: the record, the folder `<channel>/desk/<name>`, the switches as grants after Never, and auto-close. Batch 2/3. The UI waits for the owner's OK on frames 87–95.

### Tasks
Batch 1 (now):
- [ ] F1 frames 87–95 (Sonnet medium) · new `desktop/tests/design/roam.rs`; SEAM `desktop/tests/design.rs` (mod line + 9 rows) · script `roam/f1_frames.py` · main records it with design_test_record and copies the PNGs to scratchpad/roam/frames/, without committing goldens.
- [ ] X1 layout-discovery fixtures, Claude/Codex/Copilot (Sonnet medium) · new `daemon/conversation/fixtures/`, new `daemon/conversation/BUILD.bazel` (fixture targets only) · script `roam/x1_fixtures.py` · targets `//apps/humata2/daemon/conversation:{claude,codex,copilot}_layout_test` (manual, real-cli).
- [ ] C1 crate API + failing tests for the Claude adapter (Opus) · new `daemon/conversation/{Cargo.toml,src/,tests/,testdata/claude/}` and its BUILD lib/test blocks · SEAM root `Cargo.toml` member · script `roam/c1_crate.py`.
- [ ] A1 ask 1 daemon (Sonnet medium) · `daemon/humatad/src/channel_service/add.rs`, new `daemon/humatad/tests/channel_repo_propose.rs`; SEAMs: `api/proto/humata/v1/capabilities.proto`, `daemon/access/src/{capability,policy}.rs`, `daemon/humatad/src/{guard,access_service}.rs`, `daemon/mcp/src/tools.rs` · script `roam/a1_repo_propose.py`.
Batch 2:
- C2: Haiku implements the Claude adapter to C1's tests.
- X2: hand-fork-resume and carry fixtures per CLI, once X1's reports are in.
- C3/C4: Codex and Copilot adapters from X1/X2's testdata.
- D1: Desk→channel daemon move.
- K1: channel Desk daemon (kind, record, access, auto-close).
- A2:
  - split `SessionStopOther` (stop/archive, now AskMe) from a new `SessionDeleteOther` (Never);
  - CH-26's answer at call time, "Already in the channel";
  - the `also_to_session` tick, once AddRepoToSession is built.
- F2: frames 96 and 97, and re-render the Agent access and Delete channel frames.
- V8: CV-8, Continue in another channel… (daemon, on the C crate).
Batch 3: wire the crate into sessions fork/carry and ListTurns (SEAM: SAFE's carry.rs), add codex/copilot built-in kinds, the CLI, and the app after the frames are approved.

### Owner adds (2026-11-16, D313)
- MV-16: a session's folder follows the session on every move. That covers Desk → channel, a channel rename or restore, machine → machine, and conversation roam.
- MV-17: Desk and channel-Desk sessions can be created on any machine and moved to any machine. Moves are cheap and only one copy is ever live. This includes flow 01, step 6.
- The plan:
  1. The audit lists every path left behind and every blocker.
  2. An Opus test author writes the MV-16 tests inside the D1 rebase, plus a channel rename/restore K-fix.
  3. The same author writes the MV-17 tests: remote create and move for the Desk kinds, dedupe, progress, and faults.
  4. Implementers follow. Every edit is an anchored hunk.

### Workers
| Worker | Task | Files | Status |
|---|---|---|---|
| Sonnet medium | F1 frames 87–95 | desktop/tests/design/roam.rs; SEAM design.rs and design/sidebar.rs | script sent to main |
| Sonnet medium | X1 layout fixtures | daemon/conversation/{BUILD.bazel,fixtures/} | script sent to main |
| Sonnet medium | A1 repo proposals | channel_service/add.rs, tests/channel_repo_propose.rs; SEAMs | script sent to main |
| Opus high | C1 crate API and tests | daemon/conversation/{Cargo.toml,src,tests,testdata}; SEAM root Cargo.toml | applied |
| Sonnet medium | C2 Claude adapter | conversation/src/claude.rs | applied, gate pending |
| Opus high | C2b Claude real shape (dashing, kinds) | conversation; SEAM sessions agents.rs/carry.rs, humatad tests/context_fork.rs | script sent to main |
| Sonnet medium | X1b Copilot id probe fix | conversation/fixtures | script sent to main |
| Opus high | C3 Codex contract and tests | conversation/src/codex.rs, tests/codex.rs, testdata/codex | script sent to main |
| Haiku high | C4 Codex adapter | conversation/src/codex.rs | script sent to main (after C3, C2b) |
| Opus high | K1 channel Desk tests and stubs | access, store, sessions, humatad desk_service; SEAM desk/channels.proto, store lib/schema, serve.rs | scripts k1a, k1b sent to main |
| Sonnet medium | K2 channel Desk impl | access/src/channel_desk.rs, desk_service/channel_desk{,_close}.rs, close.rs hunk | script sent to main (after k1a, k1b); follow-up: restored channel under renamed folder |
| Opus high | D1 Desk→channel move tests | store/sessions/humatad desk_move; SEAM desk/capabilities/inbox.proto, policy.rs, serve.rs, mcp tools.rs | superseded by D1R: d1r_desk_move_tests.py sent to main (carry Landing, D315 refusal, branch re-point; CarryLanding SEAM dropped, final) |
| Opus high | MV-16 audit: paths left behind on every move | read-only | done: scratchpad/roam/mv16_audit.md |
| Sonnet medium | Gate 13 fixes | carry.rs test input (dashing); tests/channel_desk.rs waits | carry applied; channel_desk script sent |
| Sonnet medium | Move-cut fixes (Mover::undo resume, carry engine leftovers) | move_session.rs undo, humatad carry.rs engine | script sent to main (fix_move_cut.py) |
| Opus high | T-A MV-16 conversation + carry tests | conversation crate, carry replacements/AgentFiles, fork copy_conversation, session_snapshot | script sent to main (ta_mv16_conversation.py) |
| Sonnet medium | T-A impl MV-16 (conversation roams whole; D336 rows) | conversation crate, sessions agents/fork/carry (not pack/unpack), store import; SEAM serve.rs with_cli_home | script sent to main (ta_impl_mv16.py; D347); carry_roam red until ta_carry_pack |
| Sonnet medium | Carry pack/unpack through the adapter (carry_roam green) | sessions carry.rs pack/unpack after T-D | script sent to main (ta_carry_pack.py); follow-up ta_carry_prev.py sent (previous conversations, Copilot by kind) |
| Opus high | T-B MV-16 rename + restore-elsewhere tests | channel_snapshot, channel_service rename/restore, channel_desk_close after_restore; SEAM channels.proto | script sent to main (tb_mv16_channel_folder.py) |
| Sonnet medium | T-B impl MV-16 rename/restore | channel_service, trash_service, channel_desk_close, channel_snapshot; carry.rs visibility only | script sent to main (tb_impl_mv16.py); follow-up tb_restore_note.py sent; remote-machine folder rename owed after T-D |
| Opus high | T-D MV-17 create/move anywhere, cheap, one live copy | BUILD RIG_TESTS, api/GRPC.md copy-bytes bullet | script sent to main (td_mv17_machines.py) |
| Opus high | MV-4 cases 30/31 tests (Moving status, Type refused while moving) | tests: humatad move_typing, clients moving_status, live map pin; SEAM sessions.proto moving_to=33 | script sent to main (mv4_moving_tests.py) |
| Haiku high | MV-4 daemon impl (Type refused, moving_to/running_operation) | terminal.rs type_in, move_session.rs Moving::begin | script sent to main (mv4_impl_daemon.py, 17 hunks incl. type_comments); type_comments test sent (move_typing_comments.py) |
| Sonnet medium | MV-4 steps follow-up (MoveProgress.steps, card lists every step) | clients model (SEAM), live map, fake, ui pages | rebased as mv4_steps_v2.py (rerun guard), sent to main |
| Haiku high | Bug 34 move half: refuse a move to a machine needing migration | test upgrade_refusals.rs; move_session.rs machine_ready | script sent to main (bug34_move_gate.py) |
| Sonnet medium | MV-4 clients/ui impl (Status::Moving, Session.moving) | clients model (SEAM), live map, fake, ui/features matches | rebased onto pull_request_review and TERM as mv4_impl_clients_v2.py, sent to main; rest of ROAM applied |
| Sonnet medium | T-D impl MV-17 | desk start, remote_work, carry pack/unpack, move_session, transfers; SEAM serve.rs | script sent to main (td_impl_mv17.py); per-file reuse (D346) |
| Sonnet medium | MV-17 chunk-level reuse (HaveChunks RPC, SEAM proto) | move_cheap test, transfer proto, .carry/have chunk store | script sent to main (td_chunks.py); td_chunks_2.py sent to main (Target rpc SEAM, pre-placed Claude conversation, rising copy progress; D349); td_chunks_2b.py makes the progress test deterministic with a FIFO hold point, sent to main |
| Sonnet medium | Remote Desk close/clean/sweep + channel rename on every machine | desk_roam tests, relay allowlist, desk_service, rename across remote_work | script sent to main (td_remote_followup.py; D348); td_remote_followup_2.py sent to main (busy-box and clash Inbox ask, remote repo/doc lists, watcher, close_for_channel, Bin restore; no SEAM) |
| Sonnet medium | D2 Desk→channel move impl | store/src/desk_move.rs, sessions/src/desk_move.rs, desk_service/move_to_channel.rs | D2R: d2r_desk_move_impl.py sent to main (owning machine, landing journal, re-sync after stop, symlinks, Codex rollout undo); check ok on the tree with d1r; d2_followups.py (shared tree_copy, Codex git fields on undo) sent |
| manager | Rig: a link that ended by itself counts as ended (ESRCH, D317) | humatad/tests/common/remote_rig.rs | rig_cut_esrch.py sent to main |
