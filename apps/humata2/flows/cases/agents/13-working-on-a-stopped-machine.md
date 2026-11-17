# A stopped machine ends its agents, refuses their calls and scripts, and Start brings the sessions back with a report

Layer: agents. Platforms: macOS and Linux (the same daemon runs on either). The agent here is a fake agent in the tests, so no real agent is proven by this case.

## Starting state

- This Mac runs the host daemon. Its machine is `this-mac`, and `box-1` is linked to it and online.
- The channel `acme` has the repo `acme/api`. The session `fix-retry` runs on `this-mac` with an agent tab and a shell tab. Both are open and attached.
- The agent in `fix-retry` holds its session token in its environment. It has asked a question through the Inbox, and the request "Shall I drop the table?" is still pending.
- A script token named `ci` exists with `GlobalRead` and `ChannelRead` on `acme`.
- Channel rules are the defaults. An agent's stop and start are denied.
- I am in a plain terminal on this Mac. I have not run `humata login`.

## Steps

1. **I do:** in the plain terminal, run `humata stop --reason "end of day"`.
   **I see:** the command prints a stop id and exits 0. The sessions on `this-mac` show Stopped.
   **State and events:** one stop record exists. It names the reason "end of day", and it says the stop was made from a terminal through the CLI. Nothing else changes.
   **Status:** Works (real daemon). `daemon/humatad/tests/stop_access.rs::humata_stop_typed_outside_every_session_with_no_login_stops_the_machine` runs the real `humata` binary against the daemon and checks the exit, the reason and the stopper.

2. **I do:** look at the agent tab in `fix-retry`.
   **I see:** the agent's program has ended, and so has the shell tab's shell. No restart report appears for this stop.
   **State and events:** the sweep reports one agent and one shell ended. The holders for both tabs are gone. A stopped start writes no report.
   **Status:** Works (real daemon). `daemon/humatad/tests/stop_sessions.rs::a_stop_ends_every_agent_and_shell_the_sessions_started_and_writes_no_restart_report`. The agent is a fake that prints "agent up".

3. **I do:** open the Inbox.
   **I see:** the request "Shall I drop the table?" is no longer pending. It shows as withdrawn, and its reason is that the machine stopped.
   **State and events:** the request moves to Withdrawn with the reason MachineStopped. The pending list is empty.
   **Status:** Works (real daemon). `daemon/humatad/tests/stop_sessions.rs::a_stop_withdraws_the_agents_pending_inbox_request_as_stopped_with_its_machine`. The Inbox card on screen is not checked by that test.

4. **I do:** as the agent, run `humata channels` with the session token it holds in its environment. The test runs the real CLI with that token.
   **I see:** the call fails. The message says that sessions on this Mac are stopped, that agents and scripts are not answered, and that the user starts them with `humata start` before the call is made again.
   **State and events:** the daemon refuses the call with the kind `denied`, and the CLI exits 4. Nothing is run.
   **Status:** Works (real daemon). `daemon/humatad/tests/stop_sessions.rs::while_stopped_agents_and_scripts_are_refused_and_nothing_can_be_typed_attached_or_started`. The test checks only that the exit is neither 0 nor 12. It does not check the exit 4 or the kind. The `fix` field is empty; see Likely bug 1.

5. **I do:** the agent asks the user a question with `humata session ask` or the Inbox tool.
   **I see:** the ask is refused as it was in step 4. No request appears in the Inbox.
   **State and events:** no pending request is created.
   **Status:** Works (real daemon). The same test as step 4 asks through the agent's gRPC client and checks that the ask is refused with PermissionDenied and that no pending request exists.

6. **I do:** run `humata channels` with the script token `ci`, which holds `GlobalRead` and `ChannelRead`.
   **I see:** the call is refused. The script gets the same message as the agent.
   **State and events:** the script call is refused with PermissionDenied. Nothing is read through the daemon.
   **Status:** Works (real daemon). `daemon/humatad/tests/stop_sessions.rs::while_stopped_agents_and_scripts_are_refused_and_nothing_can_be_typed_attached_or_started` (the script part, through the channel service).

