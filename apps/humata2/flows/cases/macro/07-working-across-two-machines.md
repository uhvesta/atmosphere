# Working across two machines: a session moves to box-1, a file comes back, and box-1 drops off for an hour

Layer: macro. Platforms: macOS, Linux, web. (The keys shown are the macOS ones; Linux uses `Ctrl`.)

## Starting state

- Humata 2 is open on This Mac. This Mac is the host and runs the daemon.
- Machines: This Mac (online, host). box-1 (a Linux box reached over SSH, online and linked to This Mac).
- A channel named `payments` exists, with `acme/api` as its repo. Its main clone is on both machines.
- Session A, "Fix the cache", is running on This Mac. It has one shell tab, and the shell has printed some output.
- Session B, "Write the migration", is running on box-1. I started it there from This Mac.
- Nothing is in the Inbox. The file `notes/plan.md` exists on box-1 only.
- A second computer is in front of me. Its browser is not connected to Humata 2 (see step 16).

## Steps

1. **I do:** Open Hosts and select box-1.
   **I see:** box-1's page shows its health as the probe last read it, the time it was checked, and the versions of git and gh that the machine reports.
   **State and events:** The page reads box-1's own data. It does not read This Mac's.
   **Status:** Works (real daemon). `daemon/humatad/tests/machines.rs::a_linked_machines_page_shows_the_health_its_probe_read`. The tools card in the app is Fakes only: `desktop/tests/machines_flow.rs::the_hosts_card_shows_the_selected_machines_tools_as_its_daemon_reports_them`.

2. **I do:** Press New session in `payments`, pick box-1 as the machine, type "Add a test for the migration", and press Return.
   **I see:** A new session tab opens. Its terminal is attached through This Mac, and it shows the agent starting on box-1.
   **State and events:** The session is recorded on box-1 under the channel's id. Its terminal bytes come to This Mac and go to the app.
   **Status:** Works (real daemon). `daemon/humatad/tests/remote.rs::a_session_started_on_the_machine_runs_there_and_the_host_relays_its_terminal`. The picker that offers only online machines in the channel is Fakes only: `features/app/tests/new_session.rs::changing_channel_selects_an_online_host_in_that_channel`.

3. **I do:** Start a session on This Mac, in the same channel, "Check the cache keys".
   **I see:** The session's clone is on its own branch, named from its title. Its agent starts on This Mac.
   **State and events:** The session is recorded on This Mac. Its branch is named from its title.
   **Status:** Works (real daemon). `daemon/humatad/tests/sessions.rs::a_started_session_has_its_clone_on_a_branch_named_from_its_title`.

4. **I do:** Click session A's tab and press ⌥⌘M.
   **I see:** The Move dialog opens over the session. It shows the plan for box-1: the branch, the size, and what stays on This Mac until the move finishes. I pick box-1 and press Move.
   **State and events:** The dialog asks the daemon for a move. Nothing moves until I press Move.
   **Status:** Fakes only. `desktop/tests/native/fork_move.rs::the_move_dialog_matches_frame_63_and_moves_to_the_picked_machine` drives the dialog headless. No test presses ⌥⌘M itself. The key is bound at `ui/src/keys.rs:109`, and the palette route is covered by `desktop/tests/live_flows/find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds`.

5. **I do:** Nothing while it moves. The copy runs, and the agent starts on box-1.
   **I see:** The session shows Moving, then Running on box-1. The agent continues the same conversation. Its branch and its work are the same.
   **State and events:** The session keeps its id, branch, work and conversation. This Mac's folder for the session is freed only after box-1 runs it.
   **Status:** Works (real daemon). `daemon/humatad/tests/move_session.rs::moving_from_this_mac_frees_its_folder_only_after_it_runs_on_box_1`. The box-to-box variant is `daemon/humatad/tests/move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes`.

6. **I do:** Nothing. The shell tab of session A is still open.
   **I see:** The tab comes back on box-1 with its earlier output, and I can type into it.
   **State and events:** The shell tab is recreated on box-1, and its output is restored.
   **Status:** Works (real daemon). `daemon/humatad/tests/move_session.rs::a_moved_session_s_shell_tab_comes_back_on_the_new_machine_with_its_output`.

