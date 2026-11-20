# 10. To-do (formerly The Board): a person and agents plan and track work

> **Note (BOARDSIMPLE, D830–D874):** The Board (kanban columns, Table, Dashboard) was simplified to **To-do**: a single ranked list across all channels and sessions, with channel and session filters, three states (`Open`, `In progress`, `Closed`), `Waits on` dependencies (folding waiting items), and a **Needs attention** group where agent asks are answered **Yes** / **No**. The daemon still serves both `TodoService` and `BoardService` over the same items, and agents can use both `todo_*` and `board_*` MCP tools and `humata board`. Full up-to-date To-do cases live in [cases/board/](cases/board/README.md).

> "I want to see what everyone is working on, what waits on what, and what waits on me. And the agents should keep it up to date themselves."

To-do is Humata's lightweight work tracker. It holds items in a single ranked list, what waits on what, and which sessions work on what. I plan on it. Agents plan on it too, through the `humata` command and Humata's MCP tools, and they ask me before they touch what isn't theirs.

How to read this file, and what the tags mean: [README](README.md). The finer cases, one per file, are in [cases/board/](cases/board/README.md).

## Starting state

- `#payments` exists with `acme/api`, on This Mac. Its board is empty, so the next item is PAY-1.
- One session, "Ledger", runs in `#payments` with an agent.
- The Global board holds one item of mine, GLB-1 "Q4 plan".
- Agent access is at its defaults: an agent may do anything on its own session board, add to its channel's board and change the channel items it made. Changing someone else's item, or anything on the Global board, asks me.

## Steps

### I plan

1. **Do:** open `#payments`, then its **To-do** tab (formerly Board). Add "Schema", "Write path", and "Cut over".
   **See:** three items in the ranked list: PAY-1, PAY-2, PAY-3.
   **Expect:**
   - Each item gets the channel's key and the next number. Numbers are never reused.
   - Events: BoardItem created ×3.
   **Status: Works.** `features/todo/tests/todo.rs`, `desktop/tests/native/todo_asks.rs::a_channels_todo_tab_lists_only_that_channels_items`, and `desktop/tests/design/todo.rs` / `desktop/tests/design/todo_more.rs`; keys in `daemon/humatad/tests/boards.rs::keys_count_up_per_channel_and_a_taken_prefix_lengthens` (case 1).

2. **Do:** open "Cut over". Under **Waits on** (Depends on), pick PAY-1, then PAY-2.
   **See:** the drawer lists both, and the waiting item folds into the Waiting group until its dependencies close. The picker never offers the item itself or one that would make a loop.
   **Expect:**
   - A loop is refused, and the refusal names it.
   - Events: BoardItem updated ×2.
   **Status: Works.** `features/todo/tests/todo.rs` and `desktop/tests/design/todo_more.rs`; loops in `daemon/humatad/tests/boards.rs::a_dependency_that_closes_a_loop_is_refused_with_the_loop` (case 2).

3. **Do:** filter the ranked **To-do** list by channel, session, or saved filter (replacing the old Board / Table / Dashboard switcher).
   **See:** the matching items in priority order.
   **Status: Works.** `features/todo/tests/todo.rs`, `desktop/tests/native/todo_asks.rs::a_channels_todo_tab_lists_only_that_channels_items`, and `desktop/tests/design/todo.rs` / `desktop/tests/design/todo_more.rs` (case 3). A **Graph** view that draws the depends-on edges is **Not built, no milestone** (case 4).

4. **Do:** on "Schema", click **Start a session on this** and start it.
   **See:** the new session's page. On To-do, Schema is `In progress` and names the session.
   **Expect:** Events: Session created; BoardItem updated (In progress, a session added).
   **Status: Works.** `desktop/tests/live_flows/todo_asks.rs::starting_a_session_from_the_todo_drawer_opens_it_and_moves_the_item_in_progress` and `desktop/tests/native/todo_asks.rs::starting_a_session_from_the_todo_drawer_opens_the_prefilled_new_session_dialog`.

### The agent plans

