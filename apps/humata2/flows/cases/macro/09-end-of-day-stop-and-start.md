# Stopping every session at the end of the day keeps them stopped through a reboot, and starting them next morning reports the stop

Layer: macro. Platforms: macOS (this Mac) and a linked box (box-1).

Status tags in this file: a test that runs the real `humatad2` is tagged
**Works (real daemon)**. Where the milestone file says the test is not yet
green, the Evidence column says so. Engine-only and fake-client tests are
**Fakes only**. A step or branch takes the tag of its main claim. The Evidence
column names any part that is only faked or not built.

## Starting state

- This Mac runs the host daemon. Channel "Payments" has one session with
  two tabs: an agent tab and a shell tab. Both are running.
- box-1 is linked to this Mac and runs one session with an agent.
- The agent on this Mac has one pending Inbox request: "Shall I drop the
  table?"
- A terminal is attached to the shell tab on this Mac.
- The app is open on the machine page for this Mac. Nothing is stopped.

## Steps

1. **Open the machine's More menu.**
   - **I do:** click **More** on the machine page for this Mac.
   - **I see:** a menu with **Stop all sessions…** (ui/src/machine_page.rs:443).
   - **State and events:** nothing is sent to the daemon.
   - **Status:** Fakes only. `desktop/tests/machines_flow.rs::stop_all_sessions_asks_first_and_stops_only_when_confirmed`
     (fakes, headless). The menu item is not drawn in a live run.

2. **Read the confirm sheet.**
   - **I do:** click **Stop all sessions…**.
   - **I see:** the sheet "Stop all sessions on this Mac?" with these lines
     (ui/src/machine_page.rs:1211-1245):
     - "Every agent and shell in this Mac's N sessions gets 2 seconds to quit,
       then is killed."
     - Kept: "Scrollback, conversations, todos, rewind points and open tabs."
     - While stopped: "Agents can't type, call Humata or be answered. Their
       open requests end."
     - "… stay stopped, even after it restarts, until you choose Start
       sessions."
     Buttons: **Cancel** and a red **Stop all sessions**.
   - **State and events:** no stop is sent. The daemon has no record of a stop.
   - **Status:** Fakes only. `features/hosts/tests/stop.rs::stop_all_sessions_asks_first_and_sends_nothing_until_confirmed`.
     The copy is not checked against a golden, so the wording is unproven live.

3. **Confirm the stop.**
   - **I do:** click the red **Stop all sessions**.
   - **I see:** the sheet closes. The page reloads with the status "Sessions
     stopped" and a **Start sessions** button.
   - **State and events:** the app sends the stop for this Mac. The daemon
     records one stop with the user as the stopper and the page shows it.
   - **Status:** Fakes only. `desktop/tests/machines_flow.rs::stop_all_sessions_asks_first_and_stops_only_when_confirmed`
     and `features/hosts/tests/stop.rs::confirming_the_stop_shows_sessions_stopped_after_the_reload`.
     The live client call is **Code, no test**: `clients/live/src/machines.rs:1102-1116`
     has no test in `clients/live/tests`, `cli/tests` or `desktop/tests`.

4. **Agents and shells end.**
   - **I do:** nothing. The daemon does this during the stop.
   - **I see:** the agent's program is gone and the shell's child is gone.
     Nothing from Humata is still running for this Mac.
   - **State and events:** the sweep hangs up each program group, waits 2 s,
     kills what is left, then ends each holder (SIGTERM, 2 s, SIGKILL). The
     daemon reports how many agents and shells it ended (`ended`: 1 agent,
     1 shell in the test).
   - **Status:** Works (real daemon). `daemon/humatad/tests/stop_sessions.rs::a_stop_ends_every_agent_and_shell_the_sessions_started_and_writes_no_restart_report`.
     Sweep timing is also covered by unit tests in `daemon/humatad/src/stop/sweep.rs`.