7. **I do:** Click into session A's terminal during the move.
   **I see:** The terminal is read-only until the session resumes on box-1. Typing does nothing until then.
   **State and events:** Input is held by the daemon during the move. It is not sent to This Mac's old holder.
   **Status:** Works (real daemon). `daemon/humatad/tests/move_session.rs::a_moving_session_s_terminal_is_read_only_until_it_resumes`.

8. **I do:** Open the file manager on box-1, select `notes/plan.md`, press Copy, and pick This Mac's `payments` folder as the destination.
   **I see:** A transfer appears in the transfers tray. When it ends, `plan.md` is in the folder on This Mac, with the same bytes. The file on box-1 is still there.
   **State and events:** The transfer is a copy. Its source on box-1 is left as it was.
   **Status:** Code, no test. The copy from a machine to This Mac has no test. The nearest tests are `daemon/humatad/tests/remote_transfers.rs::a_move_from_a_machine_puts_the_source_in_that_machines_trash_only_once_it_is_done` (a move in that direction, Works real daemon) and `daemon/humatad/tests/remote_transfers.rs::a_folder_copied_from_one_machine_to_another_arrives_whole` (a copy between two boxes).

9. **I do:** Nothing. box-1's link to This Mac drops (the network goes away).
   **I see:** box-1 shows as offline on the sidebar and on its page. Session B shows as offline, and its last output stays on screen.
   **State and events:** The host marks box-1 as offline. Session B is not stopped; it keeps running on box-1. The host does not start it again.
   **Status:** Works (real daemon). `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice`. The offline label and the last detail staying in the app are Fakes only: `desktop/tests/machines_flow.rs::an_offline_machine_keeps_its_last_detail_and_reconnects`.

10. **I do:** Click session B's terminal and type a line.
    **I see:** The daemon sends a line on its own saying the connection to box-1 was lost, that the session keeps running there, and that this view will catch up. The app does not show that line.
    **State and events:** The daemon sends a Disconnected notice to the terminal. The app's terminal link drops it.
    **Status:** Likely bug. The daemon sends the notice at `daemon/humatad/src/remote_terminal.rs:596`. `clients/terminal_link/src/lib.rs:191` drops every Notice frame, with the comment "Humata's own lines are the app's to draw", and nothing in the app draws them. No test asserts that the line reaches the app. Confirm by dropping the link during a live session and reading the terminal.

11. **I do:** Press ⌥⌘M on session B and pick This Mac as the destination.
    **I see:** The move is refused at once. The message says why (box-1 is offline), and session B does not change.
    **State and events:** No move is recorded. Session B stays on box-1.
    **Status:** Works (real daemon). `daemon/humatad/tests/move_session.rs::a_move_to_an_offline_machine_or_one_outside_the_channel_is_refused_and_changes_nothing`.

12. **I do:** Start a fork of session B onto This Mac, or onto box-1 while it is offline.
    **I see:** The fork is refused with the reason, and no branch is made.
    **State and events:** Nothing is recorded; the parent session keeps running.
    **Status:** Works (real daemon). `daemon/humatad/tests/fork_remote.rs::a_fork_onto_an_offline_box_is_refused_and_leaves_nothing`.

13. **I do:** Create a repo in `payments` for box-1 while it is offline: add `acme/web`.
    **I see:** The repo is listed as waiting for box-1. Its clone on box-1 is not made yet.
    **State and events:** The request is kept. The clone is made when the link returns.
    **Status:** Works (real daemon). `daemon/humatad/tests/remote.rs::a_repo_added_while_the_machine_is_offline_waits_then_clones_when_the_link_returns`.

14. **I do:** Open box-1's files while it is offline.
    **I see:** The listing names box-1 as offline. It does not show This Mac's files as box-1's.
    **State and events:** The listing is refused by the host, and nothing is sent to box-1.
    **Status:** Works (real daemon). `daemon/humatad/tests/remote_files.rs::with_the_link_down_a_listing_names_the_machine_offline_and_works_again_when_it_is_back`.

15. **I do:** Nothing for an hour. The link returns.
    **I see:** box-1 shows as online. Session B is live again, and it was not started a second time. Its history catches up with what happened while the link was down. The repo from step 13 is cloned on box-1.
    **State and events:** The host reattaches session B. The machine's history and the waiting repo catch up in order.
    **Status:** Works (real daemon). `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice`, `daemon/humatad/tests/remote_history.rs::a_machine_that_was_offline_catches_up_when_its_link_returns`, and `daemon/humatad/tests/remote.rs::a_repo_added_while_the_machine_is_offline_waits_then_clones_when_the_link_returns`.

