# 9. A plain terminal on a remote machine

> "Hey, I just want a shell on devbox-1 to tail some logs and poke at the database. No agent. But I want it to survive like everything else."

Sometimes the work is me in a shell, not an agent. I want a Terminal session on devbox-1 that I can split beside an agent, copy from and paste into, and that survives the Mac sleeping, the app restarting and the link dropping. Inside an agent's session I also want a `zsh` tab in the same folder.

How to read this file, and what the tags mean: [README](README.md).

## Starting state

- devbox-1 is linked, and `#ledger` has a clone of `acme/ledger` on it.
- "Backfill" runs on devbox-1 in `#ledger`, with its agent working.

## Steps

### A shell tab inside an agent's session

1. **Do:** on Backfill, click **New terminal** in the tab strip.
   **See:** a `zsh` tab in the session's folder, beside the agent's tab, with the cursor ready.
   **Expect:**
   - The shell runs on the session's machine, in its clone.
   - Typing goes to the selected tab without an extra click.
   - Event: Session updated (a new tab).
   **Status:**
   - On This Mac: **Works**, `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` (real daemon and holder). The new tab taking typing without a click **Works** in the app on a real daemon: `desktop/tests/live_flows/terminals.rs::a_new_terminal_tab_takes_typing_without_a_click_and_runs_the_command` (the tab strip's menu row is sent as its action; clicking the plus button itself is checked on fakes, `desktop/tests/native/terminal.rs::new_sessions_and_selected_tabs_receive_input_without_an_extra_click`).
   - On devbox-1: the daemon relays a remote session's terminal over a fake `ssh` (`daemon/humatad/tests/remote.rs::a_session_started_on_the_machine_runs_there_and_the_host_relays_its_terminal`). No test opens a shell tab on a remote session.

2. **Do:** type `tail -f log/backfill.log`. Select a few lines and press ⌘C. Resize the window.
   **See:**
   - The output scrolls as it comes.
   - The copy is exact, including Unicode and multi-line text.
   - The program redraws to the new size.
   **Expect:** the shell gets the size the window attaches with.
   **Status:**
   - The daemon **Works**: `daemon/humatad/tests/terminal.rs::typed_input_comes_back_as_contiguous_output` and `::the_size_a_writer_attaches_with_reaches_the_shell`.
   - Drawing through the app's terminal is **Fakes only**: `desktop/tests/terminal_replica.rs::typing_in_a_new_shell_reaches_its_program_and_the_answer_is_drawn` and `::the_grid_has_the_fitted_size_and_the_agent_redraws_to_it`. Left to the TERM lane.
   - Exact copy, paste back and resize over real SSH are proven only in the standalone terminal experiment, not through Humata: `terminal/lab/tests/clipboard_ssh.rs::ssh_unicode_multiline_color_copy_pasteback_and_focus_are_exact` and `terminal/lab/tests/vertical_resize_ssh.rs`, against the Lima VM (see [SSH demo setup](../terminal/platform_test/SSH_DEMO.md)).

### A Terminal session of its own

3. **Do:** on `#ledger`, click **New session**. Pick **Terminal** as what it runs, devbox-1 as the machine, and click **Start session**.
   **See:** a session with one `zsh` tab, no agent and no turns.
   **Expect:**
   - The session's status is Idle or Exited; it never reads Working.
   - Event: Session created.
   **Status:**
   - The sheet **Works** on This Mac against a real daemon: `desktop/tests/live_flows/terminals.rs::a_terminal_started_from_new_session_runs_a_shell_that_answers_and_never_reads_working`.
   - On This Mac, starting it through the app's client **Works** (real daemon and holder): `clients/live/tests/term_restart.rs::a_terminal_session_starts_a_shell_with_no_turns`. The daemon runs its shell as the agent `zsh` (D312).
   - On devbox-1: no test starts a Terminal session on a remote machine.

4. **Do:** on devbox-1's machine page, click **Open a shell**.
   **See:** the same sheet with Terminal picked.
   **Expect:** a shell in the machine's home folder, not tied to a channel.
   **Status:**
   - The button **Works** on This Mac against a real daemon: `desktop/tests/live_flows/terminals.rs::open_a_shell_on_a_machine_page_starts_a_terminal_there`.
   - It starts a Terminal session as in step 3, so it runs in a channel's session folder, not the machine's home folder.
   - A shell tied to no channel: the daemon's own machine shell **Works** on This Mac (`daemon/humatad/tests/terminal.rs::machine_shells_are_listed_with_their_offsets`), but the app never calls it. **Not built, no milestone.**

### It survives

5. **Do:** quit Humata and open it again.
   **See:** the shell tab shows what `tail` printed while the app was gone. Typing waits until the tab has caught up.
   **Status: Works** on This Mac: `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced`. Also `daemon/humatad/tests/terminal.rs::a_tab_outlives_its_daemon` and `::reattaching_from_the_cursor_sends_only_newer_output`.

6. **Do:** put the Mac to sleep for an hour, then wake it.
   **See:** the tab reattaches. If it fell too far behind, it shows the current screen instead of replaying everything.
   **Expect:** the link drops and comes back; the shell on devbox-1 never stopped.
   **Status:**
   - Resync **Works** in the daemon: `daemon/humatad/tests/lagged_attach.rs::an_attach_that_fell_behind_is_resynced_with_the_current_screen`.
   - The link coming back **Works** over a fake `ssh`: `daemon/humatad/tests/two_machines.rs::a_lost_link_comes_back_and_the_session_is_the_same`.
   - Over real SSH, reconnecting and resuming are proven only in the terminal experiment (`terminal/host:ssh_reconnect_test`, `:agent_resume_ssh_test`, `:hard_kill_ssh_test`).

7. **Do:** devbox-1 reboots.
   **See:** the tab comes back as a fresh shell in the same folder. The note says `tail` was lost.
   **Expect:** a shell can't resume what it was running; only its folder and scrollback come back.
   **Status:**
   - On This Mac, **Works** in the daemon: `daemon/humatad/tests/restart.rs::a_shell_whose_holder_died_comes_back` and `::a_shell_that_cant_come_back_is_one_line_and_the_others_still_do`.
   - On a real remote machine: **Not built: LINUX** (blocked).

## Expected outcome

I have a `zsh` tab in Backfill and a Terminal session on devbox-1. Both survive the app restarting and the link dropping. After a reboot they come back as fresh shells in their folders, with their scrollback, and say what they lost.

## Gaps this flow shows

- **Open a shell** on a machine page starts a Terminal session in a channel's folder; the daemon's machine shell (a shell tied to no channel) is never used. No milestone.
- Humata has never driven a real SSH machine. The SSH terminal work (exact copy, resize, reconnect, kill and resume) is proven only in the standalone terminal experiment, and a real machine for humatad2 is LINUX, which is blocked.
- No test opens a shell tab on a remote session.
- Drawing a shell in the app runs only on fakes.
