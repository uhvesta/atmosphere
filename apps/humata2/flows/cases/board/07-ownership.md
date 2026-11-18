# Who made an item decides who may change it

Layer: verify (Board, the person and the rules). Platforms: macOS.

## Starting state

- `#payments` has two sessions with agents, "Ledger" and "Refunds". `#infra` has one, "Deploy".
- On the `#payments` board: PAY-1, made by me; PAY-2, made by Ledger's agent; PAY-3, made by Refunds' agent.
- The Global board has GLB-1, made by me.
- Agent access for `#payments` is at its defaults.

## Reproduce headless

- The rules alone: `bazel test //apps/humata2/daemon/access:refusals_test --test_filter=board_items_an_agent_made_are_its_to_edit` and `bazel test //apps/humata2/daemon/access:global_board_test`.
- A real daemon: `bazel test //apps/humata2/daemon/humatad:boards_test --test_filter=an_agents` and `bazel test //apps/humata2/daemon/humatad:board_tools_test`.
- By hand on the rig: start two `zsh` sessions in `payments`. In Ledger's tab, `humata --json board add "Mine"`, then in Refunds' tab `humata --json board move <that id> done`. The second exits 12 with a request id. Keep both outputs, and `humata --json board history <id>`.
- Not headless: nothing. The Settings › Agent access rows are in `X-E-agent-switches.md`.

## Steps

1. **I do:** open PAY-2's drawer and look for who made it.
   **I see:** (expected, BD-3) "Made by <agent> · Ledger. That agent may change it freely; others must ask you." On PAY-1: "You made this. Agents need your approval to change it."
   **I see (today):** no such line. History's first entry names the maker.
   **State and events:** `BoardItem.created_by` is set by the daemon and can't be changed.
   **Status:** Not built (BD-3). The app holds it (`clients/src/model.rs:1467`, `clients/live/src/boards.rs:121`) and `ui/src/board.rs` never draws it. The history line: Works (real daemon), `daemon/humatad/tests/boards.rs::history_records_what_happened_in_order_with_its_kind`.

2. **I do:** have Ledger's agent move PAY-2, which it made, to In review.
   **I see:** the card moves. No ask.
   **State and events:** rule "edit, move or delete channel items it created": Allow. BoardItem updated.
   **Status:** Works for the rule (unit level, no daemon): `daemon/access/tests/refusals.rs::board_items_an_agent_made_are_its_to_edit`. On a real daemon with a channel item: Code, no test. `board_tools.rs` uses the session board.

3. **I do:** have Ledger's agent move PAY-3, which Refunds' agent made.
   **I see:** an ask on PAY-3: "<agent> asks to move this to …". The Inbox card offers **Always** for this board.
   **State and events:** capability `channel_board_edit_others`, Ask me. InboxRequest created.
   **Status:** Works for the rule: same access test (it checks the capability and the Always target). Real daemon: Code, no test for one agent on another agent's channel item.

4. **I do:** have Ledger's agent move PAY-1, which I made.
   **I see:** the same ask as step 3.
   **Status:** Works (real daemon). `desktop/tests/live_boards.rs::approving_an_agents_move_from_the_items_drawer_moves_it_and_returns_the_call`.

5. **I do:** have Ledger's agent add an item to the Global board, then edit GLB-1.
   **I see:** an Inbox card each time. Nothing is on the Global board until I allow it.
   **State and events:** capability `global_board_write`, Ask me, whoever made the item.
   **Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::an_agents_write_on_the_global_board_waits_in_the_inbox_until_approved`; `board_tools.rs::creating_on_the_global_board_waits_for_the_user`.

6. **I do:** tick **Always** on that card and allow it.
   **I see:** the agent's next write on the Global board lands with no ask. A write on `#infra`'s board still asks.
   **State and events:** a grant for that board only.
   **Status:** Works for the rule: `daemon/access/tests/global_board.rs::an_always_allow_covers_later_writes_on_that_board_only`. The tick in the app with a board ask: Code, no test.

7. **I do:** have Deploy's agent (in `#infra`) list `#payments`' board, then add to it.
   **I see:** the list comes back. The add asks me.
   **State and events:** reading other channels' boards is Allow; changing them is Ask me (D180).
   **Status:** Works for the rule: `global_board.rs::reading_the_global_board_or_another_channels_is_allowed` and `::adding_or_editing_there_asks_and_offers_always_on_that_board`.

8. **I do:** as myself, move PAY-2 and PAY-3 from the app and from `humata board move`.
   **I see:** both land at once. I never ask anyone.
   **Status:** Works (real daemon). `clients/live/tests/boards.rs::a_moved_item_is_in_its_new_column_on_the_next_load`.

## Branches

### An agent's own session board

When an agent adds, edits, moves and deletes on its own session board. All allowed, no ask.
**Status:** Works (real daemon). `board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it`.

### A rule set to Never

When I set "Channel board: edit anyone else's items" to Never. The agent's call is refused at once as denied, with the rule named, and no Inbox card.
**Status:** Fakes only. `daemon/mcp/tests/refusals.rs::a_never_rule_refusal_reaches_the_agent_as_denied_with_its_source_and_capability`.

### The maker's session is archived

When Ledger is archived. PAY-2 stays on the channel board. No agent owns it any more, so every agent must ask.
**Status:** Code, no test. `daemon/access/src/policy.rs:766` compares `created_by` with the caller's session id only.

### A fork of the maker

When Ledger's fork edits PAY-2. The fork is another session, so it must ask.
**Status:** Code, no test. Same line. Whether a fork should inherit is open (BD-10).

## Expected outcomes

- [ ] The drawer says who made the item and what that means.
- [ ] An agent changes channel items it made without asking.
- [ ] An agent asks before changing an item made by me or by another agent.
- [ ] Any agent write on the Global board or another channel's board asks, and **Always** covers that board only.
- [ ] I can change every item without asking.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Ownership line | Not built | `clients/src/model.rs:1467` |
| 2 Own channel item | Works for the rule; daemon Code, no test | `refusals.rs::board_items_an_agent_made_…` |
| 3 Another agent's item | Works for the rule; daemon Code, no test | same |
| 4 My item | Works (real daemon) | `live_boards.rs::approving_…` |
| 5 Global board | Works (real daemon) | `boards.rs::an_agents_write_on_the_global_board_…` |
| 6 Always | Works for the rule | `global_board.rs::an_always_allow_…` |
| 7 Another channel | Works for the rule | `global_board.rs` |
| 8 The person | Works (real daemon) | `clients/live/tests/boards.rs` |
| Own session board | Works (real daemon) | `board_tools.rs` |
| Never | Fakes only | `mcp/tests/refusals.rs` |
| Maker archived | Code, no test | `policy.rs:766` |
| Fork of the maker | Code, no test | `policy.rs:766` |

## Gaps and likely bugs

- Not built: the drawer's ownership line (BD-3). Without it a person can't tell why one move asked and another didn't.
- Not built: handing an item to another session. Ownership is fixed at creation; adding a session under **Sessions** does not let that session's agent change the item without asking. A person would expect "worked on by" to mean "may move it". No milestone.
- No real-daemon test has one agent touch a channel item made by another agent, or its own channel item. Only the session board and my items are covered.
- Likely bug: an item whose maker is archived can never again be changed without an ask (`daemon/access/src/policy.rs:766`).
