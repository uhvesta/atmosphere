# MILESTONES (ATM-8..13) — Start a session for a milestone

## Locked-in decisions

1. **ATM-8 — Milestone detection rule:**
   - Any board/To-do item with a non-empty `depends_on` list is treated as a milestone (no proto change required).
   - `humata2_todo_feature::milestone` exposes `is_workspace_milestone`, `workspace_milestone`, `is_todo_milestone`, `todo_milestone`, `MilestoneInfo`, and `MilestoneStep` (plus `TodoState::is_milestone` and `TodoState::milestone`).
   - Steps are resolved in deterministic topological dependency order (dependencies before dependents, preserving `depends_on` order for independent steps) with cycle protection.

2. **ATM-9 — Pre-filled milestone brief:**
   - `App::start_new_for_item` (`features/app/src/new_session.rs`) pre-fills `NewSession::first_message` with `MilestoneInfo::brief()`: milestone id/title/description, open steps in dependency order (skipping `Done`/`Closed`), and instructions to work steps in order and update with `humata board`.
   - Plain tasks keep an empty `first_message`.
   - `App::start_new_for_item_plain` opens the New session sheet for a milestone with an empty `first_message` (for the chevron / `⌥S` plain start).

3. **ATM-10 — Drawer UI & shortcuts (per `DECISIONS.md` §3 & `01-todo-milestone-and-autopr.png`):**
   - Milestone drawer shows a `Milestone` pill + `N of M done` count, a `MILESTONE STEPS · N OF M DONE` section (`in Waits-on order`), and ONE primary split button in the footer: **`Start milestone session ⌄`** (`S` = milestone brief; chevron / `⌥S` = plain start) with `Pre-fills N open steps · S` helper copy.
   - No `···` menu added to board rows.
   - `S` and `⌥S` shortcuts registered in `ui/core/src/keys.rs` (`TODO_MILESTONE` context & `SHORTCUTS` sheet, rendered by `ui/overlays/src/overlay_shortcuts.rs`).

4. **ATM-11 — Session attachment:**
   - Starting a milestone session attaches the session to the **milestone item only** (`SessionOptions::item = Some(milestone_id)`) and moves an open milestone to `In progress`.
   - Individual step items are left untouched for the agent to claim/move as it works through them.

5. **ATM-12 — Tests:**
   - `features/todo` unit tests: milestone detection, topological ordering, cycle handling, Done-step omission, brief formatting.
   - `features/app/tests/board_start.rs`: milestone brief pre-fill, Done omission, plain task empty message, plain-start empty message, milestone `InProgress` + attachment without attaching steps, `TodoAction::StartSession` dispatch.
   - `ui/core` & `ui/workspace` unit tests: `S`/`⌥S` bindings & shortcuts sheet, milestone split-button helper & shortcut dispatch.

## Shared seam needed (main agent)

In `apps/humata2/features/app/src/lib.rs` (after `AppAction::Todo(TodoAction::StartSession(key))` around line 1489):
```rust
AppAction::Todo(TodoAction::StartPlainSession(key)) => {
    self.start_new_for_item_plain(state, ItemId::new(key))
}
```
