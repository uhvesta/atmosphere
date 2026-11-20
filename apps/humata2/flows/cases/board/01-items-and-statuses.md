# An item is made, gets its key, and moves between states

Layer: verify (To-do, the person). Platforms: macOS.

BOARDSIMPLE (D830–D874) replaced the five kanban columns (`Backlog`, `To do`, `In progress`, `In review`, `Done`) with one ranked **To-do** list and three states: `Open`, `In progress` and `Closed` (with `Show closed` / `Reopen` for items closed in the last 7 days). `BoardService` and `humata board` still map the five wire columns onto those three states (`Backlog` / `To do` → `Open`, `In progress` / `In review` → `In progress`, `Done` → `Closed`).

## Starting state

- A real `humatad2` runs with one channel, `#payments`, on this Mac. The To-do list is empty.
- The app is open on the sidebar's **To-do** page (or `#payments`'s **To-do** tab).
- No agent session is needed.

## Reproduce headless

- Real daemon through `LiveTodo`: `bazel test //apps/humata2/clients/live_daemon:todo_test`.
- Reducer and quick-add rules on fakes: `bazel test //apps/humata2/features/todo:todo_test` and `bazel test //apps/humata2/todo:all`.
- Daemon unit tests (`TodoService` + migration + states): `bazel test //apps/humata2/daemon/svc_board:svc_board_unit_test`.
- Daemon `BoardService` keys and `done_at`: `bazel test //apps/humata2/daemon/humatad:boards_test`.
- By hand on the rig ([README](README.md)): `humata channel create payments`, then the `humata board` commands in the steps. Keep each command's stdout and exit code.
- Not headless: nothing. There is no drag-between-columns gesture in To-do; order is computed by rank and `Pin`.

## Steps

1. **I do:** type `Schema #payments` (or `fix login redirect loop fri #bug payments s high`) into the quick-add field at the top of To-do and press Enter.
   **I see:** a live preview under the field showing the parsed title, due date, tags, channel, effort, impact and where the item would rank; pressing Enter adds it (`PAY-1`) in `Open`.
   **State and events:** `TodoService.CreateTodo`. The key is the channel name's first three letters, uppercased, and the next number (`PAY-1`). `BoardItem` created; watchers refresh.
   **Status:** Works (real daemon) for `CreateTodo` and listing: `clients/live_daemon/tests/todo.rs::the_daemon_keeps_a_to_do_the_person_adds_and_closes`; `daemon/humatad/tests/boards.rs::keys_count_up_per_channel_and_a_taken_prefix_lengthens`. Quick-add parsing and preview in the app: Fakes only, `features/todo/tests/todo.rs::quick_add_reads_the_line_previews_the_place_and_adds_the_item` and `todo/tests/quick_add.rs`.

2. **I do:** add an item with no channel (`Q4 plan`).
   **I see:** `GLB-1 Q4 plan` listed under `No channel`.
   **State and events:** an item with no channel numbers under `GLB`.
   **Status:** Works (real daemon). `clients/live_daemon/tests/todo.rs::the_daemon_keeps_a_to_do_the_person_adds_and_closes`; `daemon/humatad/tests/boards.rs::the_global_board_numbers_under_glb`.

3. **I do:** open `PAY-1` in the To-do drawer and click **In progress** in the **State** segmented control.
   **I see:** the item moves into the **In progress** group above **Next**. The drawer stays open.
   **State and events:** `TodoService.UpdateTodo` with `state: InProgress` and `expected_revision`.
   **Status:** Works (real daemon) in `svc_board`: `daemon/svc_board` unit tests; Fakes only for the reducer/UI: `features/todo/tests/todo.rs`.

4. **I do:** click the check circle (`todo-check PAY-1`) on the row, or **Closed** in the drawer. Then toggle **Show closed** and click **Reopen**.
   **I see:** the item closes and leaves the open list; turning on **Show closed** shows it under **Closed · N this week** with **Reopen**; clicking **Reopen** brings it back to **Open** (without restoring an old pin).
   **State and events:** `UpdateTodo` sets `closed_at` on close and clears it on reopen; items closed more than 7 days ago drop out of the weekly Closed group.
   **Status:** Works (real daemon) for close/reopen: `clients/live_daemon/tests/todo.rs::the_daemon_keeps_a_to_do_the_person_adds_and_closes`; `daemon/humatad/tests/boards.rs::done_at_is_set_in_done_and_cleared_out_of_it`. App reducer and 7-day window: Fakes only, `features/todo/tests/todo.rs::close_then_reopen` and `todo/tests/rank.rs::closed_older_than_a_week_is_dropped`.

