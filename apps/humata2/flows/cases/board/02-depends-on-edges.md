# Depends-on edges hide waiting items, refuse loops, and unblock on close

Layer: verify (To-do, the person). Platforms: macOS.

In To-do (BOARDSIMPLE, D830), an item with an unfinished `depends_on` edge sorts into the folded **Waiting on another item** group (with a summary line naming what waits on what) and joins **Next** automatically once its blocker closes. By owner rule, an overdue item never folds into Waiting. Dependencies still do not block state moves on the daemon (BD-14).

## Starting state

- `#payments` has three items: `PAY-1` "Schema", `PAY-2` "Write path", `PAY-3` "Cut over". None depends on another yet.
- An item with no channel: `GLB-1` "Q4 plan".
- The app is open on **To-do**.

## Reproduce headless

- Ranking, waiting group, unblocking and overdue rule: `bazel test //apps/humata2/todo:rank_test`.
- Client dependency CRUD and loop refusal against a real daemon: `bazel test //apps/humata2/clients/live_daemon:boards_test --test_filter=dependency`.
- Daemon dependency rules: `bazel test //apps/humata2/daemon/humatad:boards_test --test_filter=dependency`.
- By hand on the rig: `humata board edit PAY-3 --depends-on PAY-1`, then `humata --json board --channel payments`. Keep stdout and the exit code.
- Not headless: nothing.

## Steps

1. **I do:** set `PAY-3` to depend on `PAY-1` (via `humata board edit PAY-3 --depends-on PAY-1`, MCP `board_item_update`, or `UpdateTodo` with `depends_on_add`).
   **I see:** on the To-do page, `PAY-3` moves into the folded **Waiting on another item** group (`PAY-3 Cut over waits on PAY-1`), and `PAY-1`'s row note mentions that `PAY-3` waits on it. When a single channel is picked, Waiting unfolds by default.
   **State and events:** `UpdateItem` / `UpdateTodo` with `depends_on_add: [PAY-1]`. History gains a dependencies entry.
   **Status:** Works (real daemon) for storing and listing the edge: `clients/live_daemon/tests/boards.rs::a_dependency_is_listed_refuses_a_loop_and_can_be_removed`. Waiting group and folding: unit-tested in `todo/tests/rank.rs::waiting_ends_when_the_blocker_closes` and `ui/workspace/src/todo.rs::tests::waiting_and_hidden_fold_until_a_channel_is_picked`.

2. **I do:** open `PAY-3` in the To-do drawer.
   **I see:** a **Waits on** row showing a `PAY-1` pill.
   **State and events:** read from `Todo.depends_on`.
   **Status:** Code, no live UI test (`ui/workspace/src/todo.rs`, `Waits on` row). Interactive adding/removing of `Waits on` edges from the To-do drawer is not yet built.

3. **I do:** in a terminal, try to close a loop: `humata board edit PAY-1 --depends-on PAY-3`.
   **I see:** a refusal that names the loop, `PAY-1 → PAY-3 → PAY-1`. Exit 7.
   **State and events:** `dependency_loop`. Nothing changes.
   **Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::a_dependency_that_closes_a_loop_is_refused_with_the_loop` and `::an_item_depending_on_itself_is_a_loop_of_one`; `clients/live_daemon/tests/boards.rs::a_dependency_is_listed_refuses_a_loop_and_can_be_removed`. The CLI's exit code is Code, no test.

4. **I do:** close `PAY-1` (check circle or `Closed` in the drawer).
   **I see:** `PAY-3` leaves **Waiting on another item** and joins **Next**.
   **State and events:** `rank()` sees every blocker is `Closed`, so `PAY-3` is no longer waiting; `Dependency.column` is `Done`.
   **Status:** Works (real daemon) for `Dependency.column`: `daemon/humatad/tests/boards.rs::a_dependency_shows_its_title_and_column`. Unblocking in ranking: unit-tested in `todo/tests/rank.rs::waiting_ends_when_the_blocker_closes`.

5. **I do:** give `PAY-3` an overdue due date while it waits on an open `PAY-2`.
   **I see:** `PAY-3` stays in **Next** (never hidden in **Waiting on another item**).
   **State and events:** owner rule in `humata2_todo::rank`.
   **Status:** Unit-tested: `todo/tests/rank.rs::overdue_stays_in_next_even_when_waiting_or_hidden`.

6. **I do:** close `PAY-3` while `PAY-2` is still open.
   **I see:** the state change lands with no refusal.
   **State and events:** dependencies affect ranking/grouping, never blocking a move (BD-14).
   **Status:** Code, no test (`daemon/svc_board` never refuses a state change for an open dependency).

7. **I do:** remove the dependency (`humata board edit PAY-3 --drop-depends-on PAY-2`, MCP, or `UpdateTodo` with `depends_on_remove`).
   **I see:** the edge is removed immediately.
   **Status:** Works (real daemon). `clients/live_daemon/tests/boards.rs::a_dependency_is_listed_refuses_a_loop_and_can_be_removed`.

## Branches

### The target is deleted

When `PAY-2` is deleted while `PAY-3` depends on it. `PAY-3` loses the edge and its history says so.
**Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::deleting_an_item_drops_the_edges_to_it`.

