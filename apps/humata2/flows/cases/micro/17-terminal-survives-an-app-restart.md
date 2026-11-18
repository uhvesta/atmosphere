# A shell keeps its program and its output when the app restarts

Micro. Covers macOS and Linux for the daemon and holder; the desktop flow test is macOS only. The browser viewer is not part of the shipped app.

## Starting state

- Humata is running on this Mac: the daemon (`humatad2`) and one holder per tab. The holder owns the shell's PTY, so the shell lives in the holder, not in the app.
- Session "Fix the ledger" is in channel "payments" on this Mac. It has tab 0 (the agent) and tab 1 (a shell).
- Tab 1 is a shell in the session's folder. A build is printing a line every second into it.
- The app is open and connected. The shell tab is the selected tab and the view is at the bottom of the output.
- I am holding nothing. I have typed nothing since the build started.

## Steps

1. **I do:** Press ⌘Q.
   - **I see:** The window and the app go away. Nothing is said about the build.
   - **State and events:** The app's client and its link to the daemon end. The daemon, the holder and the shell keep running. The build keeps printing into the holder's scrollback.
   - **Status:** Code, no test. The quit is bound at `desktop/src/main.rs:202-203` (`cmd-q` runs `cx.quit()`). No test presses ⌘Q. The restart tests below drop the client instead.

2. **I do:** Leave the app closed for a minute while the build keeps running.
   - **I see:** Nothing, because no app is open.
   - **State and events:** The holder keeps the PTY and the shell alive. The build's output is kept in the holder up to its scrollback limit.
   - **Status:** Works (real daemon). `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` drops the first client and its link, then lets a background job in the real `/bin/sh -i` finish while no client is attached. The job writes its output and a `done` file.

3. **I do:** Open Humata again and select "Fix the ledger", tab 1.
   - **I see:** The session is listed. Tab 1 opens and shows a short restoring state, then the screen.
   - **State and events:** The app opens a new link to tab 1. The daemon attaches it to the same holder (same holder pid, same shell). No new shell is started. The first frame is a snapshot of the screen.
   - **Status:** Works (real daemon). `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` reattaches the second client to the same holder and gets the snapshot. The UI's own reopen path is not in this test (see the branch on the daemon restart).

4. **I do:** Look at the output.
   - **I see:** The lines printed while the app was closed are there, under the lines from before. The last line is the build's last line.
   - **State and events:** The snapshot carries the screen and the daemon sends the output after the snapshot's cursor, so nothing is shown twice.
   - **Status:** Works (real daemon). `clients/live/tests/term_restart.rs` waits for `MARKok`, which only the shell's own variable can complete, after the app is closed. `desktop/tests/live_flows.rs::after_the_app_restarts_the_session_tab_shows_what_was_printed_while_it_was_gone` checks `got: while the app was gone` in the app's view (agent tab, real daemon, fake agent).

5. **I do:** Check where the view is scrolled.
   - **I see:** The view opens at the bottom. If I had scrolled back before quitting, it does not return to that place.
   - **State and events:** No scroll offset is saved across an app restart. The replica keeps its scroll only while it is pinned (search open, a selection, or a scrolled bar) inside one run (`terminal/client/src/replica.rs:725-737`).
   - **Status:** Code, no test. The bottom-after-open behaviour is only checked in the lab (`terminal/lab/tests/interactive.rs`, scrollbar `at_bottom`), not in the shipped app.

6. **I do:** Type `echo hi` and press Return, once the screen is shown.
   - **I see:** `echo hi` appears once, and the shell prints `hi` on the next line.
   - **State and events:** One input frame goes to the holder with the next input sequence. The daemon confirms it with `InputWritten`. The line is typed once.
   - **Status:** Works (real daemon). `clients/live/tests/term_restart.rs` sends its typed line only after the link has admitted it, and asserts the screen shows `SENT2` after the snapshot. The test checks the text is there; it does not count copies.

## Branches

