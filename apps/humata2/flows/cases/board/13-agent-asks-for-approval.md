# An agent asks for approval and waits for the answer

Layer: verify (Board, agents). Platforms: macOS (this Mac).

## Starting state

- A real daemon. `#payments` has PAY-1 "Schema", In progress, made by the person.
- A session "Ledger" in `#payments`. I am its agent. `H` is the full path of `humata`.
- The Inbox is empty. Agent access is at its defaults, so changing PAY-1 and writing on the Global board both ask.

## Reproduce headless

- The CLI door, real daemon, real `humata` in a tab: `bazel test //apps/humata2/desktop:live_boards_test --test_filter=the_items_drawer`.
- The MCP door, real daemon: `bazel test //apps/humata2/daemon/humatad:board_tools_test --test_filter=creating_on_the_global_board_waits_for_the_user`.
- The daemon's held call: `bazel test //apps/humata2/daemon/humatad:boards_test --test_filter=an_agents`.
- MCP waiting rules on a fake daemon: `bazel test //apps/humata2/daemon/mcp:waiting_test`.
- By hand on the rig, no app: type `$H --json board move PAY-1 done > $D/o 2> $D/e; echo $? > $D/c` into Ledger's tab. As the person: `humata --json inbox`, `humata inbox approve <id>` (or `deny <id> --note "not yet"`), then `humata --json board --channel payments`.
- Not headless: nothing. The app's side is case 6.

## Steps

1. **I do:** MCP `board_item_move` `{"id":"PAY-1","column":"done","reason":"tests pass","wait_seconds":0}`.
   **I see:** a pending reply with a `request_id`, at once. Not an error.
   **State and events:** `channel_board_edit_others`, Ask me. InboxRequest created (Pending) with the change (status In progress → Done) and my reason. The item does not move.
   **Status:** Works (real daemon) for a pending board write: `daemon/humatad/tests/board_tools.rs::creating_on_the_global_board_waits_for_the_user`. Returning at once with `wait_seconds: 0`: Fakes only, `daemon/mcp/tests/waiting.rs::a_call_that_asks_with_no_wait_returns_pending_with_its_request_at_once`.

2. **I do:** MCP `inbox_wait` with that `request_id`. I do not call `board_item_move` again.
   **I see:** the call holds, with keepalives.
   **State and events:** nothing new. Humata types nothing into my terminal while I wait (TP-10).
   **Status:** Fakes only. `waiting.rs::inbox_wait_returns_what_the_original_call_would_have` and `::each_keepalive_is_a_progress_notification_for_the_clients_token`.