5. **I do:** click **Pin** in the drawer (or unpin).
   **I see:** the item moves to the top of **Next** with a `Pinned` reason chip, ahead of scored items. Closing a pinned item takes the pin off.
   **State and events:** `UpdateTodo` with `pin_order` (1-based on the wire; 0 unpins).
   **Status:** Fakes only in the app: `features/todo/tests/todo.rs::pin_goes_after_the_pins_and_move_pin_puts_it_on_top` and `todo/tests/rank.rs::pins_order_before_score`. Wire mapping unit-tested in `clients/live_daemon/src/todo.rs::tests::pins_count_from_one_on_the_wire`.

6. **I do:** in a terminal: `humata --json board --channel payments` and `humata --json board --channel payments --column done`.
   **I see:** the items with `id`, `title`, `column`, `revision`; then only closed/done items. `counts` names every wire column either way.
   **State and events:** `BoardService.ListItems`. Exit 0.
   **Status:** Works (real daemon) for the daemon: `daemon/humatad/tests/boards.rs::a_list_counts_every_column_and_filters_by_the_columns_asked`. The CLI command itself is Fakes only: `cli/tests/boards.rs::a_live_read_carries_no_age`.

## Branches

### An empty title

When I press Enter in quick add with only spaces typed. Nothing is sent and the field stays. From the CLI, `humata board add "  "` is refused as a missing title, exit 2.
**Status:** Fakes only in the app: `features/todo/tests/todo.rs::an_empty_quick_add_line_adds_nothing` and `todo/tests/quick_add.rs::no_title_means_no_draft`. Daemon validation in `daemon/svc_board/src/todo.rs`.

### A write against an old copy

When another screen or an agent changed the item after I loaded it. My write is refused with a revision conflict, the optimistic edit rolls back, and the list shows why.
**Status:** Fakes only in the app: `features/todo/tests/todo.rs::a_stale_revision_rolls_back_and_shows_an_error`. Daemon enforcement: Works (real daemon) in `daemon/svc_board` unit tests and `daemon/humatad/tests/boards.rs`.

### Migrated items from old boards

When upgrading from an earlier build with kanban boards. A one-time migration maps Backlog/To do → Open, In progress/In review → In progress, Done → Closed, VIP → Pinned, and keeps the owner's old order as the starting rank with "Was 1st in To do" reasons; a fresh install shows no migration banner.
**Status:** Works (real daemon) for fresh-install suppression: `clients/live_daemon/tests/todo.rs::a_fresh_daemon_has_no_boards_to_move_so_no_migration_notice`. Migration rules: `todo/tests/migrate.rs` and `daemon/svc_board` unit tests. Banner dismissal in the app: Fakes only, `features/todo/tests/todo.rs::dismissing_the_migration_banner_calls_the_client`.

## Expected outcomes

- [ ] A channel item's key is the channel's prefix and the next number; an item with no channel uses `GLB-n`.
- [ ] Quick add parses trailing due date, effort, impact, channel and `#tag` tokens and previews the rank position before submit.
- [ ] Items move cleanly between `Open`, `In progress` and `Closed`; closing sets `closed_at` and clears any pin, and reopening within 7 days restores `Open`.
- [ ] Pinned items stay ahead of scored items in pin order.
- [ ] Stale-revision writes roll back and explain why.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Quick add + channel key | Works (real daemon) for create/key; quick-add Fakes only | `clients/live_daemon/tests/todo.rs`, `features/todo/tests/todo.rs`, `todo/tests/quick_add.rs` |
| 2 No-channel (`GLB`) item | Works (real daemon) | `clients/live_daemon/tests/todo.rs`, `daemon/humatad/tests/boards.rs` |
| 3 Open → In progress | Works (real daemon) in `svc_board`; UI Fakes only | `daemon/svc_board:svc_board_unit_test`, `features/todo/tests/todo.rs` |
| 4 Close, Show closed, Reopen | Works (real daemon) | `clients/live_daemon/tests/todo.rs`, `features/todo/tests/todo.rs::close_then_reopen` |
| 5 Pin / Unpin | Fakes only in app; wire unit-tested | `features/todo/tests/todo.rs`, `todo/tests/rank.rs` |
| 6 CLI `humata board` list | Works (real daemon) in daemon; CLI Fakes only | `boards.rs::a_list_counts_every_column_…`, `cli/tests/boards.rs` |
| Empty title | Fakes only | `features/todo/tests/todo.rs::an_empty_quick_add_line_adds_nothing` |
| Stale revision | Fakes only in app; daemon Works | `features/todo/tests/todo.rs::a_stale_revision_rolls_back_and_shows_an_error` |
| Board migration | Works (real daemon) for fresh install; rules unit-tested | `clients/live_daemon/tests/todo.rs`, `todo/tests/migrate.rs` |

## Gaps and likely bugs

- Not built: a `humata todo` CLI subcommand for quick add, due dates, effort, impact, tags, steps, pin or hide-until (`humata board` covers only the legacy board fields).
- Quick-add typing and inline state/pin clicks in the headless desktop app are not yet exercised against a real daemon (covered on fakes in `features/todo/tests/todo.rs` and at the client layer in `clients/live_daemon/tests/todo.rs`).