7. **I do:** click the agent tab, type "keep going", and try to attach a terminal to the shell tab. Then I try to start a new session on `this-mac`.
   **I see:** the attach and the typing are refused. The start is refused. No new process appears.
   **State and events:** each action is refused with FailedPrecondition. The text "keep going" never reaches the agent. No agent or shell process is running.
   **Status:** Works (real daemon). The same test covers the attach refusal on both tabs, the refused typing, and the refused start. The stopped banner that the app shows on screen is not checked by this test.

8. **I do:** the host daemon is killed (for example by a crash) and started again while the machine is stopped.
   **I see:** the sessions still show Stopped. The stop time and the reason are the same as before. No new report appears.
   **State and events:** the stop record survives the kill. Nothing starts on the new run.
   **Status:** Works (real daemon). `daemon/humatad/tests/stop_sessions.rs::a_daemon_killed_and_started_again_while_stopped_stays_stopped_and_starts_nothing`. The daemon is killed with SIGKILL, not stopped cleanly.

9. **I do:** run `humata start` in the plain terminal.
   **I see:** the command exits 0 and names the stop it came after. The sessions go back to Running, and a restart report for `this-mac` is listed. The report names the stop id.
   **State and events:** the start clears the stop record, starts each session as after a shutdown, and writes a restart report. The report's `restart.json` names the stop.
   **Status:** Works (real daemon). `daemon/humatad/tests/stop_sessions.rs::start_brings_the_sessions_back_as_after_a_shutdown_with_a_report_naming_the_stop`.

10. **I do:** watch the agent tab, then type a line into it.
    **I see:** the agent resumes its conversation, and the first line I typed before the stop is in its context. The reply to my new line comes back in the same tab.
    **State and events:** the agent restarts with `--resume` in the same folder. Its session id is the same.
    **Status:** Works (real daemon, fake agent). The same test as step 9 checks "agent up --resume", that the fake agent heard "one" before the stop, and that it hears "two" after the start. A real agent's resume is not tested here.

## Branches

### A stop finishes while a terminal reply waits on its holder

- **When:** the shell asks the terminal a question (a cursor position query). The holder has written the answer and waits for its confirmation, and the stop arrives before the confirmation.
- **I do:** run `humata stop` while the shell is waiting for its reply.
- **I see:** the stop still finishes. The shell and its holder end, and the sessions show Stopped.
- **Expected outcome:** the stop does not wait for the holder's confirmation. The holder is killed if it does not stop.
- **Status:** Works (real daemon). `daemon/humatad/tests/stop_sessions.rs::a_stop_finishes_while_a_terminal_reply_waits_on_its_holder`. The holder is paused with SIGSTOP, so only SIGKILL ends it.

### A stop or a start is cut at a fault point

- **When:** the daemon is killed partway through a stop or a start: as the record is written, as it restarts itself, once the holders are signalled, once the inbox is withdrawn, or once the report is written.
- **I do:** nothing. The daemon is cut at the fault point by a test build.
- **I see:** after the restart, the machine is either fully stopped or fully running. A stop cut mid-way is still a stop, and a start cut mid-way leaves a report that names the stop.
- **Expected outcome:** nothing is lost, and the state is one of the two whole states.
- **Status:** Works (real daemon, fault build). `daemon/humatad/tests/stop_sessions.rs::a_stop_cut_as_its_record_is_written_ends_stopped_or_still_running_with_nothing_lost`, `a_stop_cut_as_the_daemon_restarts_itself_ends_stopped_with_nothing_lost`, `a_stop_cut_once_the_holders_are_gone_ends_stopped_with_nothing_lost`, `a_start_cut_once_its_report_is_written_ends_running_with_a_report_naming_the_stop`.

### Under the default rule an agent cannot start the machine

