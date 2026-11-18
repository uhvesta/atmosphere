# 10. The Board: a person and agents plan and track work

> "I want to see what everyone is working on, what waits on what, and what waits on me. And the agents should keep it up to date themselves."

The Board is Humata's small issue tracker. It holds items in columns, the order between them, and which sessions work on what. I plan on it. Agents plan on it too, through the `humata` command and Humata's MCP tools, and they ask me before they touch what isn't theirs.

How to read this file, and what the tags mean: [README](README.md). The finer cases, one per file, are in [cases/board/](cases/board/README.md).

## Starting state

- `#payments` exists with `acme/api`, on This Mac. Its board is empty, so the next item is PAY-1.
- One session, "Ledger", runs in `#payments` with an agent.
- The Global board holds one item of mine, GLB-1 "Q4 plan".
- Agent access is at its defaults: an agent may do anything on its own session board, add to its channel's board and change the channel items it made. Changing someone else's item, or anything on the Global board, asks me.

## Steps

### I plan

1. **Do:** open `#payments`, then its **Board** tab. Click **+** on To do and type "Schema". Do the same for "Write path" and "Cut over".
   **See:** three cards in To do: PAY-1, PAY-2, PAY-3.
   **Expect:**
   - Each item gets the channel's key and the next number. Numbers are never reused.
   - Events: BoardItem created ×3.
   **Status: Works.** `desktop/tests/live_boards.rs::new_items_get_the_channels_key_and_a_dependency_is_set_from_the_picker`; keys in `daemon/humatad/tests/boards.rs::keys_count_up_per_channel_and_a_taken_prefix_lengthens`. A **+** on a column other than To do makes the item and then moves it, in two calls (case 1).

2. **Do:** open "Cut over". Under **Depends on**, click **+** and pick PAY-1, then PAY-2.
   **See:** the drawer lists both. The picker never offers the item itself or one that would make a loop.
   **Expect:**
   - A loop is refused, and the refusal names it.
   - A dependency blocks nothing. I can still move Cut over to Done first.
   - Events: BoardItem updated ×2.
   **Status: Works.** Same live test; loops in `daemon/humatad/tests/boards.rs::a_dependency_that_closes_a_loop_is_refused_with_the_loop`.

3. **Do:** switch between **Board**, **Table** and **Dashboard**.
   **See:** the same three items in each. The Dashboard counts them, and each figure opens what it counts.
   **Status: Works.** `desktop/tests/live_boards.rs::the_three_views_show_the_same_items_and_a_dashboard_figure_opens_what_it_counts`. A **Graph** view that draws the depends-on edges is **Not built, no milestone** (case 4). The edges show only as the drawer's list.

4. **Do:** on "Schema", click **Start a session for this item** and start it.
   **See:** the new session's page. On the board, Schema is In progress and names the session.
   **Expect:** Events: Session created; BoardItem updated (In progress, a session added).
   **Status: Works.** `desktop/tests/live_boards.rs::starting_a_session_for_a_to_do_item_opens_it_and_moves_the_item_in_progress`.

### The agent plans

5. **Do:** in the Ledger session, tell the agent: "Break 'Write path' into steps on your board, and say what depends on what."
   **See:** the agent runs Humata's board tools. New items appear on the session's board without asking me.
   **Expect:**
   - The agent knows the Board exists and how to reach it, without me naming a command.
   - Items on its own board need no approval.
   - Events: BoardItem created, by the agent.
   **Status:**
   - The tools **Work** against a real daemon: `daemon/humatad/tests/board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it` (MCP) and `desktop/tests/live_boards.rs` (the real `humata board move`).
   - The agent knowing about them is **Not built, no milestone.** No agent is started with Humata's MCP server, no skill mentions the Board, and `humata` is not put on the agent's `PATH` (case 9). Today I have to tell the agent the command.