5. **The attached terminal is told to connect again.**
   - **I do:** nothing. The terminal on the shell tab was open in step 3.
   - **I see:** the terminal stops taking input and shows that it ended. The
     daemon's words are: "humatad2 is stopping or starting itself again, so
     this stream ended; connect again in a moment" (daemon/humatad/src/stop/streams.rs:35).
   - **State and events:** the attach stream ends with gRPC `UNAVAILABLE`
     within 2.5 s of the stop, not after the 3 s shutdown grace.
   - **Status:** Works (real daemon). `daemon/humatad/tests/stop_sessions.rs::a_terminal_attached_when_the_sessions_stop_is_told_to_connect_again`.
     The test checks the status code and the timing, not the words on screen.
     The app's handling of the ended stream (ui/src/terminals.rs) is not tested for this case.

6. **The pending Inbox request is withdrawn.**
   - **I do:** open the Inbox after the stop.
   - **I see:** the "Shall I drop the table?" request is no longer pending.
     Its state is Withdrawn with the reason "machine stopped".
   - **State and events:** the request is Withdrawn, reason MachineStopped
     (daemon/humatad/src/stop/boot.rs:74-77). No request is pending.
   - **Status:** Works (real daemon). `daemon/humatad/tests/stop_sessions.rs::a_stop_withdraws_the_agents_pending_inbox_request_as_stopped_with_its_machine`.
     The card text that says "stopped" is not asserted by this test.

7. **The stop writes no restart report.**
   - **I do:** open the restart reports folder or run `humata doctor restarts`.
   - **I see:** no new report for the stop.
   - **State and events:** the start that follows a stop reports nothing, and
     no report folder is added.
   - **Status:** Works (real daemon). The same test as step 4 asserts the report
     folders are unchanged. The `humata doctor restarts` command is not run in
     that test.

8. **Nothing can reach a stopped session.**
   - **I do:** try to type in the agent tab, attach to the shell tab, or start
     a new session on this Mac. As an agent, ask a question. As a script, call
     `channels`.
   - **I see:** each is refused. The app says **Start sessions first** when the
     user tries to open a shell (`features/hosts/src/page.rs` and
     `desktop/tests/machines_flow.rs::while_stopped_open_a_shell_is_refused_with_a_note`).
   - **State and events:** no bytes reach the holders. No request is added to
     the Inbox. The agent gets PermissionDenied. The attach gets
     FailedPrecondition.
   - **Status:** Works (real daemon). `daemon/humatad/tests/stop_sessions.rs::while_stopped_agents_and_scripts_are_refused_and_nothing_can_be_typed_attached_or_started`.
     The app-side refusal is Fakes only (`desktop/tests/machines_flow.rs::while_stopped_open_a_shell_is_refused_with_a_note`).

9. **The Mac restarts overnight.**
   - **I do:** restart the Mac. Nothing else.
   - **I see:** after the restart the machine page still reads "Sessions
     stopped". No agent runs, and no shell starts.
   - **State and events:** the daemon starts, sees the stop, and skips every
     start-time recovery. It writes no restart report.
   - **Status:** Works (real daemon), with a gap. `daemon/humatad/tests/stop_sessions.rs::a_daemon_killed_and_started_again_while_stopped_stays_stopped_and_starts_nothing`
     kills and restarts the daemon, not the operating system. Whether the daemon
     starts by itself after a real Mac reboot (a launch agent) is **Code, no test**.

10. **Stop box-1 too.**
    - **I do:** on the machine page for box-1, choose **More** then
      **Stop all sessions…**, and confirm.
    - **I see:** box-1 shows "Sessions stopped" with the same confirm copy.
    - **State and events:** the host sends the stop to box-1 by its peer link,
      or by a one-off SSH script if the link is down. Box-1 ends its sessions
      and keeps the stop.
    - **Status:** Works (real daemon, fake box). `daemon/humatad/tests/stop_box.rs::a_host_stops_a_linked_boxs_sessions_and_starts_them_again_with_a_report_naming_the_stop`.
      The app side is Fakes only (`features/hosts/tests/stop.rs::this_mac_is_stopped_the_same_way`).