### B1. I type during catch-up, before the screen has arrived

- **When:** Right after the window opens, before tab 1 has drawn its screen.
- **I do:** Type `ls` and press Return straight away.
- **I see (code as written):** Nothing appears for those keys. No message is shown. When the screen arrives, there is no `ls` line in the shell.
- **State and events:** The keys never reach the link. `terminal/client/src/replica.rs:305-307` returns `false` when input is not enabled, and the enable flag stays off until the replica has a fresh snapshot (`terminal/client/src/replica.rs:537`). `terminal/pane/src/host.rs:119-122` drops the bytes when the pane is not connected, and it is created as connected (`:111`), so the drop is silent. Nothing is pushed to `errors`, so no toast is raised.
- **Expected outcome:** Typing before the screen is either held and sent once the screen is up, or refused with a message the person can see ("wait for the screen, then type it again"). The daemon-link layer already refuses with that message (`clients/src/daemon.rs:403-410`, `clients/live/src/link.rs:60-66`), but the window never gets that far.
- **Status:** Fakes only. Until the screen arrives the pane takes no input, and the first keys raise one toast: "This tab is still getting its screen back, so what you typed was not sent. Type it again once the screen shows." Once the screen is up the keys reach the shell (`terminal/client/tests/component.rs::keys_before_the_first_screen_are_told_once_then_the_screen_takes_them`).

### B2. The daemon also restarted while the app was closed (daemon side)

- **When:** After ⌘Q, the daemon is killed and started again, and the holder survives.
- **I do:** Open the app and select tab 1.
- **I see:** The shell is still there, with its earlier output. Its program is still running.
- **State and events:** The new daemon finds the holder by its saved socket and attaches to it. The tab keeps its id.
- **Expected outcome:** The tab's program and output survive the daemon restart.
- **Status:** Works (real daemon). `daemon/humatad/tests/terminal.rs::a_tab_outlives_its_daemon` kills and restarts a real `humatad2`, then attaches and types into the same shell.

### B3. The app reopens tab 1 by itself after the daemon restarts while the app is open

- **When:** The app is open on tab 1, and the daemon is restarted under it.
- **I do:** Nothing; the app is open.
- **I see (code as written):** The output stops for a moment. The view then reconnects and shows the current screen. Typing before the reconnect is refused.
- **State and events:** The link ends and the pump reports "The terminal's connection ended. Open the tab again" (`clients/live/src/link.rs:280`). The UI removes an ended link that had shown a screen and opens it again on the next sync (`ui/src/terminals.rs:300-310`).
- **Expected outcome:** The view reconnects to the same holder and shows what was printed meanwhile, with no new shell.
- **Status:** Code, no test. No test runs the desktop UI through a daemon restart. The daemon side is B2.

### B4. A huge backlog: the build printed about 2 MiB before I reopen

- **When:** The build printed a lot while the app was closed. The holder's scrollback is full.
- **I do:** Open the app and select tab 1.
- **I see:** The screen appears and shows the last lines. It does not show every line; the earlier ones were dropped by the scrollback limit.
- **State and events:** The app asks for a screen checkpoint (`clients/terminal_link/src/lib.rs:62`) and for no codecs (`:61`). So the daemon sends the checkpoint whole and uncompressed. The 2 MiB cap on raw output (`daemon/humatad/src/attach.rs:786`) applies to raw attaches, not to this checkpoint. The compressed and piecewise paths run only for a client that lists codecs.
- **Expected outcome:** A large backlog is shown quickly without a stall; the last line is there and the view is usable.
- **Status:** Works (real daemon) for the path the app uses. `daemon/humatad/tests/terminal.rs::a_client_that_lists_no_codecs_gets_its_snapshot_whole_and_raw` is the app's path. The piecewise and compressed paths (`a_big_snapshot_comes_in_pieces_to_a_client_that_decodes`, `output_comes_compressed_to_a_client_that_decodes_it`) are proven only for a test client that asks for them. The app does not use them yet; see the gaps.