### The target does not exist, or I may not read it

When an agent names an item it can't read, or a wrong key. Both answer the same: not found, exit 3.
**Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::a_dependency_on_a_missing_item_is_not_found`.

### Two people edit the list at once

When an agent adds `PAY-2` while I add `PAY-1` from an older copy. Both land. Lists never conflict on revision.
**Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::a_change_made_against_an_old_revision_conflicts_only_on_fields_changed_since`.

### An edge across channels

When `PAY-3` depends on `GLB-1`. It is allowed on the daemon and in To-do ranking.
**Status:** Code, no test in the app. The daemon takes any readable item (`daemon/svc_board`).

## Expected outcomes

- [ ] An item waiting on an open blocker folds into **Waiting on another item**, unless it is overdue.
- [ ] Closing the blocker moves the waiting item into **Next**.
- [ ] A loop is refused on the daemon and names the cycle.
- [ ] No state change is refused because of an open dependency.
- [ ] Deleting a target removes the edge and records it on the dependent.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Add edge & Waiting group | Works (real daemon) for edge; unit-tested for Waiting group | `clients/live_daemon/tests/boards.rs`, `todo/tests/rank.rs` |
| 2 `Waits on` in drawer | Code, no UI test; interactive picker Not built | `ui/workspace/src/todo.rs` |
| 3 Loop refused | Works (real daemon) | `boards.rs::a_dependency_that_closes_a_loop_…` |
| 4 Blocker closes → unblocks | Works (real daemon) for wire; unit-tested for rank | `boards.rs::a_dependency_shows_its_title_and_column`, `todo/tests/rank.rs` |
| 5 Overdue never folds | Unit-tested | `todo/tests/rank.rs::overdue_stays_in_next_even_when_waiting_or_hidden` |
| 6 Blocks nothing | Code, no test | `daemon/svc_board` |
| 7 Remove edge | Works (real daemon) | `clients/live_daemon/tests/boards.rs` |
| Target deleted | Works (real daemon) | `boards.rs::deleting_an_item_drops_the_edges_to_it` |
| Missing target | Works (real daemon) | `boards.rs::a_dependency_on_a_missing_item_…` |
| Concurrent list edits | Works (real daemon) | `boards.rs::a_change_made_against_an_old_revision_…` |
| Cross-channel edge | Code, no test | `daemon/svc_board` |

## Gaps and likely bugs

- Not built: an interactive `+ Add` / remove picker for **Waits on** in the To-do drawer (`ui/workspace/src/todo.rs` draws existing `depends_on` pills read-only; edges are currently added/removed via CLI, MCP or `UpdateTodo`).
- Not built: listing an item's dependents inside its drawer (the list's row notes mention what waits on it, e.g. `PAY-9 waits on it`).
- No test proves that an unfinished dependency does not block a move or a session start.
