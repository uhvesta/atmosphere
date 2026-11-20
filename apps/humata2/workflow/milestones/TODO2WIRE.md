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
(empty: the manager fills this in)