11. **The next morning I press Start sessions.**
    - **I do:** on the machine page for this Mac, click **Start sessions**, then
      confirm **Start sessions** in the sheet.
    - **I see:** the sheet "Start sessions on this Mac?" closes and the page
      shows the sessions coming back. The agent resumes on its conversation,
      and the shell tab shows what it printed before the stop.
    - **State and events:** the daemon moves the stop to "starting", runs the
      normal bring-back, and writes one restart report. The report records
      that it came after the stop (`after_stop`), with the stop's id.
    - **Status:** Works (real daemon) for the daemon side. `daemon/humatad/tests/stop_sessions.rs::start_brings_the_sessions_back_as_after_a_shutdown_with_a_report_naming_the_stop`.
      The confirm sheet is Fakes only (`desktop/tests/machines_flow.rs::start_sessions_asks_first_and_starts_only_when_confirmed`).

12. **I read the report.**
    - **I do:** look at the restart report in the app.
    - **I see:** the app shows the sessions brought back and any that were not.
      It does not yet say that the start came after a stop.
    - **State and events:** the report on disk names the stop. The app's model
      does not read `after_stop`.
    - **Status:** Not built (no milestone). `after_stop` is set by the daemon
      (daemon/humatad/src/stop/boot.rs) and read nowhere else in the repository
      (`grep -rl after_stop` finds only `daemon/humatad`). STOP.md's T6 covers
      the Stop and Start controls, not this report line.

## Branches

### Stop pressed twice acts once

- **When:** the user confirms a stop, then sends the stop again (a double click,
  or a second window).
- **I do:** confirm the stop, then send the same stop again.
- **I see:** the second request gets the same stop back. The stop time and
  reason do not change.
- **Expected outcome:** one stop. Nothing is ended twice.
- **Status:** Works (real daemon). `daemon/humatad/tests/stop_sessions.rs::a_repeated_stop_acts_once_and_a_start_with_nothing_stopped_is_refused`.
  The menu also offers no second stop while stopped: Fakes only,
  `features/hosts/tests/stop.rs::a_stopped_machine_offers_no_second_stop`.

### Start with nothing stopped is refused

- **When:** Start sessions is sent while the machine runs.
- **I do:** send **Start sessions** (or `humata start`) on a running machine.
- **I see:** an error that says nothing is stopped. The sessions keep running.
- **Expected outcome:** FailedPrecondition. No restart report, nothing started.
- **Status:** Works (real daemon). Same test as the branch above. The button is
  not offered while sessions run: Fakes only,
  `features/hosts/tests/stop.rs::start_sessions_does_nothing_while_the_sessions_run`.

### A cut at each stop step

- **When:** the power goes off, or the daemon dies, at one of the seven points of
  a stop: `stop.record.written`, `stop.exec`, `stop.programs.hungup`,
  `stop.holders.signalled`, `stop.inbox.withdrawn`, `stop.holders.gone`,
  `stop.record.settled`.
- **I do:** stop the sessions while the daemon is cut at that point, then start the
  daemon again.
- **I see:** either the sessions are stopped with nothing running, or, for a cut
  before the stop was recorded, they are still running as before. Start then
  brings everything back.
- **Expected outcome:** nothing is lost and nothing is half stopped.
- **Status:** Works (real daemon). Seven tests:
  `daemon/humatad/tests/stop_sessions.rs::a_stop_cut_as_its_record_is_written_ends_stopped_or_still_running_with_nothing_lost`
  and six others named `a_stop_cut_*` in the same file.
  These are simulated cuts (the daemon is killed at a fault point). They are
  not a real power cut. The power-cut suite for SAFE is a separate case.

### A cut at each start step

- **When:** the daemon dies at one of the four start points.
- **I do:** start the sessions while the daemon is cut, then start the daemon again.
- **I see:** the sessions are running, and a restart report names the stop.
- **Expected outcome:** running, with the report.
- **Status:** Works (real daemon). Four tests in `daemon/humatad/tests/stop_sessions.rs`:
  `a_start_cut_once_its_record_is_renamed_ends_running_with_a_report_naming_the_stop`,
  `a_start_cut_as_the_daemon_restarts_itself_ends_running_with_a_report_naming_the_stop`,
  `a_start_cut_once_its_report_is_written_ends_running_with_a_report_naming_the_stop` and
  `a_start_cut_once_its_record_is_removed_ends_running_with_a_report_naming_the_stop`.

