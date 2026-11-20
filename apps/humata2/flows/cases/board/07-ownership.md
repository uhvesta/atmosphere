# Who made an item and To-do rules decide who may change it

Layer: verify (To-do, the person and the rules). Platforms: macOS.

In addition to the board ownership rules (own items vs others' items, channel vs Global), BOARDSIMPLE added two To-do access switches (D839, D840): `CAPABILITY_TODO_SUGGEST` (default **Allow**, creating a `Suggested` item until the person says Yes) and `CAPABILITY_TODO_CLOSE` (default **Ask**, raising a Needs attention ask whenever an agent tries to close a To-do item). The person's own edits and closes never ask.

## Starting state

- `#payments` has two sessions with agents, "Ledger" and "Refunds". `#infra` has one, "Deploy".
- In `#payments`: `PAY-1`, made by me; `PAY-2`, made by Ledger's agent; `PAY-3`, made by Refunds' agent.
- An item with no channel: `GLB-1`, made by me.
- Agent access for `#payments` is at its defaults.

## Reproduce headless

- The rules alone: `bazel test //apps/humata2/daemon/access:refusals_test --test_filter=board_items_an_agent_made_are_its_to_edit` and `bazel test //apps/humata2/daemon/access:global_board_test`.
- To-do access switches (`todo_suggest` and `todo_close`): `bazel test //apps/humata2/daemon/svc_board:svc_board_unit_test`.
- Real daemon (agent moving the person's item, answered from the To-do drawer): `bazel test //apps/humata2/desktop:live_flows_test --test_filter=todo_asks`.
- Real daemon (BoardService + MCP): `bazel test //apps/humata2/daemon/humatad:boards_test --test_filter=an_agents` and `bazel test //apps/humata2/daemon/humatad:board_tools_test`.
- By hand on the rig: start two `zsh` sessions in `payments`. In Ledger's tab, `humata --json board add "Mine"`, then in Refunds' tab `humata --json board move <that id> done`. The second exits 12 with a request id. Keep both outputs, and `humata --json board history <id>`.
- Not headless: nothing. The Settings › Agent access rows are in `X-E-agent-switches.md`.

## Steps

1. **I do:** open `PAY-2`'s drawer in To-do and look for who made it.
   **I see:** (expected, BD-3) who made the item and whether that agent may change it freely.
   **I see (today):** the drawer header shows the key and creation date ("PAY-2 · made by you, …"), not the per-session ownership line. History's first entry on the daemon names the maker.
   **State and events:** `BoardItem.created_by` is set by the daemon and can't be changed.
   **Status:** Not built in the To-do drawer (`ui/workspace/src/todo.rs:1469` hardcodes "made by you"). Daemon history: Works (real daemon), `daemon/humatad/tests/boards.rs::history_records_what_happened_in_order_with_its_kind`.

2. **I do:** have Ledger's agent move `PAY-2`, which it made, to In review (or edit its non-close fields).
   **I see:** the change lands. No ask.
   **State and events:** rule "edit, move or delete channel items it created": Allow. `BoardItem` updated.
   **Status:** Works for the rule (unit level, no daemon): `daemon/access/tests/refusals.rs::board_items_an_agent_made_are_its_to_edit`. On a real daemon with a channel item: Code, no test. `board_tools.rs` uses the session board.

3. **I do:** have Ledger's agent close an item through `TodoService.UpdateTodo` (`todo_close`).
   **I see:** under default `CAPABILITY_TODO_CLOSE` (**Ask**), it raises a Needs attention ask on the item; set to **Allow**, it closes directly; set to **Never**, it is refused. The person's own close is unaffected.
   **State and events:** `BoardSvc::gate_todo` checks `Capability::TodoClose` on top of the board rules (`daemon/svc_board/src/todo.rs`).
   **Status:** Unit-tested on `svc_board`: `daemon/svc_board:svc_board_unit_test` (4 tests for Allow / Ask / Never / owner).

4. **I do:** have Ledger's agent move `PAY-1`, which I made, to `done`.
   **I see:** a Needs attention ask on `PAY-1` in To-do (`Close Schema?` with **Yes** / **No**).
   **Status:** Works (real daemon). `desktop/tests/live_flows/todo_asks.rs::approving_an_agents_ask_from_the_todo_drawer_closes_the_item_and_returns_the_call`.

5. **I do:** have Ledger's agent add an item to the Global board, then edit `GLB-1`.
   **I see:** an ask each time. Nothing is on the Global board until I allow it.
   **State and events:** capability `global_board_write`, Ask me, whoever made the item.
   **Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::an_agents_write_on_the_global_board_waits_in_the_inbox_until_approved`; `board_tools.rs::creating_on_the_global_board_waits_for_the_user`.

6. **I do:** tick **Always** on that ask and allow it.
   **I see:** the agent's next write on the Global board lands with no ask. A write on `#infra` still asks.
   **State and events:** a grant for that board only.
   **Status:** Works for the rule: `daemon/access/tests/global_board.rs::an_always_allow_covers_later_writes_on_that_board_only`. End-to-end in the app: Code, no test.

7. **I do:** have Deploy's agent (in `#infra`) list `#payments`, then add to it.
   **I see:** the list comes back. The add asks me.
   **State and events:** reading other channels' items is Allow; changing them is Ask me (D180).
   **Status:** Works for the rule: `global_board.rs::reading_the_global_board_or_another_channels_is_allowed` and `::adding_or_editing_there_asks_and_offers_always_on_that_board`.

8. **I do:** as myself, close or edit any item from To-do or from `humata board move`.
   **I see:** both land at once. I never ask anyone.
   **Status:** Works (real daemon). `clients/live_daemon/tests/todo.rs::the_daemon_keeps_a_to_do_the_person_adds_and_closes`; `clients/live_daemon/tests/boards.rs::a_moved_item_is_in_its_new_column_on_the_next_load`.

## Branches

### An agent's own session board

When an agent adds, edits, moves and deletes on its own session board. All allowed, no ask.
**Status:** Works (real daemon). `board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it`.

### A rule set to Never

When I set "Close or edit an item it didn't make" or `todo_suggest` to **Never**. The agent's call is refused at once as denied, with the rule named, and no ask.
**Status:** Fakes / unit tests. `daemon/mcp/tests/refusals.rs::a_never_rule_refusal_reaches_the_agent_as_denied_with_its_source_and_capability` and `daemon/svc_board:svc_board_unit_test`.

### The maker's session is archived

When Ledger is archived. `PAY-2` stays in the list. No agent owns it any more, so every agent must ask.
**Status:** Code, no test. `daemon/access/src/policy.rs:766` compares `created_by` with the caller's session id only.

### A fork of the maker

When Ledger's fork edits `PAY-2`. The fork is another session, so it must ask.
**Status:** Code, no test. Same line. Whether a fork should inherit is open (BD-10).

## Expected outcomes

- [ ] The drawer says who actually made the item.
- [ ] An agent changes channel items it made without asking (except closing under `CAPABILITY_TODO_CLOSE = Ask`).
- [ ] An agent asks before changing an item made by me or by another agent.
- [ ] Any agent write on the Global board or another channel asks, and **Always** covers that board only.
- [ ] I can change or close every item without asking.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Ownership line in drawer | Not built | `ui/workspace/src/todo.rs:1469` |
| 2 Own channel item | Works for the rule; daemon Code, no test | `refusals.rs::board_items_an_agent_made_…` |
| 3 `todo_close` / `todo_suggest` switches | Unit-tested on `svc_board` | `daemon/svc_board:svc_board_unit_test` |
| 4 My item | Works (real daemon) | `live_flows/todo_asks.rs` |
| 5 Global board | Works (real daemon) | `boards.rs::an_agents_write_on_the_global_board_…` |
| 6 Always | Works for the rule | `global_board.rs::an_always_allow_…` |
| 7 Another channel | Works for the rule | `global_board.rs` |
| 8 The person | Works (real daemon) | `clients/live_daemon/tests/todo.rs`, `boards.rs` |
| Own session board | Works (real daemon) | `board_tools.rs` |
| Never | Fakes / unit-tested | `mcp/tests/refusals.rs`, `svc_board_unit_test` |
| Maker archived | Code, no test | `policy.rs:766` |
| Fork of the maker | Code, no test | `policy.rs:766` |

## Gaps and likely bugs

- Not built: the drawer's real maker/ownership line (BD-3); `ui/workspace/src/todo.rs:1469` currently renders `"made by you"` for every item.
- Not built: handing ownership of an item to another session.
- No real-daemon test has one agent touch a channel item made by another agent, or its own channel item.
- Likely bug: an item whose maker is archived can never again be changed without an ask (`daemon/access/src/policy.rs:766`).