6. **Do:** tell it: "Add 'Backfill' to the channel board, after Write path."
   **See:** PAY-4 on the channel board, depending on PAY-2. No ask: adding to its channel's board is allowed, and the item is its own.
   **Status: Works** in the daemon's rules: `daemon/access/tests/refusals.rs::board_items_an_agent_made_are_its_to_edit`. No test runs this exact add on a channel board through MCP or the CLI on a real daemon: **Code, no test** (cases 11 and 12).

### The agent asks, I answer

7. **Do:** the agent finishes Schema, which I made, and moves it to Done.
   **See:** the move waits. The card turns orange with **Review**. The drawer says "Copilot asks to move this to Done", with **Move to Done**, **Keep in progress** and **Open session**.
   **Expect:**
   - Nothing moves until I answer.
   - Events: InboxRequest created (Pending).
   **Status: Works.** `desktop/tests/live_boards.rs::approving_an_agents_move_from_the_items_drawer_moves_it_and_returns_the_call`. The agent's reason is shown only when it came through MCP. The `humata` command has no way to give one (case 13).

8. **Do:** click **Move to Done**.
   **See:** Schema is in Done. The agent's waiting call returns with the moved item.
   **Do (other case):** click **Keep in progress**. The agent's call returns as denied, and the item stays.
   **Expect:** Events: InboxRequest updated (Approved or Denied); BoardItem updated (Done).
   **Status: Works.** The same test, and `::denying_an_agents_move_from_the_items_drawer_keeps_it_in_progress`.

9. **Do:** the agent adds "Tell finance" to the Global board.
   **See:** an Inbox card for a new Global item, with **Allow**, **Deny** and an **Always** offer for that board.
   **Expect:** the item exists only after I allow it.
   **Status: Works** in the daemon: `daemon/humatad/tests/board_tools.rs::creating_on_the_global_board_waits_for_the_user` and `daemon/access/tests/global_board.rs::an_always_allow_covers_later_writes_on_that_board_only`. A create ask shows in the Inbox only, never on a board.

### I track

10. **Do:** open the sidebar's **Board**, then **Dashboard**.
    **See:** **Waiting on you**, **In progress**, **Done this week**, **Unassigned**, a bar per channel, done per day, and the sessions that hold items.
    **Do:** click **Waiting on you**.
    **See:** the Table, showing only the items with an ask.
    **Status: Works** on fakes: `features/board/tests/presentation.rs::dashboard_drills_show_exactly_the_counted_items`. One figure is clicked against a real daemon in the live test of step 3.

11. **Do:** in a terminal, run `humata board --channel payments` and `humata board history PAY-1`.
    **See:** the same items as the app, and who changed PAY-1 and when.
    **Status: Works** in the daemon: `daemon/humatad/tests/boards.rs::history_records_what_happened_in_order_with_its_kind`. The command's own tests use a fake daemon (`cli/tests/boards.rs`).

## Expected outcome

- I planned three items and their order. An agent added its own steps without asking, and asked before it touched mine.
- Every change is in the item's history, with who made it.
- The app, the `humata` command and the MCP tools show the same board.

## Gaps this flow shows

- **Agents are not told the Board exists.** Not built, no milestone. Humata starts no agent with its MCP server (`daemon/sessions/src/agents.rs:572`, and the Copilot plugin holds hooks only, `daemon/agent_hooks/src/lib.rs:373`), ships no Board skill (`skills/`), and its MCP greeting never names the Board (`daemon/mcp/src/server.rs:552`).
- **No Graph view.** Not built, no milestone. `ui/src/board.rs:256` offers Board, Table and Dashboard only.
- **I can't rename, describe or delete an item in the app.** Not built (D177 leaves the editor to the owner). `features/board/src/lib.rs:439` has no such action. `humata board edit` and `rm` do it.
- **The drawer doesn't say who owns the item.** Not built (BD-3). The app holds `created_by` (`clients/src/model.rs:1467`) and never draws it.
- **`humata board` can't give a reason, read one item or guard a write with a revision.** `cli/src/commands/boards.rs:84`, `:112`.
- **The card's ask line is fixed text.** Likely bug: "Asks to move to Done" for every ask (`ui/src/board.rs:421`).
