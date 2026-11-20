# 5. A long-running project: spec, milestones, sessions, machines

> "Hey, I'm writing a long running thing. I want to make a doc/spec, then make the milestones, then spawn sessions to iteratively work on these. Oh, now I want to move across machines, now I want a remote machine spun up, etc..."

This is weeks of work on a new ledger service. I want one spec, a board of milestones with the order between them, a session per milestone I can come back to, and the freedom to move heavy work off my laptop.

How to read this file, and what the tags mean: [README](README.md).

## Starting state

- `#ledger-v2` exists, with `acme/ledger` and `acme/api`, on This Mac only. Its board is empty.
- `spec.md` was written on the Desk (flow 4) and is in a kept Desk folder.
- devbox-1 is a linked machine and online. No other machines exist yet.
- A Lima provider is set up in Settings.

## Steps

### The spec

1. **Do:** on the Desk, click **Copy to a channel…** on `spec.md` and pick `#ledger-v2`.
   **Expect:** `#ledger-v2/shared/spec.md` exists, and every session's agent in the channel can read it.
   **Status: Works** in the daemon: `daemon/humatad/tests/desk.rs::markdown_in_the_folder_is_listed_and_copies_into_a_channels_shared_folder`. The app **Works**: `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel` clicks **Copy to a channel…** and picks the channel (flow 1, step 7). Moving the spec's Desk session into the channel is **Not built: ROAM** (DK-15–DK-19).

### The milestones

> **UI update (BOARDSIMPLE, D830–D874):** The Board (kanban/table/dashboard) was simplified to **To-do**: a single ranked list with channel/session filters, three states (`Open`, `In progress`, `Closed`), `Waits on` dependencies, and a **Needs attention** group for agent asks answered **Yes** / **No**.

2. **Do:** open the channel's **To-do** tab (formerly Board). Add five items: "Schema", "Write path", "Read path", "Backfill" and "Cut over". In Backfill's drawer, set **Waits on** (Depends on) to Write path. In Cut over's, set it to the other four.
   **See:** five items in the ranked To-do list (waiting items fold into the Waiting group). Each drawer lists what the item waits on.
   **Expect:**
   - Each item gets the channel's key (for example LV-1).
   - A dependency that would make a loop is refused.
   - Events: BoardItem created ×5; BoardItem updated per dependency.
   **Status: Works** in the client and To-do UI: `clients/live/tests/boards.rs::a_channel_item_gets_the_channels_key_and_is_listed_in_to_do` and `::a_dependency_is_listed_refuses_a_loop_and_can_be_removed`; `features/todo/tests/todo.rs`, `desktop/tests/native/todo_asks.rs::a_channels_todo_tab_lists_only_that_channels_items`, and `desktop/tests/design/todo.rs` / `desktop/tests/design/todo_more.rs`.

3. **Do (instead of 2):** in a session, ask the agent: "Read `shared/spec.md` and put the milestones on To-do."
   **Expect:**
   - The agent makes the items through Humata's tools, with no ask, because the channel board rule is Allow.
   - On the global board it would wait for me.
   - Events: BoardItem created, by the agent.
   **Status: Works** in the daemon: `daemon/humatad/tests/board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it` and `::creating_on_the_global_board_waits_for_the_user`.

4. **Do:** filter the ranked **To-do** list by channel, session, or saved filter (replacing the old Board / Table / Dashboard switcher).
   **See:** the matching items in priority order, with channel/session narrowing.
   **Status: Works.** `desktop/tests/native/todo_asks.rs::a_channels_todo_tab_lists_only_that_channels_items`, `features/todo/tests/todo.rs`, and `desktop/tests/design/todo.rs` / `desktop/tests/design/todo_more.rs`. A graph of the dependency edges is **Not built, no milestone**: waiting items fold under Waiting and show in the drawer's `Waits on` list.

### Working the milestones

5. **Do:** on "Schema", click **Start a session on this** and start it from the filled-in sheet.
   **See:** the session page. On To-do, Schema is `In progress`, with the session linked.
   **Expect:**
   - The session is linked to the item, and the agent is told what the item is.
   - Events: Session created; BoardItem updated (In progress, a session added).
   **Status: Works** in the client and live app: `clients/live/tests/boards.rs::a_session_started_for_a_to_do_item_works_on_it_and_moves_it_in_progress`, `::a_session_set_on_an_item_is_listed_until_it_is_taken_off`, and `desktop/tests/live_flows/todo_asks.rs::starting_a_session_from_the_todo_drawer_opens_it_and_moves_the_item_in_progress`. Fake-client version: `desktop/tests/native/todo_asks.rs::starting_a_session_from_the_todo_drawer_opens_the_prefilled_new_session_dialog`.

