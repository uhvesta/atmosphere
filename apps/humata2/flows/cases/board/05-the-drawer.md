# The drawer shows an item whole and edits its sessions and edges

Layer: verify (Board, the person). Platforms: macOS.

## Starting state

- `#payments` has PAY-1 "Schema" (To do, no session), PAY-2 "Write path" (In progress, session "Ledger") and PAY-3 "Cut over" (To do).
- Two sessions run in `#payments`: "Ledger" and "Refunds". A third, "Deploy", runs in `#infra`.
- The app is open on the `#payments` board.

## Reproduce headless

- Against a real daemon: `bazel test //apps/humata2/desktop:live_flows_test --test_filter=drawer` (moves, links, assigns, and the two refused-then-refreshed cases), and `bazel test //apps/humata2/desktop:live_boards_test --test_filter=starting_a_session_for_a_to_do_item`.
- On fakes: `bazel test //apps/humata2/features/board:drawer_test`. Frame 22: `bazel test //apps/humata2/desktop:native_test --test_filter=board_reference_images`, evidence `desktop/tests/__snapshots__/native/22-board-item-open.view.txt`.
- Not headless: hover. The **+** and **×** controls paint only on hover or focus. Headless, they are reached by focus and by their accessibility labels.

## Steps

1. **I do:** click the card "PAY-2 Write path".
   **I see:** the drawer: the title, **Expand item**, **Close item**, **Status** (In progress), **Channel**, **Sessions** (Ledger, with its status dot), **Depends on**, **Description** ("No description yet."), **History**, and **Start a session for this item**.
   **State and events:** `GetItemHistory` is read on open. A late answer for an item I already left is dropped.
   **Status:** Fakes only. `features/board/tests/drawer.rs::opening_an_item_reads_its_history_and_late_reads_are_dropped`; frame 22.

