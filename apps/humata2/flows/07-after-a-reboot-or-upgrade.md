# 7. After a reboot, a power cut or an upgrade

> "Hey, my Mac just rebooted for an update. Did my sessions survive? And there's a new Humata, so let me upgrade everything."

I left two agents working on This Mac and one on devbox-1. I want them all back where they were, in the same conversations, without retyping anything. When I install a new Humata, every machine should end up on it, and nothing should run on mixed versions.

How to read this file, and what the tags mean: [README](README.md).

## Starting state

- On This Mac:
  - "Fix export rounding" is mid-turn, with its agent working.
  - "Read path" is idle.
  - "Read path" also has a `zsh` tab running `tail -f` on a log.
  - "Old spike" is idle, and its conversation file has been deleted by hand.
- On devbox-1, "Backfill" is working.
- Humata is installed in `/Applications`, and humatad starts at login.

## Steps

### This Mac reboots

1. **Do:** the Mac reboots. Log in.
   **See:** nothing yet; Humata isn't open.
   **Expect:**
   - humatad starts at login, takes its lock and reads the session records.
   - It finds the local holders gone. It resumes each running agent in its pinned conversation, and brings plain shells back in their folders.
   - It checks that each resume came up, then tells each agent it was interrupted and to check `git status`.
   - devbox-1's sessions were never interrupted.
   **Status:**
   - The resume **Works** in the daemon: `daemon/humatad/tests/sessions.rs::a_reboot_brings_back_the_session_and_a_shell` and `::a_lost_holder_is_resumed_in_its_conversation_after_a_restart`; `daemon/revive/tests/kill_matrix.rs::a_killed_holder_is_resumed_with_its_conversation_and_its_output`.
   - Start at login is installed by the app: **Works** with a fake launchd, `desktop/install/tests/install.rs::first_install_copies_everything_and_registers`. A real reboot has never been run in a test.

2. **Do:** open Humata.
   **See:** each session tab shows its screen and what was printed while the app was gone. "Backfill" on devbox-1 reattaches with a snapshot and its output since.
   **Expect:**
   - The app attaches to the same holders, and no input is replayed.
   - Typing waits until the tab has caught up.
   - Events: Session updated for each, back to Working or Idle.
   **Status:**
   - **Works** locally: `desktop/tests/live_flows.rs::after_a_reboot_the_session_is_running_again_in_its_conversation` (real daemon and holders killed together, then restarted on the same folder, with a scripted agent); `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced`.
   - devbox-1 reattaching **Works** in the daemon, over a fake `ssh`: `daemon/humatad/tests/two_machines.rs::a_lost_link_comes_back_and_the_session_is_the_same`.

3. **Do:** read the toast.
   **See:** "This Mac restarted: 3 sessions started again, 1 couldn't. Mid-turn agents are idle." with **Show Home**.
   **Expect:** a newer restart of the same machine replaces the older toast.
   **Status:**
   - The toast **Works** against a real daemon that had already brought its sessions back before the app opened: `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt`. A restart known at the first load is told once (`features/app/tests/restart.rs::a_restart_the_first_load_brings_is_told_once`, D390). Replacing an older toast is checked on fakes: `features/app/tests/restart.rs::a_newer_restart_of_the_same_host_replaces_the_older_one`.
   - **Show Home**: **Not built, no milestone.** It's in the spec (26.8), but not in the code or in frame 47.
   - The toast covers This Mac only. A restart of another machine is never shown.

4. **Do:** open Home.
   **See:** "Couldn't resume after the restart · 1" with "Old spike", and "Idle after the restart · 1" with "Fix export rounding", the agent that was mid-turn (frame 47).
   **Status:**
   - "Couldn't resume after the restart" **Works** against a real daemon: `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt` (the lost session is in it, the resumed one isn't).
   - "Idle after the restart" **Works** against a real daemon, with a scripted agent that reports its turn through `humata turn` as a real agent's hooks do: `desktop/tests/live_flows/restart.rs::after_a_reboot_an_agent_that_reports_idle_is_listed_under_idle_after_the_restart` (the resumed agent reports idle and Home lists it; the session that couldn't resume is not in the group). The split itself is also checked on fakes: `features/app/tests/restart.rs::home_groups_split_failed_from_idle_after_the_restart`.

5. **Do:** open "Old spike".
   **See:** "Couldn't resume" (frame 46), with **Try again** and **Start fresh with a summary**. If nothing is left to summarize, the button reads "Start fresh (nothing left to summarize)".
   **Expect:**
   - The session's status is Resume failed.
   - **Try again** resumes it if what it needed is back.
   - **Start fresh** starts a new conversation and gives it a summary of the old one.
   - Event: Session updated (Resume failed, then Starting).
   **Status:**
   - The daemon **Works**: `daemon/humatad/tests/sessions.rs::a_reboot_that_lost_the_conversation_says_the_resume_failed` and `daemon/humatad/tests/start_fresh.rs::a_failed_resume_starts_fresh_on_a_new_conversation_and_hears_the_summary`.
   - The app gets as far as the status: `desktop/tests/live_flows.rs::a_session_whose_conversation_is_lost_cannot_resume`. Opening it shows the card and no error **Works** against a real daemon: `desktop/tests/live_flows/restart.rs::opening_a_session_that_couldnt_resume_shows_no_error_and_keeps_its_card`. Opening it marks it read on the daemon and keeps the card (D392).
   - The buttons **Work** against a real daemon: **Try again** with the conversation file restored, `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt`; **Start fresh with a summary**, `desktop/tests/live_flows/restart.rs::start_fresh_on_the_couldnt_resume_card_starts_a_new_conversation_that_hears_the_summary`.