3. **I do:** (the person) approve it, from the item's drawer, the Inbox card or `humata inbox approve <id>`.
   **I see:** my `inbox_wait` returns the moved item, exactly what the move would have returned.
   **State and events:** the daemon runs the held move. InboxRequest updated (Approved); BoardItem updated (Done).
   **Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::an_agents_move_waits_and_applies_on_approval_and_only_the_user_sees_it_pending`; `board_tools.rs` (the Global create lands after approval).

4. **I do:** the same through the CLI: `$H --json --wait 600 board move PAY-1 done`.
   **I see:** the command holds. After the approval it prints the moved item and exits 0.
   **State and events:** as steps 1 to 3 in one call. The Inbox card has no reason line.
   **Status:** Works (real daemon). `desktop/tests/live_boards.rs::approving_an_agents_move_from_the_items_drawer_moves_it_and_returns_the_call`.

5. **I do:** `$H --json board move PAY-1 done`, with no `--wait`.
   **I see:** exit 12 at once, with the request id in the JSON error. The request stays in the Inbox.
   **State and events:** with `--json` the default wait is 0. In a terminal without `--json` the default is to wait until answered.
   **Status:** Code, no test for a board call. `cli/src/call.rs:134`. I found no CLI test that asserts exit 12 on `board`.

6. **I do:** (the person) deny it with a note.
   **I see:** MCP: an error of kind denied, with the note. CLI: exit 4. PAY-1 stays In progress.
   **Status:** Works (real daemon). `live_boards.rs::denying_an_agents_move_from_the_items_drawer_keeps_it_in_progress`. MCP side: Fakes only, `waiting.rs::a_denial_while_waiting_comes_back_as_the_calls_error`.

7. **I do:** MCP `board_item_create` `{"board":"global","title":"Tell finance","reason":"Q4 dependency"}`.
   **I see:** pending. After the person allows it, the item with a `GLB-` id.
   **State and events:** `global_board_write`. The Inbox card shows the new item's board, title, column and description. No board shows it until it exists.
   **Status:** Works (real daemon). `board_tools.rs::creating_on_the_global_board_waits_for_the_user`; `boards.rs::an_agents_write_on_the_global_board_waits_in_the_inbox_until_approved`.

## Branches

### I repeat the call instead of waiting

When I send `board_item_move` again with a new request id. A second Inbox request is made. The server's greeting tells me not to.
**Status:** Fakes only. `waiting.rs::each_new_request_id_is_a_new_intent`. A client retry of the same request id reuses the request: `::a_client_retry_of_the_same_request_gets_the_same_inbox_request`.

### I stop while a call waits

When my process exits during the wait. The request is withdrawn. If the call had already returned pending, it stays and can still be approved.
**Status:** Fakes only. `waiting.rs::the_agent_exiting_while_a_call_waits_withdraws_its_request` and `::a_request_that_returned_pending_survives_the_agent_exiting`.

### The person ticks Always

When the person allows with **Always** (or `humata inbox approve <id> --always`). My next write on that board lands with no ask.
**Status:** Works for the rule: `daemon/access/tests/global_board.rs::an_always_allow_covers_later_writes_on_that_board_only`. End to end on a board: Code, no test.

### The rule is Never

When the person set the rule to Never. The call fails at once as denied, naming the rule. No Inbox card.
**Status:** Fakes only. `daemon/mcp/tests/refusals.rs::a_never_rule_refusal_reaches_the_agent_as_denied_with_its_source_and_capability`.

### The daemon restarts while I wait

When humatad2 restarts. My call ends and the request is gone. I must ask again.
**Status:** Code, no test. D155.

## Expected outcomes

- [ ] A write that needs the person returns pending with a request id, and changes nothing.
- [ ] Waiting on the request returns what the original call would have.
- [ ] An approval applies the change; a denial returns the note and changes nothing.
- [ ] The Inbox card shows the change and the agent's reason.
- [ ] The CLI exits 12 when it stops waiting and 4 when denied.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 MCP pending | Works (real daemon); no-wait Fakes only | `board_tools.rs`, `waiting.rs` |
| 2 `inbox_wait` | Fakes only | `waiting.rs::inbox_wait_returns_…` |
| 3 Approved | Works (real daemon) | `boards.rs::an_agents_move_waits_…` |
| 4 CLI `--wait` | Works (real daemon) | `live_boards.rs::approving_…` |
| 5 CLI exit 12 | Code, no test | `cli/src/call.rs:134` |
| 6 Denied | Works (real daemon) | `live_boards.rs::denying_…` |
| 7 Global create | Works (real daemon) | `board_tools.rs` |
| Repeat | Fakes only | `waiting.rs` |
| Exit while waiting | Fakes only | `waiting.rs` |
| Always | Works for the rule | `global_board.rs` |
| Never | Fakes only | `mcp/tests/refusals.rs` |
| Restart | Code, no test | D155 |

## Gaps and likely bugs

- Not built: a reason from the CLI. `humata board add`, `edit`, `move` and `rm` send `reason: String::new()` (`cli/src/commands/boards.rs:84`, `:113`, `:128`, `:137`). The person's card says who asks and for what, never why. MCP's `reason` works.
- Not built: `humata inbox wait <id>` for an agent that got exit 12. The CLI can wait only inside the original call (`--wait`). An agent that stopped waiting must poll the board, or try `humata inbox show <id>` (whether an agent may read its own request that way: Code, no test). `cli/src/args.rs:644` has no wait.
- Likely bug: asks die with a daemon restart (D155), and nothing tells the agent to ask again.
- `inbox_wait` against a real daemon for a board ask is not tested; only the mcp crate's fake daemon covers it.