2. **I do:** read **History**.
   **I see:** one line per change, oldest first: made, moved, sessions changed. Each names who did it.
   **State and events:** entries carry a kind (created, changed, moved, sessions, dependencies, deleted).
   **Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::history_records_what_happened_in_order_with_its_kind`; `clients/live/tests/boards.rs::an_items_history_has_its_move_and_an_unknown_item_is_not_found`.

3. **I do:** click **+** on **Sessions**, type "Ref", press Return.
   **I see:** the picker offers the channel's other sessions (Refunds), not Deploy. Refunds is added. The picker stays open.
   **State and events:** `UpdateItem` with `sessions_add`. BoardItem updated.
   **Status:** Works (real daemon). `desktop/tests/live_flows/board_drawer.rs::the_drawer_moves_links_and_assigns_an_item_on_the_daemon`; picker rules in `drawer.rs::the_session_picker_offers_the_channels_other_sessions_and_stays_open`.

4. **I do:** click **×** beside Refunds ("Remove session Refunds").
   **I see:** it goes at once.
   **State and events:** `UpdateItem` with `sessions_remove`.
   **Status:** Works (real daemon) in the client: `clients/live/tests/boards.rs::a_session_set_on_an_item_is_listed_until_it_is_taken_off`. The click: Code, no test.

5. **I do:** click the session line "Ledger".
   **I see:** that session's page.
   **Status:** Code, no test. BD-3.

6. **I do:** open PAY-1 and click **Start a session for this item**. In the sheet, the title reads "Schema" and the sheet says "for PAY-1". I start it.
   **I see:** the new session's page. Back on the board, PAY-1 is In progress and lists the session.
   **State and events:** Session created. Then the app adds the session to the item and moves it. BoardItem updated.
   **Status:** Works (real daemon). `desktop/tests/live_boards.rs::starting_a_session_for_a_to_do_item_opens_it_and_moves_the_item_in_progress`. An item already In review keeps its column: `features/app/tests/board_start.rs::starting_a_session_for_an_item_in_review_keeps_its_column`.

7. **I do:** click **Expand item**, then again.
   **I see:** the drawer fills the page, then goes back.
   **Status:** Fakes only. `features/board/tests/presentation.rs::drawer_rejects_unknown_items_and_new_items_keep_the_requested_column`.

8. **I do:** click **Close item**, or change the scope.
   **I see:** the drawer and any open picker close.
   **Status:** Fakes only. `drawer.rs::closing_the_drawer_closes_the_picker` and `::picking_the_scope_already_shown_keeps_the_drawer_and_another_scope_closes_it`.

## Branches

### An agent changed the item while the drawer was open

When I add a dependency or move the item from a copy that is now old. The write is refused. A red line on the board says why. The board refreshes, and the drawer and picker stay open on the new state. My next try lands.
**Status:** Works (real daemon). `desktop/tests/live_flows/board_conflict.rs::a_drawer_edit_refused_after_the_item_changed_on_the_daemon_refreshes_and_keeps_the_picker`; `board_revision.rs::a_drawer_move_after_an_agents_move_is_refused_refreshes_and_then_lands`.

### I want to rename the item or write its description

When I click the title or the description. Nothing is editable.
**Status:** Not built (D177 defers the editor to the owner). `features/board/src/lib.rs:439` has no title or description action. From a terminal, `humata board edit PAY-1 --title … --description …` works: Works (real daemon) for the daemon, `boards.rs::a_change_made_against_an_old_revision_conflicts_only_on_fields_changed_since`.

### I want to delete the item

When I look for a delete control. There is none.
**Status:** Not built, no milestone. `humata board rm PAY-1` does it and leaves a tombstone in history: Works (real daemon), `boards.rs::deleting_an_item_drops_the_edges_to_it`.

### The item was deleted elsewhere

When an agent deletes the item while its drawer is open. The drawer should close.
**Status:** Code, no test. The client turns the event into `ItemGone`; `presentation.rs::drawer_rejects_unknown_items_…` covers opening an unknown item, not one that goes.

## Expected outcomes

- [ ] The drawer shows status, channel, sessions, dependencies, description and history for the item.
- [ ] Adding and removing a session or a dependency changes the item on the daemon.
- [ ] A write from an old copy is refused, explained, and lands on the retry, with the drawer still open.
- [ ] Starting a session from a To do item adds the session and moves the item to In progress.
- [ ] History names each change and who made it.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Open | Fakes only | `drawer.rs::opening_an_item_reads_its_history_…` |
| 2 History | Works (real daemon) | `boards.rs::history_records_what_happened_…` |
| 3 Add a session | Works (real daemon) | `live_flows/board_drawer.rs` |
| 4 Remove a session | Works (real daemon) in the client | `clients/live/tests/boards.rs` |
| 5 Open the session | Code, no test | `ui/src/board.rs:1196` |
| 6 Start a session | Works (real daemon) | `live_boards.rs::starting_a_session_…` |
| 7 Expand | Fakes only | `presentation.rs` |
| 8 Close | Fakes only | `drawer.rs::closing_the_drawer_closes_the_picker` |
| Old copy | Works (real daemon) | `live_flows/board_conflict.rs`, `board_revision.rs` |
| Rename, describe | Not built | `features/board/src/lib.rs:439` |
| Delete | Not built | same |
| Deleted elsewhere | Code, no test | none found |

## Gaps and likely bugs

- Not built: a title and description editor, and delete, in the app. A person who made a typo must use the terminal.
- Not built: the ownership line (BD-3). See case 7.
- Likely bug: step 6 is three calls from the app (start, add session, move), not one on the daemon (`OVERVIEW.md` "SE-3 runs in the client"). If the app quits between them, the session exists and the item still says To do with no session. Confirm by failing the second call in a fake daemon.
- Likely bug: the description shows as plain text. An agent's Markdown description (up to 20,000 characters) is not rendered (`ui/src/board.rs:1374`).
