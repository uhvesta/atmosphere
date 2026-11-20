# PROGRESS: Anything that takes more than a moment shows that it is moving, how far it is, and how long it has run

- **Status:** open (owner, 2026-11-18 11:47: "could we show some progress indicators"). Planned by Fable 5.2 on the owner's behalf; picks are provisional decisions P1–P10 at the end.
- **Depends on:**
  - LIVEHOLDER T6 (Migrate moves terminals) for the holder-migration count only (T6 here). Everything else opens now.
  - DIFFVIEW for the big-diff indicator only (T7 here): `ui/src/panel_changes.rs` is claimed by DIFFVIEW; T7 does not open until DIFFVIEW closes or releases the file.
- **Unblocks:** nothing; it closes with its own quick release.
- **Layer:** L13 · **Lane:** daemon (units) + clients (one model) + UI (one widget, placed)

## Why
Owner: "could we show some progress indicators". Main's reading: long operations look frozen: cloning a session's repos, moves and holder migrations, big diff loads, an agent's turn (elapsed time).

## Today (read 2026-11-18)
- **The model is already there.** `OperationStep` carries `done_units`, `total_units`, `output_tail` (`api/proto/humata/v1/operations.proto`); `Units::set` in `daemon/humatad/src/operations.rs` only lets them rise.
- **Only one step reports units:** the move's `copy` (`session_service/move_session.rs:739–768`). Every other step of every kind stays 0/0.
- **Clones can't report:** `daemon/sessions/src/git.rs` runs `git` with `Command::output()`, so nothing is read until git exits. New channel's bar (`ui/src/overlay_new_channel.rs` `bar`, `clients/live/src/setup.rs` `percent`) therefore sits at 0% against a real daemon; only the fake moves it.
- **Two one-off UIs, no shared part:** the moving step card with a thin bar (`ui/src/session_tabs.rs` `steps_card`, `copy_progress`; frames 64, 97) and New channel's 160 px bar (frame 45-new-channel-cloning).
- **Everything else is a word:** "Starting…" on a disabled button (New session, PR review, Fork), "Loading…" lines, "migrating…" on the machine page. No motion, no step, no time.
- **A turn has no start time.** `Session` has `started_at` and `last_output_at`, nothing for when the status last changed.

## Outcome
When I start something that takes more than a moment, the place I started it shows the step it is on and a bar: filled to a real fraction when the daemon can measure it, sweeping when it can't, with elapsed time once it has run a while. A working agent's tab says how long the turn has run.

## Exit criterion
On a live daemon, with a repo large enough to take ~10 s to clone:
1. New channel and New session show "Receiving objects 42%" rising, per repo, then the session opens.
2. Move a session to another machine: the step card (frame 64) shows the copy bar as today and a sweeping bar on the unmeasured steps.
3. Fork, Start a PR review, Add repo and Continue in channel show the running step's title under their button, not just "Starting…".
4. Migrate the holders shows "3 of 7 terminals moved".
5. A working session's tab header reads "Working · 2m 10s" and stops counting when the turn ends.
6. A big diff shows the sweep within 300 ms of the click until rows draw (after DIFFVIEW).
7. An operation under 300 ms shows no indicator at all (no flicker).

Tests: the ones listed per task; frames 122–126 approved by Fable.

## Look and placement (quiet chrome)

**One widget, `progress_line`, three forms.** No spinners, no modal progress windows, no global activity tray, no percent in the sidebar.

| Form | When | Drawn as |
|---|---|---|
| Measured | `total_units > 0` | 2 px track in the border colour, fill in the accent colour, label right of the step title: "42%" or "120 of 512 MB" |
| Unmeasured | running, `total_units == 0` | same 2 px track, a 25%-wide accent segment sweeping left to right every 1.6 s; no number |
| Elapsed | any running form after 10 s | dim "· 14s", then "· 2m 10s", after the label |

- **Delay:** nothing draws until the operation has run 300 ms.
- **Motion** runs only while the element is on screen; goldens and `look_test` draw the sweep at a fixed phase (segment at 37%).
- **Failure is not this milestone's look:** a failed step keeps today's ✕ row with its `output_tail` and Try again (MP-20, HO-2). Progress never shows a state the person can't act on; a stalled step shows only the truth (elapsed keeps counting) and the operation's existing Cancel where the kind is cancellable.

