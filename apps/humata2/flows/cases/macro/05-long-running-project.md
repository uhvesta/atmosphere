# A long-running project goes from spec to board items to sessions on two machines, through a reboot

Layer: macro. Platforms: macOS, Linux, web. (The keys shown are the macOS ones; Linux uses `Ctrl`.)

## Starting state

- Humata 2 is open on This Mac. The daemon is running. This Mac is the host.
- Machines: This Mac (host, online) and box-1 (Linux, online, linked to This Mac). box-2 is not added yet.
- A channel named `payments` exists, with `acme/api` as its repo. Its key prefix is `PAY`.
- The channel board has one item, `PAY-1 Write the cache spec`, in To do. I wrote it.
- Nothing is running. The Inbox is empty.
- I am holding the spec as a text file. I have not started any agent yet.

## Steps

1. **I do:** Open the `payments` board. Press `New item`, type "Add retry to the uploader", choose the To do column and press Return.
   **I see:** A card at the top of To do with the key `PAY-2`. The card shows the channel's name.
   **State and events:** The item is stored on the channel's board, not on a session. Its key uses the channel prefix and the next free number. New items go to the top of their column.
   **Status:** Works (real daemon). `clients/live/tests/boards.rs::a_channel_item_gets_the_channels_key_and_is_listed_in_to_do` creates the item through the live client. The `New item` button and its form Work (real daemon): `desktop/tests/live_boards.rs::new_items_get_the_channels_key_and_a_dependency_is_set_from_the_picker`. Fake-client version: `desktop/tests/native/board.rs::board_creates_in_selected_column_and_keeps_channel_scope`.

2. **I do:** Start a session for `PAY-1` on This Mac with the button "Start a session for this item". Nothing else.
   **I see:** A session tab opens. `PAY-1` moves from To do to In progress, and the drawer lists the session under Sessions.
   **State and events:** The item gets the session id. Its status changes from To do to In progress.
   **Status:** Works (real daemon). `clients/live/tests/boards.rs::a_session_started_for_a_to_do_item_works_on_it_and_moves_it_in_progress`. The start button Works (real daemon): `desktop/tests/live_boards.rs::starting_a_session_for_a_to_do_item_opens_it_and_moves_the_item_in_progress`. Fake-client version: `features/app/tests/board_start.rs::starting_a_session_for_a_to_do_item_adds_it_and_starts_the_item`.

3. **I do:** The agent in the session makes an item on its own session board: "Check the cache key format". It does this through the `humata` board tools.
   **I see:** The item appears in the session's panel, under the session's own items.
   **State and events:** The item is stored with the session as its owner. The agent may change its own item freely; others must ask me (SPEC BD-3).
   **Status:** Works (real daemon). `daemon/humatad/tests/board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it` drives `humatad2 mcp` over stdio against a real daemon.

4. **I do:** Nothing. The agent tries to add an item to the global board, which only I can change.
   **I see:** An Inbox request appears: the agent's name, the item's title, and buttons to approve or deny. The agent's call waits.
   **State and events:** A pending request is recorded against the item. The agent's call does not return until I answer.
   **Status:** Works (real daemon). `daemon/humatad/tests/board_tools.rs::creating_on_the_global_board_waits_for_the_user` and `daemon/humatad/tests/boards.rs::an_agents_write_on_the_global_board_waits_in_the_inbox_until_approved`.

5. **I do:** Open `PAY-2` in the drawer. Under Depends on, press Add dependency and pick `PAY-1 Write the cache spec`.
   **I see:** `PAY-1 Write the cache spec` appears as a row under Depends on. `PAY-2` itself and any item that would close a loop are not offered in the picker.
   **State and events:** An edge is recorded from `PAY-2` to `PAY-1`. The edge does not change either item's status. When `PAY-1` is Done, its row shows a tick.
   **Status:** Works (real daemon). `clients/live/tests/boards.rs::a_dependency_is_listed_refuses_a_loop_and_can_be_removed` adds and removes an edge through the live client. The picker Works (real daemon), including leaving out the item itself and one that would close a loop: `desktop/tests/live_boards.rs::new_items_get_the_channels_key_and_a_dependency_is_set_from_the_picker`. Fake-client version of the filtering: `features/board/tests/drawer.rs::the_dependency_picker_leaves_out_itself_existing_edges_and_loops`.

