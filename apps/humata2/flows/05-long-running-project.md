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
   **Status: Works** in the daemon: `daemon/humatad/tests/desk.rs::markdown_in_the_folder_is_listed_and_copies_into_a_channels_shared_folder`. The app is **Fakes only** (flow 1, step 7). Moving the spec's Desk session into the channel is **Not built: ROAM** (DK-15–DK-19).

### The milestones

2. **Do:** open the channel's **Board** tab. Click **New item** in To do five times, for "Schema", "Write path", "Read path", "Backfill" and "Cut over". In Backfill's drawer, set **Depends on** to Write path. In Cut over's, set it to the other four.
   **See:** five cards in To do. Each drawer lists what the item depends on.
   **Expect:**
   - Each item gets the channel's key (for example LV-1).
   - A dependency that would make a loop is refused.
   - Events: BoardItem created ×5; BoardItem updated per dependency.
   **Status: Works** in the client: `clients/live/tests/boards.rs::a_channel_item_gets_the_channels_key_and_is_listed_in_to_do` and `::a_dependency_is_listed_refuses_a_loop_and_can_be_removed`. The clicks are **Fakes only**: `desktop/tests/native/board.rs::board_creates_in_selected_column_and_answers_the_item_request` and `features/board/tests/drawer.rs`.

3. **Do (instead of 2):** in a session, ask the agent: "Read `shared/spec.md` and put the milestones on the board."
   **Expect:**
   - The agent makes the items through Humata's tools, with no ask, because the channel board rule is Allow.
   - On the global board it would wait for me.
   - Events: BoardItem created, by the agent.
   **Status: Works** in the daemon: `daemon/humatad/tests/board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it` and `::creating_on_the_global_board_waits_for_the_user`.

4. **Do:** switch between **Board**, **Table** and **Dashboard**, and set the scope to **By channel**.
   **See:** the same items in each view.
   **Status: Fakes only.** `desktop/tests/native/board.rs::board_reference_images` and `features/board/tests/scope.rs`. A graph of the dependency edges is **Not built, no milestone**: they show only as the drawer's Depends on list.

### Working the milestones

5. **Do:** on "Schema", click **Start a session for this item** and start it from the filled-in sheet.
   **See:** the session page. On the board, Schema is in In progress, with the session under **Worked on by**.
   **Expect:**
   - The session is linked to the item, and the agent is told what the item is.
   - Events: Session created; BoardItem updated (In progress, a session added).
   **Status: Works** in the client: `clients/live/tests/boards.rs::a_session_started_for_a_to_do_item_works_on_it_and_moves_it_in_progress` and `::a_session_set_on_an_item_is_listed_until_it_is_taken_off`. The button is **Fakes only**: `features/app/tests/board_start.rs::starting_a_session_for_a_to_do_item_adds_it_and_starts_the_item`.

6. **Do:** while the agent works, type two more asks.
   **See:** they wait under **Waiting to send**, each with **Send now** and **Remove**.
   **Expect:**
   - When the agent's turn ends, Humata types the next one. Nothing is typed while the turn's end is uncertain.
   - A message that fails to send stays, for retry.
   - Events: Session updated (Working → Idle → Working).
   **Status:**
   - The queue: **Fakes only**, `desktop/tests/native/queue.rs::queued_messages_can_be_shown_removed_and_sent_and_presence_follows_the_tab`, `::uncertain_turn_end_keeps_the_queue_visible_until_explicit_send` and `::a_failed_queued_message_stays_visible_for_retry`. The queue lives in the app (SE-23), and no live test drives it.
   - Knowing the turn ended: **Works** in the daemon, from the agent's hooks: `daemon/humatad/tests/turns.rs::the_hooks_move_a_session_between_idle_working_and_needing_input`. An agent with no hooks shows "Probably idle": `::an_agent_with_no_hook_that_goes_quiet_is_probably_idle_until_it_writes_again`.

7. **Do:** the agent asks to move Schema to Done. Open the item's drawer, review it, and click **Move to Done**.
   **See:** "Claude Code asks to move this to Done", with **Move to Done** and **Keep in progress**.
   **Expect:**
   - The item is in Done, and the agent's waiting `humata board move` returns.
   - Keep in progress returns it as denied.
   - Events: InboxRequest created, then Approved; BoardItem updated (Done).
   **Status: Works.** `desktop/tests/live_boards.rs::approving_an_agents_move_from_the_items_drawer_moves_it_and_returns_the_call` and `::denying_an_agents_move_from_the_items_drawer_keeps_it_in_progress`. The agent runs the real `humata` command against a real daemon.

8. **Do:** on "Write path", click **Fork**. In "Fork “Write path”" (frame 62), pick **Code + context** and **Fork from** **Now**, then click **Fork**.
   **See:** a second session under the first in the tree, on its own branch.
   **Expect:**
   - The fork has a copy of the work and of the conversation.
   - Code only gives a fresh conversation. Context only works in the parent's code.
   - Forking from an earlier turn carries the conversation up to that turn, with the code as it is now.
   - Events: Session created (with a fork origin); Session updated (the parent's forks).
   **Status:**
   - **Works** in the daemon: `daemon/humatad/tests/fork.rs::code_and_context_copies_the_work_and_the_conversation` and `::code_only_copies_the_work_into_a_fresh_conversation`; `daemon/humatad/tests/context_fork.rs::a_context_only_fork_carries_the_conversation_and_works_in_the_parents_code` and `::a_fork_before_a_turn_carries_the_conversation_up_to_it_and_the_code_as_now`.
   - The dialog is **Fakes only**: `desktop/tests/native/fork_move.rs::the_fork_dialog_matches_frame_62_and_forks_what_it_shows`. No live client or live app test forks.
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
   - The dialog is **Fakes only**: `desktop/tests/machines_flow.rs::the_popup_lists_the_hosts_then_the_providers_and_a_provider_adds_with_its_params`, `::a_login_shows_the_terminal_takes_keys_and_leads_to_the_unattended_offer` and `::an_add_over_ssh_fails_at_install_and_try_again_retries_the_same_add`.
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
    **Status: Works** in the daemon: `daemon/humatad/tests/archive.rs::forks_move_up_while_their_parent_is_archived_and_back_on_restore`. The screen is **Fakes only**, and Undo fails live (flow 2, step 14).

15. **Do:** a month later, the work moves to a new team channel. On the main session, pick **Continue in another channel…**.
    **Expect:** a session in the new channel that carries the conversation.
    **Status: Not built: ROAM** (CV-8).

## Expected outcome

`#ledger-v2` has five items in Done. Each was worked by its own session, linked on the board. Sessions ran on This Mac, devbox-1 and a Lima VM. Every move, fork and reconnect kept its conversation. Finished sessions are archived, with nothing unpushed.

## Gaps this flow shows

- No app test moves, forks or archives a session against a real daemon, and the live app rig has one machine. These are the steps where losing work hurts most.
- A channel's repos and machines are fixed when it's made. The app can't add either later.
- **Reconnect now** fails against a real daemon, and nothing tells me when a machine is back.
- Codex and Copilot appear under **Runs**, but a real daemon refuses them without an agents file (ROAM batch 3).
- Machines are only ever reached through a fake `ssh` in daemon tests. Real SSH and a real VM are LINUX, which is blocked.
- The board knows the order between milestones but does nothing with it, and there's no graph view.
- The waiting-message queue, the main way to iterate without watching, is proven only on fakes.
