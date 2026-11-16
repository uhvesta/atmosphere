# FORK: Fork, move, archive and rebuild a session

- **Status:** blocked
- **Depends on:**
  - MACH: forks and moves across machines need the link
  - ASK: an agent's fork is an Inbox request
  - FRAMES2: Fork, Move, Archive with unpushed work and an archived session have no frame yet
- **Unblocks:** SAFE
- **Layer:** L3 · **Lane:** daemon + UI

## Outcome
Fork a session three ways, onto another machine. Move one. Archive with the unpushed-work check and Undo. Restore. Forget a dead machine and rebuild its sessions elsewhere.

## Exit criterion
Flows §26.5, §26.6, §26.11, §26.18, §26.19 and §26.23 pass against real daemons. A move or fork cut at any step finishes or undoes itself (PER-27 fault points, in this milestone). Undo after Archive is the 10-second delayed delete.
- Delete channel (CH-24, frame 71, D11 (a)) passes against real daemons with its PER-27 fault points; restoring from trash within 30 days brings the channel back, and its rules row goes with it (D127.6). (D128)

## Read these (pointers, nothing else)
- SPEC: FK-1–FK-13, MV-1–MV-15, LD-1–LD-12, SL-3, SL-4, FC-11–FC-17, TP-6, PER-27; flows §26.5, §26.6, §26.11, §26.18, §26.19, §26.23. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 16, 32, and the Fork, Move and Archive frames from FRAMES2 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D102, D110, D63 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `features/session/src/lib.rs`, `features/session/tests/fork.rs`, `features/session/tests/archive.rs`, `daemon/humatad/src/operations.rs`, `daemon/humatad/src/not_built.rs`, `daemon/store/src/journal.rs`, `cli/src/commands/sessions.rs`
- API: `api/proto/humata/v1/sessions.proto`, `operations.proto`
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

- CH-8 (main, 2026-11-15, from FL): ChannelService.PlanRemoveMachine/RemoveMachine is FORK's. A leaving machine's sessions are moved (MV-1–MV-15) or archived (LD-10) first; until FORK serves it, the call answers not_built.

## Out of scope
Trash, Storage and retention (FILES). 

## Lane (written by the manager)
- Design notes:
  - Early lane: local fork, archive with Undo, restore, CH-8 plan side. Cross-machine fork/move/rebuild/forget wait for MACH's relay; FK-6 handover waits for FL's start fresh summary; before-turn forks wait for FL's turns.
  - Store keeps the true fork origin (`fork_origins`) and never rewrites it: a session's shown parent is its nearest ancestor that isn't archived (FK-13), so Restore puts the tree back with no bookkeeping.
  - Archive writes its `archives` row first (the intent), then stops the agent, writes the diff summary, and frees the clones after the undo window (D110, 10 s). A restart finishes any pending free (MV-15, PER-27). Restore before the free is the Undo: drop the row and resume the agent in place, uncommitted work included. After the free, Restore clones the branch from the remote, or main when it was never pushed (LD-4), and resumes the conversation. (D172, provisional)
  - With unpushed or uncommitted work, plain ARCHIVE is refused; ARCHIVE_AND_DELETE_WORK needs the plan token (MV-9). COMMIT_PUSH_PR_THEN_ARCHIVE answers not built until batch 2 (MV-10, PlanPush).
  - Fork (same machine, same agent, from now): CODE_AND_CONTEXT and CODE_ONLY clone each parent repo onto a new branch (FC-13 slug from the fork title) at the parent's HEAD and carry its staged, unstaged and untracked files; CODE_AND_CONTEXT copies the conversation file to the fork's new conversation. CONTEXT_ONLY waits for batch 2 (FK-7 read-only clones). A fork cut before its launch is undone at restart; one past it is finished (PER-27).
  - Test windows: admin builds read `HUMATA2_ARCHIVE_UNDO_MS` so tests needn't wait 10 s.
- Claims: new `daemon/store/src/lifecycle.rs`, new `daemon/store/tests/lifecycle.rs`; new `daemon/sessions/src/{work,fork,restore}.rs`; new `daemon/humatad/src/session_service/{fork,archive}.rs`, new `daemon/humatad/src/channel_service/remove_machine.rs`; new `daemon/humatad/tests/{fork,archive}.rs`; `workflow/milestones/FORK.md` Lane; OVERVIEW block "Pick up here (minion FORK)". Seam lines go to main as scripts in the session scratchpad `fork/`: `daemon/store/src/{schema,lib,store}.rs` (migration, mod lines, remove_session cascade), `daemon/sessions/src/lib.rs` (mod/pub use), `daemon/humatad/src/session_service.rs` (trait arms, session_pb, ListSessions, mod lines, after-restart hook), `daemon/humatad/src/channel_service.rs` (PlanRemoveMachine arm).
- Tasks:
  - [x] T1 store `fork_origins` + `archives` tables and Store methods · manager · seam s1-store.py sent to main · `daemon/store/src/lifecycle.rs`, `daemon/store/tests/lifecycle.rs` · test first
  - [x] T2 sessions crate: repo work check + diff summary (`work.rs`), fork clone copy (`fork.rs`), restore clone (`restore.rs`), cut forks undone in `recover.rs` · worker T2 (Sonnet, medium) · done: sessions_unit_test 29 pass, humatad2 builds; start.rs edits: pick/channel_record pub(crate), launch_with split out of launch_session; git.rs helpers pub(crate)
  - [ ] T3 daemon (not started: the worker spawn was refused by the permission check; brief ready in the FORK handback): PreviewFork, ForkSession, PlanArchive, ArchiveSession, RestoreSession, ListSessions include_archived, fork_origin/forks/archive on Session; restart finishes frees and undoes cut forks · worker (Sonnet, medium) · `session_service/{fork,archive}.rs` + seams · e2e `tests/{fork,archive}.rs` written first by the manager
  - [ ] T4 ChannelService.PlanRemoveMachine plan side (CH-8) · `channel_service/remove_machine.rs` + seam
- Waiting on: MACH relay (cross-machine), FL start fresh summary (FK-6), FL turns (before-turn forks)
- Build gates:
- Ready to commit: no
