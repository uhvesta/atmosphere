# An item is made in a column, gets its key, and moves between statuses

Layer: verify (Board, the person). Platforms: macOS.

## Starting state

- A real `humatad2` runs with one channel, `#payments`, on this Mac. Its board is empty. The Global board is empty.
- The app is open on `#payments`, **Board** tab, kanban. Five lanes show: Backlog, To do, In progress, In review, Done.
- No agent session is needed.

## Reproduce headless

- App clicks against a real daemon: `bazel test //apps/humata2/desktop:live_boards_test --test_filter=new_items_get_the_channels_key`. Status menu: `bazel test //apps/humata2/desktop:live_flows_test --test_filter=the_drawer_moves_links_and_assigns`.
- Daemon only: `bazel test //apps/humata2/daemon/humatad:boards_test`.
- By hand on the rig ([README](README.md)): `humata channel create payments`, then the `humata board` commands in the steps. Keep each command's stdout and exit code.
- Not headless: the drag gesture. It needs a pointer in a window. The nearest check is the reducer's `Move` action, which a drag and the Status menu both send.

## Steps

1. **I do:** click **+** on To do, type "Schema", click **Create item**.
   **I see:** a card "PAY-1 Schema" in To do. The lane count is 1.
   **State and events:** `CreateItem` on the channel board. The key is the channel name's first three letters, uppercased, and the next number. BoardItem created.
   **Status:** Works (real daemon). `desktop/tests/live_boards.rs::new_items_get_the_channels_key_and_a_dependency_is_set_from_the_picker`; `clients/live/tests/boards.rs::a_channel_item_gets_the_channels_key_and_is_listed_in_to_do`.

2. **I do:** click **+** on Backlog, type "Cut over", create it.
   **I see:** "PAY-2 Cut over" in Backlog.
   **State and events:** the app sends `CreateItem` (To do), then `MoveItem` (Backlog). Two history entries, not one.
   **Status:** Fakes only. `desktop/tests/native/board.rs::board_creates_in_selected_column_and_keeps_channel_scope`. Unproven live: the second call landing. Likely bug, see Gaps.

3. **I do:** on the sidebar's **Board**, click **New item**, type "Q4 plan", create it.
   **I see:** "GLB-1 Q4 plan" with a "Global" label.
   **State and events:** the Global board numbers under `GLB`.
   **Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::the_global_board_numbers_under_glb`; `clients/live/tests/boards.rs::a_global_item_gets_the_global_key_and_scope`.

4. **I do:** open PAY-1, click **Status**, pick **In review**.
   **I see:** the menu lists the five columns. The card moves to In review. The drawer stays open.
   **State and events:** `MoveItem` with the revision the app last saw. BoardItem updated. History gains "status To do → In review".
   **Status:** Works (real daemon). `desktop/tests/live_flows/board_drawer.rs::the_drawer_moves_links_and_assigns_an_item_on_the_daemon`; `features/board/tests/drawer.rs::a_move_from_the_current_copy_lands`.

5. **I do:** drag PAY-2 from Backlog onto In progress.
   **I see:** the card in In progress, at the top.
   **State and events:** the same `Move` as step 4.
   **Status:** Code, no test. `ui/src/board.rs:436` builds the drag. I searched `features/board/tests` and `desktop/tests/live_flows/board_*.rs` for a drop and found none.

6. **I do:** move PAY-1 to Done, then back to In review.
   **I see:** it enters Done, then leaves.
   **State and events:** `done_at` is set in Done and cleared outside it. The Dashboard's "Done this week" follows it.
   **Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::done_at_is_set_in_done_and_cleared_out_of_it`.

7. **I do:** in a terminal: `humata --json board --channel payments` and `humata --json board --channel payments --column done`.
   **I see:** both items with `id`, `title`, `column`, `revision`; then an empty list. `counts` names every column either way.
   **State and events:** `ListItems`. Exit 0.
   **Status:** Works (real daemon) for the daemon: `daemon/humatad/tests/boards.rs::a_list_counts_every_column_and_filters_by_the_columns_asked`. The command itself is Fakes only: `cli/tests/boards.rs::a_live_read_carries_no_age`.

## Branches

### An empty title

When I press **Create item** with only spaces typed. Nothing is sent and the field stays. From the CLI, `humata board add "  "` is refused as a missing title, exit 2.
**Status:** Code, no test. `features/board/src/lib.rs:684`; `daemon/humatad/src/board_service.rs:650`.

### Blocked and Won't do

When an item sits in Blocked or Won't do. The app can show those lanes, but the daemon has only five columns, so no real item can be there. The Status menu and the CLI don't offer them.
**Status:** Not built, no milestone. `clients/live/src/boards.rs:38` maps both to nothing; `ui/src/board.rs:443` shows the lanes only when the fake data fills them.

### A move against an old copy

When an agent moved the item after I opened it. My move is refused, the board shows why, refreshes, and my next try lands. See case 5.
**Status:** Works (real daemon). `desktop/tests/live_flows/board_revision.rs::a_drawer_move_after_an_agents_move_is_refused_refreshes_and_then_lands`.

## Expected outcomes

- [ ] A channel item's key is the channel's prefix and the next number; a Global item's is `GLB-n`.
- [ ] An item made from a lane's **+** ends in that lane.
- [ ] A move from the Status menu and a drag both end with the item in the chosen column, on the daemon.
- [ ] `done_at` is set only while the item is in Done.
- [ ] `humata board` lists the same items and columns as the app.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 New item in To do | Works (real daemon) | `live_boards.rs::new_items_get_the_channels_key_…` |
| 2 **+** on Backlog | Fakes only | `native/board.rs::board_creates_in_selected_column_…` |
| 3 Global item | Works (real daemon) | `boards.rs::the_global_board_numbers_under_glb` |
| 4 Status menu | Works (real daemon) | `live_flows/board_drawer.rs` |
| 5 Drag | Code, no test | `ui/src/board.rs:436` |
| 6 Done and back | Works (real daemon) | `boards.rs::done_at_is_set_in_done_…` |
| 7 CLI list | Works (real daemon); CLI Fakes only | `boards.rs::a_list_counts_every_column_…` |
| Empty title | Code, no test | `features/board/src/lib.rs:684` |
| Blocked, Won't do | Not built | `clients/live/src/boards.rs:38` |
| Old copy | Works (real daemon) | `live_flows/board_revision.rs` |

## Gaps and likely bugs

- Not built: Blocked and Won't do as real columns. The app model has seven statuses (`clients/src/model.rs:1390`), the daemon five (`api/proto/humata/v1/common.proto:157`).
- Likely bug: a **+** on a lane other than To do is two calls (`features/board/src/lib.rs:699`). If the move fails, the item stays in To do with an error line. `CreateItemRequest.column` exists and would make it one call. Confirm by refusing the move in a fake daemon and reading the item's column.
- Likely bug: the app creates in To do (`clients/live/src/boards.rs:294`), while the daemon's default for the CLI and MCP is Backlog (`daemon/humatad/src/board_service.rs:51`). The same "new item" lands in different columns by door.
- No test drives a drag.