## Branches

### Fork a session onto box-1

- **When:** I fork session A to box-1 with its work and its context.
- **I do:** Press Fork, pick box-1 and a new branch name, and press Fork.
- **I see:** A new session on box-1 with a branch that has A's work, and a conversation copied from A.
- **Expected outcome:** A keeps running on This Mac. The new session is on box-1 with the same work.
- **Status:** Works (real daemon). `daemon/humatad/tests/fork_remote.rs::code_and_context_fork_onto_box_1_has_the_parents_work_on_a_new_branch_and_a_copied_conversation`.

### A typed branch name is bad or taken

- **When:** I type a branch name for the fork that is not valid, or that box-1 already has.
- **I do:** Type the name and press Fork.
- **I see:** The fork is refused before anything is copied, and the message says why.
- **Expected outcome:** Nothing is copied and nothing is made.
- **Status:** Works (real daemon). `daemon/humatad/tests/fork_remote.rs::a_bad_or_taken_typed_branch_is_refused_when_the_fork_is_asked_for`.

### A context-only fork goes to another machine

- **When:** I choose context only (no code) for a fork onto box-1.
- **I do:** Pick context only, pick box-1, and press Fork.
- **I see:** The fork is refused, and the message says that a context-only fork cannot go to another machine.
- **Expected outcome:** Nothing is made.
- **Status:** Works (real daemon). `daemon/humatad/tests/fork_remote.rs::a_context_only_fork_onto_another_machine_is_refused`.

### The copy is cut off when box-1 goes away

- **When:** the link drops in the middle of a copy from box-1.
- **I do:** Nothing. The copy waits.
- **I see:** The transfer shows as waiting for box-1. When the link returns, it finishes.
- **Expected outcome:** The copy is finished, with the same bytes. It is not started over from nothing.
- **Status:** Works (real daemon). `daemon/humatad/tests/remote_transfers.rs::a_copy_whose_link_is_lost_part_way_waits_for_the_machine_and_finishes_when_it_is_back`.

### A transfer is started while box-1 is offline

- **When:** I start a copy to box-1 while it is offline.
- **I do:** Start the copy.
- **I see:** The transfer is accepted and waits for box-1. It does not fail.
- **Expected outcome:** The transfer runs when box-1 is back.
- **Status:** Works (real daemon). `daemon/humatad/tests/remote_transfers.rs::a_transfer_started_while_the_machine_is_offline_is_accepted_and_waits_for_it`.

### A copy left on box-1 by a move that was cut off

- **When:** a move to box-1 was cut off after part of its copy was on box-1, and box-1 is offline.
- **I do:** Nothing. When box-1 returns, its link comes up.
- **I see:** The leftover copy on box-1 is taken back, and the session stays on This Mac.
- **Expected outcome:** No stray copy stays on box-1 after the link returns.
- **Status:** Works (real daemon). `daemon/humatad/tests/move_session.rs::a_copy_left_on_an_offline_target_is_taken_back_when_its_link_is_up`.

### The host restarts during a fork or a move

- **When:** This Mac's daemon stops while a fork or a move to box-1 is being copied.
- **I do:** Start the daemon again.
- **I see:** The restart report names the fork or move and says whether it was undone or finished.
- **Expected outcome:** The fork or move is either undone or finished. Nothing is left half made.
- **Status:** Works (real daemon). `daemon/humatad/tests/fork_remote.rs::a_fork_cut_short_by_a_host_restart_is_undone_or_finished` and `daemon/humatad/tests/move_session.rs::a_move_cut_short_after_the_copy_is_undone_at_restart`.

### This Mac is closed while session B keeps working

- **When:** I close the lid of This Mac for the day, and session B is on box-1.
- **I do:** Open This Mac again later.
- **I see:** Session B kept working on box-1 while This Mac was closed. What it did is there when This Mac returns.
- **Expected outcome:** Nothing is lost while This Mac is closed, and the host shows the same session when it returns.
- **Status:** Works (real daemon). `daemon/humatad/tests/flow_26_10.rs::flow_26_10_a_person_works_on_devbox_1_with_the_mac_closed_and_the_mac_sees_it_on_return`.