### A program that ignores hangup and terminate

- **When:** a shell ignores HUP and TERM (`trap '' HUP TERM`).
- **I do:** run that shell in the tab, then stop the sessions.
- **I see:** the shell is killed after the grace period, and the stop reports
  at least one killed program and none left.
- **Expected outcome:** nothing is left running.
- **Status:** Works (real daemon). `daemon/humatad/tests/stop_sessions.rs::a_shell_that_ignores_hangup_and_terminate_is_killed_by_the_stop`.

### An agent tries to stop its machine

- **When:** an agent calls `humata stop` or the StopSessions call.
- **I do:** the agent runs `humata stop --reason loop`.
- **I see:** under the default rule, the agent gets exit 4 (denied) with a fix.
  Nothing stops.
- **Expected outcome:** denied, nothing stopped. With a rule of Ask me, a request
  waits in the Inbox and nothing stops meanwhile. With a rule of Allow, the stop
  happens and is named as the agent's.
- **Status:** Works (real daemon). `daemon/humatad/tests/stop_access.rs::an_agent_under_the_default_rule_is_denied_a_stop_at_once_and_nothing_stops`,
  `an_agents_humata_stop_under_the_default_rule_exits_denied_with_a_fix`,
  `a_rule_of_ask_puts_an_agents_stop_in_the_inbox_and_stops_nothing_meanwhile`,
  `an_agent_whose_rule_allows_it_stops_its_machine_itself_included_and_is_named_as_the_stopper`.

### An agent tries to start the machine

- **When:** an agent calls Start sessions after the user stopped the machine.
- **I do:** the agent runs `humata start`.
- **I see:** PermissionDenied. The machine stays stopped.
- **Expected outcome:** refused, still stopped. This holds even with a rule of
  Allow, because only the user starts the user's stop.
- **Status:** Works (real daemon). `daemon/humatad/tests/stop_access.rs::under_the_default_rule_an_agent_cannot_start_a_stopped_machine`
  and `an_agent_whose_rule_allows_it_still_cannot_start_the_users_stop`.

### `humata stop` typed in a plain terminal, with no login

- **When:** a person types `humata stop` in a terminal that is not inside any
  Humata session and has no login.
- **I do:** run `humata stop --reason "from a terminal"`.
- **I see:** exit 0 with the stop in the JSON reply. The machine reads Stopped.
- **Expected outcome:** stopped, named as typed in the terminal, via the CLI.
- **Status:** Works (real daemon). `daemon/humatad/tests/stop_access.rs::humata_stop_typed_outside_every_session_with_no_login_stops_the_machine`.

### `humata start` with no login

- **When:** the same terminal runs `humata start` while the machine is stopped.
- **I do:** run `humata start`.
- **I see:** exit 5 (needs the user), with a fix. The machine stays stopped.
- **Expected outcome:** refused.
- **Status:** Works (real daemon). `daemon/humatad/tests/stop_access.rs::humata_start_with_no_login_is_refused_and_the_machine_stays_stopped`.

### `humata stop` inside a session without its token, or with a bad token

- **When:** a shell inside a session runs `humata stop` without a session
  token, or with a token that is not valid.
- **I do:** run the command.
- **I see:** a non-zero exit. Nothing stops.
- **Expected outcome:** refused. A bad token is not taken for a terminal.
- **Status:** Works (real daemon). `daemon/humatad/tests/stop_access.rs::humata_stop_typed_inside_a_session_without_its_token_is_refused`
  and `a_stop_carrying_an_unusable_session_token_is_refused_not_taken_as_a_terminal`.

### A script whose token lacks the stop capability

