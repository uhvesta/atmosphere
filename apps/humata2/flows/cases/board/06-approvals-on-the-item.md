# An agent's ask is answered Yes or No on the item

Layer: verify (To-do, the person). Platforms: macOS.

In BOARDSIMPLE (D830–D874, D846), an agent's pending ask about a To-do item (closing it, editing a field, or suggesting a new item) stays out of the **Agent inbox** (`ListRequestsRequest.agent_inbox = true` filters it out) and sorts into **Needs attention** at the top of **To-do** (and opens directly in the session's **To-do** panel when that session has one pending ask).

## Starting state

- `#payments` has `PAY-1` "Schema", made by the person.
- A session "Ledger" runs in `#payments`. Its agent has just run `humata --json --wait 600 board move PAY-1 done` (or MCP `todo_close`). Because the person made the item (and `CAPABILITY_TODO_CLOSE` defaults to **Ask**), the call waits for the person's answer.
- The app is open on **To-do**.

## Reproduce headless

- Against a real daemon, clicking `todo-row <key>` then `todo-yes` / `todo-no`: `bazel test //apps/humata2/desktop:live_flows_test --test_filter=todo_asks`. A real `humata --json --wait 600 board move` runs as the session's agent and waits for the drawer click. Covers `approving_an_agents_ask_from_the_todo_drawer_closes_the_item_and_returns_the_call` and `denying_an_agents_ask_from_the_todo_drawer_keeps_the_item_open_and_fails_the_call`.
- Native window click on fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=approving_an_items_ask_answers_it_and_leaves_the_other_asks`.
- Reducer and ask-card rendering on fakes: `bazel test //apps/humata2/features/todo:todo_test //apps/humata2/ui/workspace:workspace_unit_test`.
- Daemon ask detail population and pending visibility: `bazel test //apps/humata2/daemon/svc_board:svc_board_unit_test --test_filter=domain_item_fills_ask_detail_from_the_inbox_request` and `bazel test //apps/humata2/daemon/humatad:boards_test --test_filter=an_agents_move_waits`.
- By hand on the rig, no app: in the session tab run `humata --json board move PAY-1 done` (exit 12, prints the request id). As the person: `humata inbox`, `humata inbox approve <id>`, then `humata --json board --channel payments`. Keep each stdout and exit code.
- Not headless: nothing for the drawer.

## Steps

1. **I do:** look at **To-do**.
   **I see:** `PAY-1` is in **Needs attention** at the top of the list with an amber `Needs attention · close?` pill (or `new due date?` / `edit?` / `add?` depending on the ask kind).
   **State and events:** `InboxRequest` created (Pending). `BoardItem.pending` holds the ask for the user only. `LiveTodo::watch` subscribes to `Boards` and `Inbox` and pushes the updated list.
   **Status:** Works (real daemon). `desktop/tests/live_flows/todo_asks.rs::approving_an_agents_ask_from_the_todo_drawer_closes_the_item_and_returns_the_call`; `daemon/humatad/tests/boards.rs::an_agents_move_waits_and_applies_on_approval_and_only_the_user_sees_it_pending`.

2. **I do:** click `PAY-1` (`todo-row PAY-1`).
   **I see:** the drawer opens with the amber ask card on top: the question ("Close Schema?"), the agent's reason, a white detail box with **Evidence** (clickable links), **Change** (`before → after`), **Also** (what else it affects), and **Asked by** (`<agent> · <time ago>`), followed by **Yes** (`todo-yes`), **No** (`todo-no`), **Ask why** (or **Resume the session to ask why** if the asking session ended), and **Open session**.
   **State and events:** `TodoAction::Open(key)`. `LiveTodo::list_all` enriches pending asks with `ask_detail` from `InboxService.ListRequests`.
   **Status:** Works (real daemon) for the ask card and buttons: `desktop/tests/live_flows/todo_asks.rs`. Detail fields and links: `daemon/svc_board` unit test `domain_item_fills_ask_detail_from_the_inbox_request` and `ui/workspace/src/todo.rs` unit tests (`the_ask_card_lists_evidence_change_also_and_asker_in_order`, `an_ended_session_cannot_be_asked_why`, `evidence_links_produce_open_evidence_actions`, `only_web_evidence_opens_in_the_browser`).

3. **I do:** click **Yes** (`todo-yes`).
   **I see:** the ask card clears, `PAY-1` closes and leaves **Needs attention**, and no pending request remains on the daemon.
   **State and events:** `TodoClient::answer(ask_id, true)` calls `InboxService.AnswerRequest(Approve)`. The daemon runs the held change and updates `PAY-1` to `Closed`. The agent's waiting `humata` call exits 0 and prints the moved item. Other items' asks stay untouched.
   **Status:** Works (real daemon). `desktop/tests/live_flows/todo_asks.rs::approving_an_agents_ask_from_the_todo_drawer_closes_the_item_and_returns_the_call`; isolation across asks: `desktop/tests/native/todo_asks.rs::approving_an_items_ask_answers_it_and_leaves_the_other_asks`, `features/todo/tests/todo.rs::answering_yes_clears_needs_attention`.