### Another machine's storage is read from This Mac

- **When:** I open Storage for box-1 from This Mac.
- **I do:** Open Hosts, select box-1, and press Storage.
- **I see:** The app says that box-1's storage cannot be read from This Mac yet. It does not show This Mac's storage as box-1's.
- **Expected outcome:** A clear refusal that names the gap. Nothing is shown as box-1's.
- **Status:** Not built (FILES milestone, an open lane). `daemon/humatad/src/machine_service.rs:650` answers `not_built` for storage of another machine. The machine's own storage is Works (real daemon): `daemon/humatad/tests/machines.rs::the_storage_page_reads_this_machines_own_storage_and_no_other`.

### A macOS machine is used as a remote

- **When:** I try to run sessions on a Mac that is not This Mac.
- **I do:** Add the Mac as a machine, or start a session on it.
- **I see:** The add is refused. The message says Humata 2 sets up Linux machines only.
- **Expected outcome:** A refusal with the reason. No sessions are started on a Mac.
- **Status:** Fakes only. `daemon/machines/tests/setup.rs::the_check_refuses_machines_setup_cant_use` runs with a fake runner, not a real Mac. Remote Macs that keep their sessions alive after a logout need launchd, which is Not built (see the gaps).

### Open the app in a browser on a second computer

- **When:** I want to see session A in a browser on the second computer.
- **I do:** Open the browser on the second computer and go to Humata 2.
- **I see:** Nothing. There is no web listener to connect to.
- **Expected outcome:** Not possible yet.
- **Status:** Not built (ANYHOST, blocked; the web listener is an open gap). See `OVERVIEW.md` lines 114 and 576.

## Expected outcomes

