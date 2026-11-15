# Work log

The written record of who is working on what. Read top to bottom when
starting or resuming (see `workflow/roles/MAIN.md` › Starting or resuming).
Each agent edits only its own lane; the main agent edits the rest.

## Resume here

- **State:** walking the DAG. D0 and P0 done (P0 1a3831dd). Lanes open:
  FL-D (daemon, minion FL-D) and FRAMES1 (docs, minion FRAMES1).
- **Next:** gate each batch the managers report. Open the UI lane (LOOK2,
  with P0's deferred panels.rs/native.rs split) once C1 commits LOOK1 and
  releases `ui/`. ASK after FL-D. W1 is waiting on the owner (D113 frames,
  D103–D112).
- **Not ours in the tree:** C1's uncommitted LOOK1 work (queue/presence in
  clients workspace.rs, features/session, ui, native.rs + new snapshots, the
  `terminal_peers` line in clients/src/fixture/design_session.rs);
  `ui-prototype/` belongs to another session.
- **Lane limit:** 3 managers (2 build, 1 docs).
- **Last pushed:** 1a3831dd (origin/main).

## Roster

| Agent | Model | Role | Lane / task | Started | Status |
|---|---|---|---|---|---|
| main | Opus 5.5 | main agent | — | 2026-11-15 | active |
| minion C1 | — | restyle app to goldens (launched by the owner) | N1 restyle; uncommitted work in `clients/`, `features/{app,session}`, `ui/`, `desktop/tests/native*` | 2026-11-15 | active, outside this workflow |
| minion R1 | Fable | reviewer | cohesion, goal milestones, DAG; report at scratchpad `r1-review.md` | 2026-11-15 | done |
| minion A1 | Fable | advisor | how to edit code and docs, given R1; report at scratchpad `a1-advice.md` | 2026-11-15 | done |
| minion S1 | Opus 5.5 | safety pass | pushed state, inventory, WIP backup ref `refs/wip/2026-11-15-safety` | 2026-11-15 | done |
| minion ASK | Opus 5.5 | manager | ASK (agent a7b618c1fe3289cef) | 2026-11-15 | active |
| minion J1 | Fable 5.2 (high) | judge | rule on R1 (and A1) findings; tidy SPEC, DECISIONS, OVERVIEW (not C1's block, not goldens); change log at scratchpad `j1-changes.md` | 2026-11-15 | done |
| minion P0 | Opus 5.5 | pre-manager | docs sweep left after J1; mechanical split of hot files C1 isn't editing (panels.rs, native.rs deferred) | 2026-11-15 | done |
| minion D1 | Fable | DAG writer | `workflow/DAG.md` + `workflow/milestones/<ID>.md` from R1, A1, J1 | 2026-11-15 | done |
| minion FL-D | Opus 5.5 | manager | milestone FL-D (daemon lane) | 2026-11-15 | done (9ec5b5ed) |
| minion FRAMES1 | Opus 5.5 | manager | milestone FRAMES1 (docs lane, draft frames) | 2026-11-15 | done (59340201) |

## Claims

Files a lane is editing. No two lanes claim the same file.

| Lane | Paths |
|---|---|
| C1 | `clients/src/{daemon,fixture,workspace}.rs`, `clients/tests/fake_daemon.rs`, `desktop/tests/native.rs`, `desktop/tests/__snapshots__/native/`, `features/app/src/lib.rs`, `features/session/src/lib.rs`, `ui/BUILD.bazel`, `ui/src/{lib,overlays,pages,session_tabs,shell}.rs`, its OVERVIEW pickup block |
| P0 | `LAYOUT.md`, `api/{BOUNDARIES,MCP,GRPC}.md` ("Always let" → "Always allow"); `features/app/src/` (lib.rs and new sibling files); `clients/BUILD.bazel`, `clients/src/{daemon,fixture}.rs`, new `clients/src/{daemon,fixture}/`; `ui/src/overlays.rs` and new `ui/src/overlay_*.rs` (all clean in git status when claimed; not panels.rs, native.rs, ui/src/lib.rs) |
| ASK | `daemon/humatad/src/{identity,restart,terminal_session,inbox,inbox/deferred,inbox/kept,grant_target,session_service,channel_service,terminal,files}.rs`, `daemon/humatad/tests/{auth,inbox,sessions,channels,files,terminal}.rs`, `daemon/humatad/tests/common/mod.rs` (additive), `workflow/milestones/ASK.md` Lane |
| LOOK1 | `features/review/src/lib.rs`, `features/review/tests/`, `features/session/src/{lib,message}.rs`, `features/session/tests/queue.rs`, `features/app/src/messages.rs`, `features/app/tests/messages.rs`, `ui/src/{panels,session_tabs,sidebar,terminals,theme,chrome}.rs`, `features/sidebar/`, `clients/src/fixture/{design_session,sessions,requests,terminal}.rs`, `desktop/tests/{native,panels,shell}.rs`, `desktop/tests/__snapshots__/{native,panels,shell}/`, OVERVIEW block "Pick up here (minion LOOK1)", `workflow/milestones/LOOK1.md` Lane (takes over C1's row) |

## Lanes

Each lane's plan, tasks and workers live in its milestone file
(`workflow/milestones/<ID>.md` › Lane). Here, one line per active lane:

| Lane | Milestone file | Manager | Workers running | State |
|---|---|---|---|---|
| ASK | `milestones/ASK.md` | minion ASK | sec15, cards, handlers | batch 2 (daemon): SEC-15, deferred card previews + store flag, more handlers deferred |
| LOOK1 | `milestones/LOOK1.md` | minion LOOK1 | send-core, send-ui, audit | batch 1: direct/individual comment Send (D126), frame audit 01–17 |
| P0 | none (pre-manager step) | minion P0 | none | done, waiting on main build: LAYOUT.md rewritten, api "Always allow"; split features/app lib.rs, clients daemon.rs + fixture.rs (glob now src/**), ui overlays.rs (#[path] children); panels.rs, native.rs deferred (C1) |

Old template, kept for reference:

```text
### <lane>: <milestone> (manager: minion <lane>, Opus 5.5)
- Goal / exit criterion:
- Design notes:
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on: (main agent / owner / another lane)
- Last build gate: <time> pass|fail (targets)
- Ready to commit: no | yes: <summary for the reviewer>
```

## Build gates

| When | Lanes | Targets | Result |
|---|---|---|---|

## Event log

Newest last. One line per event: spawns, reports, gates, commits, decisions.

- 2026-11-15 main: wrote `workflow/` (WORKFLOW, roles, WORKLOG); started R1.
- 2026-11-15 R1 done: 10 cohesion problems, 19 goal milestones, layered DAG (critical path LOOK → FL → MACH → SAFE → LINUX → ANYHOST → V1; 2 building lanes + 1 docs lane). Started A1 (advice) and S1 (safety) in parallel.
- 2026-11-15 S1 done: HEAD = origin/main, LFS complete, no half states, no secrets in the last 20 commits. C1's 32 uncommitted files backed up to `refs/wip/2026-11-15-safety` (67a307ea, pushed). Delete that ref after C1 commits. Two untracked `.claude/handoffs/*.md` stay uncommitted (`.claude/` is never committed).
- 2026-11-15 main: milestone files (`workflow/milestones/`) added on owner request; agents read their milestone file and its pointers, not the big docs.
- 2026-11-15 main: started J1 (judge, Fable high) on owner request. Order: owner directions > goldens > SPEC; golden changes listed for the owner, not made.
- 2026-11-15 main: A1 done (scratchpad `a1-advice.md`), forwarded to J1. Added the `dag-walk` skill (`tools/skills/dag-walk/`, linked locally at `.claude/skills/dag-walk`) and `workflow/CONFIG.md`.
- 2026-11-15 main: owner asked: after J1, an Opus 5.5 agent (P0) does A1's pre-manager work: C1 landed first (C1 is active and commits itself; main waits for its files to go clean), docs sweep left after J1, one-time split of the hot shared files. Then dag-walk plan step 3 (DAG.md + milestone files), then dag-walk resume.
- 2026-11-15 main: C1 landed its work (9158b063, eb4cb93f, pushed). Pre-manager step (a) done. Safety ref kept for now (it also holds the other session's ui-prototype work).
- 2026-11-15 main: J1 done; committed its SPEC/DECISIONS/OVERVIEW edits (C1's pickup block staged from HEAD). D103–D112 Provisional, D113 frame changes await the owner.
- 2026-11-15 main: J1 committed (ba1a357e). Started P0 (Opus) and D1 (Fable, DAG files). C1 active again on Changes wrapping (ui panels/shell/lib, native.rs).
- 2026-11-15 main: D1 done: DAG.md + 26 milestone files. Scripted syncs moved into MACH (D49). Skill honours `*` edges. Next: P0 report → gate → commit → dag-walk resume (FL-D daemon lane, FRAMES1 docs lane first).
- 2026-11-15 main: gate for P0: build pass; tests 20/21, native_test 8 failures all from C1's uncommitted queue/presence work. rustfmt check pass. `--config=lint` broken repo-wide. Committed P0 (C1's `terminal_peers` line in fixture/design_session.rs left unstaged).
- 2026-11-15 main: dag-walk resume. P0 done. Opened FL-D (daemon) and FRAMES1 (docs). UI lane waits for C1 (LOOK1) to release `ui/`; LOOK2 then takes P0's deferred splits.
- 2026-11-15 main: spawned minion FL-D (agent a69abb57) and minion FRAMES1 (agent a0c47d8d), both Opus 5.5 managers in the background.
- 2026-11-15 FRAMES1: batch 1 reported: frames 44–55 drafted (design.rs, design/{session,channel,pages,overlays}.rs additive; new design/{new_channel,resume,channel_tabs,comments,cleanup}.rs). Waiting on the gate.
- 2026-11-15 main: FRAMES1 batch 1 (frames 44–55 drafted); gate build blocked by C1's uncommitted Delegate::CommentsSent/CommentsUnqueued (features/app non-exhaustive match); waiting for C1 to settle. FL-D batch 1 reported (store tables, daemon/sessions crate, Channel/Session services, 14 boundary tests); main applied its serve.rs seam lines (--agents and --remote-template hidden, not admin-gated); FL-D build gate running.
- 2026-11-15 main: gate 1 FL-D: build pass (all daemon crates). Tests 39/43; failures: bridge_test + daemon_test (ChannelService no longer not-built), channels_test half-clone, sessions_test LastStart empty. Sent to minion FL-D.
- 2026-11-15 main: gate 1 FRAMES1: design_test builds once C1 fixed its Delegate match; 01–43 unchanged; recorded 44–55 (untracked drafts). Sent back: 55 date/size overlap, 51 duplicate 'addressed'.
- 2026-11-15 main: FRAMES1 fixes re-recorded (51, 55 clean); all 12 drafts sent to owner for approval. FRAMES1 holds until owner answers (not committed: goldens need owner OK). FL-D gate 2 tests running.
- 2026-11-15 main: gate 2 FL-D: 43/43 pass. rustfmt applied. Closed FL-D: 9ec5b5ed; decisions D115–D125 Provisional; claims released, lane ended.
- 2026-11-15 main: opened ASK (daemon lane), manager minion ASK (Opus 5.5).
- 2026-11-15 main: ASK batch 1 reported (T1–T5, decisions A1–A11). Main applied seams S1 (errors.proto ApprovalPending=57), S2 (mod rules_kept), S3 (serve.rs restore + Access store + inbox to ChannelSvc/SessionSvc; DaemonError::RulesKept), S4 (events Rules filter). S5 deferred. Gate 1 ASK running.
- 2026-11-15 main: gate 1 ASK: main fixed its own RulesKept {0} and added exit_codes pending arm; build pass; humatad+cli 28/29, store+sessions pass; matrix_test 3 cases fail (--wait ends pending after deny; MCP went quiet). Sent to minion ASK.
- 2026-11-15 main: gate 2 ASK: 45/45 pass. Committed ASK batch 1 as 4d873e4c (D127). ASK stays open: batch 2 next.
- 2026-11-15 main: owner approved frames 44–55 with main's picks (FRAMES1 asked for the 45 Cancel + failed/Retry change) and reads allowed by default (D127.4, sent to ASK). C1 stopped (9e185a5b, C1_HANDOFF.md); its D126 committed, ASK's renumbered D127 (e5ac5a24). Opened LOOK1 as a workflow lane, manager minion LOOK1.
- 2026-11-15 main: FRAMES1 recorded with the 45 change; design_test pass, 01–43 unchanged. Closed FRAMES1: 59340201. Claims released.
- 2026-11-15 main: opened FRAMES2 (docs lane), manager minion FRAMES2 (Opus 5.5).
