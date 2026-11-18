# An agent creates, edits, moves and deletes through the CLI

Layer: verify (Board, agents). Platforms: macOS (this Mac).

## Starting state

- A real daemon. `#payments` has two sessions, "Ledger" and "Refunds". All boards are empty.
- I am Ledger's agent, in my session's tab. The tab has `HUMATA_SESSION_TOKEN` and `HUMATA_SOCKET`. `H` is the full path of `humata`.
- Agent access is at its defaults.

## Reproduce headless

- The flags the CLI sends, on a fake daemon: `bazel test //apps/humata2/cli:boards_test`.
- A real `humata board move` from a session tab against a real daemon: `bazel test //apps/humata2/desktop:live_boards_test --test_filter=approving_an_agents_move`.
- By hand on the rig ([README](README.md)): start `Ledger` with `--agent zsh`, then type each step's command into its tab with `humata session type`, sending stdout, stderr and `$?` to files under `$D`. After each step, as the person: `humata --json board --channel payments` and `humata --json board --session Ledger`.
- Not headless: nothing. A real model typing these commands is case 14.

## Steps

1. **I do:** `$H --json board add "Refund path" --column to-do --description "refunds over the limit"`.
   **I see:** the item, `PAY-1`, on the **channel** board, `created_by` my session. Exit 0.
   **State and events:** with no flag the CLI picks my channel's board. `channel_board_add`, Allow. BoardItem created.
   **Status:** Code, no test for a create from a tab on a real daemon. The flags: Fakes only, `cli/tests/boards.rs::add_sends_every_session_and_dependency_in_order`. The rule: `daemon/access/tests/refusals.rs`.

2. **I do:** `$H --json board --session Ledger add "Webhooks"`.
   **I see:** `PAY-2` on my own session board, in Backlog.
   **State and events:** `own_board_write`, Allow.
   **Status:** Code, no test for the CLI. The daemon: Works (real daemon), `daemon/humatad/tests/boards.rs::a_session_item_uses_its_channels_prefix`.

3. **I do:** `$H --json board edit PAY-1 --assign Ledger --assign Refunds --depends-on PAY-2`.
   **I see:** both sessions and one dependency on the item.
   **State and events:** `UpdateItem` with `sessions_add` and `depends_on_add`. I made PAY-1, so no ask.
   **Status:** Fakes only for the CLI: `cli/tests/boards.rs::edit_adds_and_removes_without_replacing`. The daemon: Works (real daemon), `boards.rs::sessions_are_added_and_removed_and_unknown_ones_refused`.

4. **I do:** `$H --json board edit PAY-1 --unassign Refunds --title "Refund path v2"`.
   **I see:** one session left, the new title.
   **Status:** Fakes only for the CLI (same test, no title). Daemon as step 3.

5. **I do:** `$H --json board move PAY-1 in-progress`, then `$H --json board move PAY-2 done --after PAY-1`.
   **I see:** each item in its new column. PAY-2 has `done_at`.
   **State and events:** `MoveItem`, no revision sent. BoardItem updated ×2.
   **Status:** Works (real daemon) for a move from a tab: `desktop/tests/live_boards.rs::approving_an_agents_move_from_the_items_drawer_moves_it_and_returns_the_call` (it runs the real binary; that move asks, then lands). A move that needs no ask: Code, no test.

6. **I do:** `$H --json board history PAY-1`.
   **I see:** created, sessions, dependencies, changed, moved, each by my session.
   **Status:** Works (real daemon) for the daemon: `boards.rs::history_records_what_happened_in_order_with_its_kind`.

7. **I do:** `$H --json board rm PAY-2`.
   **I see:** the reply for a delete. PAY-1 loses its dependency, and its history says so. Exit 0.
   **Status:** Works (real daemon) for the daemon: `boards.rs::deleting_an_item_drops_the_edges_to_it`. The CLI: Code, no test.

## Branches

### A loop

When I run `board edit PAY-2 --depends-on PAY-1` while PAY-1 depends on PAY-2. Refused, naming the loop. Exit 7.
**Status:** Works (real daemon) for the daemon: `boards.rs::a_dependency_that_closes_a_loop_is_refused_with_the_loop`. Exit code: Code, no test.

### A session name that doesn't exist

When I `--assign Nobody`. The CLI can't resolve it. Exit 3.
**Status:** Code, no test. `cli/src/commands/boards.rs:37`. The daemon refuses unknown ids too (`boards.rs::sessions_are_added_and_removed_and_unknown_ones_refused`).

### Someone changed the item first

When the person retitled PAY-1 a moment before my `edit --title`. Mine wins without a word. The CLI sends no revision, so it can't notice.
**Status:** Likely bug. `cli/src/commands/boards.rs:112`, `:127`, `:136` send `expected_revision: None`. MCP can send one. Confirm with two edits and read the history.

### A write that needs the person

When I move an item I didn't make, or write on `--global`. With `--json` and no `--wait` the call ends at once, exit 12, with the request id. See case 13.
**Status:** Works (real daemon). `live_boards.rs` (with `--wait 600`).

### A blocked column

When I `board move PAY-1 blocked`. A usage error, exit 2: the CLI has five columns.
**Status:** Code, no test. `cli/src/args.rs:1021`.

## Expected outcomes

- [ ] `humata board add` from a tab makes an item owned by that tab's session.
- [ ] `edit` adds and removes sessions and dependencies without replacing the lists.
- [ ] `move` and `rm` on the agent's own items need no approval.
- [ ] Every change is in the item's history under the agent's session.
- [ ] Exit codes tell pending (12), not found (3) and a loop (7) apart.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Add to the channel board | Code, no test; flags Fakes only | `cli/tests/boards.rs` |
| 2 Add to the session board | Code, no test | `boards.rs` for the daemon |
| 3 Edit lists | Fakes only; daemon Works | `cli/tests/boards.rs::edit_adds_and_removes_…` |
| 4 Edit title | Fakes only | same |
| 5 Move | Works (real daemon) with an ask; plain move Code, no test | `live_boards.rs` |
| 6 History | Works (real daemon) in the daemon | `boards.rs::history_records_…` |
| 7 Delete | Code, no test | `boards.rs` for the daemon |
| Loop | Works (real daemon) in the daemon | `boards.rs` |
| Unknown session | Code, no test | `cli/src/commands/boards.rs:37` |
| Lost update | Likely bug | `cli/src/commands/boards.rs:112` |
| Needs the person | Works (real daemon) | `live_boards.rs` |
| Blocked column | Code, no test | `cli/src/args.rs:1021` |

## Gaps and likely bugs

- Not built: `humata` on the tab's `PATH` (case 9). Every step needs the full path.
- Likely bug: the CLI never sends a revision, so an agent's edit silently overwrites a newer one. Add `--if-revision` or read-then-write.
- Likely bug: `board add` with no flag goes to the channel board, where MCP's natural default is the session board. An agent told "put it on your board" will put it on everyone's.
- Not built: `--reason` on `add`, `edit`, `move` and `rm` (`cli/src/commands/boards.rs:84`).
- Only `move` has ever run from a real tab against a real daemon in a test. `add`, `edit`, `rm` and the list are proven on a fake daemon only.
