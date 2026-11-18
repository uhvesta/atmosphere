# A plain shell on devbox-1 runs as its own Terminal session that never reads Working, and a shell tab beside an agent runs on that machine

Layer: macro, verify layer (flow 09). Platforms: macOS (the app and this Mac's daemon); the machine side is run over a fake `ssh` only, and Linux is not covered.

## Starting state

- `devbox-1` is a linked machine and online. Its daemon runs under its own folder, and this Mac's daemon reaches it over a fake `ssh`.
- `#ledger` has a clone of `acme/ledger` on devbox-1.
- "Backfill" runs on devbox-1 in `#ledger`, with its agent working in its conversation.
- This Mac has no other session open. The app is focused on the `#ledger` channel page.
- I hold no shell on devbox-1 yet. The approval passphrase is not needed for any step here, since this Mac is not away.

## Reproduce headless

- Machine side and host relay: `//apps/humata2/daemon/humatad:remote_test` (file `daemon/humatad/tests/remote.rs`). It starts a host and one fake box per test, and it is the rig any remote terminal step would extend. Run the one test `a_session_started_on_the_machine_runs_there_and_the_host_relays_its_terminal` to see the rig work before adding a shell.
- This Mac's own Terminal session, the reference for the never-Working rule: `//apps/humata2/desktop:live_flows_test`, file `desktop/tests/live_flows/terminals.rs`, test `a_terminal_started_from_new_session_runs_a_shell_that_answers_and_never_reads_working`. Capture the test log; it asserts the session never reads Working and settles at Idle.
- The same rule through the client: `//apps/humata2/clients/live:term_restart_live_test`, test `a_terminal_session_starts_a_shell_with_no_turns`.
- The new-tab path on This Mac: `//apps/humata2/desktop:live_flows_test`, test `a_new_terminal_tab_takes_typing_without_a_click_and_runs_the_command`.
- Headless CLI equivalent for a remote shell: run a throwaway `humatad2` with its own `--root` and `HOME`, add a fake box with the remote rig, and call `humata` against it. No checked-in CLI test starts a remote shell, so this is a new check, not an existing one.
- Evidence to capture: the test log lines, and for the CLI route the `humata ls --json` output and its exit code (0 on success).
- What cannot be done headless: a real devbox-1 (a real Linux machine over real SSH is LINUX, blocked), and a real window for the tab strip. The nearest headless equivalent is the remote rig with a fake box.

## Steps

1. **I do:** on Backfill, click **New terminal** in the tab strip.  
   **I see:** a `zsh` tab in the session's folder on devbox-1, beside the agent's tab, with the cursor ready.  
   **State and events:** the tab is created on devbox-1 (not on this Mac). The host relays it through `OpenShellTab` on the machine. Session updated (a new tab).  
   **Status:** Code, no test for the remote case. The host relays `OpenShellTab` to a machine in `daemon/humatad/src/remote_terminal.rs:112` (`open_shell_tab`). No test in `daemon/humatad/tests/remote*.rs` calls `OpenShellTab` on a remote session; every `open_shell_tab` caller in the tests uses this Mac or the host. The This Mac version **Works (real daemon)**: `desktop/tests/live_flows/terminals.rs::a_new_terminal_tab_takes_typing_without_a_click_and_runs_the_command`.

2. **I do:** in the `#ledger` channel, click **New session**. Pick **Terminal** as what it runs, pick devbox-1 as the machine, type "Scratch shell" as the title, and click **Start session**.  
   **I see:** a session page with one `zsh` tab and no agent. Its first screen shows a prompt from devbox-1.  
   **State and events:** Session created with the agent kind of a plain shell (`Terminal`, which the daemon runs as the agent `zsh`, D312) and the machine devbox-1. The session never has turns.  
   **Status:** Code, no test for devbox-1. The start is routed by `start_session_on` in `daemon/humatad/src/remote_work.rs:728-760`, which carries the request, including the kind, to the machine. The kind decides "no turns" in `daemon/sessions/src/agents.rs:227` and `:709`. The This Mac version **Works (real daemon)**: `desktop/tests/live_flows/terminals.rs::a_terminal_started_from_new_session_runs_a_shell_that_answers_and_never_reads_working`.

3. **I do:** type `echo SHELL$((2+3))` and press Return.  
   **I see:** `SHELL5` on the next line of the tab.  
   **State and events:** the input travels from this Mac through the host to devbox-1's holder, and the answer comes back over the same route. No input is sent twice.  
   **Status:** Works (real daemon) on This Mac: `clients/live/tests/term_restart.rs::a_terminal_session_starts_a_shell_with_no_turns` (it types the same echo and reads `SHELL5`). Over the remote relay, Code, no test. The relay of a remote agent's terminal is proven in `daemon/humatad/tests/remote.rs::a_session_started_on_the_machine_runs_there_and_the_host_relays_its_terminal`, but that test uses an agent tab, not a plain shell.

4. **I do:** watch the session's status while the shell sits at its prompt for a while.  
   **I see:** the status reads Idle. It never reads Working.  
   **State and events:** Session updated (Idle). No Working event is recorded for the session.  
   **Status:** Works (real daemon) on This Mac: `desktop/tests/live_flows/terminals.rs::a_terminal_started_from_new_session_runs_a_shell_that_answers_and_never_reads_working` (it runs the loop and asserts `worked` stays false, then waits for Idle). Code, no test on devbox-1.

5. **I do:** on devbox-1's machine page, click **Open a shell**.  
   **I see:** the same sheet with **Terminal** picked and devbox-1 chosen. Clicking **Start session** opens a shell tab.  
   **State and events:** a Terminal session is started on devbox-1. The flow's expectation was a shell in the machine's home folder, not tied to a channel. The app starts it as a session in a channel, so the home-folder shell is not what happens.  
   **Status:** The button starts a shell **Works (real daemon)** on This Mac: `desktop/tests/live_flows/terminals.rs::open_a_shell_on_a_machine_page_starts_a_terminal_there`. The test asserts only that the shell runs on This Mac; it does not check the folder. A shell on a machine with no channel: **Not built, no milestone** (the daemon's machine shell is listed by `daemon/humatad/tests/terminal.rs::machine_shells_are_listed_with_their_offsets`, but the app never calls `OpenMachineShell`).

## Branches

### The link to devbox-1 drops while I type into the plain shell

- **When:** the link between this Mac and devbox-1 goes down while the shell is open.
- **I do:** type `date` and press Return, then cut the link (or let the Mac sleep for a minute), and type `pwd` and press Return once the tab says it is back.
- **I see:** while the link is down the tab says the connection is lost and the session keeps running. Keys typed while down are not sent. When the link returns, the tab catches up to the shell's output and typing is accepted again.
- **Expected outcome:** the shell on devbox-1 never stopped; no input is replayed; the typed `pwd` runs once.
- **Status:** Code, no test for a plain shell. The daemon's reattach of a remote session is proven for an agent session in `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice`. The lost-link notice is dropped by the app (see Gaps). The catch-up for a shell on This Mac is **Works (real daemon)**: `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced`.

### devbox-1 is stopped when I start the Terminal session

- **When:** devbox-1 was stopped with **Stop sessions** before I click **Start session** in step 2.
- **I do:** click **Start session**.
- **I see:** the sheet refuses the start with a note that the machine's sessions are stopped, and to start them first.
- **Expected outcome:** nothing starts; the session is not created on devbox-1.
- **Status:** Works (real daemon) for the refusal of any new session on a stopped machine: `daemon/humatad/tests/stop_sessions.rs::while_stopped_agents_and_scripts_are_refused_and_nothing_can_be_typed_attached_or_started`. The sheet's note is **Fakes only**: `desktop/tests/machines_flow.rs::while_stopped_open_a_shell_is_refused_with_a_note`. Shell on a stopped remote machine specifically: not tested.

### devbox-1 reboots while the shell is open

- **When:** devbox-1 reboots for updates while this shell is running.
- **I do:** wait for devbox-1 to come back, then open the shell tab.
- **I see:** the tab shows a fresh shell in the same folder. The note says the earlier program was lost. The scrollback that the holder kept is shown.
- **Expected outcome:** a shell cannot resume what it was running. Only its folder and its scrollback come back.
- **Status:** Works (real daemon) on This Mac: `daemon/humatad/tests/restart.rs::a_shell_whose_holder_died_comes_back` and `::a_shell_that_cant_come_back_is_one_line_and_the_others_still_do`. On a real remote machine: **Not built: LINUX** (blocked; a real Linux reboot is not run).

## Expected outcomes

- [ ] The New terminal tab opens on devbox-1, in the session's clone, beside the agent's tab.
- [ ] A Terminal session started on devbox-1 runs a shell that answers a typed command.
- [ ] That session's status is Idle, never Working, while the shell is at its prompt.
- [ ] Typing goes through the host to devbox-1 and the answer comes back once.
- [ ] Open a shell on devbox-1's page starts a Terminal session on devbox-1 (the folder it gets is a channel folder, not the home folder; see Gaps).
- [ ] A shell whose link drops keeps running on devbox-1, and typing resumes without replay after catch-up.
- [ ] A stopped devbox-1 starts no new shell.
- [ ] After a devbox-1 reboot, a shell comes back fresh in its folder, and the tab says what was lost (real reboot not run).

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. New terminal tab on devbox-1 | Code, no test (remote); Works (real daemon) on This Mac | `daemon/humatad/src/remote_terminal.rs:112`; `desktop/tests/live_flows/terminals.rs::a_new_terminal_tab_takes_typing_without_a_click_and_runs_the_command` |
| 2. Start a Terminal session on devbox-1 | Code, no test (remote); Works (real daemon) on This Mac | `daemon/humatad/src/remote_work.rs:728`; `desktop/tests/live_flows/terminals.rs::a_terminal_started_from_new_session_runs_a_shell_that_answers_and_never_reads_working` |
| 3. Typed command answers over the relay | Works (real daemon) on This Mac; Code, no test (remote) | `clients/live/tests/term_restart.rs::a_terminal_session_starts_a_shell_with_no_turns` |
| 4. Status stays Idle, never Working | Works (real daemon) on This Mac; Code, no test (remote) | `desktop/tests/live_flows/terminals.rs::a_terminal_started_from_new_session_runs_a_shell_that_answers_and_never_reads_working` |
| 5. Open a shell on devbox-1's page | Works (real daemon) on This Mac; Not built (home folder, no channel) | `desktop/tests/live_flows/terminals.rs::open_a_shell_on_a_machine_page_starts_a_terminal_there`; `daemon/humatad/tests/terminal.rs::machine_shells_are_listed_with_their_offsets` |
| Branch: link drops while typing | Code, no test (shell); Works (real daemon) on This Mac | `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice`; `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` |
| Branch: devbox-1 stopped | Works (real daemon); sheet note Fakes only | `daemon/humatad/tests/stop_sessions.rs::while_stopped_agents_and_scripts_are_refused_and_nothing_can_be_typed_attached_or_started` |
| Branch: devbox-1 reboots | Works (real daemon) on This Mac; Not built: LINUX on a remote machine | `daemon/humatad/tests/restart.rs::a_shell_whose_holder_died_comes_back` |

## Gaps and likely bugs

- Not built (no milestone): a shell on a machine with no channel. The machine page's **Open a shell** starts a Terminal session in a channel, not in the machine's home folder. The daemon's machine shell is never used by the app.
- Code, no test: any remote shell. No test opens a shell tab, types into, or resizes a plain shell on devbox-1 (see `daemon/humatad/tests/remote.rs`, which only covers an agent tab).
- Likely bug (already in the coach's findings, micro-16): the app's terminal link drops the daemon's Notice frames (`clients/terminal_link/src/lib.rs:193` (re-check: the writer guide puts this drop at line 191), `Some(ServerMessage::Notice(_)) => Ok(Vec::new())`), so a shell on devbox-1 never shows the "connection lost, the session keeps running" notice that the host sends (`daemon/humatad/src/remote_terminal.rs:629`, `Kind::Disconnected`). Confirm: drop the link while a remote shell is open and read the tab.
- Not built: LINUX. A real devbox-1 reboot, and real SSH for a plain shell, have not been run. The SSH shell proofs are in the terminal experiment only (`terminal/lab/tests/clipboard_ssh.rs`, `terminal/lab/tests/vertical_resize_ssh.rs`).
- Likely bugs: none found in this flow's code path. The offline start is documented to refuse at once (`daemon/humatad/src/remote_work.rs:728-735`), but no shell-specific test checks it, so it stays unverified rather than a bug.
