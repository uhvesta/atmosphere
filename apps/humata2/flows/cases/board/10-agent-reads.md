# An agent reads To-do and boards through the CLI and through MCP

Layer: verify (To-do, agents). Platforms: macOS (this Mac).

## Starting state

- A real daemon. `#payments` has items `PAY-1` (`Open`, mine) and `PAY-2` (`Closed`, depends on `PAY-1`). An item with no channel: `GLB-1`. `#infra` has `INF-1`.
- A session "Ledger" in `#payments`. It has an item `PAY-3` (`In progress`) on its session board.
- I am Ledger's agent. My shell has `HUMATA_SESSION_TOKEN` and `HUMATA_SOCKET`. `H` is the full path of `humata`.

## Reproduce headless

- MCP `todo_next` unit tests: `bazel test //apps/humata2/daemon/svc_mcp:svc_mcp_unit_test` and `bazel test //apps/humata2/daemon/svc_board:svc_board_unit_test`.
- MCP board reads on a real daemon: `bazel test //apps/humata2/daemon/humatad:board_tools_test --test_filter=an_agent_works_its_session_board`.
- The lists and counts: `bazel test //apps/humata2/daemon/humatad:boards_test --test_filter=a_list`.
- CLI shapes on a fake daemon: `bazel test //apps/humata2/cli:boards_test`.
- By hand on the rig ([README](README.md)): type each command of the steps into Ledger's tab with its output sent to a file, for example `humata session type Ledger "$H --json board > $D/out1.json 2> $D/err1; echo \$? > $D/code1"`. For MCP, pipe the JSON-RPC lines into `humatad2_admin mcp --socket "$HUMATA_SOCKET"` inside the tab.
- Not headless: nothing.

## Steps

1. **I do:** `$H --json board`.
   **I see:** my channel's items: `PAY-1` and `PAY-2`, each with `id`, `title`, `column`, `sessions`, `depends_on`, `created_by`, `revision`. `PAY-2`'s `depends_on` names `PAY-1` with its title and column. `pending` is empty: only the person sees asks. Exit 0.
   **State and events:** `BoardService.ListItems` for the channel board. Reading my channel is allowed with no rule.
   **Status:** Works (real daemon) for the daemon: `daemon/humatad/tests/boards.rs::a_list_counts_every_column_and_filters_by_the_columns_asked` and `::an_agents_move_waits_and_applies_on_approval_and_only_the_user_sees_it_pending` (the empty `pending`). The CLI from a tab: Code, no test; `cli/tests/boards.rs` runs on a fake daemon.

2. **I do:** `$H --json board --session Ledger`, then `$H --json board --column in-progress --session Ledger`.
   **I see:** `PAY-3` both times.
   **State and events:** my own session board. The CLI has no "my board" word; I name my session.
   **Status:** Code, no test for the CLI. Daemon: Works (real daemon), `clients/live_daemon/tests/boards.rs::a_session_board_item_is_listed_on_that_sessions_board`.

3. **I do:** `$H --json board --global`, then `$H --json --channel infra board`.
   **I see:** `GLB-1`; then `INF-1`. No ask: reading Global and other channels' items is Allow by default.
   **Status:** Works for the rule: `daemon/access/tests/global_board.rs::reading_the_global_board_or_another_channels_is_allowed`. On a real daemon as an agent: Code, no test.

4. **I do:** `$H --json board history PAY-2`.
   **I see:** entries oldest first, each with `kind`, `change`, who and when.
   **Status:** Works (real daemon) for the daemon: `boards.rs::history_records_what_happened_in_order_with_its_kind`.

5. **I do:** look for a way to read one item from the CLI: `$H board show PAY-1`.
   **I see:** a usage error, exit 2. There is no `board show` (and no `humata todo` subcommand yet). I must list with `humata board` and pick the item out.
   **Status:** Not built, no milestone. `cli/src/args.rs` has `add`, `edit`, `move`, `rm`, `history`. `GetItem` and `TodoService` exist on the daemon.

