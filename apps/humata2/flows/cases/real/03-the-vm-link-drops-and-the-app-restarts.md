# A dropped SSH link reconnects without replaying input, and an app restart keeps the VM's holders running

Real. Platforms: macOS (the app and the link). The VM is Debian on arm64. Paths are relative to `apps/humata2`.

This case is for a run by hand or a later test. The terminal component has real-SSH proofs for a cut link and for a restart. **The app's own path has never been cut or restarted against real SSH.** That path is: desktop app, then `humatad2` on this Mac, then the provider's `ssh`, then `humatad2 --role machine` on the VM, then the holder. Each step says which half is proven. Related cases: [micro 16 typing over a slow or dropped link](../micro/16-typing-over-a-slow-or-dropped-ssh-link.md), [micro 17 survives an app restart](../micro/17-terminal-survives-an-app-restart.md), [flow 09](../../09-ssh-terminal-session.md), [real/02](02-a-shell-on-the-vm-in-the-app.md), [real/01](01-the-vm-becomes-a-machine.md).

## Starting state

- The VM `h2-terminal-c1` is running and is a machine in the app, as in real/01. Its link is up.
- A Terminal session has a shell tab on the VM (real/02, step 1). In it, `echo $$` printed a process id, which I wrote down as P. A second line, `sleep 1000 &`, started a background job.
- A long job is printing: `for i in $(seq 1 100000); do echo line $i; sleep 1; done` in that tab.
- The app window is open and the tab is focused.

## Test setup

- VM: `./terminal-ssh-demo.sh start` then `status`. Never stop, restart or delete the VM, and never run `limactl stop`, `limactl restart` or any network stop. Other runs share it (`terminal/platform_test/SSH_DEMO.md`).
- Isolation: the run has its own temp home and its own `humatad2` state folder, never the owner's `~/.humata2`. Give the daemon the VM's SSH file with `humatad2 serve --ssh-config "$HOME/.lima/h2-terminal-c1/ssh.config"` (`daemon/humatad/src/serve.rs:131-134`). Whether the desktop app attaches to such a daemon is unproven.
- Cutting the link: list this Mac's `ssh` processes with `ps -axo pid,command | grep 'role machine'`. Kill only the `ssh` that the daemon started for the VM (its command line has `lima-h2-terminal-c1` and `--role machine`). Never kill Lima's own `limactl` host agent or its control-master `ssh`. The provider uses `ControlPath=none` (`daemon/machines/src/link.rs:390`), so the VM's own mux is not touched. The same kill is how `Devbox::cut_link` cuts the link in `desktop/tests/live_flows/link_drop.rs`, with a stand-in for `ssh`.
- **Needs a free screen** for every step that looks at the app window. Headless equivalents: the `humata` CLI to list and read the session, and the `live_flows` style test named in each step.
- No model login is needed. Needs the owner: a free screen and a running VM.

## Steps

1. **I do:** with the job printing, note the last number N on the screen. Find the `ssh` for the VM and kill it. **Needs a free screen.**
   **I see:** within a few seconds the tab shows that the connection is lost. The prompt line stops moving.
   **State and events:** the machine goes Away. The shell, the job and the holder keep running on the VM, because the VM does not depend on this Mac's `ssh`.
   **Status:** Works (real shell) for the component only: `terminal/session/tests/ssh.rs::killing_the_ssh_frontend_reaps_its_tunnel_and_keeps_the_remote_shell` (manual target `//apps/humata2/terminal/session:ssh_test`, real VM) and `terminal/host/src/roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay` (target `ssh_reconnect_test`). In the app, the real daemon with a stand-in `ssh` shows the same tab text: `desktop/tests/live_flows/link_drop.rs::a_dropped_link_refuses_typing_and_the_same_shell_takes_it_again_once_back` (Works (real daemon), no real SSH). The app over real SSH is unproven: Code, no test.

2. **I do:** on the VM, in a second ssh session of my own (`ssh -F ~/.lima/h2-terminal-c1/ssh.config lima-h2-terminal-c1 'ps -p P -o pid,etime,cmd'`), check the shell is still alive.
   **I see:** the process P is there with an elapsed time that kept counting.
   **State and events:** same pid, same start time. Nothing started a replacement.
   **Status:** Works (real shell) for the component only: the same `roaming_tests.rs` test asserts the uuid and pid are the same before and after (`assert_eq!(before.uuid, after.uuid)`). The app's path is Code, no test.