- **When:** a script calls StopSessions with a token that does not list it.
- **I do:** call the stop with the script's token.
- **I see:** PermissionDenied. Nothing stops.
- **Expected outcome:** refused.
- **Status:** Works (real daemon). `daemon/humatad/tests/stop_access.rs::a_script_whose_token_lacks_the_capability_is_refused_a_stop_and_nothing_stops`.

### Box-1's link hangs during the stop

- **When:** the link to box-1 is down or hangs when the stop is sent.
- **I do:** stop box-1 while its link hangs.
- **I see:** the stop still reaches box-1 through its one-off SSH script, and
  box-1 reads Stopped.
- **Expected outcome:** stopped through the SSH path.
- **Status:** Works (real daemon, fake box). `daemon/humatad/tests/stop_box.rs::a_box_whose_link_hangs_is_still_stopped_through_its_one_off_ssh`.

### The stop reaches box-1 neither way

- **When:** box-1 is offline for the whole stop, and the host restarts meanwhile.
- **I do:** stop box-1 while it is unreachable. Restart the host. Bring box-1 back.
- **I see:** box-1 reads "Stop not delivered" until the stop lands. When the
  box links back, the stop lands once and the sessions stop.
- **Expected outcome:** the stop is kept across the host restart and lands once.
- **Status:** Works (real daemon, fake box). `daemon/humatad/tests/stop_box.rs::a_stop_that_reaches_a_box_neither_way_is_kept_across_a_host_restart_and_lands_once_when_the_box_links_back`.
  The app's "Stop not delivered" row is Fakes only (`features/hosts/tests/stop.rs::a_stop_the_machine_never_got_is_cancelled_without_a_confirm`).

### Starting a box whose stop never arrived cancels the stop

- **When:** box-1 never got the stop and the user chooses Cancel stop.
- **I do:** click **Cancel stop** on box-1.
- **I see:** no confirm. box-1 reads running.
- **Expected outcome:** the stop is cancelled. A later link does not stop box-1.
- **Status:** Works (real daemon, fake box). `daemon/humatad/tests/stop_box.rs::starting_a_box_whose_stop_was_never_delivered_cancels_the_stop`.
  The UI click is Fakes only (`desktop/tests/machines_flow.rs::a_stop_that_has_not_reached_the_machine_is_cancelled_without_a_confirm`).

### PR reviews stop following while stopped

- **When:** a review is open on this Mac and the machine is stopped.
- **I do:** change the PR head on GitHub while the machine is stopped, then
  start the machine.
- **I see:** nothing changes while stopped. After the start, the review follows
  the PR again.
- **Expected outcome:** no GitHub polling while stopped. Polling resumes after
  the start.
- **Status:** Works (real daemon, fake GitHub). `daemon/humatad/tests/pr_stopped.rs::a_stopped_machine_stops_following_its_reviews_and_its_start_follows_them_again`.
  A review submit cut short while stopped is settled at the start: `daemon/humatad/tests/pr_stopped.rs::a_submit_cut_short_on_a_stopped_machine_is_settled_at_its_start`.

### The CLI stops or starts box-1 from this Mac

- **When:** the user runs `humata host stop box-1` (or `humata host start box-1`).
- **I do:** run the command on this Mac.
- **I see:** box-1 stops (or starts), with a reason shown on its page.
- **Expected outcome:** the same as the button.
- **Status:** Code, no test. `cli/src/commands/hosts.rs:171-186` handles the
  command, and the daemon side is covered by `stop_box.rs`, but no CLI test calls
  `host stop` or `host start`. `cli/tests/stop.rs` covers `humata stop` and
  `humata start` only.

## Expected outcomes

