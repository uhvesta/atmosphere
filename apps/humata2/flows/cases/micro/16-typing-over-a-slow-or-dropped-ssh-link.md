# Typing over a slow or dropped SSH link is sent in order, dropped while the link is down, and resumes after the screen catches up

Micro. Platforms: macOS for the app, with the SSH VM (Linux, Debian) as the remote box. Paths are relative to `apps/humata2`.

## Starting state

- Humata 2 is open on this Mac, with one session open. Its tab is a zsh shell on `box-1`, reached over SSH.
- `box-1` is the dedicated SSH VM in the lab. Its `humata2` daemon and its holder are running. The session's holder holds the shell.
- The terminal tab has keyboard focus. The tab shows `box-1`. The connection is up, and the screen is current.
- Nothing is queued. The user is typing a word, and the shell has `ec` on the command line.
- The Humata 2 host is on this Mac. The peer link to `box-1` runs over SSH.

## Steps

1. **I do:** type `ho` and then `          ` (a space), then `world`, one key at a time.
   **I see:** each key appears on the command line once, in order, with no repeats and nothing swapped.
   **State and events:** each key is sent to the shell in the order typed. The shell gets the bytes once, in order.
   **Status:** Works (real shell)
   Evidence: `terminal/host/src/roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay` (target `ssh_reconnect_test`, manual, real OpenSSH VM) injects `one` with submit and checks `COUNT=1 INPUT=one` came back from the remote shell. It sends further lines in order and checks each count. The test sends whole lines, not single keys.

2. **I do:** type on a link that lags. Press `ls` and wait two seconds before the output shows.
   **I see:** the command line fills in as the echo arrives. The echo arrives late, but in order, with no gap and no repeat.
   **State and events:** the output carries contiguous offsets. The pane draws it in order. The host does not hold input for the echo.
   **Status:** Works (real daemon)
   Evidence: `daemon/holder/tests/terminal.rs::output_carries_contiguous_offsets_and_a_reattach_resumes_from_a_cursor` checks offsets are contiguous. No test adds delay to the link, so the late echo itself is not shown. The SSH test has no added delay.

3. **I do:** the link drops while I am mid-word. I was typing `hel` and the network goes down.
   **I see:** the pane shows a disconnected banner, and the cursor stays where it was. The typed `lo` after the drop does not appear.
   **State and events:** the replica sets the pane to not connected (`terminal/client/src/lib.rs:282-289`, `terminal/client/src/replica.rs:528-541`). The pane drops keys and pastes (`terminal/pane/src/host.rs:255-259`, `:350-355`), and it shows the banner (`terminal/pane/src/pane.rs:882-884`).
   **Status:** Fakes only
   Evidence: `terminal/pane/tests/pane.rs::while_disconnected_input_is_dropped_and_the_pane_says_so` checks the banner is up, `a` and a paste are dropped while disconnected, and `b` is sent once reconnected. It runs against a recording PTY in a headless window, not a real link. The real link-drop path (`client/src/lib.rs:282-289`) has no test that goes from a dropped SSH link to the banner.

4. **I do:** keep typing `lo` while the link is down.
   **I see:** nothing is sent. The keys are gone. The banner stays up.
   **State and events:** the keys are refused, not queued. `Shared::send` returns without writing when not connected (`terminal/pane/src/host.rs:119-123`). The client's message says "Unsent input is not replayed" (`terminal/client/src/lib.rs:322`).
   **Status:** Fakes only
   Evidence: the same pane test as step 3. The gate in the SSH case is `clients/live/src/link.rs:215-217` (`discard_input`), with unit tests `input_while_reattaching_fails_and_is_never_queued` and `input_in_flight_when_the_attach_dropped_is_stale_in_the_next_one` (Fakes only, channel tests).

5. **I do:** the link comes back on its own, with no button press.
   **I see:** the banner says it is reconnecting, then goes. The shell is the same one, with the same process.
   **State and events:** the host retries with bounded backoff. The holder is reattached by the same UUID. The uncertain input is not sent again.
   **Status:** Works (real shell)
   Evidence: `roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay` cuts the path, waits for the session to stop running, then checks at least five failed dials and a reattach to the same holder (`assert_eq!(before.uuid, after.uuid)`). It runs against the real SSH VM with a fault proxy.