### B5. The tab is the agent's tab (tab 0), not a shell

- **When:** I quit and reopen while the session's agent is running in tab 0.
- **I do:** Open the app and select tab 0.
- **I see:** The agent's earlier answer, then the answer to a line typed while the app was closed. The agent is the same run, not a new one.
- **State and events:** The holder for the agent is reattached. The agent is not started again with `--resume` or `--session-id`. The conversation id stays the same.
- **Expected outcome:** Same agent, same conversation, output from the gap shown.
- **Status:** Works (real daemon). `desktop/tests/live_flows.rs::after_the_app_restarts_the_session_tab_shows_what_was_printed_while_it_was_gone` checks the view has `agent up --session-id` and not `agent up --resume`. This test is macOS only.

## Expected outcomes

- [ ] After reopen, tab 1 is attached to the same holder and the same shell; no new shell starts.
- [ ] Output printed while no app was attached is on screen after reopen, after the output that was there before.
- [ ] A line typed after the screen arrives is written once.
- [ ] A line typed before the screen arrives is either held and sent once, or refused with a message. (The code currently drops it silently; see B1.)
- [ ] A daemon restart does not end the shell's program.
- [ ] Reopening the agent tab shows the same agent in the same conversation, not a new one.
- [ ] A large backlog shows its last lines quickly.
- [ ] The view opens at the bottom of the output after a restart.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. ⌘Q quits and the link ends | Code, no test | `desktop/src/main.rs:202-203` |
| 2. Shell keeps running while the app is closed | Works (real daemon) | `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` |
| 3. Reopen attaches the same holder | Works (real daemon) | `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` |
| 4. Shows what was printed while closed | Works (real daemon) | `clients/live/tests/term_restart.rs`; `desktop/tests/live_flows.rs::after_the_app_restarts_the_session_tab_shows_what_was_printed_while_it_was_gone` |
| 5. Scroll position or the bottom | Code, no test | `terminal/client/src/replica.rs:725-737` |
| 6. Typing once the screen is up | Works (real daemon) | `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` |
| B1. Typing during catch-up | Fakes only | `terminal/client/tests/component.rs::keys_before_the_first_screen_are_told_once_then_the_screen_takes_them` |
| B2. Daemon restarted, tab outlives it | Works (real daemon) | `daemon/humatad/tests/terminal.rs::a_tab_outlives_its_daemon` |
| B3. App reopens tab after daemon restart | Code, no test | `clients/live/src/link.rs:280`, `ui/src/terminals.rs:300-310` |
| B4. Huge backlog | Works (real daemon) | `daemon/humatad/tests/terminal.rs::a_client_that_lists_no_codecs_gets_its_snapshot_whole_and_raw` (the app's path) |
| B5. Agent tab, not a shell | Works (real daemon) | `desktop/tests/live_flows.rs::after_the_app_restarts_the_session_tab_shows_what_was_printed_while_it_was_gone` |

Tag count, steps: Works 4, Code no test 2, Fakes 0, Not built 0, Likely bug 0.
Tag count, branches: Works 3, Code no test 1, Likely bug 1.

## Gaps and likely bugs

- Fixed: keys typed before the screen arrives raise one toast that says to type again once it shows (`terminal/client/tests/component.rs::keys_before_the_first_screen_are_told_once_then_the_screen_takes_them`, Fakes only).
- Not built in the app: compressed and piecewise terminal snapshots. The daemon supports them, but the app lists no codecs (`clients/terminal_link/src/lib.rs:61`), so only a test client uses them. No milestone found. Decide whether the app should ask for them.
- Code, no test: ⌘Q (`desktop/src/main.rs:202-203`) and the app's own reopen after a daemon restart (`clients/live/src/link.rs:280`, `ui/src/terminals.rs:300-310`).
- Code, no test: the scroll position is not kept across a restart (`terminal/client/src/replica.rs:725-737`). No requirement found that asks for it.