- [ ] Stop asks first; cancelling it stops nothing.
- [ ] After a confirmed stop, every agent and shell this Mac started has ended, and nothing Humata started is still running.
- [ ] A program that ignores hangup is killed, and the stop reports it.
- [ ] A terminal attached at the stop ends with a status that says to connect again.
- [ ] The pending agent request is withdrawn, with the reason "machine stopped".
- [ ] No restart report is written by the stop.
- [ ] While stopped, agents, scripts, typing, attaching and new sessions are refused.
- [ ] A reboot (here: a daemon restart) keeps the machine stopped and starts nothing.
- [ ] Start brings the sessions back as after a shutdown, and the start's report names the stop.
- [ ] A repeated stop acts once; a start with nothing stopped is refused.
- [ ] A cut at any stop or start point leaves everything stopped or running, with nothing lost.
- [ ] An agent can never start the user's stop, and an agent's stop needs its rule.
- [ ] `humata stop` in a plain terminal stops the machine; `humata start` with no login is refused.
- [ ] Box-1 is stopped through its link, or through SSH when the link hangs, and a stop that never arrived is cancelled by Start.
- [ ] A stopped machine stops following its PR reviews.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Open More menu | Fakes only | `desktop/tests/machines_flow.rs::stop_all_sessions_asks_first_and_stops_only_when_confirmed` |
| 2. Read confirm sheet | Fakes only | `features/hosts/tests/stop.rs::stop_all_sessions_asks_first_and_sends_nothing_until_confirmed` |
| 3. Confirm stop (app to daemon) | Fakes only; live path Code, no test | `clients/live/src/machines.rs:1102-1116` |
| 4. Agents and shells end | Works (real daemon) | `daemon/humatad/tests/stop_sessions.rs::a_stop_ends_every_agent_and_shell_the_sessions_started_and_writes_no_restart_report` |
| 5. Attached terminal told to connect again | Works (real daemon) | `daemon/humatad/tests/stop_sessions.rs::a_terminal_attached_when_the_sessions_stop_is_told_to_connect_again` |
| 6. Pending request withdrawn | Works (real daemon) | `daemon/humatad/tests/stop_sessions.rs::a_stop_withdraws_the_agents_pending_inbox_request_as_stopped_with_its_machine` |
| 7. No restart report on stop | Works (real daemon) | same test as step 4 |
| 8. Nothing reaches a stopped session | Works (real daemon); UI refusal Fakes only | `daemon/humatad/tests/stop_sessions.rs::while_stopped_agents_and_scripts_are_refused_and_nothing_can_be_typed_attached_or_started` |
| 9. Restart stays stopped | Works (real daemon), daemon restart only | `daemon/humatad/tests/stop_sessions.rs::a_daemon_killed_and_started_again_while_stopped_stays_stopped_and_starts_nothing` |
| 10. Stop box-1 | Works (real daemon, fake box); UI Fakes only | `daemon/humatad/tests/stop_box.rs::a_host_stops_a_linked_boxs_sessions_and_starts_them_again_with_a_report_naming_the_stop` |
| 11. Start sessions | Works (real daemon); confirm sheet Fakes only | `daemon/humatad/tests/stop_sessions.rs::start_brings_the_sessions_back_as_after_a_shutdown_with_a_report_naming_the_stop` |
| 12. Report names the stop in the app | Not built (no milestone) | `after_stop` read nowhere outside `daemon/humatad` |
| Branch: stop twice | Works (real daemon) | `daemon/humatad/tests/stop_sessions.rs::a_repeated_stop_acts_once_and_a_start_with_nothing_stopped_is_refused` |
| Branch: start with nothing stopped | Works (real daemon) | same test |
| Branch: cut at each stop step | Works (real daemon, simulated cut) | seven `a_stop_cut_*` tests in `stop_sessions.rs` |
| Branch: cut at each start step | Works (real daemon, simulated cut) | four `a_start_cut_*` tests in `stop_sessions.rs` |
| Branch: shell ignores hangup | Works (real daemon) | `daemon/humatad/tests/stop_sessions.rs::a_shell_that_ignores_hangup_and_terminate_is_killed_by_the_stop` |
| Branch: agent stops (denied, ask, allow) | Works (real daemon) | `daemon/humatad/tests/stop_access.rs::an_agent_under_the_default_rule_is_denied_a_stop_at_once_and_nothing_stops` |
| Branch: agent starts (refused) | Works (real daemon) | `daemon/humatad/tests/stop_access.rs::under_the_default_rule_an_agent_cannot_start_a_stopped_machine` |
| Branch: `humata stop` in a plain terminal | Works (real daemon) | `daemon/humatad/tests/stop_access.rs::humata_stop_typed_outside_every_session_with_no_login_stops_the_machine` |
| Branch: `humata start` with no login | Works (real daemon) | `daemon/humatad/tests/stop_access.rs::humata_start_with_no_login_is_refused_and_the_machine_stays_stopped` |
| Branch: `humata stop` in a session, bad token | Works (real daemon) | `daemon/humatad/tests/stop_access.rs::humata_stop_typed_inside_a_session_without_its_token_is_refused` |
| Branch: script without capability | Works (real daemon) | `daemon/humatad/tests/stop_access.rs::a_script_whose_token_lacks_the_capability_is_refused_a_stop_and_nothing_stops` |
| Branch: box link hangs | Works (real daemon, fake box) | `daemon/humatad/tests/stop_box.rs::a_box_whose_link_hangs_is_still_stopped_through_its_one_off_ssh` |
| Branch: stop reaches box neither way | Works (real daemon, fake box); UI Fakes only | `daemon/humatad/tests/stop_box.rs::a_stop_that_reaches_a_box_neither_way_is_kept_across_a_host_restart_and_lands_once_when_the_box_links_back` |
| Branch: cancel a stop never delivered | Works (real daemon, fake box); UI Fakes only | `daemon/humatad/tests/stop_box.rs::starting_a_box_whose_stop_was_never_delivered_cancels_the_stop` |
| Branch: PR reviews while stopped | Works (real daemon, fake GitHub) | `daemon/humatad/tests/pr_stopped.rs::a_stopped_machine_stops_following_its_reviews_and_its_start_follows_them_again` |
| Branch: `humata host stop box-1` | Code, no test | `cli/src/commands/hosts.rs:171-186` |

