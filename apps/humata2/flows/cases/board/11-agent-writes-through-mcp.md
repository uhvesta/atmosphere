# An agent creates, edits, moves and deletes through MCP

Layer: verify (Board, agents). Platforms: macOS (this Mac).

## Starting state

- A real daemon. `#payments` has two sessions, "Ledger" and "Refunds". All boards are empty.
- I am Ledger's agent, with Humata's MCP server connected and bound to my session. Today that means the server was started by hand inside my tab (case 9).
- Agent access is at its defaults.

## Reproduce headless

- The whole case on a real daemon: `bazel test //apps/humata2/daemon/humatad:board_tools_test`. It starts `humatad2 mcp` with a session token and speaks JSON-RPC to it.
- Schemas and limits on fakes: `bazel test //apps/humata2/daemon/mcp:schemas_test`. Tiers: `bazel test //apps/humata2/daemon/mcp:tiers_test`.
- By hand on the rig: inside Ledger's tab, pipe `initialize` and then one `tools/call` line per step into `humatad2_admin mcp --socket "$HUMATA_SOCKET"`. Save the replies. As the person, `humata --json board --session Ledger` after each step.
- Not headless: a real agent CLI choosing these tools. No agent is started with the server (case 9).

## Steps

1. **I do:** `board_item_create` `{"board":"session","title":"Refund path","column":"to_do"}`.
   **I see:** the item, with an id like `PAY-1`, `column: "to_do"`, `sessions: []`, `created_by` my session.
   **State and events:** own session board, Allow. BoardItem created. A session board shares its channel's numbering.
   **Status:** Works (real daemon). `daemon/humatad/tests/board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it`; key in `boards.rs::a_session_item_uses_its_channels_prefix`.

2. **I do:** `board_item_create` `{"board":"session","title":"Webhooks"}`.
   **I see:** a second item, in Backlog (the default).
   **Status:** Works (real daemon). Same test.

3. **I do:** `board_item_update` on the first with `description`, `sessions_add: [Ledger, Refunds]` and `depends_on_add: [<second>]`.
   **I see:** the item with the description, both sessions in order, and one dependency with `done: false`.
   **State and events:** BoardItem updated. History gains a changed, a sessions and a dependencies entry.
   **Status:** Works (real daemon). Same test.

4. **I do:** `board_item_update` with `sessions_remove: [Refunds]`.
   **I see:** one session left; the dependency untouched.
   **Status:** Works (real daemon). Same test.

5. **I do:** `board_item_move` `{"id":<second>,"column":"done"}`.
   **I see:** `column: "done"` and a `done_at`. A `board_item_get` of the first now shows its dependency `done: true`.
   **Status:** Works (real daemon). Same test.

6. **I do:** `board_item_move` with `after: <other item>` in the same column.
   **I see:** the item placed right after it. With no `after`, it goes to the top.
   **Status:** Works (real daemon) for the daemon: `boards.rs::new_items_go_to_the_top_and_a_move_goes_after_the_item_named`. Through MCP: Code, no test.

7. **I do:** `board_item_create` `{"board":"channel","title":"Backfill","depends_on":[<first>]}`.
   **I see:** a channel item, no ask. Then `board_item_move` on it to `in_progress`: no ask, I made it.
   **State and events:** `channel_board_add` and `channel_board_edit_own`, both Allow.
   **Status:** Works for the rules: `daemon/access/tests/refusals.rs::board_items_an_agent_made_are_its_to_edit`. On a real daemon through MCP: Code, no test. `board_tools.rs` only uses `session` and `global`.

8. **I do:** `board_item_delete` on the first item.
   **I see:** `{deleted: true}`. A get answers `not_found`. The person's list no longer has it; its history keeps a tombstone.
   **Status:** Works (real daemon). Same test as step 1.

## Branches

### A loop

When I add a dependency that closes a loop. The call fails with `dependency_loop`, naming the cycle. Nothing changes.
**Status:** Works (real daemon). `board_tools.rs::a_dependency_that_would_close_a_loop_is_refused`.

### A stale revision

When I send `expected_revision` from before someone else's change. It fails with a conflict only if a field I set changed since. Sessions and dependencies never conflict.
**Status:** Works (real daemon). `board_tools.rs::a_stale_revision_conflicts_only_on_a_field_that_changed`.

### A title too long, or an unknown field

When the title is over 200 characters, or I send `requester`. The server refuses before the daemon sees it.
**Status:** Fakes only. `daemon/mcp/tests/schemas.rs::a_title_longer_than_200_characters_is_refused_before_the_daemon_sees_it` and `::a_requester_argument_is_refused_as_not_a_field`.

### The same call sent twice

When my client retries a create. One item is made.
**Status:** Works (real daemon) for the daemon: `boards.rs::a_repeated_create_with_the_same_key_makes_one_item`. MCP's keys per request id: Fakes only, `daemon/mcp/tests/waiting.rs::a_client_retry_of_the_same_request_gets_the_same_inbox_request`.

### An archived session's board

When I create on a board whose session is archived. Refused as not found, with a hint to restore it or use the channel board.
**Status:** Works (real daemon). `boards.rs::an_archived_sessions_board_takes_no_new_items_until_restored`.

### A write that needs the person

When the board is the Global board or the item is not mine. The call returns pending. See case 13.
**Status:** Works (real daemon). `board_tools.rs::creating_on_the_global_board_waits_for_the_user`.

## Expected outcomes

- [ ] An agent makes, changes, moves and deletes items on its own session board with no ask.
- [ ] An agent adds to its channel's board and changes what it added with no ask.
- [ ] Sessions and dependencies go in and out as lists, without replacing each other.
- [ ] A loop and a real conflict are refused with a kind the agent can branch on.
- [ ] A deleted item is gone from lists and kept in history.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Create, To do | Works (real daemon) | `board_tools.rs::an_agent_works_its_session_board_…` |
| 2 Create, default column | Works (real daemon) | same |
| 3 Update fields and lists | Works (real daemon) | same |
| 4 Remove a session | Works (real daemon) | same |
| 5 Move to Done | Works (real daemon) | same |
| 6 Move after an item | Works (real daemon) in the daemon; MCP Code, no test | `boards.rs::new_items_go_to_the_top_…` |
| 7 Channel board | Works for the rules; MCP Code, no test | `refusals.rs` |
| 8 Delete | Works (real daemon) | `board_tools.rs` |
| Loop | Works (real daemon) | `board_tools.rs::a_dependency_that_would_close_a_loop_is_refused` |
| Stale revision | Works (real daemon) | `board_tools.rs::a_stale_revision_…` |
| Bad arguments | Fakes only | `mcp/tests/schemas.rs` |
| Retry | Works (real daemon) in the daemon | `boards.rs::a_repeated_create_…` |
| Archived board | Works (real daemon) | `boards.rs::an_archived_sessions_board_…` |
| Needs the person | Works (real daemon) | `board_tools.rs::creating_on_the_global_board_…` |

## Gaps and likely bugs

- Not built: any real agent reaching these tools (case 9). Every "Works" here is the tool, driven by a test client.
- No real-daemon MCP test uses `board: "channel"` or `{session: …}`. The channel board is where agents are meant to plan together.
- Likely bug: `api/MCP.md:290` still describes `board_item_update` as changing the "assignee". The tool takes `sessions_add` and `sessions_remove`. The spec text is stale, not the code.
- Likely bug: the default column is Backlog through MCP and the CLI, and To do in the app (case 1).
