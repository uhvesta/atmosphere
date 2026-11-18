# An agent's ask is answered on the item

Layer: verify (Board, the person). Platforms: macOS.

## Starting state

- `#payments` has PAY-1 "Schema", In progress. I made it.
- A session "Ledger" runs in `#payments` and is on PAY-1. Its agent has just run `humata --wait 600 board move PAY-1 done`. I made the item, so the move asks me.
- The app is open on the `#payments` board. The Inbox badge shows 1.

## Reproduce headless

- Against a real daemon, with clicks: `bazel test //apps/humata2/desktop:live_boards_test --test_filter=the_items_drawer`. A scripted agent runs the real `humata board move` in its tab and waits. It matches `approving_an_agents_move_from_the_items_drawer_moves_it_and_returns_the_call` and `denying_an_agents_move_from_the_items_drawer_keeps_it_in_progress`.
- Ask wording on fakes: `bazel test //apps/humata2/features/board:drawer_test --test_filter=the_ask_names_the_change_it_waits_on`.
- By hand on the rig, no app: in the session tab run `humata --json board move PAY-1 done` (exit 12, prints the request id). As the person: `humata inbox`, `humata inbox approve <id>`, then `humata --json board --channel payments`. Keep each stdout and exit code.
- Not headless: nothing for the drawer. The same answer from a notification or from a phone is out of scope.

## Steps

1. **I do:** look at the board.
   **I see:** PAY-1's card is tinted orange, with a line about the ask and **Review**.
   **State and events:** InboxRequest created (Pending). `BoardItem.pending` holds the ask, for the user only. The app re-reads the item when an ask comes or goes.
   **Status:** Works (real daemon). `desktop/tests/live_boards.rs::approving_an_agents_move_…`; `daemon/humatad/tests/boards.rs::an_agents_move_waits_and_applies_on_approval_and_only_the_user_sees_it_pending`.

2. **I do:** click **Review**.
   **I see:** the drawer, with an orange box: "<agent> asks to move this to Done", the agent's reason in quotes if it gave one, and **Move to Done**, **Keep in progress**, **Open session**.
   **State and events:** none yet.
   **Status:** Works (real daemon). Same test. Frame 22 on fakes.

3. **I do:** click **Move to Done**.
   **I see:** the box goes. PAY-1 is in Done. The Inbox badge drops to 0.
   **State and events:** the request is answered Approved. The daemon runs the held move. InboxRequest updated (Approved); BoardItem updated (Done). The agent's `humata` call exits 0 and prints the moved item.
   **Status:** Works (real daemon). Same test; it reads the agent's exit code and stdout.

4. **I do:** (a second run) click **Keep in progress** instead.
   **I see:** the box goes. PAY-1 stays In progress.
   **State and events:** answered Denied. The agent's call ends with a denial, exit 4. Nothing is written to the item.
   **Status:** Works (real daemon). `::denying_an_agents_move_from_the_items_drawer_keeps_it_in_progress`.

5. **I do:** (a third run) answer the same request from its Inbox card instead of the drawer.
   **I see:** the same result. The drawer's box goes too.
   **State and events:** one request, two doors, first answer wins.
   **Status:** Works (real daemon) for one answer only: `daemon/humatad/tests/inbox.rs::a_request_takes_only_its_first_answer`. Both doors on one request in the app: Code, no test.

6. **I do:** the agent asks to rename PAY-1 (an edit ask). I open the drawer.
   **I see:** "<agent> asks to edit this", each field as before → after, **Apply edit** and **Keep as is**.
   **Status:** Fakes only. `features/board/tests/drawer.rs::the_ask_names_the_change_it_waits_on`. Unproven live: an edit ask approved from the drawer.

7. **I do:** the agent asks to delete PAY-1. I open the drawer.
   **I see:** "<agent> asks to delete this", **Delete item** (red) and **Keep item**.
   **Status:** Fakes only. Same test. Unproven live.

## Branches

### The item changed before I approved

When I renamed the item after the agent asked to rename it. Approving runs the write fresh; it ends as a revision conflict for the agent, and my title stays.
**Status:** Code, no test. D155 says so; `daemon/humatad/src/board_service.rs:858` (`attempt` with `approved`). I found no test that approves a stale ask.

### The item is deleted while an ask waits

When I run `humata board rm PAY-1`. The ask is withdrawn and the agent's call ends.
**Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::deleting_an_item_withdraws_the_requests_about_it`.

### The daemon restarts while an ask waits

When humatad2 restarts. The ask is gone, the card loses its tint, and the agent's waiting call ends. The agent must ask again.
**Status:** Code, no test. D155: "Asks don't survive a daemon restart". Reboots are meant to lose nothing, so this is a gap.

### The agent stops waiting

When the agent's call was made with `--json` and no `--wait`. It returns pending at once (exit 12) and the ask stays answerable. Approving still moves the item (TP-13).
**Status:** Works (real daemon) for MCP: `daemon/mcp/tests/waiting.rs::a_request_that_returned_pending_survives_the_agent_exiting` (fake daemon), and the held call in `boards.rs` above.

## Expected outcomes

- [ ] A card and a table row with an ask offer **Review**.
- [ ] The drawer names who asks, for what, and why.
- [ ] Approving applies the change and returns it to the agent; denying changes nothing and returns a denial.
- [ ] The Inbox card and the drawer answer the same request.
- [ ] Deleting the item withdraws its asks.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Card shows the ask | Works (real daemon) | `live_boards.rs::approving_…` |
| 2 Review opens the box | Works (real daemon) | same |
| 3 Move to Done | Works (real daemon) | same |
| 4 Keep in progress | Works (real daemon) | `live_boards.rs::denying_…` |
| 5 From the Inbox card | Works (real daemon) for one answer | `inbox.rs::a_request_takes_only_its_first_answer` |
| 6 Edit ask | Fakes only | `drawer.rs::the_ask_names_the_change_it_waits_on` |
| 7 Delete ask | Fakes only | same |
| Stale approve | Code, no test | `board_service.rs:858` |
| Item deleted | Works (real daemon) | `boards.rs::deleting_an_item_withdraws_…` |
| Daemon restart | Code, no test | D155 |
| Agent not waiting | Works (real daemon), partly fakes | `waiting.rs` |

## Gaps and likely bugs

- Likely bug: the card's line is fixed text, "Asks to move to Done", for every ask: an edit, a delete, a move to In review (`ui/src/board.rs:421`, `ui/src/panel_board.rs:170`). The drawer's box is right. Confirm with a move-to-In-review ask and read the card.
- Likely bug: an ask does not survive a daemon restart (D155). The person sees the card vanish with no word, and the agent's call just ends.
- Not built: a create ask on the board. An agent's new Global item waits in the Inbox only; no board shows it.
- The reason line is empty when the agent used the `humata` command, which can't send one (case 13).
- Edit and delete asks are proven only on fakes.
