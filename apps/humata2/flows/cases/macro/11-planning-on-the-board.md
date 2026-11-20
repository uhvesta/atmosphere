# Planning the week on the board: items are added, moved and linked, agents move only what they may, and an edit made from box-1 while this Mac is away waits or lands

Layer: macro. Platforms: macOS (this Mac, the host) and a linked box (box-1).

Status tags in this file: a test that runs the real `humatad2` (a daemon, or
its `mcp` process) is tagged **Works (real daemon)**. Tests that drive the app
headless on fake clients are **Fakes only**. Where a user action has no code
path or no test, the tag says so. A step or branch takes the tag of its main
claim; the Evidence column names any part that is only faked or not built.

## Starting state

- This Mac runs the host daemon. Channel "Payments" has two sessions: "Cache
  warmup" (running an agent) and "Ledger fix" (exited). Each has its own
  session board.
- The global board has a few items. The Payments board has three items in To
  do and one in In review (PAY-5), and one item already depends on another.
- The app is open on the home page. Nothing is selected, and no drawer is open.
- box-1 is linked. Its agent has a session on the Payments board.

## Steps

1. **Open the Board with ⌘3.**
   - **I do:** press ⌘3.
   - **I see:** the global board, with the columns Backlog, To do, In progress,
     In review and Done, each with a count (SPEC BD-1). Blocked and Won't do are
     shown only when they have items.
   - **State and events:** nothing is written. The app reads the global and the
     channel item lists.
   - **Status:** Fakes only for the key and the view. The key mapping is in
     `ui/src/keys.rs:105`, and `features/app/tests/every_action.rs` checks the key
     table. The global view is `features/board/tests/presentation.rs::global_board_includes_channel_items_and_drills_without_leaking_other_items`.
     The data behind it is Works (real daemon): `clients/live/tests/boards.rs::a_global_item_gets_the_global_key_and_scope`.

2. **Switch between the global, channel and session boards.**
   - **I do:** open the scope menu and pick "Payments", then "Cache warmup".
   - **I see:** the channel board shows only Payments items. The session's panel
     shows its channel's board with the session's own items first (BD-8; the
     panel, not the full board, is what the test covers).
   - **State and events:** the scope is saved. No item changes.
   - **Status:** Fakes only: `features/board/tests/scope.rs::each_channel_board_scope_shows_its_own_set`,
     `features/board/tests/scope.rs::the_global_board_shows_global_and_channel_items_but_no_session_items`.
     The daemon lists a session's board, forks included, as Works (real daemon):
     `daemon/humatad/tests/boards.rs::a_session_and_its_forks_lists_every_forks_board_and_nothing_else`
     and `clients/live/tests/boards.rs::a_session_board_item_is_listed_on_that_sessions_board`.

3. **Add items for the week.**
   - **I do:** press **New item** (or the **+** on a column), type a title, and
     press Enter.
   - **I see:** the item appears at the top of the chosen column with its key.
     A channel's items take the channel's key prefix; the global board numbers
     under GLB (`daemon/humatad/tests/boards.rs::the_global_board_numbers_under_glb`).
   - **State and events:** the daemon creates the item with the next key for its
     board. A repeated create with the same key makes one item.
   - **Status:** Works (real daemon) for the write and key: `clients/live/tests/boards.rs::a_channel_item_gets_the_channels_key_and_is_listed_in_to_do`,
     `daemon/humatad/tests/boards.rs::keys_count_up_per_channel_and_a_taken_prefix_lengthens`,
     `daemon/humatad/tests/boards.rs::a_repeated_create_with_the_same_key_makes_one_item`.
     The UI create is Fakes only (To-do UI, BOARDSIMPLE D830–D874): `features/todo/tests/todo.rs`, `desktop/tests/native/todo_asks.rs::a_channels_todo_tab_lists_only_that_channels_items`, and `desktop/tests/design/todo.rs` / `desktop/tests/design/todo_more.rs`.

4. **Open an item's drawer.**
   - **I do:** click an item card.
   - **I see:** the drawer with the id, Status, Channel, Sessions, Depends on,
     any pending request, the description, the history, and who owns the item
     (SPEC BD-3).
   - **State and events:** the history is read once, when the drawer opens. A
     late history read after the drawer closes is dropped.
   - **Status:** Fakes only: `features/board/tests/drawer.rs::opening_an_item_reads_its_history_and_late_reads_are_dropped`.