**Where each one sits** (always where the person started it, or on the thing it is about):

| Operation | Place | Form |
|---|---|---|
| Channel setup clones | New channel setup rows (existing bar, now the shared widget) | Measured per repo |
| New session, Fork, Continue in channel, Add repo, Restore, Rebuild, Start PR review | under the dialog's primary button, replacing nothing: one line "Cloning acme/api" + bar; button keeps "Starting…" | Measured on clone steps, unmeasured otherwise; "step 2 of 5" is not shown (P4) |
| Move to a machine, desk move | the session's step card (frames 64, 97) | `copy` measured (bytes, as today); the running unmeasured step gets the sweep |
| Add machine, machine scripts, provider steps | their existing step lists | sweep on the running row only |
| Migrate the holders | the Migrate item's details (frame 83) and the machine banner (frame 79) | Measured: "3 of 7 terminals moved" |
| Agent's turn | session tab header, beside the status word: "Working · 2m 10s" | Elapsed only, no bar |
| Big diff load | top edge of the Diff panel's body, full width, no label | Unmeasured |
| File manager / Files panel first load, conversation Earlier/Later | top edge of the pane, replacing the "Loading…" word | Unmeasured |

## Which operations report real units

| Step | Units | Source |
|---|---|---|
| `copy` (move) | bytes | already done |
| `clone_main:<repo>`, `clone:<repo>` over the network | 0–1000: Receiving objects 0–900, Resolving deltas 900–1000 | `git clone --progress` stderr |
| `clone:<repo>` from the local main clone, PR head clone, restore | 0–1000 from "Updating files: N%" when git prints it; else unmeasured | same parser |
| `trash:<machine>`, archive push, restore download | unmeasured for now (P7) | — |
| Migrate the holders | terminals moved / terminals to move | LIVEHOLDER's migrate loop |
| Skills sync, provider create/action, add machine check/install, machine script steps, launch, verify, write_record | unmeasured | no honest total exists |
| Agent turn | elapsed since `status_since` | new `Session` field |
| Diff load, pane loads | unmeasured | client-side request in flight |

## Data path
1. **Daemon.** The step's worker calls `Units::set(key, done, total)`; `operations.rs` bumps `version` and publishes the `Operation` on the events feed (`events.proto` field 18) and to `WatchOperation`. Unit updates are coalesced to at most 4 per second per step (P6) so a clone doesn't flood the feed or the store.
2. **Client.** `clients/live/src/map.rs` maps any `pb::Operation` to one model, `OpProgress { title, fraction: Option<f32>, label, started_ms }` (in `clients/src/model.rs`); `MoveProgress` and `SetupStep` keep their shapes and are filled through the same mapper. Fakes script the same model (`clients/src/daemon/fake/*`).
3. **Features.** Each dialog that today holds `creating: bool` or `Start::Starting` holds `Option<OpProgress>` for the operation its start call returned, fed by the existing operation events. No new polling; nothing ticks in reducers.
4. **UI.** `widgets::progress_line(look, &OpProgress, now_ms)` draws it. Elapsed text and the sweep are view-local: a 1 s timer on the one visible element, never a store action.

## Read these (pointers, nothing else)
- SPEC: MV-4, MV-17, MP-20, HO-2, SE-2, PER-27 (`grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md`)
- Frames: 64-session-moving, 97-desk-moving-copy-progress, 79-machine-migrating, 83-migrate-holders, 45-new-channel-cloning
- Decisions: D373 (Migrate the holders), D784 (sessions accountable), D76 (goldens)
- Code: `daemon/humatad/src/operations.rs` (`Units`, `set_units`), `daemon/sessions/src/git.rs`, `daemon/sessions/src/start.rs:641` (`clone_repos`), `clients/live/src/{map,setup,events}.rs`, `clients/src/model.rs:498` (`MoveProgress`), `ui/src/session_tabs.rs:271–340`, `ui/src/overlay_new_channel.rs:291`, `ui/src/widgets.rs`
- Tests to follow: `clients/tests/moving_status.rs`, `features/app/tests/new_channel_setup.rs`, `daemon/humatad/tests/move_cheap.rs`
- API: `api/GRPC.md` §7 (step keys and what each reports)

