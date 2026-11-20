# An agent creates, edits, moves and deletes through the CLI

Layer: verify (To-do, agents). Platforms: macOS (this Mac).

The CLI still exposes `humata board` (over `BoardService`, backed by the same items as To-do). There is no `humata todo` subcommand yet.

## Starting state

- A real daemon. `#payments` has two sessions, "Ledger" and "Refunds". The list is empty.
- I am Ledger's agent, in my session's tab. The tab has `HUMATA_SESSION_TOKEN` and `HUMATA_SOCKET`. `H` is the full path of `humata`.
- Agent access is at its defaults.

## Reproduce headless

- The flags the CLI sends, on a fake daemon: `bazel test //apps/humata2/cli:boards_test`.
- A real `humata --json --wait 600 board move <key> done` as the session's agent against a real daemon, answered Yes / No in the To-do drawer: `bazel test //apps/humata2/desktop:live_flows_test --test_filter=todo_asks`.
- By hand on the rig ([README](README.md)): start `Ledger` with `--agent zsh`, then type each step's command into its tab with `humata session type`, sending stdout, stderr and `$?` to files under `$D`. After each step, as the person: `humata --json board --channel payments` and `humata --json board --session Ledger`.
- Not headless: nothing. A real model typing these commands is case 14.

## Steps

1. **I do:** `$H --json board add "Refund path" --column to-do --description "refunds over the limit"`.
   **I see:** the item, `PAY-1`, on the **channel** board, `created_by` my session. Exit 0. It appears in To-do as `Open`.
   **State and events:** with no flag the CLI picks the channel board (`cli/src/commands/boards.rs:31`). `channel_board_add`, Allow. `BoardItem` created.
   **Status:** Code, no test for a create from a tab on a real daemon. The flags: Fakes only, `cli/tests/boards.rs::add_sends_every_session_and_dependency_in_order`. The rule: `daemon/access/tests/refusals.rs`.

2. **I do:** `$H --json board --session Ledger add "Webhooks"`.
   **I see:** `PAY-2` on my own session board, in Backlog (`Open` in To-do, attached to Ledger).
   **State and events:** `own_board_write`, Allow.
   **Status:** Code, no test for the CLI. The daemon: Works (real daemon), `daemon/humatad/tests/boards.rs::a_session_item_uses_its_channels_prefix`.

3. **I do:** `$H --json board edit PAY-1 --assign Ledger --assign Refunds --depends-on PAY-2`.
   **I see:** both sessions and one dependency on the item.
   **State and events:** `UpdateItem` with `sessions_add` and `depends_on_add`. I made `PAY-1`, so no ask.
   **Status:** Fakes only for the CLI: `cli/tests/boards.rs::edit_adds_and_removes_without_replacing`. The daemon: Works (real daemon), `boards.rs::sessions_are_added_and_removed_and_unknown_ones_refused`.

4. **I do:** `$H --json board edit PAY-1 --unassign Refunds --title "Refund path v2"`.
   **I see:** one session left, the new title.
   **Status:** Fakes only for the CLI (same test). Daemon as step 3.

5. **I do:** `$H --json board move PAY-1 in-progress`, then `$H --json --wait 600 board move <person's item> done`.
   **I see:** my own item moves at once; moving the person's item waits until the person clicks **Yes** in the To-do drawer, then prints the moved item and exits 0 (or exits 4 on **No**).
   **State and events:** `MoveItem`. `BoardItem` updated.
   **Status:** Works (real daemon) for a real `humata board move` that asks and lands/fails: `desktop/tests/live_flows/todo_asks.rs::approving_an_agents_ask_from_the_todo_drawer_closes_the_item_and_returns_the_call` and `::denying_an_agents_ask_from_the_todo_drawer_keeps_the_item_open_and_fails_the_call`. A move that needs no ask: Code, no test.