6. **Do:** while the agent works, type two more asks.
   **See:** they wait under **Waiting to send**, each with **Send now** and **Remove**.
   **Expect:**
   - When the agent's turn ends, Humata types the next one. Nothing is typed while the turn's end is uncertain.
   - A message that fails to send stays, for retry.
   - Events: Session updated (Working → Idle → Working).
   **Status:**
   - The queue **Works** against a real agent: `desktop/tests/live_flows/queue.rs::messages_typed_mid_turn_wait_can_be_shown_and_removed_and_the_rest_reaches_the_agent` (nothing is typed while the turn's end is uncertain; Remove drops one; Send now types the other). Fake-client versions: `desktop/tests/native/queue.rs::queued_messages_can_be_shown_removed_and_sent_and_presence_follows_the_tab`, `::uncertain_turn_end_keeps_the_queue_visible_until_explicit_send` and `::a_failed_queued_message_stays_visible_for_retry`.
   - Typing an ask into the queue: **Not built** (D381). No control in `ui/src` sends `SessionAction::Draft` or `Submit`; keys go to the terminal. The tests add the two asks as app actions.
   - Knowing the turn ended: **Works** in the daemon, from the agent's hooks: `daemon/humatad/tests/turns.rs::the_hooks_move_a_session_between_idle_working_and_needing_input`. An agent with no hooks shows "Probably idle": `::an_agent_with_no_hook_that_goes_quiet_is_probably_idle_until_it_writes_again`.

7. **Do:** the agent asks to close Schema. Open the item's drawer (under **Needs attention**), review it, and click **Yes** (or **No** to keep it open).
   **See:** an ask card in **Needs attention** with **Yes** and **No**.
   **Expect:**
   - The item moves to `Closed`, and the agent's waiting call returns.
   - **No** returns it as denied and keeps the item open.
   - Events: InboxRequest created, then Approved; BoardItem updated (Closed).
   **Status: Works.** `desktop/tests/live_flows/todo_asks.rs::approving_an_agents_ask_from_the_todo_drawer_closes_the_item_and_returns_the_call` and `::denying_an_agents_ask_from_the_todo_drawer_keeps_the_item_open_and_fails_the_call` (native: `desktop/tests/native/todo_asks.rs::approving_an_items_ask_answers_it_and_leaves_the_other_asks`). The agent runs the real `humata` command against a real daemon.

8. **Do:** on "Write path", click **Fork**. In "Fork “Write path”" (frame 62), pick **Code + context** and **Fork from** **Now**, then click **Fork**.
   **See:** a second session under the first in the tree, on its own branch.
   **Expect:**
   - The fork has a copy of the work and of the conversation.
   - Code only gives a fresh conversation. Context only works in the parent's code.
   - Forking from an earlier turn carries the conversation up to that turn, with the code as it is now.
   - Events: Session created (with a fork origin); Session updated (the parent's forks).
   **Status:**
   - **Works** in the daemon: `daemon/humatad/tests/fork.rs::code_and_context_copies_the_work_and_the_conversation` and `::code_only_copies_the_work_into_a_fresh_conversation`; `daemon/humatad/tests/context_fork.rs::a_context_only_fork_carries_the_conversation_and_works_in_the_parents_code` and `::a_fork_before_a_turn_carries_the_conversation_up_to_it_and_the_code_as_now`.
   - The dialog **Works** for Code + context from Now: `desktop/tests/live_flows/lifecycle.rs::forking_from_the_fork_dialog_starts_a_child_session_on_the_daemon` (Fork opens the dialog, a child session opens and nests under its parent, and the daemon lists it with the parent). The other choices **Work** too, clicked in the dialog: `desktop/tests/live_flows/fork_kinds.rs``::a_code_only_fork_copies_the_files_into_a_fresh_conversation`, `::a_context_only_fork_carries_the_conversation_and_shares_the_parents_files` and `::a_fork_from_before_a_turn_carries_the_conversation_up_to_it`. Fake-client version: `desktop/tests/native/fork_move.rs::the_fork_dialog_matches_frame_62_and_forks_what_it_shows`.
   - Forking to Codex or Copilot: the daemon can hand over a summary (`fork.rs::a_fork_with_another_agent_is_handed_the_conversation_and_a_summary`). But a real daemon only knows Claude Code and OpenCode unless an agents file adds more. **Not built: ROAM** (batch 3, built-in Codex and Copilot kinds).

### Across machines

9. **Do:** click **Add a machine…**. Pick the Lima provider from the pop-up, fill in its fields, and add the machine. If it asks me to log in, log in in the terminal it shows, click **Done, I'm logged in**, then **Make it unattended**.
   **See:** the steps tick off (frames 56–58): "Check the system (arch, disk, git, gh)", "Install humatad", then the new machine's page (frame 59). A failed step says what to run on the machine, then **Try again**.
   **Expect:**
   - The machine is created by the provider, checked, given humatad and the Humata tools, and added.
   - Skills sync first as a preview I confirm.
   - Events: Operation created, then updated per step, then Succeeded; Machine created.
   **Status:**
   - **Works** in the client and daemon, over a fake `ssh`: `clients/live/tests/machines.rs::a_trusted_add_runs_every_step_and_ends_with_the_machine`, `::a_provider_that_needs_a_login_is_logged_in_by_hand_and_becomes_a_machine` and `::a_first_skills_sync_is_previewed_then_applied_with_its_plan`; `daemon/humatad/tests/providers.rs::a_machine_is_created_through_its_provider_then_run_and_deleted_by_the_provider`.
   - The dialog **Works** for an SSH add: `desktop/tests/live_machines.rs::adding_a_machine_over_ssh_shows_its_host_key_and_lists_it_once_confirmed` (real daemon and machines client, a fake machine behind a fake `ssh`; nothing installs until Confirm). Through a provider and its login it **Works** too: `desktop/tests/live_machines.rs::the_popup_lists_a_provider_and_it_adds_a_machine_with_its_params` (the typed param and the default reach the provider), `::a_login_by_hand_in_the_dialog_leads_to_the_unattended_offer_and_the_machine` and `::an_add_over_ssh_fails_at_install_and_try_again_finishes_it` (Try again finishes the add with no new Add). Fake-client versions: `desktop/tests/machines_flow.rs::the_popup_lists_the_hosts_then_the_providers_and_a_provider_adds_with_its_params`, `::a_login_shows_the_terminal_takes_keys_and_leads_to_the_unattended_offer` and `::an_add_over_ssh_fails_at_install_and_try_again_retries_the_same_add`.
   - A real VM over real SSH: **Not built: LINUX** (blocked). Lima is only an example in the provider list (`providers.rs::providers_are_listed_with_their_params_and_names_of_variables_and_the_lima_example`).
   - Host variables: **Not built, no milestone** (a MACH follow-up). The client answers "not served yet" (`clients/live/tests/machines.rs::host_variables_answer_not_served_yet`).

10. **Do:** add devbox-1 to `#ledger-v2`. Then, on the Backfill session, pick **Move to another machine…** › devbox-1.
    **See:**
    - "Moving to devbox-1…" with its steps: "Paused the agent", "Copying the conversation (fixing paths)…".
    - The tab is read-only until the agent resumes (frames 63, 64). Then **Resume claude on devbox-1** and **Delete the copy on This Mac**.
    **Expect:**
    - The session's clones and conversation are on devbox-1, and the agent resumes there in the same conversation.
    - If the copy fails, the session keeps running on This Mac, and the failed step is named.
    - Events: Session updated (Moving), then (Working, machine devbox-1); Operation updated per step.
    **Status:**
    - Adding a machine to a channel that exists: **Not built, no milestone** (like Add a GitHub repo…, flow 2 step 8).
    - On devbox-1, every repo picker and `humata repos search` already know `acme/ledger` and `acme/api`. Nothing is typed again, because the library is on the host (RL-1, RL-12). Backfill needs `warehouse-co/loader`, which is outside the `acme` scope: **+ Org** `warehouse-co` finds it for that search only (RL-8). The pickers **Work** live: `desktop/tests/live_flows/repo_library.rs::a_repo_added_once_is_listed_first_in_the_next_new_channel_even_with_github_out_of_reach`. A box agent's `repo_search` reaching the host **Works** in the daemon: `daemon/humatad/tests/away_forward.rs::an_agents_repo_search_on_a_machine_goes_up_and_the_hosts_answer_is_the_agents` and `::away_an_agents_repo_search_is_host_unreachable_and_answers_nothing`, with the host side in `daemon/humatad/src/board_service/replay.rs::a_forwarded_library_list_answers_with_the_hosts_library` (D568). **+ Org** for one search is **Fakes only**: `features/app/tests/repo_search.rs::widening_with_an_owner_keeps_the_scope_and_adds_the_owner_to_it`, with the daemon part in `daemon/humatad/tests/repos.rs::a_scope_for_one_search_is_used_once_and_never_saved`. My own `humata repos search` over SSH, outside every session, is **Not built, no milestone**: a machine refuses it with a fix pointing at the host, since AWAY cut MachineUser forwarding.
    - The move **Works** in the daemon: `daemon/humatad/tests/move_session.rs::a_moving_session_s_terminal_is_read_only_until_it_resumes`, `::a_move_whose_copy_fails_leaves_it_running_on_the_source_and_names_the_step` and `::a_moved_conversation_is_mirrored_once`.
    - The dialog is **Fakes only**: `desktop/tests/native/fork_move.rs::the_move_dialog_matches_frame_63_and_moves_to_the_picked_machine`. The palette's Move asks for a machine against a real daemon, but that rig has one machine, so nothing moves (`desktop/tests/live_flows/find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds`).

11. **Do:** the Backfill agent asks to fork a helper onto the new Lima machine. Allow it in the Inbox.
    **Expect:**
    - The fork starts there only after I allow it.
    - A context-only fork onto another machine is refused, since it would have no code there.
    - Events: InboxRequest created, then Approved; Session created.
    **Status: Works** in the daemon: `daemon/humatad/tests/fork_remote.rs::an_agent_forks_itself_onto_another_machine_once_the_user_approves` and `::previewing_a_fork_onto_or_from_a_box_names_its_branch_and_size_and_makes_nothing`.

12. **Do:** devbox-1 drops off the network overnight. In the morning, open its machine page and click **Reconnect now**.
    **See:** the machine shows offline, its sessions show Offline, then they come back. A note says "devbox-1 is back. 1 session reattached".
    **Expect:**
    - The same sessions are reattached, not started again.
    - Events: Machine updated (offline, then online); Session updated (Offline → its last status).
    **Status:**
    - Coming back **Works** in the daemon: `daemon/humatad/tests/two_machines.rs::a_lost_link_comes_back_and_the_session_is_the_same` and `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice`.
    - **Reconnect now** and the "is back" note: **Not built, no milestone** (HO-6). The live client answers "not in this release" for reconnecting.

### Wrapping up

13. **Do:** when the other four are Done, start "Cut over", review its diff, and push (flow 2, steps 10–13).
    **Expect:** as in flow 2.
    **Status:**
    - **Works**, as in flow 2.
    - Starting or flagging Cut over by itself once its dependencies are Done: **Not built, no milestone.** Dependencies are shown but nothing acts on them.

14. **Do:** archive the finished sessions.
    **Expect:**
    - Forks of an archived session move up in the tree and keep running.
    - Restoring the parent puts them back under it.
    **Status: Works** in the daemon: `daemon/humatad/tests/archive.rs::forks_move_up_while_their_parent_is_archived_and_back_on_restore`. The archive menu and Undo **Work** against a real daemon: `desktop/tests/live_flows/lifecycle.rs::archiving_a_session_from_its_menu_and_undoing_brings_it_back` (flow 2, step 14). Forks moving up in the tree is **Fakes only**: no live test archives a session that has forks.

15. **Do:** a month later, the work moves to a new team channel. On the main session, pick **Continue in another channel…**.
    **Expect:** a session in the new channel that carries the conversation.
    **Status: Works.** `desktop/tests/live_flows/continue_in.rs::a_claude_session_continues_in_another_channel_from_before_turn_2`.

## Expected outcome

`#ledger-v2` has five items closed on To-do. Each was worked by its own session, linked on To-do. Sessions ran on This Mac, devbox-1 and a Lima VM. Every move, fork and reconnect kept its conversation. Finished sessions are archived, with nothing unpushed.

## Gaps this flow shows

- No app test moves a session against a real daemon, and the live app rig has one machine. Forking (Code + context) and archiving with Undo are now tested live, but not archiving a session that has forks.
- A channel's repos and machines are fixed when it's made. The app can't add either later.
- Repos used before are offered in later channels, and a box agent's search reaches the host's library (D568). My own `humata repos search` on a box isn't forwarded (MachineUser forwarding, cut in AWAY; no milestone).
- **Reconnect now** fails against a real daemon, and nothing tells me when a machine is back.
- Codex and Copilot appear under **Runs**, but a real daemon refuses them without an agents file (ROAM batch 3).
- Machines are only ever reached through a fake `ssh` in daemon tests. Real SSH and a real VM are LINUX, which is blocked.
- To-do hides waiting items until their `Waits on` items close, and there's no graph view.
- The waiting-message queue is tested live via app actions (`desktop/tests/live_flows/queue.rs`), though typing an ask directly into the queue from the terminal UI is still not wired (step 6, D381).
