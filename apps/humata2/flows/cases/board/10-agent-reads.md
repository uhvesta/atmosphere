# An agent reads boards through the CLI and through MCP

Layer: verify (Board, agents). Platforms: macOS (this Mac).

## Starting state

- A real daemon. `#payments` has channel items PAY-1 (To do, mine) and PAY-2 (Done, depends on PAY-1). The Global board has GLB-1. `#infra` has INF-1.
- A session "Ledger" in `#payments`. Its own board has PAY-3 (In progress).
- I am Ledger's agent. My shell has `HUMATA_SESSION_TOKEN` and `HUMATA_SOCKET`. `H` is the full path of `humata`.

## Reproduce headless

- MCP reads on a real daemon: `bazel test //apps/humata2/daemon/humatad:board_tools_test --test_filter=an_agent_works_its_session_board`.
- The lists and counts: `bazel test //apps/humata2/daemon/humatad:boards_test --test_filter=a_list`.
- CLI shapes on a fake daemon: `bazel test //apps/humata2/cli:boards_test`.
- By hand on the rig ([README](README.md)): type each command of the steps into Ledger's tab with its output sent to a file, for example `humata session type Ledger "$H --json board > $D/out1.json 2> $D/err1; echo \$? > $D/code1"`. For MCP, pipe the JSON-RPC lines into `humatad2_admin mcp --socket "$HUMATA_SOCKET"` inside the tab.
- Not headless: nothing.

## Steps

1. **I do:** `$H --json board`.
   **I see:** my channel's board: PAY-1 and PAY-2, each with `id`, `title`, `column`, `sessions`, `depends_on`, `created_by`, `revision`. PAY-2's `depends_on` names PAY-1 with its title and column. `pending` is empty: only the person sees asks. Exit 0.
   **State and events:** `ListItems` for the channel board. Reading my channel is allowed with no rule.
   **Status:** Works (real daemon) for the daemon: `daemon/humatad/tests/boards.rs::a_list_counts_every_column_and_filters_by_the_columns_asked` and `::an_agents_move_waits_and_applies_on_approval_and_only_the_user_sees_it_pending` (the empty `pending`). The CLI from a tab: Code, no test; `cli/tests/boards.rs` runs on a fake daemon.

2. **I do:** `$H --json board --session Ledger`, then `$H --json board --column in-progress --session Ledger`.
   **I see:** PAY-3 both times.
   **State and events:** my own session board. The CLI has no "my board" word; I name my session.
   **Status:** Code, no test for the CLI. Daemon: Works (real daemon), `clients/live/tests/boards.rs::a_session_board_item_is_listed_on_that_sessions_board`.

3. **I do:** `$H --json board --global`, then `$H --json --channel infra board`.
   **I see:** GLB-1; then INF-1. No ask: reading the Global board and other channels' boards is Allow by default.
   **Status:** Works for the rule: `daemon/access/tests/global_board.rs::reading_the_global_board_or_another_channels_is_allowed`. On a real daemon as an agent: Code, no test.

4. **I do:** `$H --json board history PAY-2`.
   **I see:** entries oldest first, each with `kind`, `change`, who and when.
   **Status:** Works (real daemon) for the daemon: `boards.rs::history_records_what_happened_in_order_with_its_kind`.

5. **I do:** look for a way to read one item: `$H board show PAY-1`.
   **I see:** a usage error, exit 2. There is no such subcommand. I must list the board and pick the item out.
   **Status:** Not built, no milestone. `cli/src/args.rs:549` has `add`, `edit`, `move`, `rm`, `history`. `GetItem` exists on the daemon.

6. **I do:** MCP `board_items_list` with `{"board":"session"}`, then with `{"board":"channel","columns":["done"]}`.
   **I see:** `{items, counts}`; PAY-3, then PAY-2. `counts` has every column.
   **Status:** Works (real daemon). `daemon/humatad/tests/board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it` (session board, with and without `columns`). The `channel` and `global` words: Code, no test on a real daemon (`daemon/humatad/src/mcp/boards.rs:23`).

7. **I do:** MCP `board_item_get` with `{"id":"PAY-2"}`, then `board_item_history` with the same id.
   **I see:** the item, with `depends_on[0].done` false; then its entries.
   **Status:** Works (real daemon). Same test (get and history on session items).

## Branches

### An item that isn't there, or that I may not read

When I ask for `PAY-99`, or for an item on an archived session's board. Both answer the same: `not_found`, CLI exit 3. I can't tell a hidden item from a missing one.
**Status:** Works (real daemon). `board_tools.rs` (the get after a delete); `boards.rs::an_archived_sessions_items_are_hidden_and_come_back_on_restore`.

### The host is away

When I am on a remote machine and the Mac is unreachable. A list I made while it was linked is answered from the last copy, marked with its age (`stale_as_of`). A list I never made is refused as host unreachable, exit 8.
**Status:** Works (real daemon, fake box). `daemon/humatad/tests/away_board_journal.rs::row_2_a_read_made_while_the_host_was_linked_is_answered_while_it_is_away_marked_with_its_age`; `cli/tests/boards.rs::a_read_from_a_machines_copy_keeps_its_age` (fake daemon).

### A long board

When the board has more items than one page. The CLI reads every page and prints one list. MCP returns `next_page_token`.
**Status:** Works (real daemon) for the daemon: `boards.rs::a_list_is_paged`. The CLI joining pages: Code, no test.

### Reading is set to Never

When the person sets "Global board, or other channels and their boards: read" to Never. Step 3 is refused as denied, exit 4, naming the rule.
**Status:** Works (real daemon, fake box) for a copy: `away_board_journal.rs::a_read_the_rules_now_say_never_is_not_served_from_the_copy`. Linked, on a board read: Code, no test.

## Expected outcomes

- [ ] The CLI and MCP list the same items for the same board.
- [ ] An agent never sees `pending` asks.
- [ ] A dependency carries its title and whether it is done.
- [ ] A missing item and an unreadable item answer the same.
- [ ] A read from a copy says how old it is.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 CLI, channel board | Works (real daemon); CLI Code, no test | `boards.rs::a_list_counts_…` |
| 2 CLI, session board | Code, no test | `clients/live/tests/boards.rs` for the daemon |
| 3 CLI, Global and other channel | Works for the rule | `global_board.rs` |
| 4 CLI history | Works (real daemon) | `boards.rs::history_records_…` |
| 5 CLI one item | Not built | `cli/src/args.rs:549` |
| 6 MCP list | Works (real daemon) | `board_tools.rs` |
| 7 MCP get, history | Works (real daemon) | `board_tools.rs` |
| Not found | Works (real daemon) | `board_tools.rs`, `boards.rs` |
| Host away | Works (real daemon, fake box) | `away_board_journal.rs` |
| Paging | Works (real daemon); CLI Code, no test | `boards.rs::a_list_is_paged` |
| Read set to Never | Code, no test (linked) | `away_board_journal.rs` for the copy |

## Gaps and likely bugs

- Not built: `humata board show <item>`. An agent using the CLI can't read one item's description without listing the whole board.
- Not built: the mixed scopes (channel with sessions, a session and its forks) for agents, on both doors.
- Likely bug: the two doors default to different boards. `humata board` with no flag is the channel board (`cli/src/commands/boards.rs:31`); MCP's `board: "session"` is the agent's own. An agent that learned one will misread the other.
- No test runs `humata board` (a read) inside a session tab on a real daemon. The only real CLI board call tested is `move`.
