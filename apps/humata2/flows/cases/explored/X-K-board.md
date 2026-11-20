# Board: items in columns, the drawer, moving, linking, conflicts, dashboard and the session panel

Layer: explored · macOS · harness: desktop:native_test, desktop:live_boards_test, desktop:live_flows_test (filter `drawer`), features/board and features/desk and features/app tests

Phase 1 (map from code and snapshots, no run). Steps marked PENDING RUN need the phase 2 commands in the report.

## Setup

- Global Board snapshots (frames 21 to 24) use `fixture::design`: items PAY-12, PAY-9, PAY-3 (asking), PAY-5, PAY-2, PAY-7, INF-1, INF-2, INF-4, shown with the channel "All channels" scope. Snapshot text is in `apps/humata2/desktop/tests/__snapshots__/native/21-board-kanban.view.txt`, `22-board-item-open.view.txt`, `23-board-table.view.txt`, `24-board-dashboard.view.txt`, and `07-session-board-panel.view.txt`.
- Live board tests (`live_boards_test`, `live_flows_test`) start a real `humatad2_admin` and holder with a scripted agent. They never touch the owner's home folders.
- Feature tests (`features/board/tests/*.rs`) run the reducer against fake clients and a fake daemon. They need no run to read.

## Steps

### 1. Open the global Board (rail "Board", kanban)

- **Screen:** Filter field ("Filter" placeholder). Scope button "Scope: All channels". "New item" button. Tabs "Board" (selected), "Table", "Dashboard". Lanes "Add item to Backlog", "Add item to To do", "Add item to In progress", "Add item to In review", "Add item to Done". Cards `ListItem "<KEY> <title>"`, e.g. "PAY-3 Ledger schema v2" with a "Review" button (the item asks to move). Blocked and Won't do lanes appear only when they hold items (`ui/src/board.rs` `statuses`).
- **Offered:** Filter text; scope menu; New item; Board / Table / Dashboard tabs; lane "Add item to <lane>"; each card (open); card "Review" (opens the drawer on an asking card); drag a card onto a lane.
- **Action:** Open the Board from the rail.
- **Result:** Items grouped by status. Items with a waiting request show an orange tint and a line that says "Asks to move to Done" (see Surprises: the line is fixed text).
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/21-board-kanban.view.txt` and `21-board-kanban.png`; design golden `desktop/tests/__snapshots__/design/21-board-kanban.png`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=board_reference_images` exits 0 (its first assertion compares frame 21).

### 2. Open an item's drawer

- **Screen:** The drawer (`board-drawer`) shows: title, "Expand item" and "Close item" buttons, a Status popup (current column name), "Sessions" with "Add session", "Depends on" with "Add dependency", a line for the ask if one waits, "Description" ("No description yet." when empty), "History", and "Start a session for this item". When the item is in a session: "Open session" and the session chip. Snapshot 22 shows "Move to Done" and "Keep in progress" for PAY-2's ask and the "Expand item" and "Close item" buttons.
- **Offered:** Expand item; Close item; Status; Add session; Remove session (per row); Add dependency; Remove dependency (per row); Move to Done / Keep in progress (asks only); Deny (the ask); Open session; Start a session for this item.
- **Action:** Click card "PAY-3 Ledger schema v2" in the kanban.
- **Result:** The drawer opens with the item's history read from the daemon (`BoardAction::History`). Opening an item clears the status menu and any picker.
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/22-board-item-open.view.txt` (and `22-board-item-open.png`); design golden `desktop/tests/__snapshots__/design/22-board-item-open.png` (dark).
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=board_reference_images` exits 0 (asserts the drawer frame).

### 3. Move an item from the drawer's Status menu

- **Screen:** Status popup opens a column list: Backlog, To do, In progress, In review, Done. Blocked and Won't do are left out of the menu, although the board can show them (`ui/src/board.rs` status menu comment).
- **Offered:** Each column (`board-move-<column>`).
- **Action:** Open Status, click "In review" for PAY-3.
- **Result:** Sends `move_item(id, status, seen_revision)`. The menu closes. A success clears the board error. The card moves lane.
- **Evidence:** none yet. PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=board_reference_images` exits 0 (its last assertion: PAY-3 becomes "In review" on the daemon). Feature level: `bazel test //apps/humata2/features/board:drawer_test --test_filter=a_move_from_the_current_copy_lands`.