## Tasks
Each is a Haiku-sized task unless marked; main runs Bazel, one batched invocation per wave.

- [x] **T0 Shared widget** · `ui/src/widgets.rs`, `ui/src/session_tabs.rs` (move `copy_progress` onto it), `ui/src/overlay_new_channel.rs` (its `bar` becomes the widget).
  Tests: given done/total, the fraction and label come out (0 total → unmeasured; done > total → full); under 300 ms nothing is drawn; elapsed text for 9 s, 10 s, 130 s; frames 64 and 97 don't change.
- [x] **T1 Clones report units** (Sonnet: process plumbing) · `daemon/sessions/src/git.rs` (a `git_with_progress` that reads stderr as it comes, plus a pure parser), `daemon/sessions/src/start.rs`, call sites in `daemon/humatad/src/session_service.rs` and `channel_service.rs` passing `Units`.
  Tests: parser on recorded git stderr (`\r`-separated lines, Receiving → Resolving → Updating files, a non-English or garbled line yields nothing rather than an error); cloning a local fixture repo reports values that only rise and end at the total; a failed clone still returns today's `GitError` with its output.
- [x] **T2 Coalescing** · `daemon/humatad/src/operations.rs`.
  Tests: 1,000 `Units::set` calls in a burst publish a bounded number of versions and the last value is always published; a step's state change is never delayed.
- [x] **T3 One client model** · `clients/src/model.rs`, `clients/live/src/map.rs`, `clients/live/src/setup.rs`, `clients/src/daemon/fake/sessions.rs`.
  Tests: a `pb::Operation` with a running measured step maps to that fraction and title; unmeasured maps to `None`; all steps done maps to nothing running; `clients/tests/moving_status.rs` still passes.
- [x] **T4 Dialogs show the running step** · `features/app/src/new_session.rs`, the fork / PR review / add repo / continue-in reducers, `ui/src/overlay_new_session.rs`, `overlay_fork.rs`, `overlay_pr_review.rs`, `overlay_session_add_repo.rs`, `overlay_continue.rs`. Split per dialog if two workers.
  Tests (TestStore, on fakes): starting → operation events → state holds the running step's progress → success clears it and closes the sheet; failure clears it and shows today's error; a second event for another operation is ignored.
- [x] **T5 Turn elapsed** (seam: main owns the proto change) · `api/proto/humata/v1/sessions.proto` (`Session.status_since`), where the daemon sets status (`daemon/humatad/src/session_service.rs` and the status writer it calls), `clients/live/src/map.rs`, `clients/src/model.rs`, `ui/src/session_tabs.rs` header.
  Tests: status Working → Idle → Working moves `status_since` each time and a repeat of the same status doesn't; it survives a daemon restart; the header formats "Working · 2m 10s" from a given now; other statuses show no time.
- [x] **T6 Holder migration count** (after LIVEHOLDER T6) · LIVEHOLDER's migrate loop reports `Units` (terminals moved / to move), `ui/src/machine_page.rs` banner, the Migrate details card.
  Tests: migrating N fake holders reports 0..N in order and N of N at the end; a holder that can't hand over counts as handled, not stuck.
- [x] **T7 Diff load sweep** (after DIFFVIEW releases `panel_changes.rs`) · `ui/src/panel_changes.rs`, the review feature's loading flag.
  Tests: a diff request in flight for over 300 ms sets the indicator; rows arriving clear it; switching file mid-load shows the new file's state only.
- [x] **T8 Pane loads** · `ui/src/file_manager.rs`, `ui/src/session_tabs.rs:555`, `ui/src/conversation_tab.rs`, `ui/src/machine_page.rs:639`: the "Loading…" words become the sweep.
  Tests: loading and not loaded → indicator; loaded → none; a reload of a loaded pane keeps its content and shows the sweep above it.