- [ ] A session started on box-1 runs there, and This Mac relays its terminal.
- [ ] A moved session keeps its id, branch, work and conversation, and frees This Mac's folder only after box-1 runs it.
- [ ] A shell tab comes back on box-1 with its output.
- [ ] While a move runs, the terminal is read-only.
- [ ] A copy from box-1 to This Mac leaves the source as it was.
- [ ] When box-1 goes offline, its sessions keep running and are not started again.
- [ ] A move, a fork or a file listing against an offline box is refused, or waits, and says why.
- [ ] When the link returns, sessions reattach, history catches up, and waiting clones are made.
- [ ] A fork onto another machine keeps the parent running, and a context-only fork is refused across machines.
- [ ] Nothing is left half made after a restart in the middle of a fork or a move.
- [ ] A second computer's browser can open Humata 2. (Not built.)

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Machine page and health | Works (real daemon) | `daemon/humatad/tests/machines.rs::a_linked_machines_page_shows_the_health_its_probe_read` |
| 2. Session started on box-1 | Works (real daemon) | `daemon/humatad/tests/remote.rs::a_session_started_on_the_machine_runs_there_and_the_host_relays_its_terminal` |
| 3. Session on This Mac | Works (real daemon) | `daemon/humatad/tests/sessions.rs::a_started_session_has_its_clone_on_a_branch_named_from_its_title` |
| 4. Move dialog and ⌥⌘M | Fakes only | `desktop/tests/native/fork_move.rs::the_move_dialog_matches_frame_63_and_moves_to_the_picked_machine` |
| 5. Move runs, same session | Works (real daemon) | `daemon/humatad/tests/move_session.rs::moving_from_this_mac_frees_its_folder_only_after_it_runs_on_box_1` |
| 6. Shell tab comes back with output | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_moved_session_s_shell_tab_comes_back_on_the_new_machine_with_its_output` |
| 7. Terminal read-only during a move | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_moving_session_s_terminal_is_read_only_until_it_resumes` |
| 8. Copy a file from box-1 to This Mac | Code, no test | Move direction tested; copy direction not (see step) |
| 9. box-1 goes offline, session keeps running | Works (real daemon) | `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice` |
| 10. Terminal line about the lost link | Likely bug | `clients/terminal_link/src/lib.rs:191` |
| 11. Move to This Mac refused while offline | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_move_to_an_offline_machine_or_one_outside_the_channel_is_refused_and_changes_nothing` |
| 12. Fork onto an offline box refused | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::a_fork_onto_an_offline_box_is_refused_and_leaves_nothing` |
| 13. Repo waits for box-1 | Works (real daemon) | `daemon/humatad/tests/remote.rs::a_repo_added_while_the_machine_is_offline_waits_then_clones_when_the_link_returns` |
| 14. Files on an offline box named offline | Works (real daemon) | `daemon/humatad/tests/remote_files.rs::with_the_link_down_a_listing_names_the_machine_offline_and_works_again_when_it_is_back` |
| 15. Link returns, catch up | Works (real daemon) | `daemon/humatad/tests/remote_history.rs::a_machine_that_was_offline_catches_up_when_its_link_returns` |
| Branch: fork onto box-1 | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::code_and_context_fork_onto_box_1_has_the_parents_work_on_a_new_branch_and_a_copied_conversation` |
| Branch: bad or taken branch name | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::a_bad_or_taken_typed_branch_is_refused_when_the_fork_is_asked_for` |
| Branch: context-only fork across machines | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::a_context_only_fork_onto_another_machine_is_refused` |
| Branch: copy cut off when box-1 goes away | Works (real daemon) | `daemon/humatad/tests/remote_transfers.rs::a_copy_whose_link_is_lost_part_way_waits_for_the_machine_and_finishes_when_it_is_back` |
| Branch: transfer started while offline | Works (real daemon) | `daemon/humatad/tests/remote_transfers.rs::a_transfer_started_while_the_machine_is_offline_is_accepted_and_waits_for_it` |
| Branch: leftover copy on an offline target | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_copy_left_on_an_offline_target_is_taken_back_when_its_link_is_up` |
| Branch: host restart during fork or move | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::a_fork_cut_short_by_a_host_restart_is_undone_or_finished` |
| Branch: This Mac closed, session keeps working | Works (real daemon) | `daemon/humatad/tests/flow_26_10.rs::flow_26_10_a_person_works_on_devbox_1_with_the_mac_closed_and_the_mac_sees_it_on_return` |
| Branch: another machine's storage | Not built | FILES milestone (open); `daemon/humatad/src/machine_service.rs:650` |
| Branch: a macOS machine as a remote | Fakes only | `daemon/machines/tests/setup.rs::the_check_refuses_machines_setup_cant_use` (fake runner; launchd not built) |
| Branch: second computer's browser | Not built | ANYHOST (blocked); `OVERVIEW.md:114` |

## Gaps and likely bugs

- **Not built (FILES milestone, open):** storage of another machine. `daemon/humatad/src/machine_service.rs:650` answers `not_built`.
- **Not built (MACH T10):** launchd for macOS remotes. `OVERVIEW.md:559`. A Mac can only be a host today; the setup refuses it as a remote.
- **Not built (ANYHOST, blocked; LINUX):** the web listener, gRPC-web on loopback with pairing. `OVERVIEW.md:114` and `:576`.
- **Not built (no milestone):** DiffService on the peer link, so a diff of a session on box-1 cannot be read from This Mac. `OVERVIEW.md:503`.
- **Not built (no milestone, OVERVIEW gap):** running the peer link over real SSH. `OVERVIEW.md:563`. Every remote test here uses a fake `ssh` that runs the real `humatad2`.
- **Code, no test:** copying a file from a machine to This Mac (step 8).
- **Likely bug:** Notice lines are dropped. `clients/terminal_link/src/lib.rs:191` drops every Notice frame, so the "connection lost, the session keeps running" line from `daemon/humatad/src/remote_terminal.rs:596` never reaches the app. Confirm by dropping the link during a live session.
- **Likely bug (low):** ⌥⌘M does nothing when no session is open. `ui/src/shell.rs:541` to `:549` act only when the route is a session, and say nothing otherwise. The key is bound at `ui/src/keys.rs:109`. A key that does nothing with no hint is the kind of silent miss to fix.
- **Correction to the brief:** the brief's pointer to `desktop/tests/machines_flow.rs` and `desktop/tests/native/*` names fake-backed suites. Their rows above are Fakes only.
- **Correction to the brief:** the brief says the move's shortcut is the way to start a move from the palette. The palette route is `desktop/tests/live_flows/find.rs`, which opens Move session. The key itself is not pressed by any test.