### 4. Drag a card to another lane

- **Screen:** Kanban as step 1. Each lane is a drop target.
- **Offered:** Drag any card to any visible lane.
- **Action:** Drag "PAY-7 Refund retry with backoff" from To do onto In progress.
- **Result:** Sends the same `Move(id, status)` as step 3 with the item's revision as last seen, so an agent's newer move is refused, not overwritten. Dropping onto the Blocked or Won't do lane is possible only when those lanes already show.
- **Evidence:** none. No drag test was found. PENDING RUN.
- **Replay check:** none named. The closest is `bazel test //apps/humata2/features/board:drawer_test --test_filter=a_move_from_an_old_copy_shows_why_refreshes_and_keeps_the_drawer`, which covers the revision check, not the drag gesture. Phase 2 should confirm drag with a headless event or record it as not reachable.

### 5. Add an item into a lane

- **Screen:** Lane "+" (Add item to <lane>) opens a title field with "New item" heading and a "Create" action; the new item's column is the lane's.
- **Offered:** Type a title; Create item; Cancel.
- **Action:** Click "Add item to Backlog", type "A new milestone", click "board-create-item".
- **Result:** Calls `create_item(scope, title)`, then `move_item` when the column is not To do. Two calls, not one: if the move fails, the item exists in To do and the error line shows.
- **Evidence:** none yet. PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=board_creates_in_selected_column_and_keeps_channel_scope` exits 0 (asserts a Backlog item with Global scope).

### 6. Approve an agent's move from the drawer

- **Screen:** Inside the drawer, above History: an orange box with "<agent> asks to move this to Done", the quoted reason, and three buttons: "Move to Done" (primary), "Keep in progress" (the column the item is in now), "Open session". Snapshot 22 shows this for PAY-2's ask.
- **Offered:** Move to Done (approve); Keep in progress (deny); Open session.
- **Action:** Click "Move to Done".
- **Result:** Answers the item's Inbox request with `Approved`. The item moves, the request leaves Waiting, the box disappears, and the call returns to the agent.
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/22-board-item-open.view.txt` shows the box. PENDING RUN for the result.
- **Replay check:** `bazel test //apps/humata2/desktop:live_boards_test --test_filter=approving_an_agents_move_from_the_items_drawer_moves_it_and_returns_the_call` exits 0. Its counterpart: `--test_filter=denying_an_agents_move_from_the_items_drawer_keeps_it_in_progress`.

### 7. Edit and delete asks in the drawer

- **Screen:** An edit ask: title "<agent> asks to edit this", each changed field as "field: before → after", buttons "Apply edit" and "Keep as is". A delete ask: "<agent> asks to delete this", buttons "Delete item" (red) and "Keep item".
- **Offered:** Apply edit / Keep as is; Delete item / Keep item.
- **Action:** Not run in phase 1. PENDING RUN.
- **Result:** Apply sends Approved to the item's request, and the edit or delete happens on the daemon. Keep sends Denied, and nothing changes.
- **Evidence:** none. PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/features/board:drawer_test --test_filter=the_ask_names_the_change_it_waits_on` exits 0 (covers the Move, Edit and Delete ask text in the reducer state).

### 8. A drawer edit or move refused after an agent changed the item

- **Screen:** The refused action shows a red error line on the board page (`board.error`), the board refreshes, and the drawer and picker stay open with the new state.
- **Offered:** The action buttons again (Status, picker rows).
- **Action:** Open the drawer, have an agent move or edit the item on the daemon, then move it from the drawer (or add a dependency from the picker) using the old revision.
- **Result:** The daemon refuses the stale revision. `Done(Err)` sets the error and sends `Delegate::Refresh`. The drawer is not closed.
- **Evidence:** none yet. PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=drawer` exits 0 (matches `a_drawer_edit_refused_after_the_item_changed_on_the_daemon_refreshes_and_keeps_the_picker` and `a_drawer_move_after_an_agents_move_is_refused_refreshes_and_then_lands`). Feature level: `bazel test //apps/humata2/features/board:drawer_test --test_filter=a_refused_edit_shows_why_refreshes_and_keeps_the_picker`.

### 9. Add a session or a dependency from the drawer's pickers