- **When:** the machine is stopped and the agent calls start.
- **I do:** the agent runs `humata start`, or calls the start through its gRPC client.
- **I see:** the call is refused with PermissionDenied. The machine stays stopped.
- **Expected outcome:** the stop stays in place, and the agent is told it cannot start the sessions.
- **Status:** Works (real daemon). `daemon/humatad/tests/stop_access.rs::under_the_default_rule_an_agent_cannot_start_a_stopped_machine`.

### A rule that allows an agent's stop still does not let it start the user's stop

- **When:** the user has stopped the machine, and the channel rule for stops is Allow.
- **I do:** the agent calls start.
- **I see:** the call is refused. The machine stays stopped. The user's own start still works.
- **Expected outcome:** only the user starts a stop the user made.
- **Status:** Works (real daemon). `daemon/humatad/tests/stop_access.rs::an_agent_whose_rule_allows_it_still_cannot_start_the_users_stop`.

### An agent's own stop under the default rule is denied

- **When:** the agent runs `humata stop` while the machine is running.
- **I do:** run `humata stop --reason loop` in the agent's tab.
- **I see:** the command exits 4 with the kind `denied`, and it prints a fix. Nothing stops.
- **Expected outcome:** the default rule denies an agent's stop at once.
- **Status:** Works (real daemon). `daemon/humatad/tests/stop_access.rs::an_agents_humata_stop_under_the_default_rule_exits_denied_with_a_fix` and `a_script_whose_token_lacks_the_capability_is_refused_a_stop_and_nothing_stops`.

### The rule is Ask: the agent's stop waits for the user

- **When:** the channel rule for stops is Ask.
- **I do:** the agent runs `humata stop`.
- **I see:** a request appears in the Inbox. Nothing stops while it is pending.
- **Expected outcome:** the stop waits for the user's answer.
- **Status:** Works (real daemon). `daemon/humatad/tests/stop_access.rs::a_rule_of_ask_puts_an_agents_stop_in_the_inbox_and_stops_nothing_meanwhile`.

### A stop from a session tab without its token is refused

- **When:** a command in a session's tab runs `humata stop` with no token, or with one the daemon does not know.
- **I do:** run the command in that tab.
- **I see:** the command is refused. It is not taken as a plain terminal's stop.
- **Expected outcome:** nothing stops.
- **Status:** Works (real daemon). `daemon/humatad/tests/stop_access.rs::humata_stop_typed_inside_a_session_without_its_token_is_refused` and `a_stop_carrying_an_unusable_session_token_is_refused_not_taken_as_a_terminal`.

### PR reviews stop following while the machine is stopped

- **When:** a review session on `this-mac` watches a pull request while the machine is stopped.
- **I do:** stop the machine, wait for the review's polling interval, then start it.
- **I see:** while stopped, the review asks GitHub nothing. After the start, it follows the review again.
- **Expected outcome:** a stopped machine makes no GitHub calls on its own. A start brings the following back.
- **Status:** Works (real daemon, fake `gh`). `daemon/humatad/tests/pr_stopped.rs::a_stopped_machine_stops_following_its_reviews_and_its_start_follows_them_again`.

### A submit cut short on a stopped machine is settled at its start

- **When:** a review submit was cut short by the stop.
- **I do:** start the machine.
- **I see:** the submit is settled at the start, with the same result as if it had not been cut.
- **Expected outcome:** the submit is not posted twice.
- **Status:** Works (real daemon, fake `gh`). `daemon/humatad/tests/pr_stopped.rs::a_submit_cut_short_on_a_stopped_machine_is_settled_at_its_start`.

### A linked box's sessions are stopped and started from the host

- **When:** `box-1` is linked to the host and has sessions.
- **I do:** run `humata host stop box-1 --reason "box maintenance"` on this Mac, then `humata host start box-1`.
- **I see:** the box's sessions stop and start again. The box's restart report names the stop.
- **Expected outcome:** the host's stop reaches the box, and the box's start names it.
- **Status:** Works (real daemon, multi-machine rig, real daemons, fake ssh machine). `daemon/humatad/tests/stop_box.rs::a_host_stops_a_linked_boxs_sessions_and_starts_them_again_with_a_report_naming_the_stop`.

### The box's link hangs

