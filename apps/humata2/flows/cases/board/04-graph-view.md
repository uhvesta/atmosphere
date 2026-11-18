# The Graph view draws the depends-on edges

Layer: verify (Board, the person). Platforms: macOS.

The owner's VERIFY list names a Graph view beside kanban, table and dashboard (`workflow/milestones/VERIFY.md:101`). It does not exist. This case says what a person would expect, so the gap is plain and a test can be written when it is built.

## Starting state

- `#payments` has PAY-1 "Schema" (Done), PAY-2 "Write path" (In progress), PAY-3 "Backfill" (To do, depends on PAY-2), PAY-4 "Cut over" (To do, depends on PAY-1, PAY-2 and PAY-3).
- The app is open on the `#payments` board.

## Reproduce headless

- There is nothing to run for the view. To show the gap: `bazel test //apps/humata2/desktop:native_test --test_filter=board_reference_images` and read `desktop/tests/__snapshots__/native/21-board-kanban.view.txt`. The view switch lists "Board", "Table", "Dashboard" and no fourth entry.
- The data a graph needs is there today. On the rig: `humata --json board --channel payments` prints each item's `depends_on` with `id`, `title` and `column`. Keep that output as the evidence that only the drawing is missing.
- Not headless: nothing.

## Steps

1. **I do:** look for **Graph** in the view switch.
   **I see:** Board, Table and Dashboard only.
   **State and events:** none.
   **Status:** Not built, no milestone. `ui/src/board.rs:256` lists three modes. I searched `ui/src`, `features/board/src`, `SPEC.md` §13 and `DECISIONS.md` for "graph" and found no board graph. D106 settled the header as Board / Table / Dashboard.

2. **I do:** (expected) click **Graph**.
   **I see:** (expected) one box per item in scope with its key, title and status, and an arrow from each item to what it depends on. PAY-4 points at PAY-1, PAY-2 and PAY-3. PAY-3 points at PAY-2. Done items are marked ✓.
   **State and events:** (expected) no new call. The edges are already in `BoardItem.depends_on`.
   **Status:** Not built, no milestone.

3. **I do:** (expected) click a box.
   **I see:** (expected) the item's drawer, as from a card.
   **Status:** Not built, no milestone.

4. **I do:** (expected) type in **Filter** and change **Scope**.
   **I see:** (expected) the graph narrows as the other views do. An edge to an item outside the scope still shows, with the other board's name.
   **Status:** Not built, no milestone.

5. **I do:** today's nearest thing. Open PAY-4's drawer.
   **I see:** **Depends on** lists PAY-1 ✓, PAY-2 and PAY-3. Each chip opens that item.
   **State and events:** `GetItem` fills each dependency's title and column.
   **Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::a_dependency_shows_its_title_and_column`; `desktop/tests/live_boards.rs::new_items_get_the_channels_key_and_a_dependency_is_set_from_the_picker`.

6. **I do:** today's other nearest thing. In a terminal, `humata --json board --channel payments`.
   **I see:** the edges as data, per item.
   **Status:** Works (real daemon) for the daemon. The CLI output is Fakes only (`cli/tests/boards.rs`).

## Branches

### An agent wants the plan as a picture

When I ask an agent "show me what waits on what". It can read the edges with `board_items_list` and print a list. It can prepare a board view (`view_prepare` with `board`), which opens the kanban for a scope, not a graph.
**Status:** Works (real daemon) for the read (`daemon/humatad/tests/board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it`). A prepared graph: Not built.

### What waits on an item

When I open PAY-2 and want to see that PAY-3 and PAY-4 wait on it. The drawer shows only what PAY-2 depends on.
**Status:** Not built, no milestone. The app computes dependents only to keep loops out of the picker (`features/board/src/lib.rs:388`).

## Expected outcomes

- [ ] The view switch offers Graph on every board page.
- [ ] Each depends-on edge in scope is drawn once, pointing at the item depended on.
- [ ] A box opens the same drawer as a card.
- [ ] Scope and Filter narrow the graph as they narrow the other views.
- [ ] Until then: every edge can be read from the drawer and from `humata board`.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Graph in the switch | Not built | `ui/src/board.rs:256` |
| 2 Boxes and arrows | Not built | none |
| 3 Box opens the drawer | Not built | none |
| 4 Scope and filter | Not built | none |
| 5 Drawer list | Works (real daemon) | `boards.rs::a_dependency_shows_its_title_and_column` |
| 6 CLI edges | Works (real daemon); CLI Fakes only | `cli/tests/boards.rs` |
| Agent picture | Works for the read; graph Not built | `board_tools.rs` |
| Dependents | Not built | `features/board/src/lib.rs:388` |

## Gaps and likely bugs

- Not built: the Graph view. It has no requirement in `SPEC.md` §13, no decision, no design frame and no milestone. The owner's VERIFY note is the only place it is named. It needs a BD requirement and a frame before code.
- Not built: listing an item's dependents.
- `flows/05-long-running-project.md` step 4 already records this gap.