- **Screen:** "Add session" or "Add dependency" opens a picker under the row: a filter field ("Filter sessions" or "Filter dependencies"), a list with the highlighted row (`board-picker-sessions` or `board-picker-dependencies`), up to 160 px tall.
- **Offered:** Type to filter; arrow keys move the highlight; Enter adds the highlighted row; click a row; close the picker.
- **Action:** Click "Add dependency", type "PAY", click the row "PAY-2".
- **Result:** Calls `add_dependency(item, on)`. The picker never offers the item itself, its existing dependencies or anything that already waits on it, however far (no cycles, BD-7). Session rows are limited to the item's channel, and a global item offers every channel's sessions.
- **Evidence:** none yet. PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/features/board:drawer_test --test_filter=the_dependency_picker_leaves_out_itself_existing_edges_and_loops` exits 0. Sessions: `--test_filter=the_session_picker_offers_the_channels_other_sessions_and_stays_open` and `--test_filter=a_global_item_offers_every_session`.

### 10. Remove a session or a dependency from the drawer

- **Screen:** Each session row and dependency row has a remove control ("Remove session <title>", "Remove dependency <id>").
- **Offered:** Remove control per row.
- **Action:** Click "Remove dependency PAY-2".
- **Result:** Removes at once, with no confirmation. A failed removal shows the board error line.
- **Evidence:** none. PENDING RUN.
- **Replay check:** none named. Covered only by the desktop drawer flow above if phase 2 extends it. Record as a gap.

### 11. Table view and the text filter

- **Screen:** Table with columns Key (60 px), Title, Worked on by (220 px), Channel (130 px), and a Review button on an asking row. Filter field narrows by title, key or session title.
- **Offered:** Table tab; Filter; clicking a row opens its drawer; Review on an asking row.
- **Action:** Click "Table", then type "INF" in Filter (or click "Table" after step 1).
- **Result:** Only matching rows show; the Show all button clears a drill-down, not the text.
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/23-board-table.view.txt`; design golden `desktop/tests/__snapshots__/design/23-board-table.png`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=board_reference_images` exits 0 (frame 23 and the filtered table rows after the progress tile click). Also `bazel test //apps/humata2/features/board:presentation_test --test_filter=global_board_includes_channel_items_and_drills_without_leaking_other_items`.

### 12. Dashboard tiles and drill-down

- **Screen:** Tiles "Waiting on you: N", "In progress: N", "Done this week: N", "Unassigned: N". Bars per channel and column ("payments-rewrite To do: 2"). "Done per day" bars "Day 1: 0 done" to "Day 14". "By session" list with Open or Resume.
- **Offered:** Each tile (drills to Table with that filter); each bar (drills); each day bar (drills to completed-on-that-day); each session (opens the session).
- **Action:** Click the "progress" tile (`board-tile-progress`), then "Show all" to clear.
- **Result:** Drill sets the table mode with that filter and closes the drawer. Show all clears the filter. The table then shows only the counted items.
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/24-board-dashboard.view.txt`; design golden `desktop/tests/__snapshots__/design/24-board-dashboard.png` (dark).
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=board_reference_images` exits 0 (asserts `board-table INF-2` shows and `PAY-12` does not after the tile click). Feature: `bazel test //apps/humata2/features/board:presentation_test --test_filter=dashboard_drills_show_exactly_the_counted_items`.

### 13. Scope menu: global, channel, and session boards