6. **Do:** click **Restart report 09:12** on the session, or run `humata doctor restarts` in a shell.
   **See:** what the daemon logged before it stopped, and which sessions weren't brought back and why.
   **Status:**
   - The report **Works** in the daemon: `daemon/humatad/tests/restart.rs::the_restart_report_has_what_the_daemon_logged_before_it_stopped` and `daemon/humatad/tests/doctor.rs::a_session_that_was_not_brought_back_is_repaired_once_what_it_needed_is_back`.
   - The CLI is tested against a fake daemon only (`cli/tests/doctor.rs::explain_reads_the_newest_report_and_says_what_wasnt_brought_back`). Left to the SAFE lane (doctor).
   - In the app: **Not built, no milestone.** The button only opens Hosts, with no report view.

### devbox-1 loses power

7. **Do:** nothing. devbox-1 loses power, then comes back.
   **See:**
   - While it's down: "devbox-1 is offline" on Backfill, and Hosts shows it offline.
   - When it's back: "devbox-1 is back. 1 session reattached."
   **Expect:**
   - Its own humatad starts at boot (systemd user service, with linger) and resumes Backfill in its conversation.
   - Programs that were running in plain shells are reported lost.
   - Events: Machine updated (offline, then online); Session updated (Offline, then Working).
   **Status:**
   - The daemon **Works** over a fake `ssh`: `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice`, plus the power-cut suites `daemon/humatad/tests/power_cut_machine.rs` and `power_cut_both.rs`.
   - "is back. n sessions reattached" and **Reconnect now**: **Not built, no milestone** (HO-6). The live client answers "not in this release" for reconnecting.
   - A real reboot of a real machine: **Not built: LINUX** (blocked).

### Upgrade

8. **Do:** open the new Humata; it installs itself.
   **See:** This Mac's daemon restarts once, and every tab is still attached.
   **Expect:**
   - The new daemon and holder are staged beside the old ones, then swapped in.
   - The daemon restarts, and its holders keep running untouched.
   - The upgrade is recorded as done once the new daemon is ready.
   - Installing the same release again changes nothing.
   **Status:**
   - **Works** in the daemon and installer: `daemon/humatad/tests/upgrade_local.rs::an_upgrade_swaps_the_staged_release_in_and_restarts_the_daemon_with_its_holders_untouched` and `::the_upgrade_record_is_settled_into_last_json_once_the_new_daemon_is_ready`; `desktop/install/tests/install.rs::changed_daemon_is_staged_beside_the_installed_one_not_swapped` and `::the_same_release_stages_nothing`.
   - Moving running holders to the new holder program: **Not built: UPGRADE** (U6).
   - UPGRADE is open, and its lane says "Ready to commit: no".

9. **Do:** open the Inbox.
   **See:** one VIP item, "Migrate devbox-1", from no agent, saying what release it's on and what it needs.
   **Expect:**
   - One item per machine, however many times its release changes.
   - **Answer all** leaves it waiting, and an agent can't answer it.
   - Until it's migrated, starting a session, typing into one, or a writable attach on devbox-1 is refused with a pointer to the item. Reading still works.
   - Event: InboxRequest created (Pending, VIP).
   **Status:**
   - **Works** in the daemon: `daemon/humatad/tests/upgrade_inbox.rs::a_machine_at_another_release_is_one_vip_migrate_item_with_no_requester`, `::answering_all_leaves_a_migrate_item_waiting` and `::an_agent_cannot_answer_a_migrate_item`; `daemon/humatad/tests/upgrade_refusals.rs::starting_a_session_on_a_machine_at_another_release_is_refused_with_its_item` and `::a_machine_at_another_release_can_still_be_read`.
   - The Migrate card: **Not built: UPGRADE** (U7). Today the live client shows it as a plain command request.

10. **Do:** click **Migrate** on the item.
    **See:** progress, then "Migrated to <release>". The item resolves.
    **Expect:**
    - The release is staged on devbox-1 over SSH and swapped in, and the machine comes back on it.
    - A power cut partway through leaves the old release whole, or rolls back to it.
    - Event: InboxRequest updated (Resolved).
    **Status:**
    - The daemon tests exist, but the lane calls that batch uncompiled, so **unverified**: `daemon/humatad/tests/upgrade_box.rs::approving_migrate_stages_the_release_over_ssh_and_the_machine_comes_back_at_it` and `daemon/humatad/tests/upgrade_cut.rs::a_cut_between_the_daemon_and_the_bundle_being_swapped_leaves_release_a_whole`.
    - The card and its progress: **Not built: UPGRADE** (U7).

11. **Do:** in an old shell, run `humata ls` with the old CLI still on the path.
    **See:** a refusal that says the CLI needs migrating and how.
    **Expect:** the old CLI still gets server info, so it can say what's wrong.
    **Status:**
    - **Works** in the daemon: `daemon/humatad/tests/upgrade_identity.rs::a_cli_at_another_release_is_refused_with_migration_needed` and `::a_cli_at_another_release_still_gets_server_info`.
    - The app noticing an old CLI and posting "Migrate the CLI on This Mac": **Not built: UPGRADE** (U7).

## Expected outcome

Every agent is back in its own conversation, and every shell is back in its folder. The one session that couldn't resume says so and offers a way forward. After the upgrade, This Mac and devbox-1 run the same release, and no holder was killed to get there.

## Gaps this flow shows

- **Show Home** and an in-app restart report aren't built, and neither has a milestone. The restart is told but can't be followed up from the toast.
- A remote machine's restart or return is never shown in the app. **Reconnect now** fails against a real daemon.
- The whole Migrate experience in the app (U7), and moving live holders to a new release (U6), are open in UPGRADE. Upgrading a remote machine over SSH is unverified.
- No test reboots a real Mac or a real Linux machine. Those runs are LINUX, which is blocked.