## Gaps and likely bugs

- **Not built (no milestone):** the restart report does not name the stop in the
  app. The daemon sets `after_stop` (daemon/humatad/src/stop/boot.rs), and no
  client or UI code reads it. STOP.md T6 covers the controls only.
- **Not built (STOP T4 to T7 pending gate):** STOP.md says batches 6 to 9 (the box
  and access gates, frames 72 to 76, and the app model and views) were waiting on
  main's gate when it was written. The stop_box and stop_access tests are in the
  tree, but the milestone does not record them as green. Only `stop_sessions` is
  recorded green at gate_b3 (17/17).
- **Code, no test:** the live client calls `stop_machine_sessions` and
  `start_machine_sessions` (clients/live/src/machines.rs:1102-1131). No test drives
  them against a real daemon from the app or from the CLI.
- **Code, no test:** the `humata host stop|start` CLI (cli/src/commands/hosts.rs:171-186).
- **Gap in the tests, not a bug:** the "cut" tests kill the daemon at a fault point
  (`HUMATA2_STOP_AT`). They do not cut power to the machine.
- **Gap in the tests:** the "reboot" test restarts the daemon process. It does not
  reboot the Mac, so a launch agent that starts the daemon after a real reboot is
  unproven.
- **Likely bug (low):** daemon/humatad/src/stop/boot.rs:83-86 logs a failed Inbox
  withdrawal and goes on to report the machine stopped. A request that failed to
  withdraw stays Pending while the machine is stopped. Confirm by making
  `withdraw_all_pending` return an error in a test, then reading the Inbox.
- **Likely bug (low):** daemon/humatad/src/stop/service.rs:184-188 records a
  script's stop as `ByKind::Agent` with no session. The stop then reads as an agent's
  in the stopper line and the Inbox. Confirm by stopping through a script token and
  reading `by.kind` in the reply.
- **Brief correction:** `daemon/humatad/tests/stop.rs` is not a stop test. It checks
  that SIGTERM ends an open event subscription and the daemon exits. Its only test is
  `sigterm_ends_an_open_event_subscription_and_the_daemon_exits`.
- **Brief correction:** "Stop pressed twice acts once" is proven by the daemon test
  only. The app's no-second-stop rule is a Fakes test.