4. **I do:** (a second run) click **No** (`todo-no`) instead.
   **I see:** the ask card clears, `PAY-1` stays open, and no pending request remains on the daemon.
   **State and events:** `TodoClient::answer(ask_id, false)` calls `InboxService.AnswerRequest(Deny)`. The agent's waiting `humata` call exits 4 (denied). Nothing is written to the item.
   **Status:** Works (real daemon). `desktop/tests/live_flows/todo_asks.rs::denying_an_agents_ask_from_the_todo_drawer_keeps_the_item_open_and_fails_the_call`.

5. **I do:** (a third run) open the session's **To-do** panel while the session has one pending ask.
   **I see:** the panel opens straight onto that item's drawer with **Yes**, **No** and **Ask why** beside the running terminal (frame 191 / native frames 07 and 12).
   **State and events:** `todo_panel::open_key` picks the sole asking item in the session.
   **Status:** Fakes only. `ui/workspace/src/todo_panel.rs::tests::a_session_with_one_question_opens_it`; frames 07 and 12 in `desktop/tests/native/session.rs`.

6. **I do:** an agent suggests a new item (`todo_add`). I see it in **Needs attention** marked `Suggested` with **Accept** / **Dismiss**.
   **I see:** it has no rank number until I say **Accept** (`AcceptSuggestion`), and **Dismiss** closes it.
   **State and events:** `TodoService.CreateTodo` from an agent sets `suggested: true`; `UpdateTodo` with `accept_suggestion: true` (user only) promotes it to the ranked list.
   **Status:** Works (real daemon) in `svc_board`: `daemon/svc_board:svc_board_unit_test`. App UI/reducer: Fakes only (`ui/workspace/src/todo.rs`, `features/todo/src/lib.rs`).

## Branches

### The answer RPC fails

When `TodoClient::answer` returns an error. The reducer puts the ask back on the item and sets `state.error` so the person sees why.
**Status:** Fakes only. `features/todo/tests/todo.rs::a_failed_answer_puts_the_ask_back_and_says_so`.

### The item changed before I approved

When I edited the item after the agent asked. Approving runs the write fresh; it ends as a revision conflict for the agent, and my change stays.
**Status:** Code, no test. D155; `daemon/svc_board` replays with `approved = true`.

### The item is deleted while an ask waits

When I run `humata board rm PAY-1`. The ask is withdrawn and the agent's call ends.
**Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::deleting_an_item_withdraws_the_requests_about_it`.

### The daemon restarts while an ask waits

When `humatad2` restarts. The ask is gone, the item leaves Needs attention, and the agent's waiting call ends. The agent must ask again.
**Status:** Code, no test. D155: "Asks don't survive a daemon restart".

### The agent stops waiting

When the agent's call was made with `--json` and no `--wait` (or MCP `wait_seconds: 0`). It returns pending at once (exit 12) and the ask stays answerable on the item. Approving still applies it (TP-13).
**Status:** Works (real daemon) for the held call in `boards.rs`; MCP side: Fakes only, `daemon/mcp/tests/waiting.rs::a_request_that_returned_pending_survives_the_agent_exiting`.

## Expected outcomes

- [ ] An item with an agent's pending ask sorts into **Needs attention** with a kind-specific pill (`close?`, `new due date?`, `add?`).
- [ ] The drawer's ask card shows the question, reason, evidence, before → after change, what else it affects, who asked, and **Yes** / **No**.
- [ ] Clicking **Yes** applies the change, clears the request, and returns success to the waiting agent; clicking **No** leaves the item unchanged, clears the request, and returns denial to the agent.
- [ ] Answering one item's ask leaves other items' asks intact.
- [ ] Deleting the item withdraws its asks.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Item in Needs attention | Works (real daemon) | `live_flows/todo_asks.rs`, `boards.rs::an_agents_move_waits_…` |
| 2 Drawer ask card & detail | Works (real daemon) for card; unit-tested for detail/links | `live_flows/todo_asks.rs`, `svc_board_unit_test`, `workspace_unit_test` |
| 3 Yes closes & returns call | Works (real daemon) | `live_flows/todo_asks.rs`, `native/todo_asks.rs` |
| 4 No keeps open & fails call | Works (real daemon) | `live_flows/todo_asks.rs` |
| 5 Session To-do panel auto-open | Fakes only | `todo_panel.rs` tests, native frames 07 & 12 |
| 6 Suggested item Accept/Dismiss | Works in `svc_board`; UI Fakes only | `svc_board_unit_test` |
| Answer error rollback | Fakes only | `features/todo/tests/todo.rs::a_failed_answer_puts_the_ask_back_and_says_so` |
| Stale approve | Code, no test | D155 |
| Item deleted | Works (real daemon) | `boards.rs::deleting_an_item_withdraws_…` |
| Daemon restart | Code, no test | D155 |
| Agent not waiting | Works (real daemon) in daemon; MCP Fakes only | `waiting.rs`, `boards.rs` |

## Gaps and likely bugs

- Likely bug: an ask does not survive a daemon restart (D155). The ask vanishes with no notice and the agent's waiting call ends.
- The reason line is empty when the agent used `humata board move`, which cannot pass `--reason` yet (case 13).
- Clicking **No** in the To-do drawer sends an empty denial note (no note input on the To-do ask card yet).
- Field-edit asks and suggested-item Accept/Dismiss are proven on fakes and daemon unit tests, not yet in a live UI flow.