- [x] **T9 Step lists get the sweep** · `ui/src/overlay_add_machine.rs`, `ui/src/session_tabs.rs` `step_rows`, machine script and provider step rows.
  Tests: only the Running row carries progress; a Failed row never does.
- [x] **T10 Frames and docs** · `desktop/tests/design/{session,new_channel,machine_page,upgrade}.rs`, `api/GRPC.md` §7 (which steps report what), OVERVIEW.md, GAPS_REVIEW.md, DECISIONS.md (P1–P10 as real D numbers).

Order: T0, T1, T2, T3 in parallel → T4, T5, T8, T9 → T10; T6 and T7 when their owners release.

## New frames (Fable approves; numbers after STACKVIEW's 119–121)
- 122-new-session-cloning: the sheet with "Receiving objects 42%" under Starting….
- 123-session-moving-unmeasured-step: frame 64's card with the sweep on a non-copy step.
- 124-session-working-elapsed: tab header "Working · 2m 10s".
- 125-migrate-holders-count: "3 of 7 terminals moved".
- 126-diff-loading: the sweep on the Diff panel (drawn with T7).
- Re-record: 45-new-channel-cloning (same bar, shared widget); 64 and 97 must not change.

## Out of scope
- A global activity or operations list, Dock badge, menu-bar item, or notifications on finish.
- Time-remaining estimates and transfer speeds.
- Sidebar changes: the status dot stays as it is; no percent, no timers there.
- Agent-turn token or tool-call counts (AGENTS2).
- File transfers tray (`ui/src/transfers_tray.rs`): it already has phases; untouched.
- Changing what a failed step shows, Cancel rules, or step keys (their milestones own those).
- The starting window (`ui/src/starting.rs`): it already says what it is doing.
- Making the operations themselves faster (FAST).

## Lane (written by the manager)
- **Design notes** (minion PROGRESS, 2026-11-18):
  - `OpProgress` lives in a new `clients/src/progress.rs` (`humata2_clients::progress::OpProgress`), not `model.rs`: `model.rs` is a STACKVIEW seam main applies by script, so an additive module keeps out of its way. Fields as planned; `started_ms: i64` matches `Daemon::now_ms()`.
  - Widget: `widgets::progress_line(look, &OpProgress, now_ms)` over a pure `progress_draw(&OpProgress, now_ms) -> Option<ProgressDraw>` (the unit-tested part: delay, fraction clamp, elapsed text). The sweep is a GPUI `with_animation` (1.6 s, repeat, `with_max_fps(30.)`) whose segment sits at `(0.37 + t) % 1`, so a still render (goldens, `look_test`) draws it at 37% with no special case. Elapsed-only re-renders through a 1 fps repeating animation. Bar metrics are parameters so frames 64/97 (180×3 px, label under) and 45 (160×4 px) keep their pixels.
  - Clones: `git_with_progress` in `daemon/sessions/src/git.rs` spawns git with `--progress`, stderr piped and read as it comes (split on `\r`/`\n`), stdout to null; a pure `clone_permille(line) -> Option<u64>` (P5). Progress lines are dropped from the error's stderr so `GitError::Failed` reads as today.
  - Coalescing (P6): per step key, a unit change within 250 ms of the last published one is written into the operation silently (no version bump, no feed); a single armed flush publishes it at the window's end, so the last value is always published and any step change carries the latest units at once.
  - Dialogs (T4, batch 2): a new `Event::Operation` carrying the op id, kind and `Option<OpProgress>`; a dialog in Starting latches the first event of its kind and ignores other ids.
