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

- **State (2026-11-20):** PAUSED by main for the 0.9.0 release, during planning. No code edited, no workers started; this section is the only change. Release target is now 0.10.0.
- **Design notes (draft, from reading the code):**
  - Leaf logic goes in new `daemon/svc_board` modules on `BoardSvc`. It is unit-tested with explicit `now_ms` and fake ports, with no sleeps.
  - `session_events.rs` (T5, T6, T7 archive):
    - `observe_session(&pb::Session, now)` keeps each session's last status in memory and finds linked items (`record.sessions`, `fields.owner.session`, `Session.board_item`).
    - When a session goes from Working to Idle, NeedsInput, ProbablyIdle, Exited, Stopped or ResumeFailed, it records one run per linked item with `record_todo_run`: tool = agent, model, outcome, a session link, and cost unknown.
    - Exited, Stopped or ResumeFailed on an agent-owned In progress item sets `stuck_agent`; Working or Starting clears it.
    - Archived calls `evaluate_last_session_archived` (dedupe prompts by id).
    - `tick(now)` sets idle `session_signals` and calls `evaluate_session_idle` with `idle_nudge_days`. An owned agent session that has been idle too long with no open ask is marked stuck.
  - `automation.rs` (T7):
    - `TodoAutomationSource` port, default Ask. Auto closes the item on PR merge.
    - Closing an item offers to archive its clean sessions (Ask), or archives them with an Undo prompt (Auto). Sessions kept open get a reason.
    - `SessionControl` port: `is_clean`, `archive`, `restore`, `send_note`. `resolve_lifecycle_prompt_in` then acts on "Send to session", "Send comments to session", "Undo" and "Archive".
  - `pr_watcher.rs` + `pr_watch_gh.rs` (T8):
    - `PrStatusPort` uses `gh`, following `pr_reviews_gh.rs` and `HUMATA2_GH`. A rate limit returns a `RateLimited { wait }` error.
    - `refresh_watched_prs(now)` updates the rows and prompts only on a changed signal. Auto-wake follows `auto_wake_on_ci_fail` and `auto_wake_on_review_comments`.
    - The driver runs at most every 5 min, only while there are watched PRs, waits out rate limits, and is also triggered when a watched PR is added.
  - T6 cost: unknown cost must stay unknown. Plan: `optional uint64 cost_micros` in `boards.proto` `TodoRun` (needs a main seam), `Option` in `store/todo_runs.rs` (store -1 = unknown in the existing column, no migration), and a ledger that drops `$` when no run's cost is known. Needs claims on `todo/src/model.rs`, `clients/src/todo.rs`, `clients/src/todo/fake.rs`, `clients/live_daemon/src/todo.rs`.
  - T2: add `EditField::StepContext(usize)` in `features/todo`; committing it sends `SetStepMarkdown`. The drawer affordance in `ui/workspace/src/todo.rs` is waiting on a Fable ruling (frames 12/22 must not change; consider a hover-only "Add context" plus clicking the existing context to edit).
  - humatad wiring, after the above:
    - `wire_board.rs` adapters subscribe to session events, seed from ListSessions and run the tick and watcher tasks.
    - Seams: `Feed::subscribe_events` → `pub` in `events.rs`, and extra `BoardContext` fields in `plugins.rs`, filled in `serve.rs` (main/BROWSER).
    - `settings_service` todo_automation persistence comes after the FUSEFSWIRE hand-over.
    - One scoped daemon test on `humatad2_test_away_board`.
  - A Settings toggle UI has no frame yet; Fable needs to draw it first.
- **Files changed:** this file only.
- **Workers:** none yet.
- **Ready to commit:** no.
