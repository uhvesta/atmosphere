# Work log

The written record of who is working on what. Read top to bottom when
starting or resuming (see `workflow/roles/MAIN.md` › Starting or resuming).
Each agent edits only its own lane; the main agent edits the rest.

## Resume here

- **State:** walking the DAG. Done: D0, P0, FL-D, FRAMES1, FRAMES2, W1, TIDY. Lanes open: LOOK1 (UI; batches 4–5
  committed, re-audit of 01–17 next, then close), ASK (can't close before FL). Everything else waits on LOOK1.
- **Owner, 2026-11-15:** opinions go to a Fable advisor, not the owner (D128); the owner still sees golden changes as
  before/after images. W1 answers logged: D103–D112 settled, D9 delete, only the VM tier stays out of 1.0.
- **Next:** LOOK1 re-audit 01–17 → close LOOK1 → open FL, BUG, LOOK2 (lane limit 3; ASK keeps its lane).
- **Not ours in the tree:** `ui-prototype/` (another session), `.claude/`, `terminal/{embed,lab,session}/`, the terminal
  experiment's BUILD edits (`daemon/holder`, `terminal/pane`, `terminal/view`) and its LAYOUT/OVERVIEW/C1_HANDOFF hunks.
- **Lane limit:** 3 managers (2 build, 1 docs).
- **Last pushed:** 3ad37e47 (origin/main).

## Roster

| Agent | Model | Role | Lane / task | Started | Status |
|---|---|---|---|---|---|
| main | Opus 5.5 | main agent | — | 2026-11-15 | active |
| minion C1 | — | restyle app to goldens (launched by the owner) | N1 restyle; uncommitted work in `clients/`, `features/{app,session}`, `ui/`, `desktop/tests/native*` | 2026-11-15 | active, outside this workflow |
| minion R1 | Fable | reviewer | cohesion, goal milestones, DAG; report at scratchpad `r1-review.md` | 2026-11-15 | done |
| minion A1 | Fable | advisor | how to edit code and docs, given R1; report at scratchpad `a1-advice.md` | 2026-11-15 | done |
| minion S1 | Opus 5.5 | safety pass | pushed state, inventory, WIP backup ref `refs/wip/2026-11-15-safety` | 2026-11-15 | done |
| minion ASK | Opus 5.5 | manager | ASK (agent aade2cf78d04ce55c, fresh after batch 3; earlier a7b618c1fe3289cef) | 2026-11-15 | dormant (no claims; reopen after FL T8, see event log) |
| minion J1 | Fable 5.2 (high) | judge | rule on R1 (and A1) findings; tidy SPEC, DECISIONS, OVERVIEW (not C1's block, not goldens); change log at scratchpad `j1-changes.md` | 2026-11-15 | done |
| minion P0 | Opus 5.5 | pre-manager | docs sweep left after J1; mechanical split of hot files C1 isn't editing (panels.rs, native.rs deferred) | 2026-11-15 | done |
| minion D1 | Fable | DAG writer | `workflow/DAG.md` + `workflow/milestones/<ID>.md` from R1, A1, J1 | 2026-11-15 | done |
| minion FL-D | Opus 5.5 | manager | milestone FL-D (daemon lane) | 2026-11-15 | done (9ec5b5ed) |
| minion FRAMES1 | Opus 5.5 | manager | milestone FRAMES1 (docs lane, draft frames) | 2026-11-15 | done (59340201) |
| minion W1 | Opus 5.5 | manager | milestone W1 (docs lane, D113 redraws; agent a4944fb04f53415e9, fresh) | 2026-11-15 | done (W1 closed) |
| minion TIDY | Opus 5.5 | manager | milestone TIDY (daemon lane; spec mode removal, D9 folder deletion as its own batch; agent af1e27564f8ff1689) | 2026-11-15 | done (4a636632, d17f0d4e) |
| minion FRAMES2 | Opus 5.5 | manager | milestone FRAMES2 (docs lane, draft frames 56–71) | 2026-11-15 | done (43f7fff2) |
| minion LOOK1 | Opus 5.5 | manager | milestone LOOK1 (UI lane; agent af7e2efa98ee6c9aa, fresh after e07e8653) | 2026-11-15 | done (ed5882ed) |
| minion FL | Opus 5.5 | manager | milestone FL (agent a2a29874ff315ecb9; daemon + UI seam; opened while LOOK1's last gate waits, see event log) | 2026-11-15 | active |
| minion BUG | Opus 5.5 | manager | milestone BUG (agent ad1189090980f241c; third builder, `report/`; opened while LOOK1's last gate waits) | 2026-11-15 | done (ed5882ed) |
| minion MACH | Fable 5.2 (high) | manager | MACH early lane (agent a2d8d9980ac7c6f4e) | 2026-11-15 | active |
| minion FILES | Fable 5.2 (high) | manager | FILES early lane (agent a33ddd89c33e932bf) | 2026-11-15 | active |
| minion REVIEW | Fable 5.2 (high) | manager | REVIEW early lane, daemon+model only (agent a4356464f541e9d35) | 2026-11-15 | active |
| minion LOOK2 | Fable 5.2 (high) | manager | milestone LOOK2 (agent ab57e5f1bec944dcf; UI; plans while LOOK1's last batch commits, edits LOOK1-claimed files only after) | 2026-11-15 | starting |
| minion DESK | Fable 5.2 (high) | manager | DESK early lane (agent af25132ec5c37f4a8; new features/desk, desk fixtures; no existing view changes until LOOK2 says so) | 2026-11-15 | starting |

## Claims

Files a lane is editing. No two lanes claim the same file.

| Lane | Paths |
|---|---|
| BUG | `report/` (src, tests, BUILD.bazel), `features/app/src/report.rs` (new), `features/app/src/overlay.rs` (Escape closes the form), `features/app/src/toast.rs`, `ui/src/overlay_toasts.rs`, `features/app/tests/report.rs` (new), `ui/src/record.rs`, `ui/src/report_view.rs` (new), `ui/src/keys.rs`, `ui/src/bridge.rs` (Overlay part observes report state), `desktop/src/main.rs` (Help menu items, reports folder), `desktop/tests/report.rs` (new), `cli/tests/report.rs`, `workflow/milestones/BUG.md` Lane, OVERVIEW block "Pick up here (minion BUG)". Asks main for seam lines in `features/app/src/lib.rs`, `ui/src/{lib,shell,overlays}.rs`, `desktop/BUILD.bazel`, `cli/src/live.rs` (ASK) |
| FL | `clients/live/` (new), `desktop/install/` (new), `desktop/src/live.rs` (new), `desktop/src/main.rs` (live backend; handed over by main after BUG 2026-11-15), `daemon/humatad/src/session_service/watch.rs` (new, T5b), `daemon/app_peer/` (new), `daemon/humatad/BUILD.bazel` (visibility for clients/live, a test-only holder filegroup, the app_peer dep and identity env), `daemon/humatad/src/{session_service,channel_service,events}.rs` (T5, additive: publish calls and Session/Channel filter arms), `daemon/humatad/src/operations.rs` (one `feed()` accessor), `daemon/humatad/src/{serve,identity}.rs` (D135 app caller; handed over by main 2026-11-15), `clients/src/daemon.rs` (DaemonError::NotServedYet), root `Cargo.toml` members for the new crates, `workflow/milestones/FL.md` Lane, OVERVIEW block "Pick up here (minion FL)" |
| LOOK1 | `features/review/src/lib.rs`, `features/review/tests/`, `features/session/src/{lib,message}.rs`, `features/session/tests/queue.rs`, `features/app/src/messages.rs`, `features/app/tests/messages.rs`, `ui/src/{panels,session_tabs,sidebar,terminals,theme,chrome}.rs`, `features/sidebar/`, `clients/src/fixture/{design_session,sessions,requests,terminal,machines,items,journal,files}.rs`, `terminal/pane/src/{pane,host}.rs`, `terminal/pane/tests/unfitted.rs`, `clients/src/daemon/fake/{terminals,machines,sessions}.rs`, `clients/src/fixture.rs` (views and machines literals only), `clients/src/model.rs` (Session/Recovery/PreparedView display fields, ASK idle; main OK 2026-11-15), `terminal/view/src/frame.rs` (not its BUILD.bazel), `terminal/view/src/view.rs` (scrollbar thumb painting only; main OK 2026-11-15), `features/settings/src/lib.rs` + `features/settings/tests/preferences.rs` (wrap_diffs default only), `desktop/tests/design/{session,comments}.rs` (owner-approved golden fixes for frames 01/05/50), `desktop/tests/{native,panels,shell}.rs`, `desktop/tests/__snapshots__/{native,panels,shell}/`, OVERVIEW block "Pick up here (minion LOOK1)", `workflow/milestones/LOOK1.md` Lane (takes over C1's row) |
| MACH (early) | `daemon/machines/` (all); `daemon/humatad/src/{machine_service,peer_link,peer,peer_client,bridge,sync,link_proof}.rs`, new `daemon/humatad/src/{skills_service,peer_skills,provider_service}.rs`; `daemon/humatad/tests/{machines,peer_stdio,bridge}.rs`, new `daemon/humatad/tests/pending/{skills,providers}.rs` (move to `tests/` when gated); `cli/src/commands/hosts.rs`; `features/hosts/`; new `ui/src/{overlay_add_machine,machine_page}.rs`; new `clients/src/machines.rs` + `clients/src/machines/` (new files, inert until main adds the `mod` line); `workflow/milestones/MACH.md` Lane; OVERVIEW block "Pick up here (minion MACH)". Seam lines go to main: not_built.rs, serve.rs, main.rs `mod` lines, `daemon/humatad/BUILD.bazel` (FL), `clients/src/lib.rs`, `ui/src/lib.rs`, CLI registration |
| FILES | (early lane; new code in inert new files until the wiring step) `daemon/transfers/` (new crate `humata2_transfers`, targets `manual` until the visibility seam), `daemon/humatad/src/files/` (new: `roots.rs`, `ops.rs`), `daemon/humatad/src/{transfer_service,trash_service}.rs` (new), `daemon/humatad/tests/pending/` (new, not globbed; moves up at wiring), `clients/src/file_manager.rs` (new), `features/files/src/manager.rs` + `src/manager/` (new), `features/files/tests/manager_*.rs` (new, excluded in its BUILD until wiring), `features/files/BUILD.bazel`, new `ui/src/{file_manager,transfers_tray,storage_view}.rs`; at wiring only: `daemon/humatad/src/files.rs`, `daemon/humatad/tests/files.rs`, `features/files/src/lib.rs`, `cli/src/commands/files.rs`, `cli/tests/files.rs`, `chunk/`, `daemon/humatad/src/compression.rs`; `workflow/milestones/FILES.md` Lane, OVERVIEW block "Pick up here (minion FILES)". Seam lines go to main: `daemon/humatad/src/{main,serve,not_built}.rs`, `daemon/humatad/BUILD.bazel` (FL), `daemon/{store,durable}/BUILD.bazel` (visibility), `clients/src/lib.rs`, `ui/src/lib.rs`, `files.proto`; asks MACH (via main) for FileService on the peer link |
| REVIEW (early) | `daemon/review_diff/` (src, tests; new `src/{comments,comment_message,passage}.rs`, new `tests/{comments,comment_message}.rs`), `daemon/humatad/src/review.rs`, `daemon/humatad/src/review/` (new `comments.rs`, `sink.rs`), `daemon/humatad/src/paste.rs`, `daemon/humatad/tests/{review,paste}.rs`, new `daemon/humatad/tests/comments.rs`, `review/`, `diff/`, `markdown/` (not edited in batch 1), `workflow/milestones/REVIEW.md` Lane, OVERVIEW block "Pick up here (minion REVIEW)". Seam lines go to main: `api/proto/humata/v1/review.proto`, `api/proto/humata/store/v1/store.proto`, `daemon/store/src/{records,reviews}.rs`, `daemon/humatad/src/{serve,terminal,not_built}.rs`; later (MCP batch) `capabilities.proto`, `daemon/access/src/{capability,policy}.rs`, `daemon/mcp/src/tools.rs`, `daemon/humatad/src/{mcp,guard,access_service}.rs`, `api/{MCP,TRACE}.md`. `features/review` only after LOOK1 commits. Added 2026-11-15 (main OK): new `daemon/store/src/comments.rs` and `daemon/store/tests/comments.rs` (schema and lib lines via main); batch 2 `daemon/holder/src/{protocol,server,client}.rs`, `daemon/humatad/src/{holders,terminal}.rs` (BUILD lines in `daemon/holder` and `daemon/humatad` via main); batch 3 `daemon/access/src/{capability,policy}.rs`, `daemon/humatad/src/{mcp,guard,access_service}.rs` (additive only) |
| LOOK2 | now: `ui/src/{channel,board,lists,pages,hosts,settings,overlay_palette,overlay_new_session}.rs`, `features/{board,inbox,palette}/`, `workflow/milestones/LOOK2.md` Lane, OVERVIEW block "Pick up here (minion LOOK2)". From "LOOK1 committed": LOOK1's `ui/src/{panels,session_tabs,sidebar,terminals,theme,chrome}.rs` plus `ui/src/widgets.rs`, new `ui/src/panel_*.rs`, `desktop/tests/native.rs`, new `desktop/tests/native/`, `desktop/tests/{panels,shell}.rs`, `desktop/tests/__snapshots__/{native,panels,shell}/`, `clients/src/fixture/{design_session,sessions,requests,terminal,machines,items,journal,files}.rs`, `clients/src/fixture.rs`, `features/settings/`, `features/sidebar/`. Seam lines go to main: `ui/src/{lib,shell,overlays}.rs`, `features/app/src/lib.rs`, `clients/src/model.rs`, `desktop/BUILD.bazel`, `desktop/src/main.rs` (FL, BUG), `ui/src/overlay_toasts.rs` (BUG). Not: `ui/src/{request_card,access}.rs`, `features/hosts/` (MACH), `features/review` (REVIEW) |
| DESK (early) | (new files only until main says "LOOK1 committed") new `clients/src/desk.rs`, `clients/src/fixture/desk.rs`, `clients/src/daemon/fake/desk.rs`, `clients/tests/pending/desk.rs` (not globbed; moves up at the seam); new crate `features/desk/` (targets `manual` until the seam); batch 2: new `features/app/src/desk.rs`, `features/app/tests/desk.rs`; `features/sidebar/` only after LOOK1 commits; `workflow/milestones/DESK.md` Lane, OVERVIEW block "Pick up here (minion DESK)", DECISIONS D141. Seam lines go to main: `clients/src/{lib,workspace,daemon,fixture}.rs`, `clients/src/daemon/fake.rs`, root `Cargo.toml` + repin; later `features/app/src/{lib,route,overlay,events}.rs`, `features/app/{BUILD.bazel,Cargo.toml}`, `ui/src/{lib,shell}.rs` |

## Lanes

Each lane's plan, tasks and workers live in its milestone file
(`workflow/milestones/<ID>.md` › Lane). Here, one line per active lane:

| Lane | Milestone file | Manager | Workers running | State |
|---|---|---|---|---|
| ASK | `milestones/ASK.md` | minion ASK | tokens, caller, matrix | batch 4: D128.2 done; SEC-14 script tokens (daemon) |
| LOOK1 | `milestones/LOOK1.md` | minion LOOK1 | send-core, send-ui, audit | batch 1: direct/individual comment Send (D126), frame audit 01–17 |
| REVIEW (early) | `milestones/REVIEW.md` | minion REVIEW | none | batch 1 written, not built: comment ledger, message, CommentService for the terminal, restart recovery; seams in scratchpad `review-lane/` |
| FRAMES2 | `milestones/FRAMES2.md` | minion FRAMES2 | none | done (43f7fff2) |
| LOOK2 | `milestones/LOOK2.md` | minion LOOK2 | ch, bd, hm, ho, st, ov | batch 1: frames 18–35 view audit and fixes in unclaimed view files; batch 2 (splits, Last used, no Panes, fixtures, empty states) after LOOK1 commits |
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
- 2026-11-15 main: ASK batch 2 reported (B1 SEC-15, B2 previews + Always allow, B3 more handlers defer, B4 reads Allow). Main applied S6 (store InboxEntry.deferred=5), S7 (Authenticate store+Kernel), S8 (feed/inbox before Terminals; TerminalsConfig.inbox). Gate running: //apps/humata2/daemon/... + cli.
- 2026-11-15 main: ASK batch 2 gate: build green, 70 pass, 3 fail (refusals_test GlobalRead default; auth_test SEC-15 x2; deferred always_allow). Diagnostics sent to minion ASK.
- 2026-11-15 main: ASK batch 2 gate 73/73. Committed 970e4b28 (+ hash-fill), pushed. D127 items 7, 11–13 logged. ASK next: the app half (flow §26.4, IN-9 card, Agent access tab).
- 2026-11-15 main: FRAMES2 batch 1 recorded (56–71 new, 01–55 unchanged); 4 frame fixes sent (62 overflow/radio, 63 backdrop, 71 tick, 59 optional).
- 2026-11-15 main: LOOK1 batches 1–2 reported (D126 per-comment Send; frame audit fixes). Gate running: desktop/ui/features/clients/pane. Owner questions queued: 03/04 wording, frame 50 hint vs D126, frames 05/01 counts.
- 2026-11-15 main: FRAMES2 fixes re-recorded (59, 62, 63, 71). Owner approved 56–71 as drawn, D11 (a) as frame 71, and LOOK1's fixes to frames 50/01/05 (handed to LOOK1). Closed FRAMES2; claims released.
- 2026-11-15 main: ASK batch 3 seams applied (ui mods, OpenRequest arm). Gate 3: features/clients mostly green; sent ASK ui/access.rs Role::Radio + settings access_test, LOOK1 messages_test batch order.
- 2026-11-15 main: gate 4 green (all unit + desktop; recorded). Fable advisor → D128. Opened W1 (docs lane), minion W1 (Opus 5.5). Gate 5 running (design record now EXACT).
- 2026-11-15 main: committed LOOK1 batches 1–2 + ASK batch 3 together (shared files) as e07e8653, hash-fill 98f71e90, pushed. W1's pages/overlays/channel edits (after gate 5) left uncommitted. terminal/{embed,lab,session} and three BUILD visibility edits are another session's, left alone. Fresh managers: LOOK1 (T12 batch 3, T10, Send-per-draft 01/11), ASK (D128.2, SEC-14), W1 (log its edits, continue). Owner: managers may ask a Fable (high) advisor directly.
- 2026-11-15 main: D129 (Linux goldens via web, Fable) + SPEC NV-11/CH-12 committed ff18004b. Gate 6 (design record) green; W1 batch 1 committed (frames 18, 26, 27, 29–32, 48, 71; 33 retired), pushed 69552a55. W1 now on T6–T9 in session.rs/desk.rs.
- 2026-11-15 main: ASK batch 4 (script tokens, D132) gate 45/45, committed 21cbef1d, pushed. Fable judge → D131 (row dot = session's own status; forks only feed the badge; row_status dropped). Gates 8–9: design, native 37, panels 13, shell 2, desktop/ui/features 24 all green. Committed W1 batch 2 + LOOK1 batch 3 as 13121e1d, pushed; before/after images sent. Closed W1. Opened TIDY (minion TIDY). LOOK1 on its 01–17 audit, may take clients/src/model.rs while ASK idles (ASK waits on FL).
- 2026-11-15 main: TIDY gate fixed (tests/common spec_files) and passed with LOOK1 batches 4–5: 111 tests (machines_test flaked once, passes alone), design/native 37/panels 13/shell 2 recorded. Committed TIDY batch 2 (4a636632, D9 deletion + repin) and batch 1 (d17f0d4e, spec mode + wording); TIDY closed. Golden 07 before/after sent to the owner (L14). LOOK1 batches 4–5 committed.
- 2026-11-15 main: LOOK1 audit 01–17 done except frame 16 (batch 7 fixes the fixture). Gate held up by another session's in-progress terminal hyperlink edits (terminal/{vt,grid,view,pane}); waiting for them to settle. Opened FL and BUG now: LOOK1's audit shows their reasons (stable view tree, session page) already hold; neither touches LOOK1's files before LOOK1 commits. LOOK2 opens after LOOK1's commit (same ui files).
- 2026-11-15 main: owner asked for more parallelism and OK'd heavy Fable use. Fable advisor's plan taken: lane limit 7 (dormant lanes not counted, at most 4 editing daemon/humatad); stale C1/P0/ASK claims released (ASK batch 4 is 21cbef1d). ASK goes dormant; its session_service.rs, channel_service.rs, events.rs pass to FL (additive publish calls + Session/Channel filter arms only; ASK's Caller::Script arms and Rules filter stay; that gate runs matrix/auth/inbox/sessions/channels tests). Early lanes opened now: MACH, FILES, REVIEW (each does only the part its open deps don't block; closes when the hard deps are done). DESK-early after LOOK1 commits; BOARD daemon when BUG closes; FORK-local after FL's event batch. Main-only seam files added: daemon/store/src/{schema,lib}.rs (main assigns migration numbers), not_built.rs, daemon/mcp/src/tools.rs, CLI command registration, clients/src/model.rs; daemon/humatad/tests/common/mod.rs additive only. Two gate sets (daemon+cli, app). New managers run on Fable.
- 2026-11-15 main: g26: FL's three fixes gated; session_events, clients/live and desktop:humata2 pass, live flows §26.2 and §26.8 still time out (lost-conversation passes), so `desktop:live_flows_test` is manual. Committed FL + BUG + LOOK1 as ed5882ed, pushed. LOOK1 and BUG closed. FL continues: the two live flows, then T9, T10. MACH stage A (skills, sync_rules) applied in the tree with a one-line lifetime fix; g26-mach: machines_test, fake_ssh_test (provider create undo) and takeover_test fail, sent to MACH. REVIEW revised seams (own comment tables) to apply next.