- **Claims:** see the PROGRESS row in WORKLOG › Claims. Not mine: `clients/src/model.rs` (STACKVIEW seam), `ui/src/panel_changes.rs` (DIFFVIEW), `daemon/holder`, `revive`, humatad `upgrade/*`, `holders.rs` (LIVEHOLDER), `session_service/{turns,hooks,archive,move_session}.rs` (AGENTS2/LIVEHOLDER).
- **Tasks / workers:**
  - [x] model seam (me): `clients/src/progress.rs`, `pub mod progress;` in `clients/src/lib.rs`.
  - [ ] T0 widget · worker `progress-t0` (Haiku 5.5 high) · `ui/src/widgets.rs`, `ui/src/session_tabs.rs` (copy bar only), `ui/src/overlay_new_channel.rs` (`bar`).
  - [x] T1 clone units · worker `progress-t1` (Sonnet 5.5 medium) · written and checked, waiting on gate. `git.rs` (`clone_permille`, `git_with_progress`, `clone_*_with`), `setup.rs` (`clone_mains_with`), `start.rs` (`CloneUnits` + task-local `with_clone_units`, so `Starter` literals elsewhere don't change), `add_repo.rs`, `lib.rs` re-exports; humatad `channel_service.rs`, `session_service.rs` (`run_start`), `session_service/{continue_in,add_repo}.rs`. Unmeasured for now: remote `clone_main:<machine>:…`, PR-review channel setup (`review/pull_requests/start.rs`, STACKVIEW's area), archive/rebuild/remote clones.
  - [x] T2 coalescing · worker `progress-t2` (Haiku 5.5 high) · written and checked, waiting on gate. `operations.rs` (`UNITS_EVERY` 250 ms gate per step key, silent write + one armed flush; 3 tokio unit tests).
  - [x] T3 client mapper · worker `progress-t3` (Haiku 5.5 high) · written and checked, waiting on gate. `map.rs` `progress(&pb::Operation) -> Option<OpProgress>`, shared `percent`; `setup.rs` uses it.
  - [ ] T4, T5, T8, T9 (batch 2), T10 (batch 3).
- **Waiting on:** LIVEHOLDER T6 (for T6), DIFFVIEW releasing `panel_changes.rs` (for T7); T5's proto field and `Session.status_since` in `model.rs` are seams for main.
- **Build gates:** none yet.
- **Log:**
  - 2026-11-18 PROGRESS manager started; plan and claims written; batch 1 (T0–T3) briefed
