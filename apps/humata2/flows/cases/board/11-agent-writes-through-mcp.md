# An agent creates, edits, moves, closes and deletes through MCP

Layer: verify (To-do, agents). Platforms: macOS (this Mac).

BOARDSIMPLE added three To-do MCP tools (`todo_next`, `todo_add`, `todo_close`, `daemon/svc_mcp/src/mcp/todo.rs`) backed by `TodoService`, alongside the seven `board_item_*` tools backed by `BoardService` over the same items.

## Starting state

- A real daemon. `#payments` has two sessions, "Ledger" and "Refunds". The list is empty.
- I am Ledger's agent, with Humata's MCP server connected and bound to my session. Today that means the server was started by hand inside my tab (case 9).
- Agent access is at its defaults (`CAPABILITY_TODO_SUGGEST = Allow`, `CAPABILITY_TODO_CLOSE = Ask`).

## Reproduce headless

- The `board_*` tools on a real daemon: `bazel test //apps/humata2/daemon/humatad:board_tools_test`. It starts `humatad2 mcp` with a session token and speaks JSON-RPC to it.
- The `todo_*` MCP dispatch and formatting: `bazel test //apps/humata2/daemon/svc_mcp:svc_mcp_unit_test` and `bazel test //apps/humata2/daemon/svc_board:svc_board_unit_test`.
- Schemas and limits on fakes: `bazel test //apps/humata2/daemon/mcp:schemas_test`. Tiers: `bazel test //apps/humata2/daemon/mcp:tiers_test`.
- By hand on the rig: inside Ledger's tab, pipe `initialize` and then one `tools/call` line per step into `humatad2_admin mcp --socket "$HUMATA_SOCKET"`. Save the replies. As the person, `humata --json board --session Ledger` after each step.
- Not headless: a real agent CLI choosing these tools. No agent is started with the server (case 9).

## Steps

1. **I do:** MCP `todo_add` `{"title":"Idempotency keys for refund retries","channel":"payments","effort":"s","impact":"high","tags":["refunds"],"reason":"timeout retries can double-charge"}`.
   **I see:** the new item returned with `"suggested": true` and a note that it waits for the owner's Yes before joining the ranked list. On the To-do page it shows in **Needs attention** marked `Suggested`.
   **State and events:** `TodoService.CreateTodo` checks `CAPABILITY_TODO_SUGGEST` (default Allow) and sets `suggested = true` for agent callers.
   **Status:** Unit-tested in `daemon/svc_board:svc_board_unit_test` and `daemon/svc_mcp:svc_mcp_unit_test` (`daemon/svc_mcp/src/mcp/todo.rs`).

2. **I do:** MCP `todo_close` `{"key":"PAY-1","reason":"tests pass","evidence":["212 tests passed"]}`.
   **I see:** under default `CAPABILITY_TODO_CLOSE` (**Ask**), a pending reply with a `request_id`; `PAY-1` moves into **Needs attention** in To-do with the ask card. Once the person clicks **Yes**, `PAY-1` closes.
   **State and events:** `TodoService.UpdateTodo` with `state: Closed` → `InboxService` pending board ask.
   **Status:** Unit-tested in `daemon/svc_board:svc_board_unit_test` (default Ask waits, Allow closes, Never refuses) and `daemon/svc_mcp:svc_mcp_unit_test`.

3. **I do:** `board_item_create` `{"board":"session","title":"Refund path","column":"to_do"}` and `{"board":"session","title":"Webhooks"}`.
   **I see:** `PAY-1` (`column: "to_do"`, `created_by` my session) and a second item in Backlog.
   **State and events:** own session board, Allow. `BoardItem` created.
   **Status:** Works (real daemon). `daemon/humatad/tests/board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it`; key in `boards.rs::a_session_item_uses_its_channels_prefix`.

4. **I do:** `board_item_update` on the first with `description`, `sessions_add: [Ledger, Refunds]` and `depends_on_add: [<second>]`, then `sessions_remove: [Refunds]`.
   **I see:** the item with the description, sessions updated in order, and one dependency with `done: false`.
   **State and events:** `BoardItem` updated. History gains changed, sessions and dependencies entries.
   **Status:** Works (real daemon). Same `board_tools.rs` test.

5. **I do:** `board_item_move` `{"id":<second>,"column":"done"}`.
   **I see:** `column: "done"` and a `done_at`. A `board_item_get` of the first now shows its dependency `done: true`.
   **Status:** Works (real daemon). Same `board_tools.rs` test.

