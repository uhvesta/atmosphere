# The To-do drawer edits an item and starts a session on it

Layer: verify (To-do, the person). Platforms: macOS.

Clicking any row on the To-do page opens its right-hand drawer (`ui/workspace/src/todo.rs`, frames b1-2, b4-2, b7-1, b7-3). On a session page, the To-do panel (`ui/workspace/src/todo_panel.rs`, frame 191 / native frames 07 and 12) opens the narrow drawer with a `‹ To-do` back link and pinned `History · Pin · Hide until…` footer.

## Starting state

- `#payments` has `PAY-1` "Schema" (`Open`, no session), `PAY-2` "Write path" (`In progress`, session "Ledger") and `PAY-3` "Cut over" (`Open`).
- Two sessions run in `#payments`: "Ledger" and "Refunds".
- The app is open on **To-do**.

## Reproduce headless

- Starting a session from the To-do drawer against a real daemon: `bazel test //apps/humata2/desktop:live_flows_test --test_filter=starting_a_session_from_the_todo_drawer`.
- Clicking the drawer's Start a session button on fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=starting_a_session_from_the_todo_drawer`.
- Starting a session from an item and Do next in the app reducer: `bazel test //apps/humata2/features/app:board_start_test //apps/humata2/features/app:todo_route_test`.
- Drawer edits (title, due date, description, steps, tags, state, pin, hide until, stale revision rollback) on fakes: `bazel test //apps/humata2/features/todo:todo_test`.
- Item history on a real daemon: `bazel test //apps/humata2/daemon/humatad:boards_test --test_filter=history_records_what_happened_in_order_with_its_kind`.
- Not headless: nothing.

## Steps

1. **I do:** click a To-do row (`todo-row <key>`).
   **I see:** the drawer: key and creation date, close `×`, editable title, any pending ask card on top (or **Start the review** for a PR review item), **State** (`Open` / `In progress` / `Closed`), **Channel**, **Sessions**, **Due** (inline date + `Today` / `Tomorrow` / `Next week` / `Clear`), **Effort** (`S` / `M` / `L`) and **Impact** (`Low` / `Medium` / `High`), **Tags** (`+ Add` and click-to-remove), **Waits on** (when non-empty), **STEPS** (tickable checklist + `Add a step…`), editable **DESCRIPTION**, and footer (**Start a session on this** / **Start another session on this**, **History**, **Pin** / **Unpin**, **Hide until…** / **Stop hiding**).
   **State and events:** `TodoAction::Open(key)`.
   **Status:** Works (real daemon) for opening the drawer and clicking its buttons: `desktop/tests/live_flows/todo_asks.rs`. Design fidelity: frames b1-2, b4-2, b7-1, b7-3 in `desktop/tests/design/todo.rs` and `todo_more.rs`.

2. **I do:** click the title, description, `+ Add` tag, or `Add a step…`, type text, and press Enter.
   **I see:** the field saves optimistically and reconciles with `TodoService.UpdateTodo`. Clicking a step toggles it done; clicking a tag removes it.
   **State and events:** `StartEdit` → `EditChanged` → `CommitEdit` → `TodoClient::update`.
   **Status:** Fakes only in the app: `features/todo/tests/todo.rs::a_description_is_edited_and_saved`. Daemon persistence: `daemon/svc_board:svc_board_unit_test`.

3. **I do:** click the due date field (`todo-due-edit`), type a relative or calendar date (`tomorrow`, `fri`, `nov 28`, `next week`, or `2026-12-01`), and press Enter; or click **Today**, **Tomorrow**, **Next week**, or **Clear**.
   **I see:** the parsed date saves and updates the row's due chip and rank; unreadable input keeps the field open and shows a red error explaining accepted formats.
   **State and events:** `parse_day` (`humata2_todo`) → `TodoClient::update`.
   **Status:** Fakes only in the app: `features/todo/tests/todo.rs::typing_a_due_date_parses_it_and_bad_input_records_an_error`, `todo/tests/quick_add.rs::parses_relative_month_day_and_iso_dates_and_rejects_unreadable_input`.

4. **I do:** click **Start a session on this** (`todo-start-session`) in `PAY-1`'s drawer, then click **Start session**.
   **I see:** the prefilled **New session** dialog opens with title "Schema", channel `#payments`, an online machine, and `for PAY-1`. Starting it opens the new session's page, attaches the session to `PAY-1`, and moves `PAY-1` to **In progress**. Suggested items and PR review items do not show this button (PR reviews show **Start the review** instead).
   **State and events:** `TodoAction::StartSession(key)` → `AppAction::StartNewForItem(ItemId)` → `NewSessionAction::Create` → `daemon.create_session` + `start_item_work`.
   **Status:** Works (real daemon). `desktop/tests/live_flows/todo_asks.rs::starting_a_session_from_the_todo_drawer_opens_it_and_moves_the_item_in_progress`; native click: `desktop/tests/native/todo_asks.rs::starting_a_session_from_the_todo_drawer_opens_the_prefilled_new_session_dialog`; reducer: `features/app/tests/board_start.rs`.