- **When:** the box's link to the host hangs during the stop.
- **I do:** run the stop from the host.
- **I see:** the box is still stopped through a one-off SSH call.
- **Expected outcome:** a hung link does not leave the box running.
- **Status:** Works (real daemon, multi-machine rig, real daemons, fake ssh machine). `daemon/humatad/tests/stop_box.rs::a_box_whose_link_hangs_is_still_stopped_through_its_one_off_ssh`.

### The box can be reached neither way

- **When:** the stop cannot reach `box-1` by its link or by SSH.
- **I do:** run the stop, then restart the host, then let the box link back.
- **I see:** the stop waits on the host. When the box links back, the stop lands once.
- **Expected outcome:** the stop is kept across a host restart and applied once.
- **Status:** Works (real daemon, multi-machine rig, real daemons, fake ssh machine). `daemon/humatad/tests/stop_box.rs::a_stop_that_reaches_a_box_neither_way_is_kept_across_a_host_restart_and_lands_once_when_the_box_links_back`.

### Starting a box cancels a stop it never got

- **When:** the stop never reached `box-1`, and the user starts the box.
- **I do:** run `humata host start box-1`.
- **I see:** the undelivered stop is cancelled, and the box runs.
- **Expected outcome:** nothing stays stopped that the user started.
- **Status:** Works (real daemon, multi-machine rig, real daemons, fake ssh machine). `daemon/humatad/tests/stop_box.rs::starting_a_box_whose_stop_was_never_delivered_cancels_the_stop`.

### The agent tries the stop or the start as an MCP tool

- **When:** the agent calls the MCP tool `machine_stop` or `session_start`.
- **I do:** the agent calls the tool by name.
- **I see:** the call returns a "not built" error that names the operation.
- **Expected outcome:** the call is not served. The agent is told it is not built.
- **Status:** Not built (no milestone). `daemon/humatad/src/mcp.rs` `fn call` has no arm for `MachineStop` or `SessionStart`, so both fall to `other => Err(not_built(other))`. The tools are defined in `daemon/mcp/src/tools.rs` (`MachineStop`, `SessionStart`) and listed in `api/MCP.md`.

## Expected outcomes