6. **I do:** `$H --json board history PAY-1`.
   **I see:** created, sessions, dependencies, changed, moved, each by my session.
   **Status:** Works (real daemon) for the daemon: `boards.rs::history_records_what_happened_in_order_with_its_kind`.

7. **I do:** `$H --json board rm PAY-2`.
   **I see:** the reply for a delete. `PAY-1` loses its dependency, and its history says so. Exit 0.
   **Status:** Works (real daemon) for the daemon: `boards.rs::deleting_an_item_drops_the_edges_to_it`. The CLI: Code, no test.

## Branches

### A loop

When I run `board edit PAY-2 --depends-on PAY-1` while `PAY-1` depends on `PAY-2`. Refused, naming the loop. Exit 7.
**Status:** Works (real daemon) for the daemon: `boards.rs::a_dependency_that_closes_a_loop_is_refused_with_the_loop`. Exit code: Code, no test.

### A session name that doesn't exist

When I `--assign Nobody`. The CLI can't resolve it. Exit 3.
**Status:** Code, no test (`cli/src/commands/boards.rs:57`). The daemon refuses unknown ids too (`boards.rs::sessions_are_added_and_removed_and_unknown_ones_refused`).

### Someone changed the item first

When I pass `--revision <N>` and the item changed since, the daemon refuses as a revision conflict. Without `--revision`, the CLI sends `expected_revision: None` and overwrites without noticing.
**Status:** Code, no test for the CLI flag (`cli/src/commands/boards.rs:52`).

### A write that needs the person

When I move an item I didn't make, or write on `--global`. With `--json` and no `--wait` the call ends at once, exit 12, with the request id. With `--wait 600`, it waits for the person's Yes/No in To-do. See case 13.
**Status:** Works (real daemon). `desktop/tests/live_flows/todo_asks.rs`.

### A blocked column

When I `board move PAY-1 blocked`. A usage error, exit 2: the CLI has five wire columns.
**Status:** Code, no test (`cli/src/args.rs`).

## Expected outcomes

- [ ] `humata board add` from a tab makes an item owned by that tab's session.
- [ ] `edit` adds and removes sessions and dependencies without replacing the lists.
- [ ] `move` and `rm` on the agent's own items need no approval.
- [ ] Every change is in the item's history under the agent's session.
- [ ] Exit codes tell pending (12), denied (4), not found (3) and a loop (7) apart.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Add to channel | Code, no test; flags Fakes only | `cli/tests/boards.rs` |
| 2 Add to session | Code, no test | `boards.rs` for the daemon |
| 3 Edit lists | Fakes only; daemon Works | `cli/tests/boards.rs::edit_adds_and_removes_…` |
| 4 Edit title | Fakes only | same |
| 5 Move | Works (real daemon) with an ask; plain move Code, no test | `live_flows/todo_asks.rs` |
| 6 History | Works (real daemon) in the daemon | `boards.rs::history_records_…` |
| 7 Delete | Code, no test | `boards.rs` for the daemon |
| Loop | Works (real daemon) in the daemon | `boards.rs` |
| Unknown session | Code, no test | `cli/src/commands/boards.rs:57` |
| `--revision` | Code, no test | `cli/src/commands/boards.rs:52` |
| Needs the person | Works (real daemon) | `live_flows/todo_asks.rs` |
| Blocked column | Code, no test | `cli/src/args.rs` |

## Gaps and likely bugs

- Not built: `humata todo` CLI subcommand (quick add, due, effort, impact, tags, steps, pin, hide-until, `todo next`, `todo close --reason … --evidence …`).
- Not built: `humata` on the tab's `PATH` (case 9). Every step needs the full path.
- Not built: `--reason` on `humata board add`, `edit`, `move` and `rm` (`cli/src/commands/boards.rs`).
- Likely bug: `board add` with no flag defaults to the channel board (`cli/src/commands/boards.rs:31`), whereas MCP `board_item_create` defaults to the session board.
- Only `board move` has run as the real `humata` binary against a real daemon in a live test (`live_flows/todo_asks.rs`).