- 2026-11-18 batch 1: T1, T2, T3 written and reviewed; T0 (widget) and T8 (pane load sweeps, worker `progress-t8`, Haiku 5.5 high, `ui/src/{file_manager,conversation_tab,machine_page}.rs`) running. Model seam extended: `OpKind`, `OpUpdate` in `clients/src/progress.rs`, `Event::Operation(OpUpdate)` in `clients/src/daemon.rs`.
- 2026-11-18 batch 2 started: T4a (`progress-t4a`, Haiku: live `finish` publishes changed `OpUpdate`s, `map.rs` `op_kind`/`op_update`) and T4b (`progress-t4b`, Sonnet: dialogs latch onto their op and keep its progress; `features/app/tests/op_progress.rs`). T4c (overlays draw `progress_line`) waits for T0.
- 2026-11-18 T0 widget reviewed (manager fix: the 300 ms wait now wakes itself, so `progress_line` always returns an element). T4a, T4b, T4c, T9 written and checked. T5a (daemon `status_since` book, `<state>/status-since.json`, saved under its lock on change) written and checked; T5 seam script for main at the scratchpad `progress-t5-seam.py`. T5b (map, header "Working · 2m 10s", file-loading sweep) and T8 running.
- 2026-11-18 T8 written and checked (Files, Conversation, machine page loads draw the sweep with `Role::ProgressIndicator` "Loading"). Batches 1–2 reported to main for the gate; T5b had not written anything at report time (map.rs, fake/sessions.rs, pages.rs, session_tabs.rs:582 still open).
- 2026-11-18 T5b landed after the report: `map.rs` `status_since_ms` (+ test), fake `status_since_ms: None`, `pages.rs` `working_for` + 1 s tick note (look.secondary, tests), `session_tabs.rs` file-loading sweep (`Role::ProgressIndicator`, matching T8)..
- 2026-11-18 Review fixes: `progress_line` returns a self-waking empty element for its 300 ms wait (the reveal no longer depends on another redraw); `status_since` writes under its lock (saves can't land out of order); `features/session/tests/fork.rs` literal gets `progress`/`op`; the header's working note sits out of flow while empty (the 7 px gap under the header is gone); unused `clone_pull_head` wrapper removed.
- 2026-11-18 Presubmit: 100/101 pass. `ui:ui_unit_test` fails to build in DIFFVIEW's `ui/src/panel_changes.rs:3421` (recursion limit expanding `#[test]`), not ours; `bazel build //apps/humata2/ui:ui` is clean. Slow, scoped: humatad `session_add_repo_test`, `sessions_test`, `sessions_unit_test` pass; `desktop:native_test` 254/55: all 55 are panel-header / diff-view / review snapshot and focus changes from other lanes, none touch progress output.
- 2026-11-18 T10: frames 122 (new session cloning), 123 (moving, unmeasured step), 124 (working, elapsed) written in `desktop/tests/design{.rs,/overlays.rs,/fork_move.rs,/session.rs}`; need recording and Fable approval. 125 waits for T6.
- 2026-11-18 T10 docs: `api/GRPC.md` §7 (clone units on `clone:`/`clone_main:`, the 4/s gate), OVERVIEW.md "Pick up here (minion PROGRESS)", GAPS_REVIEW.md entry (D807). Main recorded P1–P10 as D807; frames 122–124 go into the next batched recording. T6/T7 carry over per P10. Lane stops here.
- 2026-11-18 T7 written (DIFFVIEW released panel_changes.rs; REVIEWUI owns it next, so it's a seam): `ui/src/changes_loading.rs` (sweep laid over the Diff panel body's top edge while the open session's read is in flight, hidden for 300 ms, id `changes-loading`, 4 unit tests); `scratchpad/progress/seam_t7_changes_loading.py --check` finds all 4 anchors once. The header's "Loading…" words go (P1). Frame 126 not drawn yet.
- Ready to commit: yes, once frames 122–124 are recorded and approved by Fable

## Provisional decisions (Fable 5.2, 2026-11-18; main records them in DECISIONS.md)
- **P1 One thin bar, no spinners.** A 2 px line with three forms (measured, unmeasured sweep, elapsed text). Reason: quiet chrome; one part reused everywhere. Reverse: swap the widget's drawing.
- **P2 Progress sits where the action was started or on its subject**, never in a global tray. Reason: actionable UIs only; a list of running things the person can't act on is status for its own sake.
- **P3 300 ms before anything draws; elapsed text after 10 s.** Reason: no flicker on fast operations, no ticking clock on short ones.
- **P4 The running step's title, not "step 2 of 5".** Reason: step counts change with skipped and undo steps and say less than the title.
- **P5 Clone units are 0–1000 across git's phases (receive 0–900, deltas 900–1000; local clones use Updating files).** Reason: one rising number per step fits `set_units`' only-rises rule; object counts and file counts can't share a total. If git's output can't be parsed, the step is unmeasured, never an error.
- **P6 Unit updates are coalesced to 4 per second per step in the daemon.** Reason: performance; a clone prints hundreds of lines.
- **P7 Archive push, restore download and trash stay unmeasured in this milestone.** Reason: each needs its own honest total; add when someone finds them slow.
- **P8 A turn's elapsed time comes from a new `Session.status_since`, shown only in the session tab header and only while Working.** Reason: `last_output_at` is not a turn start; the sidebar stays still (no per-second redraws across the list).
- **P9 A stalled operation shows nothing new.** Elapsed keeps counting and the existing Cancel stays; no "seems stuck" label. Reason: no unfixable detection.
- **P10 The diff indicator waits for DIFFVIEW; the holder count waits for LIVEHOLDER T6.** PROGRESS can close and release without T6/T7 if either owner is still open, carrying them as follow-ups in those milestones.
- 2026-11-19 (DIFFUI2): frames 122–124 are recorded and Fable-approved (WORKLOG, committed in 192f210c), so T10's frames are done. Still open: 125 (waits for LIVEHOLDER T6) and 126 (T7's Diff loading sweep, not drawn yet).
- 2026-11-19 minion PROGRESS (resumed for 0.3.10; T6 skipped, waits on LIVEHOLDER): gaps found against the exit criterion and closed:
  - Start a PR review showed only "Starting…" (criterion 3). Worker `progress-t4pr` (Haiku 5.5 high): `features/pr_review/src/dialog.rs` holds `progress`/`op`, new `DialogAction::OperationMoved` latches the first StartPrReview/StartSession update while Starting; `features/app/src/events.rs` forwards `Event::Operation` to it; `ui/overlays/src/overlay_pr_review.rs` draws `progress_line` under Start; TestStore tests in `features/pr_review/tests/dialog.rs`.
  - T9: a running machine script rule (`ui/machines/src/machine_page.rs` `scripts_section`) gets the sweep (`Role::ProgressIndicator`); add-machine step rows already had it.
  - Frame 126 (`desktop/tests/design/session.rs` `diff_loading`) drawn as a candidate in `design.rs` CANDIDATES for Fable.
- 2026-11-19 Gate (lint on): `features/pr_review/...` (6), `features/app:op_progress_test`, `features/app:app`, `ui/machines:machines_unit_test`, `ui/overlays:overlays_unit_test`, `desktop:design_test` all pass (bq log `.bq/logs/20261119-114419-d274cd.log`); the unused `Clients` import in `features/app/tests/op_progress.rs` is gone (`20261119-114646-0fd3f5.log`, clean). Fable 5.2 APPROVED frame 126 (sweep at 37%, 25% wide, full width under the Diff header; optional nit: the track may sit on the header hairline, not blocking). Main: move `126-diff-loading` from CANDIDATES to FRAMES in `desktop/tests/design.rs` and record it with `design_test_record`.
- Ready to commit: yes (0.3.10). Open: T6 (holder count, frame 125) waits for LIVEHOLDER T6.
- 2026-11-19 T6 (minion LIVEHOLDER): written. The daemon counts each round (`HoldersMoved` on `MigratePreview`/`HoldersReport`, `counted_round` in svc_upgrade/holders.rs). The Migrate card and the machine page show "3 of 7 terminals moved". Frame 125 is Fable-approved with changes, all applied (D866). It needs recording by main.
- 2026-11-19 Gemini lane audit & completion pass (T0–T10):
  - Verified on disk that T0–T10 were already implemented from prior passes, including `api/proto/humata/v1/sessions.proto` (`status_since = 35`), `clients/src/model.rs` (`Session.status_since_ms`), `ui/changes/src/panel_changes/mod.rs` (`changes_loading` already wired), and all 5 frames 122–126 in `desktop/tests/design.rs` `FRAMES` with recorded goldens in `desktop/tests/__snapshots__/design/`.
  - Fixed remaining bugs and test gaps found during audit:
    - **T5 (`daemon/humatad/src/session_service.rs`)**: `status_since::observe` previously ran inside `session_pb` *before* `merge_remote`, `lineage::apply`, and `move_session::show_moving` (so remote sessions never observed their real `Working`/`Idle` transitions, archived/moving sessions recorded intermediate states, and `delete_many.rs` triggered bogus observations). Moved observation to `stamp_status_since` after `show_moving` in `tell`, `show`, and `list_sessions`, with a deterministic unit test (`stamp_status_since_tracks_the_final_merged_status`).
    - **T2 (`daemon/humatad/src/operations.rs`)**: replaced the fixed 400 ms `tokio::time::sleep` in `a_burst_of_unit_updates_publishes_a_few_versions_and_ends_on_the_last` with event-driven waiting on `watch::Receiver::changed()` (5 s deadline).
    - **T4 (`features/app/tests/op_progress.rs`)**: added TestStore coverage for the success path (progress cleared and sheet/dialog closed) and error-on-failure assertions across New session, Fork, Continue in channel, and Add repo.
    - **T9 (`ui/machines/src/machine_page.rs`)**: added `script_step_sweeps` sweep bar under running machine script step rows in `scripts_section` and unit tests proving only `ScriptOutcome::Running` sweeps (`Failed`/`TimedOut`/`Passed`/`Skipped` never do).
    - **T10 (`OVERVIEW.md`)**: updated the `Pick up here (minion PROGRESS)` summary to reflect T6/T7 completion, post-split crate paths/targets, and frames 122–126.
  - Ready for main's batched build/test gate; once green, main can tick T0–T5 and T8–T10 in the task list above.