6. **I do:** press Reconnect, if the banner shows it.
   **I see:** the link tries again at once.
   **State and events:** an explicit reconnect is sent to the client.
   **Status:** Not built (no milestone)
   Evidence: the terminal pane shows no Reconnect control (`terminal/pane/src/pane.rs` draws only the banner, and `ui/src/` has no terminal Reconnect). `terminal/client/src/lib.rs:277-280` has `reconnect()`, and the Reconnect button at `terminal/client/src/lib.rs:734` is in the lab's remote viewer. The machine page has "Reconnect now" (`ui/src/machine_page.rs:571`, HO-6), which is a machine-level control, not a terminal one. No milestone names a terminal Reconnect control.

7. **I do:** the screen catches up. Output that came while the link was down shows up.
   **I see:** the screen shows what the shell printed while offline, from the cursor the view last had, not from the start.
   **State and events:** on reattach the daemon sends only the output newer than the client's cursor (`daemon/humatad/tests/terminal.rs::reattaching_from_the_cursor_sends_only_newer_output`, Works (real daemon)). When the gap is too big, the view starts from a fresh screen.
   **Status:** Works (real daemon)
   Evidence: `daemon/humatad/tests/terminal.rs::reattaching_from_the_cursor_sends_only_newer_output` checks no snapshot is sent and old output is not sent again. `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` (term_restart_live_test) restarts the app with a background job printing while no client is attached, then checks the catch-up screen.

8. **I do:** after the screen catches up, I type `world`.
   **I see:** the typed word goes to the shell once, after the caught-up output.
   **State and events:** typing is admitted only once the screen has arrived (`clients/live/src/link.rs:29-30`, `:49-70`, `:75`). The live link checks its gate before sending.
   **Status:** Works (real daemon)
   Evidence: `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` (Works (real daemon)) asserts the first typed line is not admitted before the screen is sent (`typing was admitted before the screen was sent`). The unit tests in `clients/live/src/link.rs` (`input_waits_for_the_screen_but_a_resize_does_not`) are fakes.

9. **I do:** the "reconnected; caught up N lines of output" line (SE-16).
   **I see:** the line says the view reconnected and how many lines it caught up.
   **State and events:** the daemon sends a Reconnected notice with its text, but the count is always 0, and the app's terminal link ignores notices.
   **Status:** Not built (SE-16; TERM and MACH list it)
   Evidence: SE-16 in `SPEC.md`. The daemon sends the notice with `caught_up_lines: 0` (`daemon/humatad/src/remote_terminal.rs:603` and `:634`, and `daemon/humatad/src/attach.rs:294`). `clients/terminal_link/src/lib.rs:191` drops every `Notice`. No code draws the line. Milestones TERM (open) and MACH (SE-16 listed) own this.

## Branches

### The box reboots and its holder is gone

- **When:** `box-1` is power cycled while the session is running, so the holder and the shell are gone.
- **I do:** power cycle `box-1`, then start it again, and let the host reconnect.
- **I see:** the session shows that it did not come back, with one line naming it in the restart report. The other sessions come back. Nothing is started again by itself.
- **State and events:** on start, the daemon records one `NotBroughtBack` line per shell that did not come back (`daemon/humatad/src/restart.rs:209-227`). Its restart report names the session.
- **Expected outcome:** the session is marked as lost, not as running, and typing into it is refused. Nothing from before is replayed.
- **Status:** Works (real daemon)
  Evidence: `daemon/humatad/tests/restart.rs::a_shell_that_cant_come_back_is_one_line_and_the_others_still_do` (Works (real daemon), local holders). The fake box in `daemon/humatad/tests/power_cut_machine.rs` (PER-28) cuts a machine's power at each fault point and checks what comes back. The fake box is not a real remote machine.
  The app's display of these lines is Not built (see the gaps).