6. **I do:** `board_item_create` `{"board":"channel","title":"Backfill","depends_on":[<first>]}` and `board_item_move` on it to `in_progress`.
   **I see:** a channel item, no ask; moving what I made needs no ask.
   **State and events:** `channel_board_add` and `channel_board_edit_own`, both Allow.
   **Status:** Works for the rules: `daemon/access/tests/refusals.rs::board_items_an_agent_made_are_its_to_edit`. On a real daemon through MCP: Code, no test (`board_tools.rs` uses `session` and `global`).

7. **I do:** `board_item_delete` on the first item.
   **I see:** `{deleted: true}`. A get answers `not_found`. The person's list no longer has it; its history keeps a tombstone.
   **Status:** Works (real daemon). Same `board_tools.rs` test.

## Branches

### A loop

When I add a dependency that closes a loop. The call fails with `dependency_loop`, naming the cycle. Nothing changes.
**Status:** Works (real daemon). `board_tools.rs::a_dependency_that_would_close_a_loop_is_refused`.

### A stale revision

When I send `expected_revision` from before someone else's change. It fails with a conflict only if a field I set changed since. Sessions and dependencies never conflict.
**Status:** Works (real daemon). `board_tools.rs::a_stale_revision_conflicts_only_on_a_field_that_changed`.

### Invalid date, effort, impact, or title too long

When `todo_add` gets a non-calendar `due` (`2025-02-29`) or unknown `effort`/`impact`, or `board_item_create` gets a title over 200 characters or unknown `requester`. Refused with `invalid_field` before the daemon acts.
**Status:** Unit-tested / Fakes: `daemon/svc_mcp/src/mcp/todo.rs` tests (`a_day_that_is_not_a_date_is_refused_with_the_accepted_form`, `effort_and_impact_take_only_their_named_values`); `daemon/mcp/tests/schemas.rs`.

### The same call sent twice

When my client retries a create. One item is made.
**Status:** Works (real daemon) for the daemon: `boards.rs::a_repeated_create_with_the_same_key_makes_one_item`. MCP's keys per request id: Fakes only, `daemon/mcp/tests/waiting.rs::a_client_retry_of_the_same_request_gets_the_same_inbox_request`.

### An archived session's board

When I create on a board whose session is archived. Refused as not found, with a hint to restore it or use the channel board.
**Status:** Works (real daemon). `boards.rs::an_archived_sessions_board_takes_no_new_items_until_restored`.

### A write that needs the person

When the write targets the Global board, an item I didn't make, or `todo_close` under default Ask. The call returns pending. See case 13.
**Status:** Works (real daemon). `board_tools.rs::creating_on_the_global_board_waits_for_the_user`; `svc_board_unit_test` for `todo_close`.

## Expected outcomes

- [ ] `todo_add` creates a `Suggested` item that stays unranked until the person accepts it.
- [ ] `todo_close` asks the person by default (`CAPABILITY_TODO_CLOSE`) and closes on Yes.
- [ ] An agent makes, changes, moves and deletes items on its own session board with no ask.
- [ ] A loop and a real conflict are refused with a kind the agent can branch on.
- [ ] A deleted item is gone from lists and kept in history.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 `todo_add` (suggested) | Unit-tested | `svc_board_unit_test`, `svc_mcp` unit tests |
| 2 `todo_close` (Ask / Allow / Never) | Unit-tested | `svc_board_unit_test`, `svc_mcp` unit tests |
| 3 `board_item_create` | Works (real daemon) | `board_tools.rs::an_agent_works_its_session_board_…` |
| 4 `board_item_update` | Works (real daemon) | same |
| 5 `board_item_move` | Works (real daemon) | same |
| 6 Channel board via MCP | Works for rules; MCP Code, no test | `refusals.rs` |
| 7 `board_item_delete` | Works (real daemon) | `board_tools.rs` |
| Loop & stale revision | Works (real daemon) | `board_tools.rs` |
| Bad arguments | Unit-tested / Fakes | `svc_mcp/src/mcp/todo.rs`, `mcp/tests/schemas.rs` |
| Retry & archived board | Works (real daemon) | `boards.rs` |
| Needs the person | Works (real daemon) | `board_tools.rs`, `svc_board_unit_test` |

## Gaps and likely bugs

- Not built: any real agent reaching these tools automatically at launch (case 9). Every "Works" here is driven by a test client.
- No real-daemon `board_tools.rs` integration test drives `todo_next`, `todo_add`, `todo_close` over `humatad2 mcp` yet (they are covered by `svc_board` and `svc_mcp` unit tests).
- Likely bug: `api/MCP.md:290` still describes `board_item_update` as changing the "assignee" instead of `sessions_add` and `sessions_remove`.