6. **I do:** Look for a graph of the dependencies, the way a project's milestones might be drawn as a graph.
   **I see:** There is no graph view. Dependencies are a list in each item's drawer. Each row is one item, with its title and column.
   **State and events:** No graph is stored or drawn. The edges are only the list the drawer reads.
   **Status:** Not built (no milestone). SPEC BD-14 describes a list under Depends on, not a graph, and no milestone in `workflow/milestones/` asks for one.

7. **I do:** Ask the agent to add `PAY-3` as depending on `PAY-2`, and `PAY-2` to depend on `PAY-3`, so that the chain loops.
   **I see:** The second edit is refused. The refusal names the loop, `PAY-2 → PAY-3 → PAY-2`.
   **State and events:** No edge is written. Both items are unchanged.
   **Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::a_dependency_that_closes_a_loop_is_refused_with_the_loop`. The agent's tool path is in `daemon/humatad/tests/board_tools.rs::a_dependency_that_would_close_a_loop_is_refused`.

8. **I do:** Let the agent work for an hour. Each time it moves its own items, I see the board change without reloading.
   **I see:** The board updates on its own as items move. A card that moves shows in its new column.
   **State and events:** Each item change is pushed to the app as an item change event.
   **Status:** Works (real daemon). `clients/live/tests/boards.rs::a_move_after_subscribing_is_pushed_as_an_item_change`.

9. **I do:** Open the `payments` dashboard and press the figure for In progress.
   **I see:** A list of exactly the items the figure counts. Each item is one the dashboard counted, and no other.
   **State and events:** The dashboard reads the same items the list reads.
   **Status:** Works (real daemon). `desktop/tests/live_boards.rs::the_three_views_show_the_same_items_and_a_dashboard_figure_opens_what_it_counts` shows the same items in Board, Table and Dashboard and opens exactly the items a figure counts. Fake-client version: `features/board/tests/presentation.rs::dashboard_drills_show_exactly_the_counted_items`.

10. **I do:** Open the session panel for the session on `PAY-1` and look at its board.
    **I see:** The panel shows the channel's board, with this session's own items first, then the rest of the channel's items.
    **State and events:** The panel reads the channel's items, with the session's items first.
    **Status:** Fakes only since the board page went (D874): the panel is the session's To-do panel, `desktop/tests/panels.rs::board_lists_the_channels_items_and_expands_without_losing_the_session`. The listing of session items from the daemon is Works (real daemon): `clients/live/tests/boards.rs::a_session_board_item_is_listed_on_that_sessions_board`.

11. **I do:** Press Move on the session that works on `PAY-1` and pick box-1. Repeat for a second heavy session on `PAY-4`. The shortcut is ⌥⌘M, with a session open.
    **I see:** The Move dialog shows the plan: the branch, the size, and what stays on This Mac until the move finishes. Each session's terminal is read-only while it moves. When each move ends, the session shows as running on box-1.
    **State and events:** Each move is an operation. The session's machine changes to box-1 only after the agent runs there. This Mac's folder is freed only after that.
    **Status:** Works (real daemon). `daemon/humatad/tests/move_session.rs::moving_from_this_mac_frees_its_folder_only_after_it_runs_on_box_1` and `daemon/humatad/tests/move_session.rs::a_moving_session_s_terminal_is_read_only_until_it_resumes`. The key press Works: `desktop/tests/live_flows/keys.rs::the_move_key_asks_for_a_machine_in_a_session_and_says_why_nothing_moves_outside_one` presses ⌥⌘M with a session open and the palette asks for a machine for that session; outside a session it only explains.

12. **I do:** Open Hosts, press Add a machine, type `box-2` as the SSH host, and press Add. Accept the host key when the dialog asks.
    **I see:** The dialog shows the new host key's fingerprint and asks me to confirm it. After I confirm, the steps run in order: Connect, Check, Install and Start. Then box-2 appears in Hosts as online.
    **State and events:** Nothing is installed on box-2 until I confirm the key. The machine is recorded only when all steps finish. Its host key is pinned.
    **Status:** Works (real daemon). `daemon/humatad/tests/machines.rs::a_machine_is_added_once_its_host_key_is_confirmed_then_removed_by_plan` adds a fake box through a fake `ssh`. The dialog's key card Works too: `desktop/tests/live_machines.rs::adding_a_machine_over_ssh_shows_its_host_key_and_lists_it_once_confirmed` shows the Host key card with its fingerprint, installs nothing until Confirm, then lists the box online. Fake-client version: `features/hosts/tests/add.rs::a_host_key_request_waits_for_confirmation_and_then_trusts_it`.

13. **I do:** Start a third session on box-2 with its own channel folder.
    **I see:** The session runs on box-2. Its conversation is on box-2, and This Mac holds none of its files.
    **State and events:** The session is recorded on box-2 under the channel's id.
    **Status:** Works (real daemon). `daemon/humatad/tests/two_machines.rs::a_session_started_on_box_2_runs_there_with_its_conversation_on_box_2`.

14. **I do:** Nothing for a while. I close the lid of This Mac. The host is now Away.
    **I see:** On box-1 and box-2 the agents keep working. Their own session files are read and written on the machine. A channel board change that an agent makes is kept on the machine, and it is sent to This Mac when the host comes back.
    **State and events:** The machine journals its writes while the host is away. The sessions do not stop. Away is not a switch I press: it is what a machine is while its host is closed.
    **Status:** Works (real daemon). `daemon/humatad/tests/away_boundary.rs::row_1_own_session_files_are_read_and_written_on_the_machine`, `daemon/humatad/tests/away_board_journal.rs::row_3_a_channel_board_add_while_away_is_journaled`, and the whole-day flow `daemon/humatad/tests/flow_26_10.rs::flow_26_10_a_person_works_on_devbox_1_with_the_mac_closed_and_the_mac_sees_it_on_return`.

15. **I do:** Open This Mac again after a reboot, which restarts the daemon.
    **I see:** The restart report opens first. It lists what the daemon logged before it stopped, and it names any move that was cut short and how it ended. Then the sessions on box-1 and box-2 show as live again. Each session is the same one, not a new start.
    **State and events:** The daemon relinks box-1 and box-2 on start. The restart report is written from the daemon's log.
    **Status:** Works (real daemon). `daemon/humatad/tests/restart.rs::the_restart_report_has_what_the_daemon_logged_before_it_stopped`, and `daemon/humatad/tests/two_machines.rs::the_host_restarted_with_an_env_relinks_both_machines`. The report's screen is only checked by a design golden, not by a live run.

16. **I do:** Open the session on `PAY-1` and send it a message: "Continue with the cache spec".
    **I see:** The agent answers in the same conversation. Its items and the board are unchanged by the restart.
    **State and events:** The session's conversation and board are kept across the restart. The message goes to the agent on box-1.
    **Status:** Code, no test. The session is live after a host restart (step 15, `daemon/humatad/tests/two_machines.rs::the_host_restarted_with_an_env_relinks_both_machines`), but no test sends it a message after a restart and checks the reply.

## Branches

### A move fails halfway through its copy

- **When:** box-2's sessions folder refuses writes, so the copy of a session to box-2 fails.
- **I do:** Press Move on the session and pick box-2. The copy fails.
- **I see:** The move ends as Failed. The step that failed is named, and the message says what to fix. The session keeps running on box-1 with the same agent. Nothing is deleted from box-1.
- **Expected outcome:** The session is still live on box-1, its work is untouched, and only one step is marked failed.
- **Status:** Works (real daemon). `daemon/humatad/tests/move_session.rs::a_move_whose_copy_fails_leaves_it_running_on_the_source_and_names_the_step`. The app's display of the failed step is not separately tested.

### box-2 cannot be added

- **When:** the SSH step fails, for example because the key login does not work or the address is not reachable.
- **I do:** Press Try again in the Add dialog, after fixing the login.
- **I see:** The failed step is named with its output. Try again runs the same add, under the same id.
- **Expected outcome:** No half-made machine is listed. The retry finishes the same operation.
- **Status:** Works (real daemon). `clients/live/tests/machines.rs::a_failed_add_names_its_step_and_trying_again_follows_the_same_operation` and `daemon/humatad/tests/machines.rs::a_failed_add_tried_again_runs_under_the_same_id_and_a_removal_cannot_be_cancelled`.

### A milestone's item depends on one that is not done

- **When:** `PAY-2` depends on `PAY-1`, and `PAY-1` is still In progress.
- **I do:** Start a session for `PAY-2`.
- **I see:** The session starts. The drawer shows no tick on `PAY-1`. Nothing is refused.
- **Expected outcome:** A dependency blocks nothing. No status change or session start is refused because of an edge (SPEC BD-14).
- **Status:** Code, no test. The daemon's board service (`daemon/humatad/src/board_service.rs`) stores the edge, and no session start path reads `depends_on` (a search of `daemon/humatad/src/session_service*` finds no use). Nothing in the tests asserts that a start is not refused.

### Two agents edit the same item

- **When:** the agent on `PAY-1` and I both change `PAY-1`, and my edit is based on the revision I last read.
- **I do:** Save an edit to `PAY-1`'s title after the agent has already changed its status.
- **I see:** My save is refused as a conflict. Only the fields that changed since my revision are named. The drawer refreshes and keeps the picker open.
- **Expected outcome:** The write is refused. The other fields are not overwritten. A repeated save after a refresh works.
- **Status:** Works (real daemon). `daemon/humatad/tests/boards.rs::a_change_made_against_an_old_revision_conflicts_only_on_fields_changed_since` and `daemon/humatad/tests/board_tools.rs::a_stale_revision_conflicts_only_on_a_field_that_changed`. The drawer's refresh after a refusal Works: `desktop/tests/live_flows/board_conflict.rs::a_drawer_edit_refused_after_the_item_changed_on_the_daemon_refreshes_and_keeps_the_picker` (an edge added on the daemon behind the open drawer; the drawer's pick is refused, it refreshes and keeps the picker, and the next pick saves). A move from the drawer after an agent moved the same item is refused by its revision, the drawer refreshes to the agent's column and stays open, and the next move lands: `desktop/tests/live_flows/board_revision.rs::a_drawer_move_after_an_agents_move_is_refused_refreshes_and_then_lands` (D383) and `clients/live/tests/boards.rs::a_move_from_an_old_copy_is_refused_as_a_conflict_until_the_item_is_read_again`. A title save is Not built: the drawer has no title edit (D383; a board-edit milestone). Fake-client versions: `features/board/tests/drawer.rs::a_refused_edit_shows_why_refreshes_and_keeps_the_picker` and `::a_move_from_an_old_copy_shows_why_refreshes_and_keeps_the_drawer`.

### The agent's move on the global board is denied

- **When:** the agent asks to move an item on the global board, and I decide against it.
- **I do:** Press Keep in progress on the request card in the drawer.
- **I see:** The agent's call returns a failure. The request leaves the Inbox. The item stays In progress.
- **Expected outcome:** The denial moves nothing. A fresh load shows the same status.
- **Status:** Works (real daemon). `desktop/tests/live_boards.rs::denying_an_agents_move_from_the_items_drawer_keeps_it_in_progress`. The approval case is `desktop/tests/live_boards.rs::approving_an_agents_move_from_the_items_drawer_moves_it_and_returns_the_call`.

### This Mac reboots in the middle of a move

- **When:** the power is cut while a session's move is copying to box-1.
- **I do:** Turn This Mac on again and let the daemon start.
- **I see:** The restart report names the move and says whether it was undone or finished. The session is live on the machine it was on before the cut, or on box-1 if the move had finished.
- **Expected outcome:** No work is lost. The move is either undone or finished, and the session is never left half moved.
- **Status:** Works (real daemon). `daemon/humatad/tests/power_cut_host.rs::a_move_with_the_hosts_power_cut_loses_nothing_part_1` and `..._part_2` (the power cut is traced at each point of the move, 16 shards). `daemon/humatad/tests/restart_operations.rs::a_move_cut_short_is_named_in_the_restart_report_with_how_it_ended`.

### The host is closed and box-1 asks me something

- **When:** an agent on box-1 needs an answer from me while This Mac is closed.
- **I do:** Nothing until I open This Mac, or I answer on box-1 with the passphrase login.
- **I see:** The request waits on the machine. When the host returns, the request is sent to the host's Inbox in order.
- **Expected outcome:** The request is kept, not dropped. It is answered once.
- **Status:** Works (real daemon). `daemon/humatad/tests/away_inbox.rs::a_request_answered_on_the_machine_while_away_reaches_the_host_in_order`.

## Expected outcomes

- [ ] A channel item gets the channel's key and is listed in To do.
- [ ] Starting a session for a To do item moves it to In progress and attaches the session.
- [ ] An agent's item on its session board is listed on that session's board only until it is shown on the channel's.
- [ ] An agent's write to the global board waits for my answer and applies once I approve it.
- [ ] A dependency is listed under Depends on and can be removed. A loop is refused and the refusal names it.
- [ ] No graph of dependencies exists. The list is the only view.
- [ ] Moving a session to box-1 keeps its id, branch, work and conversation, and frees This Mac's folder only after box-1 runs it.
- [ ] A failed move leaves the session running on its source, names the failed step and deletes nothing.
- [ ] box-2 is added only after I confirm its host key, and it runs the same suites as box-1.
- [ ] While This Mac is closed, the sessions on box-1 and box-2 keep running, and their board writes are kept for the host.
- [ ] After a reboot, the restart report names what was cut short, and each session is the same one.
- [ ] A conflicting edit is refused and names only the fields that changed.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Create a channel item | Works (real daemon) | `clients/live/tests/boards.rs::a_channel_item_gets_the_channels_key_and_is_listed_in_to_do` |
| 2. Start a session for an item | Works (real daemon) | `clients/live/tests/boards.rs::a_session_started_for_a_to_do_item_works_on_it_and_moves_it_in_progress` |
| 3. Agent makes an item on its session board | Works (real daemon) | `daemon/humatad/tests/board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it` |
| 4. Agent's global write waits for me | Works (real daemon) | `daemon/humatad/tests/board_tools.rs::creating_on_the_global_board_waits_for_the_user` |
| 5. Add a dependency in the drawer | Works (real daemon) | `clients/live/tests/boards.rs::a_dependency_is_listed_refuses_a_loop_and_can_be_removed` |
| 6. Graph view of dependencies | Not built (no milestone) | SPEC BD-14 (a list) |
| 7. Loop refused with its names | Works (real daemon) | `daemon/humatad/tests/boards.rs::a_dependency_that_closes_a_loop_is_refused_with_the_loop` |
| 8. Board updates as items move | Works (real daemon) | `clients/live/tests/boards.rs::a_move_after_subscribing_is_pushed_as_an_item_change` |
| 9. Dashboard figures open their items | Works (real daemon) | `desktop/tests/live_boards.rs::the_three_views_show_the_same_items_and_a_dashboard_figure_opens_what_it_counts`; fakes: `features/board/tests/presentation.rs::dashboard_drills_show_exactly_the_counted_items` |
| 10. Session panel shows the channel board | Fakes only (D874: now the To-do panel) | `desktop/tests/panels.rs::board_lists_the_channels_items_and_expands_without_losing_the_session` |
| 11. Move two sessions to box-1 | Works (real daemon) | `daemon/humatad/tests/move_session.rs::moving_from_this_mac_frees_its_folder_only_after_it_runs_on_box_1` |
| 11a. The ⌥⌘M key press | Works (real daemon) | `desktop/tests/live_flows/keys.rs::the_move_key_asks_for_a_machine_in_a_session_and_says_why_nothing_moves_outside_one` |
| 12. Add box-2 with host key | Works (real daemon) | `daemon/humatad/tests/machines.rs::a_machine_is_added_once_its_host_key_is_confirmed_then_removed_by_plan` |
| 13. Session on box-2 | Works (real daemon) | `daemon/humatad/tests/two_machines.rs::a_session_started_on_box_2_runs_there_with_its_conversation_on_box_2` |
| 14. Host closed, sessions keep working | Works (real daemon) | `daemon/humatad/tests/flow_26_10.rs::flow_26_10_a_person_works_on_devbox_1_with_the_mac_closed_and_the_mac_sees_it_on_return` |
| 15. Reboot, restart report, relink | Works (real daemon) | `daemon/humatad/tests/restart.rs::the_restart_report_has_what_the_daemon_logged_before_it_stopped` |
| 16. Work continues after restart (a message) | Code, no test | Live after restart is proven at step 15; no test sends a message after it |
| Branch: move fails mid-copy | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_move_whose_copy_fails_leaves_it_running_on_the_source_and_names_the_step` |
| Branch: box-2 cannot be added | Works (real daemon) | `clients/live/tests/machines.rs::a_failed_add_names_its_step_and_trying_again_follows_the_same_operation` |
| Branch: dependency not done | Code, no test | `daemon/humatad/src/board_service.rs` |
| Branch: two agents edit one item | Works (real daemon) | `daemon/humatad/tests/boards.rs::a_change_made_against_an_old_revision_conflicts_only_on_fields_changed_since` |
| Branch: global move denied | Works (real daemon) | `desktop/tests/live_boards.rs::denying_an_agents_move_from_the_items_drawer_keeps_it_in_progress` |
| Branch: reboot during a move | Works (real daemon) | `daemon/humatad/tests/power_cut_host.rs::a_move_with_the_hosts_power_cut_loses_nothing_part_1` |
| Branch: host closed, box asks me | Works (real daemon) | `daemon/humatad/tests/away_inbox.rs::a_request_answered_on_the_machine_while_away_reaches_the_host_in_order` |