5. **I do:** click **Pin** (or **Unpin**) and **Hide until…** (or **Stop hiding**).
   **I see:** pinning moves the item to the top of **Next**; hiding moves a non-overdue item into the folded **Hidden until a date** group until next Monday (or brings it back on **Stop hiding**).
   **State and events:** `TodoAction::Pin` / `Unpin` / `HideUntil`.
   **Status:** Fakes only. `features/todo/tests/todo.rs::pin_goes_after_the_pins_and_move_pin_puts_it_on_top`, `todo/tests/rank.rs::hidden_comes_back_on_its_day`, `ui/workspace/src/todo.rs::tests::hiding_picks_a_later_day`.

6. **I do:** click **×** (`todo-close`) or press Escape.
   **I see:** the drawer closes and any in-progress field edit is cancelled.
   **State and events:** `TodoAction::Close`.
   **Status:** Fakes only (`features/todo/src/lib.rs`, `features/app/src/overlay.rs`).

## Branches

### An agent changed the item while the drawer was open

When I edit an item whose revision advanced on another screen or by an agent. The write is refused with `DaemonError::Conflict`, the optimistic change rolls back to the prior entry, and the error explains to reload and try again.
**Status:** Fakes only in the app: `features/todo/tests/todo.rs::a_stale_revision_rolls_back_and_shows_an_error`. Daemon enforcement: Works (real daemon), `daemon/svc_board:svc_board_unit_test` and `daemon/humatad/tests/boards.rs`.

### Clicking History in the drawer footer

When I click **History** (`todo-history`). Nothing opens yet: `AppAction::Todo(TodoAction::ShowHistory(_))` is currently a no-op (`features/app/src/lib.rs:1189`). On the daemon and CLI, `GetItemHistory` / `humata board history <key>` works.
**Status:** Not built in the To-do UI. Daemon: Works (real daemon), `daemon/humatad/tests/boards.rs::history_records_what_happened_in_order_with_its_kind`; client: `clients/live_daemon/tests/boards.rs::an_items_history_has_its_move_and_an_unknown_item_is_not_found`.

### Adding or removing sessions and dependencies in the drawer

When I want to attach an existing session or add/remove a `Waits on` edge from the To-do drawer. The drawer displays session chips and `Waits on` pills read-only; no interactive picker is wired in `ui/workspace/src/todo.rs` yet. CLI (`humata board edit --assign/--unassign/--depends-on/--drop-depends-on`) and MCP (`board_item_update`) work.
**Status:** Not built in the To-do UI. Client/daemon: Works (real daemon), `clients/live_daemon/tests/boards.rs`.

### I want to delete the item

When I look for a delete control in the drawer. There is none; closing is the UI action (`humata board rm <key>` deletes on the daemon and keeps a tombstone in history).
**Status:** Not built in the UI. Daemon: Works (real daemon), `daemon/humatad/tests/boards.rs::deleting_an_item_drops_the_edges_to_it`.

### The item is deleted elsewhere while open

When an agent deletes the item while its drawer is open. The next `Loaded` or `Updated` watch event clears `state.open`, `state.editing` and `state.due_error`.
**Status:** Code, no test (`features/todo/src/lib.rs:565`).

## Expected outcomes

- [ ] The drawer edits title, state, due date, effort, impact, tags, steps, description, pin and hide-until.
- [ ] Starting a session from the drawer opens the prefilled New session sheet, creates the session, attaches it to the item, and moves an open item to In progress.
- [ ] A write from a stale revision rolls back and explains why.
- [ ] Suggested and pull-request review items do not show the Start a session button.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Open drawer | Works (real daemon) | `live_flows/todo_asks.rs`, `native/todo_asks.rs` |
| 2 Edit title, description, tags, steps | Fakes only in app; daemon unit-tested | `features/todo/tests/todo.rs`, `svc_board_unit_test` |
| 3 Due date typing & shortcuts | Fakes only | `features/todo/tests/todo.rs`, `todo/tests/quick_add.rs` |
| 4 Start a session on this | Works (real daemon) | `live_flows/todo_asks.rs`, `native/todo_asks.rs`, `features/app/tests/board_start.rs` |
| 5 Pin & Hide until… | Fakes only | `features/todo/tests/todo.rs`, `todo/tests/rank.rs` |
| 6 Close drawer | Code, no UI test | `features/todo/src/lib.rs` |
| Stale revision | Fakes only in app; daemon Works | `features/todo/tests/todo.rs`, `svc_board_unit_test` |
| History in drawer | Not built in UI; daemon Works | `features/app/src/lib.rs:1189`, `boards.rs` |
| Session & Waits-on pickers | Not built in UI; daemon Works | `clients/live_daemon/tests/boards.rs` |
| Delete in UI | Not built in UI; daemon Works | `boards.rs::deleting_an_item_drops_the_edges_to_it` |
| Deleted elsewhere | Code, no test | `features/todo/src/lib.rs:565` |

## Gaps and likely bugs

- Not built in the To-do drawer: opening the item's history (`ShowHistory` is a no-op), interactive pickers to attach/detach sessions or `Waits on` dependencies, and a UI delete control.
- Likely bug: step 4 (`start_item_work` in `clients/live_daemon/src/boards.rs`) runs in the client after `create_session`, not atomically on the daemon. If the app quits between them, the session exists and the item stays Open with no session.
- Description renders as plain text (Markdown is not rendered).