3. **I do:** while the link is down, type `echo typed-while-down` in the tab and press Return.
   **I see:** the keys are refused, and a toast says the tab is out of reach. The text is not echoed.
   **State and events:** the input gate drops the keys (`clients/live/src/link.rs`, `discard_input`). They are never queued.
   **Status:** Works (real daemon) in the app with a stand-in `ssh`: `desktop/tests/live_flows/link_drop.rs::a_dropped_link_refuses_typing_and_the_same_shell_takes_it_again_once_back`. Also Fakes only: `terminal/pane/tests/pane.rs::while_disconnected_input_is_dropped_and_the_pane_says_so` and `clients/live/src/link.rs::input_while_reattaching_fails_and_is_never_queued`. Over real SSH: Code, no test.

4. **I do:** type `echo half` (no Return) just before the cut, so there is half a line on the VM's prompt. Then, while down, press Return.
   **I see:** the Return is refused like step 3.
   **State and events:** the half line on the VM stays unfinished.
   **Status:** Works (real daemon), stand-in `ssh`: the same `link_drop.rs` test checks the half-typed line is intact and none of the refused keys arrive. Real SSH: Code, no test.

5. **I do:** wait. I do not press anything. The daemon redials by itself.
   **I see:** the tab clears its lost banner after the link is back. The shell prompt is the same one.
   **State and events:** the machine returns online. The tab attaches to the same holder from its last offset.
   **Status:** Works (real shell) for the component only: `roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay` waits for at least five failed dials and then reattaches the same holder. In the app: Works (real daemon) with a stand-in `ssh` in `link_drop.rs`, and Fakes only for the fake-`ssh` daemon test `daemon/humatad/tests/two_machines.rs::a_lost_link_comes_back_and_the_session_is_the_same`. The app over real SSH: Code, no test. The redial's back-off is 1 to 30 seconds (HANDOFF.md), so allow up to a minute.

6. **I do:** look for the lines the job printed while the link was down.
   **I see:** the lines after N appear, each once and in order, with no gap and no repeat.
   **State and events:** the replica gets only output newer than its cursor. No full snapshot is sent unless the view fell far behind.
   **Status:** Works (real daemon): `daemon/humatad/tests/terminal.rs::reattaching_from_the_cursor_sends_only_newer_output`, and in the app (stand-in `ssh`) the printed-meanwhile check in `link_drop.rs`. The real-SSH component test checks offline output and query recovery (`roaming_tests.rs`). The app over real SSH: Code, no test.

7. **I do:** look under the output for the reconnect line.
   **I see:** "Reconnected; caught up N lines of output." until I type again.
   **State and events:** the daemon sends a `CaughtUp` notice with the count (`daemon/humatad/src/attach.rs:465`). The live link turns it into a line (`clients/live/src/link.rs:188`), and `ui/src/terminals.rs:755` draws it.
   **Status:** Fakes only: `desktop/tests/native/terminal.rs::a_reconnected_tab_says_so_until_it_is_typed_into` and `clients/live/src/link.rs::a_reconnect_notice_becomes_a_line_beside_the_terminal`. micro 16 step 9 calls this Not built, and the README says the count is always 0. Both look out of date: the daemon fills the count now (`attach.rs:465`; `remote_terminal.rs:908`), and the line is drawn. Whether the real relayed path gives a true count is unproven. **Likely bug (unconfirmed):** `caught_up_lines: 0` is still written at `daemon/humatad/src/attach.rs:333` and `remote_terminal.rs:635`, but those two are the exit and disconnect notices, where 0 is right.

8. **I do:** type `echo typed-after-reconnect` and press Return.
   **I see:** the text echoes once and the shell answers once. `typed-while-down` never appears.
   **State and events:** a keyed input types once, and a repeat types nothing. Nothing from the down time is replayed.
   **Status:** Works (real daemon): `daemon/holder/tests/keyed_input.rs::a_keyed_input_types_once_and_a_repeat_types_nothing`, and `link_drop.rs` (stand-in `ssh`). Works (real shell), component only: `roaming_tests.rs` checks the remote count after the outage is one, with no replay. The app over real SSH: Code, no test.