- [ ] A stop from a plain terminal ends every agent and shell on the machine, and writes no restart report.
- [ ] A pending Inbox request is withdrawn with the reason MachineStopped.
- [ ] While stopped, an agent's call, a script's call and an ask are each refused.
- [ ] While stopped, the user cannot attach to, type into or start a session.
- [ ] A kill and restart while stopped leaves the stop record as it was and starts nothing.
- [ ] A start names the stop it came after, writes one restart report, and brings the sessions back.
- [ ] The agent resumes its conversation after the start, with what it heard before the stop.
- [ ] Under the default rule, an agent cannot stop or start the machine.
- [ ] A stop cut at any fault point ends either fully stopped or fully running, with nothing lost.
- [ ] A box stop reaches the box by link or SSH, and a box start cancels an undelivered stop.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. The user stops the machine from a terminal | Works (real daemon) | `stop_access.rs::humata_stop_typed_outside_every_session_with_no_login_stops_the_machine` |
| 2. The agent's process ends | Works (real daemon) | `stop_sessions.rs::a_stop_ends_every_agent_and_shell_the_sessions_started_and_writes_no_restart_report` |
| 3. The pending request is withdrawn | Works (real daemon) | `stop_sessions.rs::a_stop_withdraws_the_agents_pending_inbox_request_as_stopped_with_its_machine` |
| 4. The agent's call is refused | Works (real daemon) | `stop_sessions.rs::while_stopped_agents_and_scripts_are_refused_and_nothing_can_be_typed_attached_or_started` (exit code not asserted) |
| 5. The agent's ask is refused | Works (real daemon) | same test as step 4 |
| 6. The script's call is refused | Works (real daemon) | same test as step 4 |
| 7. Typing, attaching and starting are refused | Works (real daemon) | same test as step 4 (on-screen banner not covered) |
| 8. A kill and restart stays stopped | Works (real daemon) | `stop_sessions.rs::a_daemon_killed_and_started_again_while_stopped_stays_stopped_and_starts_nothing` |
| 9. Start brings the sessions back with a report | Works (real daemon) | `stop_sessions.rs::start_brings_the_sessions_back_as_after_a_shutdown_with_a_report_naming_the_stop` |
| 10. The agent resumes its conversation | Works (real daemon, fake agent) | same test as step 9 |
| Branch: stop during a terminal reply | Works (real daemon) | `stop_sessions.rs::a_stop_finishes_while_a_terminal_reply_waits_on_its_holder` |
| Branch: a stop or start cut at a fault point | Works (real daemon, fault build) | `stop_sessions.rs::a_stop_cut_as_its_record_is_written_ends_stopped_or_still_running_with_nothing_lost` and others |
| Branch: default rule, agent cannot start | Works (real daemon) | `stop_access.rs::under_the_default_rule_an_agent_cannot_start_a_stopped_machine` |
| Branch: Allow rule, agent cannot start the user's stop | Works (real daemon) | `stop_access.rs::an_agent_whose_rule_allows_it_still_cannot_start_the_users_stop` |
| Branch: agent's own stop denied | Works (real daemon) | `stop_access.rs::an_agents_humata_stop_under_the_default_rule_exits_denied_with_a_fix` |
| Branch: Ask rule | Works (real daemon) | `stop_access.rs::a_rule_of_ask_puts_an_agents_stop_in_the_inbox_and_stops_nothing_meanwhile` |
| Branch: stop without a usable token | Works (real daemon) | `stop_access.rs::a_stop_carrying_an_unusable_session_token_is_refused_not_taken_as_a_terminal` |
| Branch: PR reviews stop following | Works (real daemon, fake `gh`) | `pr_stopped.rs::a_stopped_machine_stops_following_its_reviews_and_its_start_follows_them_again` |
| Branch: submit cut short | Works (real daemon, fake `gh`) | `pr_stopped.rs::a_submit_cut_short_on_a_stopped_machine_is_settled_at_its_start` |
| Branch: box stop from the host | Works (real daemon, fake ssh machine) | `stop_box.rs::a_host_stops_a_linked_boxs_sessions_and_starts_them_again_with_a_report_naming_the_stop` |
| Branch: box link hangs | Works (real daemon, fake ssh machine) | `stop_box.rs::a_box_whose_link_hangs_is_still_stopped_through_its_one_off_ssh` |
| Branch: box reached neither way | Works (real daemon, fake ssh machine) | `stop_box.rs::a_stop_that_reaches_a_box_neither_way_is_kept_across_a_host_restart_and_lands_once_when_the_box_links_back` |
| Branch: box start cancels an undelivered stop | Works (real daemon, fake ssh machine) | `stop_box.rs::starting_a_box_whose_stop_was_never_delivered_cancels_the_stop` |
| Branch: MCP `machine_stop` or `session_start` | Not built (no milestone) | `daemon/humatad/src/mcp.rs` `fn call` has no arm for them |

## Gaps and likely bugs

- **Not built:** the agent's MCP tools `machine_stop` and `session_start` are defined and documented but not dispatched. They fall to `not_built`. No milestone found in `workflow/`.
- **Likely bug 1:** `daemon/access/src/verdict.rs:338-340` maps `MachineStopped` to `Fix::Nothing`. `daemon/humatad/src/status.rs:711` renders that as "nothing the caller can change makes this work", which is wrong here: the user can start the sessions. The message at `verdict.rs:181` gives the right advice, but the `fix` field the JSON reports is the wrong one. To confirm: while stopped, run `humata channels --json` as an agent and read `error.fix`.
- **Test gap:** `while_stopped_agents_and_scripts_are_refused_and_nothing_can_be_typed_attached_or_started` checks the CLI exit only as "not 0 and not 12". It does not check for 4 or for the kind `denied`.
- **Not covered:** the stopped banner and the "Stopped" state on screen in the app. The daemon tests above do not check the app.