5. **Edit an item's title or description.**
   - **I do:** look for an editor for the title or description in the drawer.
   - **I see:** the description is shown read-only ("No description yet." when
     empty). There is no title or description editor.
   - **State and events:** none. The app has no action that changes a title or
     description (`features/board/src/lib.rs`, the `BoardAction` enum).
   - **Status:** Not built. D177 defers "a title/description editor" to the
     owner's next wave (DECISIONS.md D177; OVERVIEW.md:411). The daemon can
     update a title, and it is tested (`daemon/humatad/tests/boards.rs::a_change_made_against_an_old_revision_conflicts_only_on_fields_changed_since`),
     but the app cannot send that change.

6. **Drag an item to another column.**
   - **I do:** drag a card onto another column.
   - **I see:** the column highlights while the card is over it. On drop, the card
     moves to that column.
   - **State and events:** a Move is sent with no revision (D177, "drags send no
     revision"). The daemon moves the item after the column it names.
   - **Status:** Code, no test. The drag handlers are `ui/src/board.rs:433-438`
     (`on_drag`) and `ui/src/board.rs:494-499` (`drag_over` and `on_drop`). No test
     drives a drag. The daemon side is Works (real daemon):
     `clients/live/tests/boards.rs::a_moved_item_is_in_its_new_column_on_the_next_load`.

7. **Move an item from the drawer's Status menu.**
   - **I do:** open the item, open **Status**, and pick a column.
   - **I see:** the card moves to that column, and the drawer shows the new
     status.
   - **State and events:** the Move call goes to the daemon. The history records
     the move.
   - **Status:** Fakes only for the To-do state control: `features/todo/tests/todo.rs` and `desktop/tests/design/todo.rs` / `desktop/tests/design/todo_more.rs`. The daemon move is Works (real daemon):
     `clients/live/tests/boards.rs::a_moved_item_is_in_its_new_column_on_the_next_load`.

8. **Move an item from the keyboard.**
   - **I do:** select an item and try to move it with keys alone.
   - **I see:** no key moves the item. The Status menu is reached by a click.
   - **State and events:** none.
   - **Status:** Not built (no milestone). `ui/src/keys.rs` binds no key for moving
     a board item. Only ⌘3 and the board's own commands are bound.

9. **Link a dependency.**
   - **I do:** in the drawer's **Depends on** row, press **+**, type in the
     filter, pick an item with the arrows and Enter.
   - **I see:** the dependency appears as a chip. The picker leaves out the item
     itself, existing links, and anything that would close a loop (D177).
   - **State and events:** the dependency is added to the item.
   - **Status:** Fakes only for the picker: `features/board/tests/drawer.rs::the_dependency_picker_leaves_out_itself_existing_edges_and_loops`.
     The add and remove calls are Works (real daemon): `clients/live/tests/boards.rs::a_dependency_is_listed_refuses_a_loop_and_can_be_removed`.

10. **Add and remove a session on an item.**
    - **I do:** in the **Sessions** row press **+**, pick a session. Press **×** on a
      session line to remove it.
    - **I see:** the session appears on the item. Only the channel's sessions are
      offered (all sessions, grouped by channel, on a global item).
    - **State and events:** the item lists the session until it is taken off.
    - **Status:** Fakes only for the picker: `features/board/tests/drawer.rs::the_session_picker_offers_the_channels_other_sessions_and_stays_open`
      and `features/board/tests/drawer.rs::a_global_item_offers_every_session`.
      Works (real daemon) for the calls: `clients/live/tests/boards.rs::a_session_set_on_an_item_is_listed_until_it_is_taken_off`.

11. **Delete an item.**
    - **I do:** look for a delete action on an item I made.
    - **I see:** no delete action in the drawer.
    - **State and events:** none.
    - **Status:** Not built (no milestone). `features/board/src/lib.rs` has no delete
      action, and `ui/src/board.rs` has no delete button. The agent's delete is
      asked for in the Inbox ("Delete item", `ui/src/board.rs:1284`). SPEC BD-2 does
      not list user delete, so the owner should decide whether it is wanted.

12. **Read the history.**
    - **I do:** open an item after a move.
    - **I see:** the history lists the moves and other changes in order, with
      their kinds.
    - **State and events:** the history is read on open, not at load (D177).
    - **Status:** Works (real daemon): `clients/live/tests/boards.rs::an_items_history_has_its_move_and_an_unknown_item_is_not_found`
      and `daemon/humatad/tests/boards.rs::history_records_what_happened_in_order_with_its_kind`.

13. **An agent works its session board through the board tools.**
    - **I do:** the agent in "Cache warmup" calls `board_items_list`,
      `board_item_create`, `board_item_update`, `board_item_move` and
      `board_item_delete` through `humatad2 mcp`.
    - **I see:** each call returns the item or its list. The agent's own items
      change at once.
    - **State and events:** each write is checked against the rules for the board.
    - **Status:** Works (real daemon). `daemon/humatad/tests/board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it`
      runs the real `humatad2 mcp` process. The tool names match `daemon/mcp/src/tools.rs:61-73`.

14. **An agent asks to move an item it may not move.**
    - **I do:** the agent in "Cache warmup" moves an item it did not make, on the
      global board.
    - **I see:** the item does not move yet. An Inbox request appears: "<agent> asks to
      move this to <column>", with **Move to <column>** and **Keep in <column>**.
      The card shows Review.
    - **State and events:** the agent's call waits. Only the user sees the pending move.
    - **Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::an_agents_move_waits_and_applies_on_approval_and_only_the_user_sees_it_pending`.
      Approving in the To-do drawer (**Needs attention** › **Yes**) is Works (real daemon, fake agent): `desktop/tests/live_flows/todo_asks.rs::approving_an_agents_ask_from_the_todo_drawer_closes_the_item_and_returns_the_call`.
      Denying (**No**) keeps the item open: `desktop/tests/live_flows/todo_asks.rs::denying_an_agents_ask_from_the_todo_drawer_keeps_the_item_open_and_fails_the_call` (native: `desktop/tests/native/todo_asks.rs::approving_an_items_ask_answers_it_and_leaves_the_other_asks`).

15. **Start a session from an item.**
    - **I do:** open an item in To do and press **Start a session for this item**.
    - **I see:** a session starts for the item. The item lists the session and moves
      to In progress. An item in In review keeps its column.
    - **State and events:** the session is linked to the item (SE-3).
    - **Status:** Works (real daemon). `clients/live/tests/boards.rs::a_session_started_for_a_to_do_item_works_on_it_and_moves_it_in_progress`.
      The In review case is `clients/live/tests/boards.rs::a_session_started_for_an_in_review_item_leaves_its_column`,
      which asserts the item stays In review. Its name says the reverse (see Gaps).
      The app's flow is Fakes only: `features/app/tests/board_start.rs::starting_a_session_for_a_to_do_item_adds_it_and_starts_the_item`.

16. **Edit from box-1 while this Mac is away.**
    - **I do:** on box-1, add an item to the channel board, while the host is
      away (not linked).
    - **I see:** the item is accepted on box-1. When the host links back, it lands
      on the Payments board. (How box-1's screen shows the waiting write is not
      tested.)
    - **State and events:** box-1 journals the write. The journal entry is replayed
      when the link is back.
    - **Status:** Works (real daemon, fake box). `daemon/humatad/tests/away_board_journal.rs::row_3_a_channel_board_add_while_away_is_journaled`
      and `daemon/humatad/tests/away_approved_board.rs::an_edit_approved_on_the_box_while_the_host_is_away_is_made_when_the_link_is_back`.

17. **An edit that the rules ask about, from box-1 while away, waits in box-1's Inbox.**
    - **I do:** on box-1, edit an item another agent made, while the host is away.
    - **I see:** a request on box-1's Inbox, with the change on its card.
    - **State and events:** nothing is journaled until the request is approved on
      box-1. Approving journals the write with its approval, and denying journals nothing.
    - **Status:** Works (real daemon, fake box). `daemon/humatad/tests/away_board_journal.rs::an_edit_the_rules_ask_about_waits_in_the_machines_inbox_with_the_change_on_its_card`,
      `approving_the_request_on_the_machine_journals_the_write_with_its_approval_after_the_answer`,
      `denying_the_request_on_the_machine_journals_no_write`.

## Branches

### A concurrent edit at an older revision is refused

- **When:** a change is made against an item revision that is older than the
  current one, and a field the change sets was changed since.
- **I do:** an agent (or any caller that sends a revision) edits an item it read
  earlier.
- **I see:** a refusal that says the item is at a newer revision, and a fix that
  says to read the item again. The refusal is recorded as lost in the history.
- **Expected outcome:** refused, and only fields changed since are in conflict. A
  write with no revision never conflicts.
- **Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::a_change_made_against_an_old_revision_conflicts_only_on_fields_changed_since`,
  `daemon/humatad/tests/boards.rs::a_write_with_no_revision_never_conflicts`,
  `daemon/humatad/tests/board_tools.rs::a_stale_revision_conflicts_only_on_a_field_that_changed`.
  The app never sends a revision: `clients/live/src/boards.rs:266` and `:304` send
  `expected_revision: None`. So the user's app cannot hit this refusal. The app's
  re-read after a refusal is Fakes only: `features/board/tests/drawer.rs::a_refused_edit_shows_why_refreshes_and_keeps_the_picker`.

### An agent edits an item another agent made

- **When:** an agent edits or moves an item on the channel board that another
  agent created (the ChannelBoardEditOthers rule).
- **I do:** the agent makes the edit.
- **I see:** under the default rule, an Inbox request. Under an Allow rule, the
  edit goes through.
- **Expected outcome:** the item's owner decides, not the agent that asks.
- **Status:** Fakes only for the rule: `daemon/access/tests/refusals.rs::board_items_an_agent_made_are_its_to_edit`
  (a pure policy function, no daemon). The daemon-level switch is Works (real daemon):
  `daemon/humatad/tests/channel_desk.rs::the_board_switch_covers_the_channel_board_for_a_channel_target`,
  which tests the switch, not the owner check. An agent editing another agent's
  item through the daemon is **Code, no test**.

### An ask on a board edit while away waits in the machine's Inbox

- **When:** the rules ask about an edit made from box-1 while the host is away.
- **I do:** make the edit on box-1 and do not answer it.
- **I see:** the request waits on box-1's Inbox with the change on its card.
- **Expected outcome:** nothing is written until someone answers.
- **Status:** Works (real daemon, fake box). `daemon/humatad/tests/away_board_journal.rs::an_edit_the_rules_ask_about_waits_in_the_machines_inbox_with_the_change_on_its_card`.

### A write under "skip" while away is refused as the user being away

- **When:** the rule is Skip while away.
- **I do:** make the edit on box-1 while the host is away.
- **I see:** the write is refused, and the reason is that the user is away.
- **Expected outcome:** refused, nothing journaled.
- **Status:** Works (real daemon, fake box). `daemon/humatad/tests/away_board_journal.rs::under_skip_a_write_the_rules_ask_about_is_refused_as_the_user_being_away`.

### A view or read while away

- **When:** a board is read on box-1 while the host is away, and the read was
  never made while linked.
- **I do:** open the board on box-1.
- **I see:** the read is answered as host unreachable (the test's word; the on-screen
  copy is not tested). A read made while linked is answered from box-1's copy, marked
  with its age.
- **Expected outcome:** nothing invented.
- **Status:** Works (real daemon, fake box). `daemon/humatad/tests/away_board_journal.rs::a_read_never_made_while_the_host_was_linked_is_host_unreachable`
  and `row_2_a_read_made_while_the_host_was_linked_is_answered_while_it_is_away_marked_with_its_age`.

### Drag and drop against keyboard moves

- **When:** the user moves an item by drag, or tries to by keyboard.
- **I do:** drag a card to a column. Then try the same with the keyboard.
- **I see:** a drag works (see step 6). The keyboard has no move (see step 8).
- **Expected outcome:** the same Move is sent either way.
- **Status:** drag: Code, no test. Keyboard: Not built (no milestone).

### A move to Blocked or Won't do

- **When:** the user picks Blocked or Won't do in the Status menu.
- **I do:** open **Status** and pick Blocked (or Won't do).
- **I see:** a refusal: "the board has no Blocked column. Pick another column".
- **Expected outcome:** the item does not move, and the menu says why.
- **Status:** Works (real daemon) for the refusal message: the client maps these
  statuses to no column (`clients/live/src/boards.rs:38`) and `move_item` refuses them
  (`clients/live/src/boards.rs:251-258`). The menu offers them, so this is a Likely bug (see Gaps).

## Expected outcomes

- [ ] ⌘3 opens the global board with a count on each column.
- [ ] The channel board shows only its channel's items; a session board shows its channel's items and its own, first.
- [ ] A new item gets its board's next key and lands at the top of its column.
- [ ] The drawer shows the item's status, sessions, dependencies, history and owner.
- [ ] A move (drag or Status menu) lands the item in its new column, and the history records it.
- [ ] A dependency that would close a loop is refused and not offered in the picker.
- [ ] An agent's own items change at once; another agent's move waits for the user's answer.
- [ ] Approving the request moves the item and returns the agent's call; denying keeps the item where it was.
- [ ] A session started from an item lists it, and moves a To do item to In progress.
- [ ] An edit from box-1 while the host is away is journaled and lands when the link is back.
- [ ] An edit the rules ask about waits in box-1's Inbox, and nothing is journaled until it is answered.
- [ ] A stale revision is refused for the fields changed since.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. ⌘3 opens the board | Fakes only (key table and view); data Works (real daemon) | `ui/src/keys.rs:105`; `clients/live/tests/boards.rs::a_global_item_gets_the_global_key_and_scope` |
| 2. Global, channel and session boards | Fakes only (scope); Works (real daemon) for lists | `features/board/tests/scope.rs::each_channel_board_scope_shows_its_own_set`; `clients/live/tests/boards.rs::a_session_board_item_is_listed_on_that_sessions_board` |
| 3. Add items | Works (real daemon); UI Fakes only | `clients/live/tests/boards.rs::a_channel_item_gets_the_channels_key_and_is_listed_in_to_do`; `features/todo/tests/todo.rs`; `desktop/tests/native/todo_asks.rs`; `desktop/tests/design/todo.rs` & `todo_more.rs` |
| 4. Open the drawer | Fakes only | `features/board/tests/drawer.rs::opening_an_item_reads_its_history_and_late_reads_are_dropped` |
| 5. Edit title or description | Not built (D177, deferred to owner) | DECISIONS.md D177; OVERVIEW.md:411 |
| 6. Drag between columns | Code, no test (UI); Works (real daemon) for the move | `ui/src/board.rs:433-438`, `:494-499`; `clients/live/tests/boards.rs::a_moved_item_is_in_its_new_column_on_the_next_load` |
| 7. State / Status move | Fakes only (UI); Works (real daemon) for the move | `features/todo/tests/todo.rs`; `desktop/tests/design/todo.rs` & `todo_more.rs` |
| 8. Keyboard move | Not built (no milestone) | `ui/src/keys.rs` |
| 9. Link a dependency | Fakes only (picker); Works (real daemon) for calls | `features/board/tests/drawer.rs::the_dependency_picker_leaves_out_itself_existing_edges_and_loops`; `clients/live/tests/boards.rs::a_dependency_is_listed_refuses_a_loop_and_can_be_removed` |
| 10. Add and remove sessions | Fakes only (picker); Works (real daemon) for calls | `features/board/tests/drawer.rs::the_session_picker_offers_the_channels_other_sessions_and_stays_open`; `clients/live/tests/boards.rs::a_session_set_on_an_item_is_listed_until_it_is_taken_off` |
| 11. Delete an item (user) | Not built (no milestone) | `features/board/src/lib.rs` has no delete action |
| 12. History | Works (real daemon) | `clients/live/tests/boards.rs::an_items_history_has_its_move_and_an_unknown_item_is_not_found` |
| 13. Agent board tools | Works (real daemon) | `daemon/humatad/tests/board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it` |
| 14. Agent's move waits for the user | Works (real daemon) | `desktop/tests/live_flows/todo_asks.rs::approving_an_agents_ask_from_the_todo_drawer_closes_the_item_and_returns_the_call` |
| 15. Start a session from an item | Works (real daemon); app flow Fakes only | `clients/live/tests/boards.rs::a_session_started_for_a_to_do_item_works_on_it_and_moves_it_in_progress` |
| 16. Edit from box-1 while away | Works (real daemon, fake box) | `daemon/humatad/tests/away_board_journal.rs::row_3_a_channel_board_add_while_away_is_journaled` |
| 17. Ask on an edit while away | Works (real daemon, fake box) | `daemon/humatad/tests/away_board_journal.rs::an_edit_the_rules_ask_about_waits_in_the_machines_inbox_with_the_change_on_its_card` |
| Branch: concurrent edit, older revision | Works (real daemon); app's re-read Fakes only | `daemon/humatad/tests/boards.rs::a_change_made_against_an_old_revision_conflicts_only_on_fields_changed_since` |
| Branch: agent edits another's item | Fakes only (rule); Code, no test (daemon) | `daemon/access/tests/refusals.rs::board_items_an_agent_made_are_its_to_edit` |
| Branch: ask while away | Works (real daemon, fake box) | `daemon/humatad/tests/away_board_journal.rs::an_edit_the_rules_ask_about_waits_in_the_machines_inbox_with_the_change_on_its_card` |
| Branch: write under skip while away | Works (real daemon, fake box) | `daemon/humatad/tests/away_board_journal.rs::under_skip_a_write_the_rules_ask_about_is_refused_as_the_user_being_away` |
| Branch: read while away | Works (real daemon, fake box) | `daemon/humatad/tests/away_board_journal.rs::a_read_never_made_while_the_host_was_linked_is_host_unreachable` |
| Branch: drag vs keyboard | Drag: Code, no test; keyboard: Not built | see steps 6 and 8 |
| Branch: move to Blocked or Won't do | Works (real daemon) for the refusal; the menu offers it (Likely bug) | `clients/live/src/boards.rs:251-258` |

## Gaps and likely bugs

- **Likely bug:** `ui/src/board.rs:1131-1139` builds the drawer's Status menu from
  `ItemStatus::ALL` (`clients/src/model.rs:1093`), which includes Blocked and Won't do.
  The live client has no column for those two (`clients/live/src/boards.rs:38`), so
  choosing either returns "the board has no Blocked column". The menu should not offer
  them, or the daemon should gain the columns. Confirm by picking Blocked in the drawer
  against a real daemon. OVERVIEW.md:411 already says the client refuses those moves.
- **Not built (D177):** no title or description editor in the drawer. The daemon's
  UpdateItem can change a title, but the app has no action for it (`features/board/src/lib.rs`).
- **Not built (no milestone):** no keyboard move of a board item (`ui/src/keys.rs`).
- **Not built (no milestone):** no user delete of a board item. Only the agent's delete
  is asked for in the Inbox.
- **Code, no test:** drag and drop (`ui/src/board.rs:433-438`, `:494-499`). No test drives it.
- **Code, no test:** an agent editing another agent's item through the daemon. The rule
  has a pure policy test (`daemon/access/tests/refusals.rs::board_items_an_agent_made_are_its_to_edit`)
  but no daemon test.
- **Gap by design:** the app sends no revision on a move or a list edit (D177 "SL-7",
  `clients/live/src/boards.rs:266` and `:304`), so a user's write cannot conflict. The
  conflict path is reachable only by an agent or a CLI caller that sends a revision.
- **Note (BOARDSIMPLE, D830–D874):** The Board UI was simplified to **To-do**; the live ask and session-start tests are now in `desktop/tests/live_flows/todo_asks.rs`, with native/design tests in `desktop/tests/native/todo_asks.rs`, `features/todo/tests/todo.rs`, `desktop/tests/design/todo.rs`, and `desktop/tests/design/todo_more.rs`. See `flows/cases/board/README.md` for current To-do cases.
- **Brief correction:** `clients/live/tests/boards.rs` is a real-daemon test file with ten
  tests. The brief does not list it, but it covers create, move, dependencies, sessions,
  history and the session start.
- **Test name mismatch:** `clients/live/tests/boards.rs::a_session_started_for_an_in_review_item_leaves_its_column`
  asserts that the item stays In review. The name says it leaves its column. Rename it, or
  fix the assertion; the other test, `features/app/tests/board_start.rs::starting_a_session_for_an_item_in_review_keeps_its_column`,
  already says "keeps".
- **Gap in the tests:** the away tests run a fake box in the rig (`daemon/humatad/tests/common/remote_rig.rs`).
  They are real-daemon, but the box is not a real machine.