9. **I do:** press ⌘Q. Wait a minute, with the job still printing on the VM. **Needs a free screen.**
   **I see:** the app is closed.
   **State and events:** the app's link ends. This Mac's daemon may keep running. The holder on the VM keeps running whether or not this Mac is reachable.
   **Status:** Code, no test for ⌘Q (`desktop/src/main.rs:202-203`). Works (real shell), component only: `terminal/session/tests/ssh.rs::ssh_terminals_keep_their_process_after_transport_and_app_restart` (real VM).

10. **I do:** open Humata again and select the same session and its VM tab. **Needs a free screen.**
    **I see:** the tab attaches to the same shell. The lines printed while the app was closed are there. The prompt and the background `sleep 1000` job are as they were (`jobs` lists it).
    **State and events:** the same holder, the same process id P. No new shell started.
    **Status:** Works (real daemon) on this Mac only: `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` and `desktop/tests/live_flows.rs::after_the_app_restarts_the_session_tab_shows_what_was_printed_while_it_was_gone`. A VM tab after a restart: Code, no test (the component test above is the only real-SSH proof).

11. **I do:** type `echo hi-again` as soon as the tab opens, before the screen shows.
    **I see:** the keys are refused with a toast. Typing works after the screen shows.
    **State and events:** keys wait for the first screen (micro 17 B1).
    **Status:** Works (real daemon) on this Mac: `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced`. Fakes only for the pane: `terminal/client/tests/component.rs::keys_before_the_first_screen_are_told_once_then_the_screen_takes_them`. On the VM: Code, no test.

12. **I do:** run `echo $$` and `jobs` in the tab.
    **I see:** the pid is P and the job is listed.
    **State and events:** the shell never restarted across the cut and the restart.
    **Status:** Works (real shell), component only: both `ssh.rs` tests and `roaming_tests.rs`. The app over real SSH: Code, no test.

## Branches

### The cut comes while vim is open
When: I cut the link with vim in insert mode and half a line typed. After reconnect, vim is still open with the half line. The screen redraws.
**Status:** Code, no test. Neovim's resume is proven locally (`terminal/lab/tests/neovim.rs::neovim_colors_mouse_keyboard_resize_and_resume`, local only). micro 15's restart branch is Fakes only.

### The link stays down
When: I leave the link cut and the app open. The machine page shows the machine Away, and the tab says it is out of reach. The tab does not close and does not start a new shell.
**Status:** Works (real daemon), fake `ssh`: `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice`. The app tab text on a long outage: Fakes only. A Reconnect button on the tab is Not built (no milestone; micro 16 step 6).

### The app is killed instead of quit
When: I `kill -9` the desktop app, not ⌘Q. On reopen, the same as steps 10 to 12.
**Status:** Code, no test. `terminal/session/tests/ssh.rs::killing_the_ssh_frontend_reaps_its_tunnel_and_keeps_the_remote_shell` kills the component's frontend only (Works (real shell), component only).

### The daemon on this Mac restarts as well
When: the app is closed and this Mac's `humatad2` is also stopped, then started. The tab outlives it (`daemon/humatad/tests/terminal.rs::a_tab_outlives_its_daemon`, Works (real daemon), local) and the session returns when the host reaches the VM again. A local shell holder that died comes back fresh: `daemon/humatad/tests/restart.rs::a_shell_whose_holder_died_comes_back`. The VM side after this Mac's daemon restart is Code, no test.

### The VM itself restarts
When: not run here. Never restart `h2-terminal-c1`. Use a dedicated throwaway VM (real/13). The holder lives at `/tmp/h2-terminal-holder`, which is not reboot-safe, so the shells would not come back.
**Status:** Not built for the shared fixture (by rule). Whole-machine proof exists in `terminal/host:agent_resume_vm_test` (own Lima VM; HANDOFF.md).

### Re-provision while a holder is running
When: `./terminal-ssh-demo.sh provision` runs while the shell is open. The running holder is not upgraded, and new sessions use the new binary.
**Status:** Code, no test through the app. The helper keeps running holders (`terminal/platform_test/SSH_DEMO.md`). The app's force-upgrade rule for holders is `daemon/humatad/tests/upgrade_remote_holders.rs` (fake `ssh`).

