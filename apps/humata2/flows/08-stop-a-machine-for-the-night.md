# 8. Stopping a machine for the night, and working away from the Mac

> "Hey, devbox-1 is burning money overnight. Let me stop everything on it and start it again in the morning. Oh, and tomorrow I'm on my laptop without the Mac."

Two things that share a machine page. First, I want a big switch that ends every agent and shell on a machine, keeps all it needs to come back, and holds even across reboots. Second, I want to keep working on devbox-1 when the Mac is closed, answer agents' asks there, and have the Mac catch up when it's back.

How to read this file, and what the tags mean: [README](README.md).

## Starting state

- devbox-1 is linked, with three sessions: two agents working, one idle.
- One agent has a pending Inbox request.
- `#ledger`'s **While I'm away** rule is Wait.
- An approval passphrase has been set.

## Steps

### Stop for the night

1. **Do:** on devbox-1's machine page, open **More** and pick **Stop all sessions…**.
   **See:** "Stop all sessions on devbox-1?", saying every agent and shell will end and come back on Start, with an optional reason. Nothing stops until I confirm.
   **Status:**
   - **Fakes only**: `desktop/tests/machines_flow.rs::stop_all_sessions_asks_first_and_stops_only_when_confirmed`, and `features/hosts/tests/stop.rs::stop_all_sessions_asks_first_and_sends_nothing_until_confirmed` and `::cancelling_the_stop_confirm_keeps_the_sessions_running`. Left to the STOP lane.
   - Frames 72–76 have no recorded snapshots yet.

2. **Do:** type "overnight" as the reason and confirm.
   **See:** the page reads "Sessions stopped", with the reason and **Start sessions**. Each session shows as Machine stopped.
   **Expect:**
   - Every agent and shell the sessions started is ended, gracefully first. A shell that ignores hang-up is killed.
   - What a restart needs is saved, and no restart report is written.
   - The agent's pending request is withdrawn, marked stopped with its machine.
   - A terminal that was attached is told to connect again later.
   - Events: Session updated (Machine stopped) for each; InboxRequest updated (withdrawn); Machine updated.
   **Status:**
   - **Works** in the daemon: `daemon/humatad/tests/stop_sessions.rs::a_stop_ends_every_agent_and_shell_the_sessions_started_and_writes_no_restart_report`, `::a_shell_that_ignores_hangup_and_terminate_is_killed_by_the_stop`, `::a_stop_withdraws_the_agents_pending_inbox_request_as_stopped_with_its_machine` and `::a_terminal_attached_when_the_sessions_stop_is_told_to_connect_again`.
   - From This Mac to devbox-1 over a fake `ssh`: `daemon/humatad/tests/stop_box.rs::a_host_stops_a_linked_boxs_sessions_and_starts_them_again_with_a_report_naming_the_stop`.
   - The page after the stop is **Fakes only**: `features/hosts/tests/stop.rs::confirming_the_stop_shows_sessions_stopped_after_the_reload`. Left to the STOP lane.
   - STOP is open, and its lane says "Ready to commit: no". Its last task waits on SAFE.

3. **Do:** try **Open a shell** or **New session** on devbox-1, or type into a stopped session.
   **See:** a note saying the machine's sessions are stopped, and to start them first.
   **Expect:** agents and scripts are refused too. Nothing can be typed, attached or started.
   **Status:**
   - The daemon **Works**: `stop_sessions.rs::while_stopped_agents_and_scripts_are_refused_and_nothing_can_be_typed_attached_or_started`.
   - The note is **Fakes only**: `desktop/tests/machines_flow.rs::while_stopped_open_a_shell_is_refused_with_a_note` and `features/hosts/tests/stop.rs::while_stopped_a_shell_or_new_session_is_refused_and_says_start_first`. Left to the STOP lane.

4. **Do:** nothing. devbox-1 reboots overnight for updates.
   **Expect:** it comes back still stopped and starts nothing.
   **Status: Works** in the daemon: `stop_sessions.rs::a_daemon_killed_and_started_again_while_stopped_stays_stopped_and_starts_nothing`. A power cut at each step of a stop or a start ends either stopped or running, with nothing lost: `stop_sessions.rs::a_stop_cut_once_the_holders_are_signalled_ends_stopped_with_nothing_lost` and its siblings.

5. **Do (other case):** devbox-1 was offline when I confirmed the stop.
   **See:** "Stop not delivered", with **Cancel stop** (no confirm needed) and **Reconnect now**.
   **Expect:**
   - The stop goes through the one-off `ssh` if the link hangs.
   - If it can't reach the machine either way, the stop is kept across a restart of This Mac's daemon and applied once when devbox-1 links back.
   **Status:**
   - The daemon **Works**: `stop_box.rs::a_box_whose_link_hangs_is_still_stopped_through_its_one_off_ssh` and `::a_stop_that_reaches_a_box_neither_way_is_kept_across_a_host_restart_and_lands_once_when_the_box_links_back`.
   - Cancel stop is **Fakes only**: `desktop/tests/machines_flow.rs::a_stop_that_has_not_reached_the_machine_is_cancelled_without_a_confirm`. Left to the STOP lane.
   - **Reconnect now**: **Not built, no milestone** (HO-6). The live client answers "not in this release".

6. **Do:** in the morning, click **Start sessions** and confirm.
   **See:** "Start sessions on devbox-1?", then each session back as after a shutdown.
   **Expect:**
   - Agents resume in their conversations, and shells come back in their folders.
   - A restart report names the stop and its reason.
   - Events: Session updated (Starting, then Working or Idle).
   **Status:**
   - The daemon **Works**: `stop_sessions.rs::start_brings_the_sessions_back_as_after_a_shutdown_with_a_report_naming_the_stop`.
   - The confirm is **Fakes only**: `desktop/tests/machines_flow.rs::start_sessions_asks_first_and_starts_only_when_confirmed` and `features/hosts/tests/stop.rs::the_hosts_row_starts_a_stopped_machine_after_its_confirm`. Left to the STOP lane.

