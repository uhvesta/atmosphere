warning: --mode auto is not available for this account; continuing in the default mode.
### Audit summary
All PROGRESS tasks (T0–T10) were already implemented on disk from prior lanes, including the two seams mentioned in the brief (`api/proto/humata/v1/sessions.proto` field 35 `status_since`, `clients/src/model.rs` `Session.status_since_ms`, and `ui/changes/src/panel_changes/mod.rs` wiring `changes_loading`), plus all five design frames (`122`–`126`) in `desktop/tests/design.rs` and `desktop/tests/__snapshots__/design/`. This pass fixed the remaining bugs, test gaps, and stale docs found during a full T0–T10 audit.

### Files changed, new and deleted
Changed:
- [session_service.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/session_service.rs) — **T5 bug fix:** moved `status_since::observe` out of `session_pb` into `stamp_status_since` / `stamp_status_since_at`, called *after* `merge_remote`, `lineage::apply`, and `move_session::show_moving` in `tell`, `show`, and `list_sessions` (so remote sessions track real `Working`/`Idle` transitions, archived/moving sessions don't record intermediate states, and `delete_many.rs` doesn't write side-effect observations). Added deterministic unit test `stamp_status_since_tracks_the_final_merged_status`.
- [operations.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/operations.rs) — **T2 test hygiene:** replaced the fixed 400 ms `tokio::time::sleep` in `a_burst_of_unit_updates_publishes_a_few_versions_and_ends_on_the_last` with event-driven waiting on `watch::Receiver::changed()` (5 s timeout).
- [op_progress.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/features/app/tests/op_progress.rs) — **T4 test coverage:** added TestStore tests proving success clears progress and closes the dialog/sheet (New session, Fork, Continue in channel, Add repo) and asserting that failures set today's error on each dialog.
- [machine_page.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/ui/machines/src/machine_page.rs) — **T9 completion + tests:** added `script_step_sweeps` sweep bar under running machine script step rows in `scripts_section` and unit tests (`script_sweep_tests::only_a_running_script_step_sweeps`) verifying only `ScriptOutcome::Running` sweeps (`Failed`/`TimedOut`/`Passed`/`Skipped` never do).
- [OVERVIEW.md](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/OVERVIEW.md) — **T10 docs:** updated `Pick up here (minion PROGRESS)` with T6/T7 completion, post-split crate paths/targets, and frames 122–126.
- [PROGRESS.md](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/workflow/milestones/PROGRESS.md) — updated the Lane section only.

New: none.
Deleted: none.

### Seam lines for the main agent
None needed — all three seams from the brief are already applied in the tree:
1. `apps/humata2/api/proto/humata/v1/sessions.proto:233` already has `google.protobuf.Timestamp status_since = 35;`.
2. `apps/humata2/clients/src/model.rs:633` already has `pub status_since_ms: Option<i64>,`.
3. `apps/humata2/ui/changes/src/panel_changes/mod.rs:886` already wires `crate::changes_loading::changes_loading(look, &state.review.fetch, session_id.as_ref())` (and no `"Loading…"` text remains in `ui/changes/src/`).

*(Once the gate below is green, main can tick `[x]` on T0–T5 and T8–T10 in [PROGRESS.md](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/workflow/milestones/PROGRESS.md#L93-L113), which stayed outside this lane's claim to `PROGRESS.md Lane`.)*

### Bazel targets to build and test
Focused targets covering our edits and the full PROGRESS surface:
- `//apps/humata2/daemon/humatad:humatad2_unit_test`
- `//apps/humata2/daemon/sessions_core:sessions_core_unit_test`
- `//apps/humata2/clients:moving_status_test`
- `//apps/humata2/clients/live_core:live_core_unit_test`
- `//apps/humata2/clients/live_daemon:live_daemon_unit_test`
- `//apps/humata2/features/app:op_progress_test`
- `//apps/humata2/features/pr_review:dialog_test`
- `//apps/humata2/ui/core:core_unit_test`
- `//apps/humata2/ui/workspace:workspace_unit_test`
- `//apps/humata2/ui/overlays:overlays_unit_test`
- `//apps/humata2/ui/machines:machines_unit_test`
- `//apps/humata2/ui/changes:changes_unit_test`
- `//apps/humata2/ui:ui_unit_test`
- `//apps/humata2/desktop:design_test`

### Goldens that need re-recording and frames Fable must judge
None. All five new frames (`122-new-session-cloning`, `123-session-moving-unmeasured-step`, `124-session-working-elapsed`, `125-migrate-holders-count`, `126-diff-loading`) are already in `FRAMES` in [design.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/desktop/tests/design.rs#L310-L318), Fable-approved, and recorded in `apps/humata2/desktop/tests/__snapshots__/design/`.

### What is still open
- Out of scope / deferred per D807 P7: archive push, restore download, trash, remote `clone_main:<machine>:…`, and PR-review channel main-clone setup (`daemon/humatad/src/wire_review.rs` `clone_pr_reviews_mains`) stay unmeasured.
- Ticking the T0–T5 and T8–T10 checkboxes in [PROGRESS.md](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/workflow/milestones/PROGRESS.md) after main's build and test gate passes.
agy exit 0 for progress