## Gaps and likely bugs

- **Not built (no milestone):** a graph view of dependencies. The SPEC asks for a list under Depends on (BD-14).
- **Not built (SPEC FC-10, ST-5):** changing a machine's root folder (`ChangeRoot`, `PlanChangeRoot`), in `daemon/humatad/src/machine_service.rs:1326` and `:1333`. MACH.md lists FC-10 among its SPEC reads, but no task builds it. This case does not need it.
- **Not built (PER-8, not in 1.0):** the VM tier, `SetVmTier`, `daemon/humatad/src/machine_service.rs:1340`. `milestones/ANYHOST.md` puts the VM tier out of 1.0.
- **Not built (MACH T10):** `TrustNewHostKey`, `daemon/humatad/src/machine_service.rs:1400`. Adding box-2 does not need it, because the add flow trusts a new key itself. It matters only for a machine whose key has changed (see macro-08).
- **Not built (MACH T10):** host variables, `daemon/humatad/src/machine_service.rs:1379` to `:1393`. Not needed here.
- **Likely bug:** none found in this case's path.
- **Correction to the brief:** `desktop/tests/live_boards.rs` has 2 real-daemon tests, not 32. They are the approve and deny cases at lines 729 and 758.
- **Correction to the brief:** adding box-2 does not depend on the four `not_built` calls the brief lists. The add flow works (`daemon/humatad/tests/machines.rs`). The four not built are `ChangeRoot`, `SetVmTier`, host variables and `TrustNewHostKey`, and only the last touches a machine's key.
- **Correction to the brief:** there is no graph view to check, so step 6 is a gap.