### The link is up, but the box's daemon is stopped

- **When:** the machine's `humata2` daemon crashes, and its link process is still up.
- **I do:** stop the daemon on `box-1` and keep the SSH connection open.
- **I see:** the host keeps the machine as reachable, because its link is still open. The session keeps running. Typing still goes through the holder, not through the daemon.
- **State and events:** the link process outlives the daemon, so the host's uplink is not ended (`daemon/humatad/tests/away_link.rs::the_link_outlives_its_machines_daemon`). Holders are separate processes, so the shell keeps running.
- **Expected outcome:** the host does not show the machine lost. Typing reaches the shell through the link.
- **Status:** Works (real daemon)
  Evidence: `daemon/humatad/tests/away_link.rs::the_link_outlives_its_machines_daemon` (a real `humatad2 serve --stdio` over a local pipe, not SSH). Whether the daemon-side typing path survives a daemon stop with a live holder is not tested over SSH.

### The box is Away from the host

- **When:** the host cannot reach `box-1` at all. The machine is Away.
- **I do:** block the link to `box-1` from the host.
- **I see:** the session shows OFFLINE. Typing is refused. Starting or stopping a session on the box is refused with a "machine offline" error. When the link returns, the session shows live again, with the same holder.
- **State and events:** the host marks the session offline and refuses its input (`daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice`). The attach gets a Disconnected notice and then a Reconnected notice.
- **Expected outcome:** nothing is started twice, the same holder answers, and typing resumes after the screen is back.
- **Status:** Works (real daemon)
  Evidence: `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice` uses a real host daemon, a fake machine and a fake `ssh` (`daemon/humatad/tests/common/remote_rig.rs`). It is not real SSH. The Disconnected and Reconnected notices are checked there, but the app drops them (step 9).
  `daemon/humatad/tests/away_link.rs::a_machine_is_away_until_the_host_opens_its_uplink_and_reachable_once_it_has` covers the Away flag itself.

### A keyed input is sent twice and types once

- **When:** the link drops after the host sent a keyed input, but before its acknowledgement arrived. The host sends the same key again on the next connection.
- **I do:** type `echo once` on a link that drops after the send.
- **I see:** `once` appears once in the shell output.
- **State and events:** the holder records each key with its state. A repeat of a key that already arrived is refused as a repeat (`daemon/holder/tests/keyed_input.rs::a_keyed_input_types_once_and_a_repeat_types_nothing`). A keyed input cut off before its last chunk types nothing (`a_keyed_input_cut_off_before_its_last_chunk_types_nothing`).
- **Expected outcome:** the typed text runs once. An uncertain input is shown as uncertain, not sent again.
- **Status:** Works (real daemon)
  Evidence: `daemon/holder/tests/keyed_input.rs::a_keyed_input_types_once_and_a_repeat_types_nothing` (a real `humata2_holder` running `/bin/sh`). Over the SSH VM, `terminal/host/src/roaming_tests.rs` checks that a lost acknowledgement is typed once (`COUNT=3 INPUT=lost_ack`) and that a duplicate is not executed (`duplicate_must_not_execute` is absent), with the real OpenSSH VM.

## Expected outcomes

