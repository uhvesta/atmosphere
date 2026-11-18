# Depends-on edges are set, refuse a loop, and block nothing

Layer: verify (Board, the person). Platforms: macOS.

## Starting state

- `#payments` has three items in To do: PAY-1 "Schema", PAY-2 "Write path", PAY-3 "Cut over". None depends on another.
- The Global board has GLB-1 "Q4 plan".
- The app is open on the `#payments` board.

## Reproduce headless

- App clicks against a real daemon: `bazel test //apps/humata2/desktop:live_boards_test --test_filter=new_items_get_the_channels_key_and_a_dependency_is_set_from_the_picker`.
- The picker's rules on fakes: `bazel test //apps/humata2/features/board:drawer_test --test_filter=the_dependency_picker`.
- Daemon: `bazel test //apps/humata2/daemon/humatad:boards_test --test_filter=dependency`.
- By hand on the rig: `humata board edit PAY-3 --depends-on PAY-1`, then `humata --json board --channel payments`. Keep stdout and the exit code.
- Not headless: nothing. The picker is driven by key and click in the headless app.

## Steps

1. **I do:** open PAY-3. Hover **Depends on** and click **+** ("Add dependency").
   **I see:** a picker under the row with a "Filter dependencies" field. It lists PAY-1, PAY-2 and GLB-1. It does not list PAY-3.
   **State and events:** nothing is sent yet.
   **Status:** Works (real daemon). `desktop/tests/live_boards.rs::new_items_get_the_channels_key_and_a_dependency_is_set_from_the_picker`.

2. **I do:** type "PAY-1", press Return.
   **I see:** a chip "PAY-1" under Depends on. The picker stays open for the next pick.
   **State and events:** `UpdateItem` with `depends_on_add: [PAY-1]`. BoardItem updated. History gains a dependencies entry.
   **Status:** Works (real daemon). Same test; `clients/live/tests/boards.rs::a_dependency_is_listed_refuses_a_loop_and_can_be_removed`.

3. **I do:** open PAY-1 and open its dependency picker.
   **I see:** PAY-3 is not offered, because PAY-3 already waits on PAY-1.
   **State and events:** the app leaves out the item, its edges, and everything that waits on it, however far.
   **Status:** Fakes only. `features/board/tests/drawer.rs::the_dependency_picker_leaves_out_itself_existing_edges_and_loops`. The live test checks the same for one hop.

4. **I do:** in a terminal: `humata board edit PAY-1 --depends-on PAY-3`.
   **I see:** a refusal that names the loop, PAY-1 → PAY-3 → PAY-1. Exit 7.
   **State and events:** `dependency_loop`. Nothing changes.
   **Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::a_dependency_that_closes_a_loop_is_refused_with_the_loop` and `::an_item_depending_on_itself_is_a_loop_of_one`. The CLI's exit code is Code, no test.

5. **I do:** move PAY-1 to Done, then open PAY-3.
   **I see:** the PAY-1 chip is marked ✓.
   **State and events:** `Dependency.column` is Done.
   **Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::a_dependency_shows_its_title_and_column`.

6. **I do:** move PAY-3 to Done while PAY-2, which it does not depend on yet, is still in To do. Then add PAY-2 as a dependency and move PAY-3 back and forth.
   **I see:** every move lands. No warning.
   **State and events:** dependencies block nothing (BD-14).
   **Status:** Code, no test. No test asserts that a move with an unfinished dependency is allowed; `daemon/humatad/src/board_service.rs:782` (`move_item`) never reads `depends_on`.

7. **I do:** hover the PAY-1 chip in PAY-3's drawer and click **×** ("Remove dependency PAY-1").
   **I see:** the chip goes at once. No confirmation.
   **State and events:** `UpdateItem` with `depends_on_remove`.
   **Status:** Works (real daemon). `clients/live/tests/boards.rs::a_dependency_is_listed_refuses_a_loop_and_can_be_removed`. The click itself: Code, no test.

## Branches

### The target is deleted

When PAY-2 is deleted while PAY-3 depends on it. PAY-3 loses the edge and its history says so.
**Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::deleting_an_item_drops_the_edges_to_it`.

### The target does not exist, or I may not read it

When an agent names an item it can't read, or a wrong key. Both answer the same: not found, exit 3.
**Status:** Works (real daemon). `::a_dependency_on_a_missing_item_is_not_found`.

### Two people edit the list at once

When an agent adds PAY-2 while I add PAY-1 from an older copy. Both land. Lists never conflict on revision.
**Status:** Works (real daemon). `::a_change_made_against_an_old_revision_conflicts_only_on_fields_changed_since`.

### An edge across boards

When PAY-3 depends on GLB-1. It is allowed, and the chip opens the Global item.
**Status:** Code, no test for a cross-board edge in the app. The daemon takes any readable item (`board_service.rs:563`).

## Expected outcomes

- [ ] The picker never offers the item itself, an existing edge, or an item that would close a loop.
- [ ] A loop sent past the picker is refused and the refusal names the cycle.
- [ ] A finished dependency shows ✓.
- [ ] No move is refused because of a dependency.
- [ ] Deleting a target removes the edge and records it on the dependent.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Open the picker | Works (real daemon) | `live_boards.rs::new_items_get_…` |
| 2 Add an edge | Works (real daemon) | same; `clients/live/tests/boards.rs` |
| 3 Loops left out | Fakes only | `drawer.rs::the_dependency_picker_leaves_out_…` |
| 4 Loop refused | Works (real daemon) | `boards.rs::a_dependency_that_closes_a_loop_…` |
| 5 ✓ on Done | Works (real daemon) | `boards.rs::a_dependency_shows_its_title_and_column` |
| 6 Blocks nothing | Code, no test | `board_service.rs:782` |
| 7 Remove | Works (real daemon) | `clients/live/tests/boards.rs` |
| Target deleted | Works (real daemon) | `boards.rs::deleting_an_item_drops_the_edges_to_it` |
| Missing target | Works (real daemon) | `boards.rs::a_dependency_on_a_missing_item_…` |
| Concurrent list edits | Works (real daemon) | `boards.rs::a_change_made_against_an_old_revision_…` |
| Cross-board edge | Code, no test | `board_service.rs:563` |

## Gaps and likely bugs

- Not built: any picture of the edges. They show only as the drawer's list (case 4).
- Not built: "what waits on this item". The drawer lists what the item depends on, never its dependents. No milestone.
- Removing an edge has no confirmation and no undo (D177). Not a bug by the decision, but easy to do by mistake: the **×** shows only on hover.
- No test proves that an unfinished dependency does not block a move or a session start.