## Expected outcomes

- [ ] Cutting the `ssh` the daemon started leaves the VM's shell, job and holder running (same pid P, rising elapsed time).
- [ ] While the link is down, the tab says so and typed keys are refused with a toast, not queued.
- [ ] The link returns by itself, with no button press.
- [ ] Output printed while down shows once, in order, with no repeat.
- [ ] Nothing typed while down arrives later. A half-typed line is intact.
- [ ] After ⌘Q and a reopen, the tab attaches to the same holder and shows what printed while closed.
- [ ] No test or step stopped, restarted or changed the VM, or touched the owner's `~/.humata2` or SSH keys.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. Kill the link's ssh | Works (real shell), component only | `terminal/session/tests/ssh.rs::killing_the_ssh_frontend_reaps_its_tunnel_and_keeps_the_remote_shell` |
| 2. Shell still alive on the VM | Works (real shell), component only | `terminal/host/src/roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay` |
| 3. Keys refused while down | Works (real daemon) | `desktop/tests/live_flows/link_drop.rs::a_dropped_link_refuses_typing_and_the_same_shell_takes_it_again_once_back` (stand-in ssh) |
| 4. Half-typed line intact | Works (real daemon) | same test |
| 5. Link returns by itself | Works (real shell), component only | `roaming_tests.rs`; app: `link_drop.rs` and `daemon/humatad/tests/two_machines.rs::a_lost_link_comes_back_and_the_session_is_the_same` (fake ssh) |
| 6. Output while down shows once | Works (real daemon) | `daemon/humatad/tests/terminal.rs::reattaching_from_the_cursor_sends_only_newer_output` |
| 7. Reconnect line with a count | Fakes only | `desktop/tests/native/terminal.rs::a_reconnected_tab_says_so_until_it_is_typed_into` |
| 8. No replay after reconnect | Works (real daemon) | `daemon/holder/tests/keyed_input.rs::a_keyed_input_types_once_and_a_repeat_types_nothing` |
| 9. Quit the app | Works (real shell), component only | `terminal/session/tests/ssh.rs::ssh_terminals_keep_their_process_after_transport_and_app_restart` |
| 10. Reopen, same holder, output | Works (real daemon) | `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` (This Mac) |
| 11. Typing before the screen | Works (real daemon) | same test; `terminal/client/tests/component.rs::keys_before_the_first_screen_are_told_once_then_the_screen_takes_them` |
| 12. Same pid P | Works (real shell), component only | both `ssh.rs` tests |
| Branch: cut in vim | Code, no test | none |
| Branch: link stays down | Works (real daemon) | `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice` (fake ssh) |
| Branch: app killed | Code, no test | none |
| Branch: this Mac's daemon restarts | Works (real daemon) | `daemon/humatad/tests/terminal.rs::a_tab_outlives_its_daemon` (local) |
| Branch: the VM restarts | Not built | by rule, never the shared VM |
| Branch: re-provision | Code, no test | `terminal/platform_test/SSH_DEMO.md` |

## Gaps and likely bugs

Not built:
- The app's own path over real SSH has no test for a cut or a restart. The only real-SSH proofs are the terminal component's manual targets. No milestone owns one.
- A Reconnect control on the tab (no milestone).
- A dedicated throwaway VM for power-off cases (real/13).

Likely bugs:
- Notice frames are dropped at `clients/terminal_link/src/lib.rs:193` (it was line 191). The "Lost the connection... the session keeps running" notice and the exit notice go that way. The reconnect line has its own channel now.
- "caught up N lines" would always say 0: partly stale. `attach.rs:465` and `remote_terminal.rs:908` fill the count. Confirm over the relayed path by cutting and reconnecting while the job prints.
- micro 16 step 9 still says Not built. It should be updated.
- Keys typed before the screen are dropped (README 4): the toast now exists for the pane, but confirm the VM tab shows it (step 11).
- Edit menu copy and paste (README 5): see [real/02](02-a-shell-on-the-vm-in-the-app.md). Not part of this case.
