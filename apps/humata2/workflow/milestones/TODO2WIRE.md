# TODO2WIRE: the TODO2 features work on real sessions, not only in tests

- **Status:** open (main, 2026-11-20). Found by the TODO2 audit (WORKLOG 2026-11-20 ~06:45); each gap is on its item's Status line in `milestones/TODO2.md`.
- **Release:** 0.9.0 (full suite).
- **Outcome sentence:** A stuck agent shows its recovery card, real runs fill the Runs ledger, To-do automation settings take effect, watched PRs wake their To-dos on CI, comment, conflict and merge events, and a step's Markdown context can be edited in the drawer.

## Gaps (from `milestones/TODO2.md`)
1. **T5 stuck agent:** nothing in the daemon sets `stuck_agent`. Set it from the session's real state (an owned item whose agent session has been waiting on nothing / errored / stopped while the item is In progress), clear it when the agent moves again. The card's Retry / Take back already exist.
2. **T6 runs:** `record_todo_run` is only called from tests. Record a run when a session that owns or is linked to an item ends a turn/exits: tool, model, outcome, links, cost when the agent reports one (no made-up cost: unknown stays unknown).
3. **T7 automation:** `todo_automation` is never read (`todo_automation: None` in `daemon/humatad/src/settings_service.rs`). Persist and return it; make Ask/Auto drive the PR merged/closed, To-do close → archive clean sessions (Auto with Undo), last session archived, and idle prompts. Call `evaluate_session_idle` and `evaluate_last_session_archived` from the live session events (archive, idle ticks), not just tests.
4. **T8 watchers:** no watcher polls or subscribes to PR/CI state. Add a daemon watcher for watched PRs using the existing gh/PR plumbing (push where available, else the existing PR refresh path; no tight poll loops; obey rate limits and say "wait" on limits), feed `pr_watch.rs` evaluations, wake the session or surface *Send comments to session*; wire that action in the To-do UI.
5. **T2 step Markdown editor:** `TodoAction::SetStepMarkdown` exists; the drawer only displays it. Add the edit control, copying the signed-off TODO2 frames (`milestones/mockups/` TODO2 frames; ask Fable if no frame covers it, and get its sign-off before re-recording any golden).

## Exit
- Each gap above has a test that fails before its fix (unit tests in the leaf crates; a scoped daemon test only where the event feed needs one, on the narrowest profile per AGENTS.md).
- TODO2.md items T2, T5–T8 ticked with commits.

## Out of scope
New TODO2 features beyond the five gaps; Settings page redesign.

## Lane

- **State (2026-11-20, resumed after 0.9.0):** batches 1 (T5–T8 + wiring) and 2 (T2) are written and waiting on main's build. Release target is 0.10.0. One gap remains: the PR watcher's Auto close on merge needs the `pr_watcher.rs` hunk that is waiting on main.
- **Design:** see the plan below (the logic lives in `daemon/svc_board` modules, with ports for settings, session control and PR status; humatad wiring comes last).
- **Manager's own edits so far:**
  - `daemon/svc_board/src/ports.rs`: appended `SeenStatus`, `SessionSeen`, `SyncMode`, `TodoAutomation` (+`IDLE_NUDGE_DAYS`), `TodoAutomationSource`, `ArchiveOutcome`, `SessionControl`, `PrStatus`, `PrStatusError`, `PrStatusPort`.
  - `daemon/svc_board/src/lib.rs`: the `mod` lines for `automation`, `pr_watch_gh`, `pr_watcher` and `session_events`; the re-exports; the `BoardSvc` fields `automation`, `sessions_control`, `pr_status`, `seen` and `pr_watch_state`; the builders `with_automation`, `with_session_control` and `with_pr_status`; and `automation_for`.
  - Stub files for the four new modules.
- **Workers (batch 1):**
  | Worker | Task | Files | Status |
  |---|---|---|---|
  | W-A (Sonnet) | T5 stuck agent, T6 runs, T7 last-archived/idle: `observe_session`, `session_tick`, `seed_sessions` | `svc_board/src/session_events.rs` | done, unbuilt |
  | W-B (Sonnet) | T7 Ask/Auto: PR merged, close → archive clean sessions with Undo, prompt choices act on sessions (incl. Send comments to session) | `svc_board/src/automation.rs`, `svc_board/src/todo.rs`, one field in `todo_fields.rs` | done, unbuilt |
  | W-C (Sonnet) | T8 watcher: gh `PrStatusPort`, `refresh_watched_prs`, driver with Notify, rate-limit waits | `svc_board/src/pr_watcher.rs`, `svc_board/src/pr_watch_gh.rs` | done, unbuilt |
  | W-D (Sonnet) | T6 unknown cost end to end | `api/proto/humata/v1/boards.proto` (TodoRun.cost_micros optional), `daemon/store/src/todo_runs.rs`, `daemon/store/tests/boards.rs`, `todo/src/model.rs`, `clients/src/todo.rs`, `clients/src/todo/fake.rs`, `clients/live_daemon/src/todo.rs`, the Runs ledger hunk of `ui/workspace/src/todo.rs` | done, unbuilt |
  | Fable | T2 step-context editor ruling, plus the Settings › To-do frame | `workflow/milestones/mockups/todo2wire/**` (new) | done: D901, D902 |
  | W-F (Sonnet) | humatad wiring: `SessionsAdapter`, event pump, tick, PR watcher, plus the scoped test `tests/todo_automation.rs` | `daemon/humatad/src/wire_board.rs`, the new test | done, unbuilt |
  | W-E (Sonnet) | T2 step context editor (D901) | `features/todo/src/lib.rs`, `ui/workspace/src/todo.rs`, `features/todo/tests/todo2.rs` | done, unbuilt |
- **Manager's seam edits:**
  - `events.rs`: `subscribe_events` is `pub`.
  - `plugins.rs`: `BoardContext.sessions`.
  - `serve.rs`: `board_sessions` OnceLock.
  - `humatad/BUILD.bazel`: the tokio and rpc_kit deps on both `humatad_plugin_board*` targets, plus `todo_automation` in `AWAY_BOARD_SCOPED_TESTS`.
  - `svc_mcp/src/mcp/todo.rs`: an unreported cost stays null.
  - DECISIONS D901–D903.
- **Batch 3:** done, unbuilt.
  - The session_events fixture fix.
  - The pr_watcher Auto-merge hunk and its test.
  - Settings persistence in `settings_service.rs`, `settings_service/sections.rs`, one line in `humatad/src/lib.rs`, and `AutomationAdapter` in `wire_board.rs`, tested in `tests/settings_sections.rs`.
  - The T2 click-through in `desktop/tests/native/todo_step_context.rs`, plus its line in `desktop/tests/native.rs`.
- **Batch 4:** done, unbuilt. The Settings › General To-do group (D902) across clients, `features/settings` and `ui/settings`.
- **GAPS_REVIEW:** TODO2WIRE entry logged; it includes the pre-existing `with_review_requests` branch.
- **Next:** main's build of batches 3 and 4. Then tick T2 and T5–T8 in TODO2.md and say "ready to close".
- **Ready to commit:** no.