5. **Do:** in the Ledger session, tell the agent: "Break 'Write path' into steps on your board, and say what depends on what."
   **See:** the agent runs Humata's `todo_*` / `board_*` tools. New items appear on the session's board without asking me.
   **Expect:**
   - The agent knows To-do / the Board exists and how to reach it, without me naming a command.
   - Items on its own board need no approval.
   - Events: BoardItem created, by the agent.
   **Status:**
   - The tools **Work** against a real daemon: `daemon/humatad/tests/board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it` (MCP) and `desktop/tests/live_flows/todo_asks.rs` (the real `humata` command).
   - See [cases/board/09-agent-learns-the-board-exists.md](cases/board/09-agent-learns-the-board-exists.md) for current agent discovery status.

6. **Do:** tell it: "Add 'Backfill' to the channel board, after Write path."
   **See:** PAY-4 on the channel board, depending on PAY-2. No ask: adding to its channel's board is allowed, and the item is its own.
   **Status: Works** in the daemon's rules: `daemon/access/tests/refusals.rs::board_items_an_agent_made_are_its_to_edit`. See cases 11 and 12 in [cases/board/](cases/board/README.md).

### The agent asks, I answer

7. **Do:** the agent finishes Schema, which I made, and asks to close it.
   **See:** the ask waits under **Needs attention**. The drawer shows the ask card with **Yes**, **No**, and **Open session**.
   **Expect:**
   - Nothing moves until I answer.
   - Events: InboxRequest created (Pending).
   **Status: Works.** `desktop/tests/live_flows/todo_asks.rs::approving_an_agents_ask_from_the_todo_drawer_closes_the_item_and_returns_the_call` and `desktop/tests/native/todo_asks.rs::approving_an_items_ask_answers_it_and_leaves_the_other_asks` (case 6 and case 13).

8. **Do:** click **Yes**.
   **See:** Schema is `Closed`. The agent's waiting call returns with the updated item.
   **Do (other case):** click **No**. The agent's call returns as denied, and the item stays open.
   **Expect:** Events: InboxRequest updated (Approved or Denied); BoardItem updated (Closed).
   **Status: Works.** `desktop/tests/live_flows/todo_asks.rs::approving_an_agents_ask_from_the_todo_drawer_closes_the_item_and_returns_the_call` and `::denying_an_agents_ask_from_the_todo_drawer_keeps_the_item_open_and_fails_the_call`.

9. **Do:** the agent adds "Tell finance" to the Global board.
   **See:** an Agent inbox card for a new Global item, with **Allow**, **Deny** and an **Always** offer for that board.
   **Expect:** the item exists only after I allow it.
   **Status: Works** in the daemon: `daemon/humatad/tests/board_tools.rs::creating_on_the_global_board_waits_for_the_user` and `daemon/access/tests/global_board.rs::an_always_allow_covers_later_writes_on_that_board_only`.

### I track

10. **Do:** open the sidebar's **To-do** and check **Needs attention** and channel/session filters (replacing the old Board › Dashboard).
    **See:** items needing attention at the top, followed by ranked open/in-progress items, with channel chips and filters.
    **Status: Works.** `features/todo/tests/todo.rs`, `desktop/tests/design/todo.rs`, and `desktop/tests/design/todo_more.rs` (cases 3 and 6).

11. **Do:** in a terminal, run `humata board --channel payments` and `humata board history PAY-1`.
    **See:** the same items as the app, and who changed PAY-1 and when.
    **Status: Works** in the daemon: `daemon/humatad/tests/boards.rs::history_records_what_happened_in_order_with_its_kind`. The command's own tests use a fake daemon (`cli/tests/boards.rs`).

## Expected outcome

- I planned three items and their order. An agent added its own steps without asking, and asked before it touched mine.
- Every change is in the item's history, with who made it.
- The app, the `humata` command and the MCP tools show the same board.

## Gaps this flow shows

- **No Graph view.** Not built, no milestone (BOARDSIMPLE keeps one ranked list with a folded Waiting group; see [cases/board/04-graph-view.md](cases/board/04-graph-view.md)).
- **`humata board` can't give a reason, read one item or guard a write with a revision.** `cli/src/commands/boards.rs:84`, `:112`.
- See [cases/board/README.md](cases/board/README.md) for the current To-do gap list (drawer editing, Needs attention asks, and agent discovery were updated in BOARDSIMPLE, D830–D874).
