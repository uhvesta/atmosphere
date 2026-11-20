# An agent asks for approval and waits for the answer

Layer: verify (To-do, agents). Platforms: macOS (this Mac).

In BOARDSIMPLE (D846), To-do / board asks stay out of the **Agent inbox** and appear under **Needs attention** on the item in **To-do** (and in the session's **To-do** panel). The agent's call still waits on `InboxService.WaitForAnswer` (or returns pending immediately with `--json` / `wait_seconds: 0`).

## Starting state

- A real daemon. `#payments` has `PAY-1` "Schema", made by the person.
- A session "Ledger" in `#payments`. I am its agent. `H` is the full path of `humata`.
- No pending requests exist. Agent access is at its defaults, so closing or moving `PAY-1` and writing on the Global board both ask.

## Reproduce headless

- The CLI door, real daemon, real `humata --json --wait 600 board move` answered Yes and No from the To-do drawer: `bazel test //apps/humata2/desktop:live_flows_test --test_filter=todo_asks`.
- The MCP door, real daemon: `bazel test //apps/humata2/daemon/humatad:board_tools_test --test_filter=creating_on_the_global_board_waits_for_the_user`.
- `todo_close` ask rules: `bazel test //apps/humata2/daemon/svc_board:svc_board_unit_test`.
- The daemon's held call: `bazel test //apps/humata2/daemon/humatad:boards_test --test_filter=an_agents`.
- MCP waiting rules on a fake daemon: `bazel test //apps/humata2/daemon/mcp:waiting_test`.
- By hand on the rig, no app: type `$H --json board move PAY-1 done > $D/o 2> $D/e; echo $? > $D/c` into Ledger's tab. As the person: `humata --json inbox`, `humata inbox approve <id>` (or `deny <id> --note "not yet"`), then `humata --json board --channel payments`.
- Not headless: nothing. The app's side is case 6.

## Steps

1. **I do:** MCP `todo_close` `{"key":"PAY-1","reason":"tests pass","evidence":["212 tests passed"]}` or `board_item_move` `{"id":"PAY-1","column":"done","reason":"tests pass","wait_seconds":0}`.
   **I see:** a pending reply with a `request_id`, at once. Not an error.
   **State and events:** `CAPABILITY_TODO_CLOSE` / `channel_board_edit_others`, Ask me. `InboxRequest` created (Pending) with the change (`Open` / `In progress` → `Closed` / `Done`) and my reason. The item does not close yet and appears under **Needs attention** in To-do.
   **Status:** Works (real daemon) for a pending write: `daemon/humatad/tests/board_tools.rs::creating_on_the_global_board_waits_for_the_user`; `svc_board_unit_test` for `todo_close`. Returning at once with `wait_seconds: 0`: Fakes only, `daemon/mcp/tests/waiting.rs::a_call_that_asks_with_no_wait_returns_pending_with_its_request_at_once`.

2. **I do:** MCP `inbox_wait` with that `request_id`. I do not repeat the write.
   **I see:** the call holds, with keepalives.
   **State and events:** nothing new. Humata types nothing into my terminal while I wait (TP-10).
   **Status:** Fakes only. `waiting.rs::inbox_wait_returns_what_the_original_call_would_have` and `::each_keepalive_is_a_progress_notification_for_the_clients_token`.

3. **I do:** (the person) click **Yes** (`todo-yes`) in `PAY-1`'s To-do drawer, or run `humata inbox approve <id>`.
   **I see:** my waiting call returns the updated item (the request leaves the pending list and `PAY-1` is `Closed`).
   **State and events:** `InboxService.AnswerRequest(Approve)` runs the held write.
   **Status:** Works (real daemon). `desktop/tests/live_flows/todo_asks.rs::approving_an_agents_ask_from_the_todo_drawer_closes_the_item_and_returns_the_call`; `daemon/humatad/tests/boards.rs::an_agents_move_waits_and_applies_on_approval_and_only_the_user_sees_it_pending`.

4. **I do:** the same through the CLI: `$H --json --wait 600 board move PAY-1 done`.
   **I see:** the command holds. After **Yes** in the To-do drawer, it prints the moved item and exits 0.
   **State and events:** as steps 1 to 3 in one call. The ask card has no reason line (the CLI does not send `--reason` yet).
   **Status:** Works (real daemon). `desktop/tests/live_flows/todo_asks.rs::approving_an_agents_ask_from_the_todo_drawer_closes_the_item_and_returns_the_call`.

5. **I do:** `$H --json board move PAY-1 done`, with no `--wait`.
   **I see:** exit 12 at once, with the request id in the JSON error. The ask stays in To-do under **Needs attention**.
   **State and events:** with `--json` the default wait is 0. In a terminal without `--json` the default is to wait until answered.
   **Status:** Code, no test for a board call (`cli/src/call.rs:170`).

6. **I do:** (the person) click **No** (`todo-no`) in the To-do drawer (or `humata inbox deny <id> --note "not yet"`).
   **I see:** MCP: an error of kind denied, with any note. CLI: exit 4. The request leaves the pending list and `PAY-1` stays open.
   **Status:** Works (real daemon). `desktop/tests/live_flows/todo_asks.rs::denying_an_agents_ask_from_the_todo_drawer_keeps_the_item_open_and_fails_the_call`. MCP side: Fakes only, `waiting.rs::a_denial_while_waiting_comes_back_as_the_calls_error`.

7. **I do:** MCP `board_item_create` `{"board":"global","title":"Tell finance","reason":"Q4 dependency"}`.
   **I see:** pending. After the person allows it, the item with a `GLB-` id.
   **State and events:** `global_board_write`.
   **Status:** Works (real daemon). `board_tools.rs::creating_on_the_global_board_waits_for_the_user`; `boards.rs::an_agents_write_on_the_global_board_waits_in_the_inbox_until_approved`.

## Branches

### I repeat the call instead of waiting

When I send the write again with a new request id. A second Inbox request is made. The server's greeting tells me not to.
**Status:** Fakes only. `waiting.rs::each_new_request_id_is_a_new_intent`. A client retry of the same request id reuses the request: `::a_client_retry_of_the_same_request_gets_the_same_inbox_request`.

### I stop while a call waits

When my process exits during the wait. The request is withdrawn. If the call had already returned pending, it stays and can still be approved.
**Status:** Fakes only. `waiting.rs::the_agent_exiting_while_a_call_waits_withdraws_its_request` and `::a_request_that_returned_pending_survives_the_agent_exiting`.

### The person ticks Always

When the person allows with **Always** (or `humata inbox approve <id> --always`). My next write on that board lands with no ask.
**Status:** Works for the rule: `daemon/access/tests/global_board.rs::an_always_allow_covers_later_writes_on_that_board_only`. End to end in To-do: Code, no test.

### The rule is Never

When the person set the rule to **Never**. The call fails at once as denied, naming the rule. No ask.
**Status:** Fakes / unit-tested. `daemon/mcp/tests/refusals.rs::a_never_rule_refusal_reaches_the_agent_as_denied_with_its_source_and_capability`, `svc_board_unit_test`.

### The daemon restarts while I wait

When `humatad2` restarts. My call ends and the request is gone. I must ask again.
**Status:** Code, no test. D155.

## Expected outcomes

- [ ] A write that needs the person returns pending with a request id, and changes nothing.
- [ ] Waiting on the request returns what the original call would have.
- [ ] Clicking **Yes** applies the change and returns success; clicking **No** changes nothing and returns denied (exit 4).
- [ ] The ask card in To-do shows the change and the agent's reason.
- [ ] The CLI exits 12 when it stops waiting and 4 when denied.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 MCP pending (`todo_close` / `board_item_move`) | Works (real daemon) / unit-tested; no-wait Fakes only | `board_tools.rs`, `svc_board_unit_test`, `waiting.rs` |
| 2 `inbox_wait` | Fakes only | `waiting.rs::inbox_wait_returns_…` |
| 3 Approved from To-do drawer | Works (real daemon) | `live_flows/todo_asks.rs`, `boards.rs` |
| 4 CLI `--wait` | Works (real daemon) | `live_flows/todo_asks.rs` |
| 5 CLI exit 12 | Code, no test | `cli/src/call.rs:170` |
| 6 Denied from To-do drawer | Works (real daemon) | `live_flows/todo_asks.rs`, `waiting.rs` |
| 7 Global create | Works (real daemon) | `board_tools.rs` |
| Repeat & exit while waiting | Fakes only | `waiting.rs` |
| Always | Works for the rule | `global_board.rs` |
| Never | Fakes / unit-tested | `mcp/tests/refusals.rs`, `svc_board_unit_test` |
| Restart | Code, no test | D155 |

## Gaps and likely bugs

- Not built: a `--reason` flag on `humata board` (`cli/src/commands/boards.rs`). The To-do ask card says who asked and for what, never why, when sent via CLI; MCP `todo_close` and `board_item_move` support `reason`.
- Not built: `humata inbox wait <id>` for an agent that got exit 12.
- Likely bug: asks die with a daemon restart (D155), and nothing tells the agent to ask again.
- `inbox_wait` against a real daemon for a To-do ask is not tested; only the mcp crate's fake daemon covers it.
