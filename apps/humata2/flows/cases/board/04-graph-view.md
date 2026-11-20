# The Graph view (not built; superseded by To-do)

Layer: verify (To-do, the person). Platforms: macOS.

The owner's earlier VERIFY list named a Graph view beside kanban, table and dashboard (`workflow/milestones/VERIFY.md:101`). It was never built, and BOARDSIMPLE (D830–D874) replaced the multi-view board (kanban, table, dashboard) with one ranked **To-do** list where blocked items fold into **Waiting on another item** until their blockers close. This case records the decision and the nearest checks today.

## Starting state

- `#payments` has `PAY-1` "Schema" (`Closed`), `PAY-2` "Write path" (`In progress`), `PAY-3` "Backfill" (`Open`, waits on `PAY-2`), `PAY-4` "Cut over" (`Open`, waits on `PAY-1`, `PAY-2` and `PAY-3`).
- The app is open on **To-do**.

## Reproduce headless

- Waiting/unblocking ranking rules: `bazel test //apps/humata2/todo:rank_test`.
- Daemon dependency data: `bazel test //apps/humata2/daemon/humatad:boards_test --test_filter=a_dependency_shows_its_title_and_column`.
- On the rig: `humata --json board --channel payments` prints each item's `depends_on` with `id`, `title` and `column`.
- Not headless: nothing.

## Steps

1. **I do:** look for a **Graph** (or Kanban/Table/Dashboard) view switch on **To-do**.
   **I see:** one ranked list (`Sort: Ranked` + **Filters**), no view switch.
   **State and events:** none.
   **Status:** Cut / superseded by BOARDSIMPLE (D830, D874).

2. **I do:** look at how dependencies show in **To-do** today.
   **I see:** `PAY-3` and `PAY-4` fold into **Waiting on another item** (`PAY-3 Backfill waits on PAY-2 · …`), and opening `PAY-4` shows its `Waits on` pills in the drawer (`PAY-1`, `PAY-2`, `PAY-3`).
   **State and events:** `humata2_todo::rank` places open items with open blockers in `Ranked.waiting` (unless overdue).
   **Status:** Unit-tested for ranking and folding: `todo/tests/rank.rs::waiting_ends_when_the_blocker_closes`, `ui/workspace/src/todo.rs::tests::waiting_and_hidden_fold_until_a_channel_is_picked`. Daemon edges: Works (real daemon), `daemon/humatad/tests/boards.rs::a_dependency_shows_its_title_and_column`.

3. **I do:** in a terminal, run `humata --json board --channel payments`.
   **I see:** the edges as data (`depends_on`), per item.
   **Status:** Works (real daemon) for the daemon. The CLI output is Fakes only (`cli/tests/boards.rs`).

## Branches

### An agent wants the plan as a picture

When I ask an agent "show me what waits on what". It can read the edges with `board_items_list` and print a list. Preparing a board view (`view_prepare` with `board`) opens the filtered To-do list, not a graph.
**Status:** Works (real daemon) for the read (`daemon/humatad/tests/board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it`); prepared board views open To-do (`features/app/tests/views_revert.rs::reverting_a_board_view_brings_back_the_page_and_the_todo_filter`). A graph view is not built.

### What waits on an item

When I open `PAY-2` and want to see that `PAY-3` and `PAY-4` wait on it. The list's second line notes waiting dependents (e.g. `PAY-9 waits on it`), while the drawer shows only `Waits on` (outgoing dependencies).
**Status:** Unit-tested for row notes (`todo/tests/rank.rs`); incoming dependents in the drawer are Not built.

## Expected outcomes

- [ ] No stale multi-view switcher remains; To-do shows one ranked list with folded Waiting items.
- [ ] Every dependency edge is readable in the drawer's `Waits on` row and from `humata --json board`.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Graph view | Superseded / Not built | BOARDSIMPLE (D830, D874) |
| 2 Waiting group & drawer `Waits on` | Unit-tested; daemon Works | `todo/tests/rank.rs`, `boards.rs::a_dependency_shows_its_title_and_column` |
| 3 CLI edges | Works (real daemon); CLI Fakes only | `cli/tests/boards.rs` |
| Agent read & prepared view | Works (real daemon) for read; view Fakes only | `board_tools.rs`, `features/app/tests/views_revert.rs` |
| Incoming dependents in drawer | Not built | `ui/workspace/src/todo.rs` |

## Gaps and likely bugs

- Not built: a graphical dependency view (superseded by BOARDSIMPLE's single ranked list; would need a new owner direction and design frame if wanted later).
- Not built: listing an item's incoming dependents inside its drawer.
