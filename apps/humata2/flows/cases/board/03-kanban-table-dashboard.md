# Kanban, Table and Dashboard show the same items, and every figure opens what it counts

Layer: verify (Board, the person). Platforms: macOS.

## Starting state

- Two channels: `#payments` (PAY-1 In progress with a session, PAY-2 To do, PAY-3 Done today) and `#infra` (INF-1 To do, no session).
- One agent ask waits on PAY-1 (a move to Done).
- The app is open on the sidebar's **Board** (Global board), kanban, scope "All channels".

## Reproduce headless

- Against a real daemon: `bazel test //apps/humata2/desktop:live_boards_test --test_filter=the_three_views_show_the_same_items`.
- The frames, on fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=board_reference_images`. It compares frames 21 (kanban), 23 (table) and 24 (dashboard) and clicks one tile. Evidence: `desktop/tests/__snapshots__/native/21-board-kanban.view.txt`, `23-board-table.view.txt`, `24-board-dashboard.view.txt`, and any `.new.png` on failure.
- Counting rules: `bazel test //apps/humata2/features/board:presentation_test`.
- Not headless: nothing. Light and dark are both rendered headless.

## Steps

1. **I do:** look at the kanban.
   **I see:** five lanes with counts. Each card has its key, its channel, its title and a chip per session. PAY-1 is tinted and has **Review**.
   **State and events:** `ListItems` for the Global view with channels. Session-board items are left out.
   **Status:** Works (real daemon). `desktop/tests/live_boards.rs::the_three_views_show_the_same_items_and_a_dashboard_figure_opens_what_it_counts`; `features/board/tests/scope.rs::the_global_board_shows_global_and_channel_items_but_no_session_items`.

2. **I do:** click **Table**.
   **I see:** the same four items, one per row, grouped by status with a count per group. Columns: Key, Title, Worked on by, Channel. PAY-1's row is tinted and has **Review**.
   **State and events:** no call. The view is kept for this board.
   **Status:** Works (real daemon). Same live test. Frame 23 on fakes.

3. **I do:** click a row.
   **I see:** the item's drawer, as from a card.
   **Status:** Fakes only. `desktop/tests/native/board.rs::board_reference_images`.

4. **I do:** type "INF" in **Filter**.
   **I see:** only INF-1. The filter matches key, title and session title.
   **State and events:** none. Local.
   **Status:** Fakes only. `features/board/tests/presentation.rs::global_board_includes_channel_items_and_drills_without_leaking_other_items`. BD-11 also names the channel as a match: Code, no test.

5. **I do:** clear the filter. Click **Dashboard**.
   **I see:** four tiles: **Waiting on you: 1**, **In progress: 1**, **Done this week: 1**, **Unassigned: 2**. Then **By channel** bars, **Done per day** over 14 days, and **By session** with **Open** or **Resume**.
   **State and events:** the figures are computed in the app from the items in scope.
   **Status:** Works (real daemon) for the same items being counted: the live test. The exact figures: Fakes only, frame 24.

6. **I do:** click **Waiting on you**.
   **I see:** the Table, showing only PAY-1, under a "Waiting on you" label with **Show all**.
   **State and events:** a drill filter. The drawer closes.
   **Status:** Fakes only. `features/board/tests/presentation.rs::dashboard_drills_show_exactly_the_counted_items`. One tile is clicked live in the test of step 1.

7. **I do:** click **Show all**, go back to the Dashboard, click the `#infra` bar, then today's bar under **Done per day**.
   **I see:** INF-1 only; then PAY-3 only.
   **Status:** Fakes only. Same test. Whether the channel bar opens that channel's board (BD-13) or drills the table: Code, no test.

8. **I do:** on **By session**, click **Open** beside PAY-1's session.
   **I see:** that session's page.
   **Status:** Code, no test. `ui/src/board.rs:874`.

## Branches

### An empty board

When the scope has no items. The kanban shows five empty lanes with **+**. The Dashboard shows zeros.
**Status:** Code, no test. `features/desk/tests/board.rs::a_workspace_with_no_items_has_no_lanes` covers only the Desk panel.

### An agent adds an item while I look

When an agent creates PAY-4 during step 2. The row appears without a reload.
**Status:** Works (real daemon) in the client. `clients/live/tests/boards.rs::a_move_after_subscribing_is_pushed_as_an_item_change`. The app redrawing: Code, no test.

### Folding a group in the Table

When I click a group's chevron. Nothing happens.
**Status:** Not built (BD-12 says each group folds). `ui/src/board.rs:537` draws the chevron with no action.

### The view after a restart

When I quit and reopen the app. The board opens on the view, scope and filter I left (BD-11).
**Status:** Fakes only for the scope: `features/board/tests/scope.rs::picking_a_channel_view_saves_it_and_closes_the_menu`. View and filter kept across a real restart: Code, no test.

## Expected outcomes

- [ ] The three views list the same items for the same scope.
- [ ] Every Dashboard figure opens exactly the items it counts.
- [ ] A card and a row with a waiting ask both offer **Review**.
- [ ] The filter narrows without a call to the daemon.
- [ ] An item made elsewhere appears without a reload.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Kanban | Works (real daemon) | `live_boards.rs::the_three_views_…` |
| 2 Table | Works (real daemon) | same |
| 3 Row opens drawer | Fakes only | `native/board.rs::board_reference_images` |
| 4 Filter | Fakes only | `presentation.rs::global_board_includes_…` |
| 5 Dashboard | Works (real daemon); figures Fakes only | live test; frame 24 |
| 6 Tile drill | Fakes only | `presentation.rs::dashboard_drills_…` |
| 7 Bars drill | Fakes only | same |
| 8 By session Open | Code, no test | `ui/src/board.rs:874` |
| Empty board | Code, no test | none found |
| Live add | Works (real daemon) | `clients/live/tests/boards.rs` |
| Fold a group | Not built | `ui/src/board.rs:537` |
| Kept after restart | Fakes only | `scope.rs::picking_a_channel_view_…` |

## Gaps and likely bugs

- Not built: folding a Table group (BD-12).
- Not built: the Graph view (case 4).
- Likely bug: "Done per day" bars are labelled "Day 1" to "Day 14" (`ui/src/board.rs:813`), not dates. A person can't tell which day a bar is. Confirm in `24-board-dashboard.view.txt`.
- Likely bug: **Waiting on you** counts only asks about an existing item. A create ask (a new Global item) is waiting on me too, and is not counted, because it has no item yet (`features/board/src/lib.rs`, `Filter::Waiting`). Confirm with one create ask pending and the tile at 0.
- The exact Dashboard figures are checked only on the design fixture.