7. **Do (other case):** an agent runs `humata host stop devbox-1`, or I run `humata stop` in a shell on the machine.
   **Expect:**
   - Under the default rule the agent is denied at once, with a fix, and nothing stops.
   - Under Ask, the stop waits in the Inbox.
   - Under Allow, the agent stops the machine, itself included, and is named as the stopper. Only I can start it again.
   **Status:**
   - The daemon **Works**: `daemon/humatad/tests/stop_access.rs::an_agents_humata_stop_under_the_default_rule_exits_denied_with_a_fix`, `::a_rule_of_ask_puts_an_agents_stop_in_the_inbox_and_stops_nothing_meanwhile`, `::an_agent_whose_rule_allows_it_stops_its_machine_itself_included_and_is_named_as_the_stopper` and `::an_agent_whose_rule_allows_it_still_cannot_start_the_users_stop`.
   - The CLI is tested against a fake daemon only: `cli/tests/stop.rs::stop_asks_the_daemon_once_from_the_cli_with_the_reason_and_a_stop_id`.

### Working away from the Mac

8. **Do:** close the Mac. From another computer, `ssh` to devbox-1, run `humata login` and type the approval passphrase.
   **See:** a login for this terminal. A wrong passphrase leaves the terminal as it was.
   **Expect:**
   - The passphrase works only while the Mac is away.
   - Five wrong tries lock it, and the lock survives a restart.
   **Status:**
   - **Works** in the daemon: `daemon/humatad/tests/away_login.rs::the_approval_passphrase_logs_in_on_a_machine_while_its_host_is_away`, `::five_wrong_tries_lock_the_passphrase_and_a_restart_keeps_the_lock` and `::a_machine_whose_host_is_reachable_refuses_the_passphrase`.
   - The real `humata login` typed at a pty prompt and refused from a pipe: `away_login.rs` above and `daemon/humatad/tests/away_login_pipe.rs::the_passphrase_piped_into_login_is_not_read_and_a_typed_one_still_logs_in`.
   - Setting the passphrase has a CLI only (`humata access passphrase`). In the app: **Not built, no milestone.**

9. **Do:** run `humata ls`, then `humata attach "Backfill"`. Type into the agent, then detach.
   **See:** the agent's screen. After I detach, the session keeps running.
   **Status: Works** with the real `humata` binary against real daemons, over a fake `ssh`: `daemon/humatad/tests/flow_26_10.rs::flow_26_10_a_person_works_on_devbox_1_with_the_mac_closed_and_the_mac_sees_it_on_return`.

10. **Do:** an agent asks to read another session's files. `#ledger`'s While I'm away rule is Wait, so it waits. Run `humata inbox`, read it and approve it.
    **Expect:**
    - The request waits in devbox-1's own Inbox, and my answer settles it there.
    - Events, on devbox-1: InboxRequest created (Pending), then updated (Approved); Grant created.
    **Status: Works**, same flow test, and `flow_26_10.rs::flow_26_10_a_request_to_read_another_sessions_files_waits_and_is_answered_on_the_machine`.

11. **Do:** open the Mac again.
    **See:** the Inbox shows the request as answered on devbox-1. Backfill shows what it printed meanwhile.
    **Expect:**
    - devbox-1 replays its Inbox journal to This Mac, in order. Entries This Mac never acknowledged are sent again after a restart.
    - If I also answered it on the Mac, the first answer wins.
    - Events: JournalReplayed; InboxRequest updated.
    **Status:**
    - The daemon **Works**: `daemon/humatad/tests/away_inbox.rs::a_request_answered_on_the_machine_while_away_reaches_the_host_in_order` and `::entries_the_host_never_acked_are_sent_again_after_a_restart`; `daemon/humatad/tests/away_replay.rs::a_relay_for_a_request_the_machine_already_answered_changes_nothing`.
    - **Works** end to end in the daemon with the real `humata` binaries: `flow_26_10.rs::flow_26_10_a_person_works_on_devbox_1_with_the_mac_closed_and_the_mac_sees_it_on_return` shows the host's Inbox with the request answered at devbox-1.
    - The app **Works** across the link going and coming back: `desktop/tests/live_flows/away_box.rs::a_request_answered_on_devbox_1_while_the_link_was_down_is_settled_when_the_link_returns` runs the app with the real machines client on a host linked to devbox-1. The app shows devbox-1 offline when the link is cut and online when it returns, the request answered on devbox-1 meanwhile reaches the host's Inbox answered at devbox-1's CLI, and the app never holds it as waiting (D397).
    - The app showing the request as answered on devbox-1 is **Not built**, **no milestone**: the live Inbox client lists waiting requests only (`include_answered: false`) and the app's `Request` has no field for where it was answered.

## Expected outcome

devbox-1 ran nothing overnight, and came back in the morning with every session in its conversation and a report naming the stop. While away, I worked on devbox-1 and answered its asks there. The Mac caught up when it opened, without asking me again.

## Gaps this flow shows

- The stop and start screens have never run against a real daemon, and their frames have no snapshots. STOP isn't ready to commit.
- **Reconnect now** isn't built, so a stop that wasn't delivered can only be cancelled or left to land later.
- The approval passphrase can be set from the CLI only.
- No live-app test shows the Inbox merged from a machine's journal: the desktop live rig has no second machine (the daemon side is tested).
- Every remote step runs over a fake `ssh`. A real remote machine is LINUX, which is blocked.