- [ ] Typed keys reach the shell in the order typed, each once.
- [ ] Output that arrives late stays in order, with no gap.
- [ ] When the link drops, the pane shows that it is disconnected, and typed keys are refused.
- [ ] Keys typed while the link is down are never sent later.
- [ ] The link comes back by itself, with backoff, to the same holder.
- [ ] On reattach, only output newer than the cursor is sent, and typing waits for the screen.
- [ ] A keyed input that was sent twice types once.
- [ ] A box whose holder is gone shows the session as not brought back.
- [ ] The app shows the "caught up N lines" line.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Keys sent in order, once | Works (real shell) | `terminal/host/src/roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay` (manual, SSH VM) |
| 2. Late echo stays in order | Works (real daemon) | `daemon/holder/tests/terminal.rs::output_carries_contiguous_offsets_and_a_reattach_resumes_from_a_cursor` (no added delay) |
| 3. Link drop shows disconnected | Fakes only | `terminal/pane/tests/pane.rs::while_disconnected_input_is_dropped_and_the_pane_says_so` |
| 4. Keys dropped while down, not replayed | Fakes only | `terminal/pane/tests/pane.rs::while_disconnected_input_is_dropped_and_the_pane_says_so`; `clients/live/src/link.rs` unit tests |
| 5. Link returns by itself | Works (real shell) | `terminal/host/src/roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay` (manual, SSH VM) |
| 6. Reconnect button | Not built | no milestone names a terminal Reconnect control; `ui/src/machine_page.rs:571` is HO-6 (machine level) |
| 7. Catch up from the cursor | Works (real daemon) | `daemon/humatad/tests/terminal.rs::reattaching_from_the_cursor_sends_only_newer_output`; `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` |
| 8. Typing after the screen | Works (real daemon) | `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` |
| 9. "caught up N lines" line | Not built | SE-16; `daemon/humatad/src/remote_terminal.rs:603`, `:634`; `clients/terminal_link/src/lib.rs:191` |
| Branch: box reboots, holder gone | Works (real daemon) | `daemon/humatad/tests/restart.rs::a_shell_that_cant_come_back_is_one_line_and_the_others_still_do` (the app's display is Not built, PER-29) |
| Branch: link up, box daemon stopped | Works (real daemon) | `daemon/humatad/tests/away_link.rs::the_link_outlives_its_machines_daemon` (local pipe, not SSH) |
| Branch: box Away | Works (real daemon) | `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice` (fake box, fake ssh) |
| Branch: keyed input sent twice | Works (real daemon) | `daemon/holder/tests/keyed_input.rs::a_keyed_input_types_once_and_a_repeat_types_nothing`; `terminal/host/src/roaming_tests.rs` (real SSH VM) |

## Gaps and likely bugs

- **Not built:** the app does not show the Disconnected and Reconnected notices or the "caught up N lines" line. The app's terminal link ignores every notice (`clients/terminal_link/src/lib.rs:191`). SE-16 (MACH, TERM).
- **Not built:** the app does not show `last_start`'s lines, such as a session that was not brought back. OVERVIEW.md, Daemon and persistence (I7): "Show `last_start`'s lines ... in the app (PER-29, PER-15)" is open.
- **Not built:** a terminal Reconnect control. No milestone names one. Machine-level "Reconnect now" is HO-6 and is built in `ui/src/machine_page.rs`.
- **Likely bug:** `caught_up_lines` is always 0. The daemon sends it as 0 (`daemon/humatad/src/remote_terminal.rs:603`, `:634`; `daemon/humatad/src/attach.rs:294`), and the comment says the client derives the count from the output that follows. No client does (`grep` for `caught_up_lines` finds only the daemon). To confirm: run `grep -rn caught_up_lines apps/humata2 --include='*.rs'`.
- **Likely bug:** the app's terminal link drops every notice (`clients/terminal_link/src/lib.rs:191`). Its comment says "Humata's own lines are the app's to draw", but no app code draws them, so the Disconnected and Reconnected text from the daemon never reaches the user. To confirm: search `ui/src` for `Reconnected` (none).
- **Code, no test:** the app-side link drop. `terminal/client/src/lib.rs:282-289` sets the pane disconnected when the transport ends, and no test drives a real transport drop to the banner.
- **Proven over SSH (lab and terminal host only):** typed keys in order, no replay after a drop, the reattach to the same holder after a route change, and a keyed input typed once. These are `terminal/host/src/roaming_tests.rs` and `terminal/lab/tests/*ssh*.rs` (manual targets, the SSH VM).
- **Proven only over the peer link with fakes or a local pipe:** the box Away and reconnect, the link outliving the daemon, and the box reboot. These are `daemon/humatad/tests/remote.rs`, `away_link.rs` and `restart.rs`, which use a local pipe or a fake `ssh`. The peer link has not run over real SSH (OVERVIEW.md, peer link: "Run the peer link client over real SSH (Linux batch)" is open).