6. **I do:** MCP `todo_next` (optionally with `{"channel": "payments"}` or `{"session": "..."}`).
   **I see:** the top-ranked open To-do item with `key`, `title`, `description`, `state`, `suggested`, `due`, `effort`, `impact`, `tags`, `steps`, `channel`, `sessions`, `revision` (or `null` when nothing is ready).
   **State and events:** `TodoService.NextTodo` runs `humata2_todo::rank` on the daemon.
   **Status:** Unit-tested in `daemon/svc_board:svc_board_unit_test` and `daemon/svc_mcp/src/mcp/todo.rs` tests.

7. **I do:** MCP `board_items_list` with `{"board":"session"}`, then `board_item_get` and `board_item_history`.
   **I see:** `{items, counts}`, the item with `depends_on`, and its history entries.
   **Status:** Works (real daemon). `daemon/humatad/tests/board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it`.

## Branches

### An item that isn't there, or that I may not read

When I ask for `PAY-99`, or for an item on an archived session's board. Both answer the same: `not_found`, CLI exit 3. I can't tell a hidden item from a missing one.
**Status:** Works (real daemon). `board_tools.rs` (the get after a delete); `boards.rs::an_archived_sessions_items_are_hidden_and_come_back_on_restore`.

### The host is away

When I am on a remote machine and the Mac is unreachable. A `BoardService` list I made while it was linked is answered from the last copy, marked with its age (`stale_as_of`). A list I never made is refused as host unreachable, exit 8. (`TodoService` calls from an agent on a remote machine currently answer `not_built`: `daemon/svc_board/src/todo.rs:1166`.)
**Status:** Works (real daemon, fake box) for `BoardService`: `daemon/humatad/tests/away_board_journal.rs::row_2_a_read_made_while_the_host_was_linked_is_answered_while_it_is_away_marked_with_its_age`; `cli/tests/boards.rs::a_read_from_a_machines_copy_keeps_its_age` (fake daemon).

### A long list

When there are more items than one page. The CLI reads every page and prints one list. MCP `board_items_list` returns `next_page_token`.
**Status:** Works (real daemon) for the daemon: `boards.rs::a_list_is_paged`; `svc_board_unit_test` covers `ListTodo` paging. The CLI joining pages: Code, no test.

### Reading is set to Never

When the person sets "Read other channels' to-dos and files" to **Never**. Step 3 is refused as denied, exit 4, naming the rule.
**Status:** Works (real daemon, fake box) for a copy: `away_board_journal.rs::a_read_the_rules_now_say_never_is_not_served_from_the_copy`. Linked, on a read: Code, no test.

## Expected outcomes

- [ ] `todo_next` returns the top-ranked To-do item matching the optional channel/session filter.
- [ ] The CLI and MCP list the same items for the same board.
- [ ] An agent never sees `pending` asks.
- [ ] A dependency carries its title and whether it is done.
- [ ] A missing item and an unreadable item answer the same.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 CLI, channel board | Works (real daemon) in daemon; CLI Code, no test | `boards.rs::a_list_counts_…` |
| 2 CLI, session board | Code, no test | `clients/live_daemon/tests/boards.rs` for the daemon |
| 3 CLI, Global and other channel | Works for the rule | `global_board.rs` |
| 4 CLI history | Works (real daemon) in daemon | `boards.rs::history_records_…` |
| 5 CLI single item / `humata todo` | Not built | `cli/src/args.rs` |
| 6 MCP `todo_next` | Unit-tested | `svc_board_unit_test`, `svc_mcp` unit tests |
| 7 MCP `board_*` list, get, history | Works (real daemon) | `board_tools.rs` |
| Not found | Works (real daemon) | `board_tools.rs`, `boards.rs` |
| Host away | Works for `BoardService`; `TodoService` remote agent Not built | `away_board_journal.rs`, `svc_board/src/todo.rs:1166` |
| Paging | Works (real daemon) | `boards.rs::a_list_is_paged`, `svc_board_unit_test` |
| Read set to Never | Code, no test (linked) | `away_board_journal.rs` for the copy |

## Gaps and likely bugs

- Not built: `humata todo` CLI commands and `humata board show <item>`.
- Not built: `TodoService` forwarding for agents on a remote machine (`daemon/svc_board/src/todo.rs:1166`).
- Likely bug: `humata board` with no flag defaults to the channel board (`cli/src/commands/boards.rs:31`), whereas MCP `board_items_list` uses `"session"`.
- No test runs `humata board` (a read) inside a session tab on a real daemon.