- **Screen:** Scope button shows "Scope: <name>". The menu offers "Everything" (all), "Global items only", each channel (`board-scope <channel>`), and on a channel's page the channel views: "Channel board", "Channel + every session board", "<session> and its forks", "<session> board". Forks are offered only for sessions that have them.
- **Offered:** Each entry; "Global items only" toggles hiding channel items on the global board.
- **Action:** Click "Scope: All channels", then "Global items only".
- **Result:** The board shows only global items. Choosing a scope resets the filter; choosing the same scope again keeps it. A saved session view whose session is gone or archived reads as the channel board.
- **Evidence:** none yet. PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/features/board:scope_test` exits 0 (covers each scope, the fork-only offer and the gone-session fallback as named in `features/board/tests/scope.rs`).

### 14. Session strip's Board panel (compact, beside the terminal)

- **Screen:** Panel header "Board" with the channel's name and "Expand panel". Lanes "In progress · 1", "In review · 1", "To do · 2", "Backlog · 1", "Done · 1"; each item is a row (ListItem "<KEY> <title>", a text with the title, and the key). Empty lanes are dropped; Done is last. Asking items show an orange status dot instead of the session chip.
- **Offered:** Expand panel; click a card (opens the item's session, or nothing for an item without one); Review on an asking card (opens the session's Inbox panel with the request selected, in the full screen board only).
- **Action:** Open the Board panel of a session with items (snapshot 07).
- **Result:** Own items of the session first, then the channel's items (`session_panel`).
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/07-session-board-panel.view.txt` (and `07-session-board-panel.png`); design golden `desktop/tests/__snapshots__/design/07-session-board-panel.png`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=session_lifecycle_and_full_screen_reference_images` exits 0. Also `bazel test //apps/humata2/desktop:panels_test --test_filter=board_lists_the_channels_items_and_expands_without_losing_the_session`.

### 15. Full-screen board from the session panel

- **Screen:** The full kanban (frame 12): all five columns always shown (Backlog, To do, In progress, In review, Done), plus Blocked and Won't do when populated; each column has a count and a "+" to add.
- **Offered:** "+" per column; Expand; drag; Review on asking cards (opens the Inbox panel with that request).
- **Action:** Expand the session's Board panel.
- **Result:** Lanes show at full width, Done kept at full strength (not faded).
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/12-panel-full-screen-board.view.txt` (and `.png`); design golden `desktop/tests/__snapshots__/design/12-panel-full-screen-board.png`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=session_lifecycle_and_full_screen_reference_images` exits 0.

### 16. Desk board (the Desk session's board)

- **Screen:** A Desk session has no board of its own. Its panel shows every channel's items (frame 37, `37-desk-board.view.txt`, 103 lines).
- **Offered:** The Desk board's cards and lanes, as the board in step 14.
- **Action:** Open a Desk session's Board panel.
- **Result:** Items from all channels, the session's own items marked and first in the lane.
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/37-desk-board.view.txt` and `37-desk-board.png`.
- **Replay check:** `bazel test //apps/humata2/features/desk:board_test` exits 0 (the lanes, marking and order tests in `features/desk/tests/board.rs`).

## Not reachable headless

- Drag and drop on a real pointer (step 4): the gesture needs a window. Phase 2 can test only the reducer path, so drag is recorded as not reachable headless unless the coach allows a window.
- The Blocked and Won't do lanes appear only when an item already sits there. The headless fixtures do not carry such items unless the test sets them, and no test here creates one through the UI.

## Surprises

1. **"Asks to move to Done" is fixed text.** `ui/src/board.rs` (card, around line 421) and `ui/src/panel_board.rs` (`board_item`, the full-board branch) print "Asks to move to Done" for every waiting item request, including edit asks, delete asks, and moves to other columns. The drawer's own box does say the real move ("asks to move this to <column>").
2. **Create in a column is two calls.** `features/board/src/lib.rs` `CreateNew` creates the item, then moves it if the column is not To do. A failed move leaves the new item in To do with the error line.
3. **Blocked and Won't do cannot be chosen by hand.** The drawer's Status menu leaves them out (`ui/src/board.rs`), and the CLI's `Column` enum has no Blocked or Won't do (`cli/src/args.rs`). An item an agent put there can be seen but not moved back by the person except by drag-and-drop onto a lane that already shows.
4. **The board error line is on the global board only.** `ui/src/board.rs` draws `board.error` at its page foot. The session panel's board (`ui/src/panel_board.rs`) does not read it, so a refused move from the session panel is silent (the drawer is not on the panel).
5. **Remove session and remove dependency have no confirmation.** `RemoveSession` and `RemoveDependency` in the reducer send at once.
6. **The "Waiting on you" tile counts requests, not items with a waiting request in the Inbox panel.** `Filter::Waiting` is `request(item).is_some()`, so it counts only items with a request on `workspace.waiting()`; a stale item request that the daemon has answered is not counted. Check in phase 2 with the dashboard fixture.
7. **Drag has no reducer test name.** Nothing named in `features/board/tests` or `desktop/tests/live_flows/board_*.rs` drives a drop.
