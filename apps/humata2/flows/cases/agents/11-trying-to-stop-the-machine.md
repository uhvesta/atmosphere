# An agent that tries to stop the machine is denied, asks, or stops it only when I allow it, and it never starts my stop

Layer: agents. Platforms: macOS and Linux. Web is not covered.

## Starting state

- Channel `Payments` on this Mac, repo `acme/api`. Its rule for "stop or start all sessions on a machine" is at the default, which is Never for an agent.
- The session `Fix login` runs on this Mac, and its agent is up. The agent's shell tab holds its session token in `HUMATA_SESSION_TOKEN`.
- Nothing is stopped. The Inbox is empty.
- I have a plain terminal on this Mac, outside every Humata session, with no `humata login`.
- A script token `ci` exists. It lists `GlobalRead` and `ChannelRead` only.

## Steps

1. **I do:** the agent runs `humata stop --reason loop` in its session.
   **I see:** the command exits 4 (denied). The JSON error has kind `denied` and a fix. Nothing stops.
   **State and events:** the source is the Never rule for `MachineSessionsStop`. No Inbox request is made.
   **Status:** Works (real daemon): `daemon/humatad/tests/stop_access.rs::an_agents_humata_stop_under_the_default_rule_exits_denied_with_a_fix`. The same file also covers the RPC: `an_agent_under_the_default_rule_is_denied_a_stop_at_once_and_nothing_stops`.

2. **I do:** check the sessions and the Inbox.
   **I see:** `Fix login` still runs. The Inbox is empty.
   **State and events:** the daemon is not stopped, the session status is not MachineStopped, and no pending request exists.
   **Status:** Works (real daemon): `stop_access.rs::an_agents_humata_stop_under_the_default_rule_exits_denied_with_a_fix` (the `assert_nothing_stopped` helper checks all three).

3. **I do:** open Agent access and set "stop or start all sessions on a machine" for `Payments` to Ask me.
   **I see:** the rule shows Ask me for that row.
   **State and events:** the channel's rule for `MachineSessionsStop` is Ask.
   **Status:** Works (real daemon) for the rule itself: `stop_access.rs` sets it through `AccessService.SetChannelRule` in its `set_rule` helper. The app's switch is not covered by this test.

4. **I do:** the agent runs `humata stop --reason loop` again.
   **I see:** the agent's call waits. The Inbox shows one request for this agent to stop the machine. Nothing stops.
   **State and events:** one pending request. The sessions are not stopped while it waits.
   **Status:** Works (real daemon): `stop_access.rs::a_rule_of_ask_puts_an_agents_stop_in_the_inbox_and_stops_nothing_meanwhile`. The test checks the pending count, not the card's text or the exit code. The card's scope (this machine only) is Fakes only: `daemon/access/tests/machine_sessions.rs::an_agent_whose_rule_says_ask_puts_the_stop_in_the_inbox_offering_that_machine_only`.

5. **I do:** answer Allow on that request.
   **I see:** all sessions on this Mac stop, `Fix login` included. The request ends Approved.
   **State and events:** the stop runs with `approved = true`. The stop is recorded with the agent as its stopper and the session as its origin.
   **Status:** Code, no test. The approval path is `daemon/humatad/src/stop/service.rs:99-125`, which hands the Ask to `answer_gated` and re-runs `stop_as` with `approved = true`. No test approves a stop from the Inbox. The generic `answer_gated` has a test for forks only (`daemon/humatad/tests/fork_remote.rs::an_agent_forks_itself_onto_another_machine_once_the_user_approves`).

6. **I do:** on a second request, answer Deny.
   **I see:** nothing stops. The agent's call ends denied.
   **State and events:** no stop is made. The request ends Denied.
   **Status:** Code, no test. Same path as step 5, with `approved = false`.

7. **I do:** set the rule to Allow, and the agent runs `humata stop --reason loop`.
   **I see:** all sessions on this Mac stop, including the agent's own. Its reply may be lost, because its session is one of those stopped.
   **State and events:** the machine's status is stopped. The stop record names the agent as the stopper and its session as the origin.
   **Status:** Works (real daemon): `stop_access.rs::an_agent_whose_rule_allows_it_stops_its_machine_itself_included_and_is_named_as_the_stopper`. The test allows a lost reply.

8. **I do:** the agent runs `humata start` while the sessions are stopped.
   **I see:** the start is refused. The machine stays stopped.
   **State and events:** the refusal is user-only. Nothing starts.
   **Status:** Works (real daemon): `stop_access.rs::an_agent_whose_rule_allows_it_still_cannot_start_the_users_stop`. The exit code 5 for a user-only refusal is Fakes only: `cli/tests/stop.rs::a_start_refused_for_want_of_the_user_exits_needs_user`.

9. **I do:** set the rule back to the default, and the agent runs `humata start` after a stop made by the user.
   **I see:** the start is refused with permission denied.
   **Status:** Works (real daemon): `stop_access.rs::under_the_default_rule_an_agent_cannot_start_a_stopped_machine`.

10. **I do:** in my plain terminal, with no login, run `humata stop --reason "from a terminal"`.
    **I see:** the command exits 0 and names the stop. The sessions stop, and the stop says it was made from a terminal, via the CLI.
    **State and events:** the stop is recorded with kind `Terminal`, via `Cli`, and carries my reason.
    **Status:** Works (real daemon): `stop_access.rs::humata_stop_typed_outside_every_session_with_no_login_stops_the_machine`.

11. **I do:** in the plain terminal, with no login, run `humata start`.
    **I see:** the command exits 5 and prints a fix. The machine stays stopped.
    **State and events:** nothing starts.
    **Status:** Works (real daemon): `stop_access.rs::humata_start_with_no_login_is_refused_and_the_machine_stays_stopped`.

12. **I do:** run `humata login`, then `humata start`.
    **I see:** the sessions start again. The daemon restarts itself and reports the start.
    **State and events:** the stop is cleared. A restart report names the stop.
    **Status:** Works (real daemon) for the user's start, through the RPC: `stop_access.rs::an_agent_whose_rule_allows_it_still_cannot_start_the_users_stop` (its last step). The `humata login` step is **Works** on a real daemon, with the real `humata` in a pty and the test playing the app that confirms it: `daemon/humatad/tests/cli_login.rs::a_confirmed_login_makes_its_terminal_the_user_until_logout`. The CLI's own fake-daemon checks are `cli/tests/login.rs`.

13. **I do:** the agent in a script with token `ci` runs a stop.
    **I see:** the stop is refused with permission denied. Nothing stops.
    **Status:** Works (real daemon): `stop_access.rs::a_script_whose_token_lacks_the_capability_is_refused_a_stop_and_nothing_stops`.

14. **I do:** the agent asks its own MCP server to stop another session.
    **I see:** the call is denied, and the error kind is `denied`.
    **Status:** Works (real daemon): `daemon/humatad/tests/mcp.rs::an_agent_cannot_stop_another_session`.

## Branches

### B1. The agent's stop is pending, and the machine stops for another reason

**When:** an agent's stop request waits in the Inbox, and the user stops the machine directly.
**I do:** stop the machine from the app or the terminal.
**I see:** the agent's request is withdrawn, with the reason "machine stopped".
**Expected:** the request is no longer pending, and the agent does not get a stop it did not ask for.
**Status:** Works (real daemon): `daemon/humatad/tests/stop_sessions.rs::a_stop_withdraws_the_agents_pending_inbox_request_as_stopped_with_its_machine`.

### B2. A stop typed in a session without its token

**When:** I type `humata stop` in an agent's shell tab, with the session token removed from the environment.
**I do:** run the command.
**I see:** the command exits with a non-zero code. Nothing stops.
**Expected:** the stop needs a login or a rule.
**Status:** Works (real daemon): `stop_access.rs::humata_stop_typed_inside_a_session_without_its_token_is_refused`. The test asserts only that the code is not zero. It would still pass if the CLI failed for an unrelated reason, so the exit code 4 is not pinned.

### B3. A stop with an unusable session token

**When:** an agent's command carries a token the daemon never gave out.
**I do:** run `humata stop --reason forged` with `HUMATA_SESSION_TOKEN=not-a-real-token`.
**I see:** the command fails. It is not taken as the plain terminal's stop. Nothing stops.
**Status:** Works (real daemon): `stop_access.rs::a_stop_carrying_an_unusable_session_token_is_refused_not_taken_as_a_terminal`. As in B2, the test asserts only that the code is not zero.

### B4. An Allow stop is asked for by a script whose token lists the capability

**When:** a script token lists `MachineSessionsStop` and calls the stop.
**I do:** run the stop.
**I see:** the stop runs and names the script as its stopper.
**Status:** Fakes only: `daemon/access/tests/machine_sessions.rs::a_script_token_stops_sessions_only_when_it_lists_the_capability` (policy unit test). No daemon test runs a stop as a script with the capability listed.

### B5. The daemon is killed while stopped, and restarts

**When:** the daemon is killed while the machine is stopped, and restarts.
**I do:** restart the daemon.
**I see:** the machine stays stopped, and no agent starts.
**Status:** Works (real daemon): `daemon/humatad/tests/stop_sessions.rs::a_daemon_killed_and_started_again_while_stopped_stays_stopped_and_starts_nothing`.

## Expected outcomes

- [ ] The default rule denies an agent's `humata stop` at once, with exit 4 and a fix. Nothing stops.
- [ ] While the rule is Ask, the agent's stop waits in the Inbox, and nothing stops meanwhile.
- [ ] Allow stops the machine, the agent's own session included, and the stop names the agent.
- [ ] An agent cannot start a stop made by the user.
- [ ] A script whose token lacks the capability is refused, and nothing stops.
- [ ] `humata stop` typed in my plain terminal, with no login, stops the machine and names the terminal.
- [ ] `humata start` with no login is refused (exit 5), and the machine stays stopped.
- [ ] `humata stop` inside a session without its token is refused.
- [ ] An unusable session token is refused, and is not taken as the terminal.
- [ ] An agent cannot stop another session through MCP.
- [ ] Approving the stop in the Inbox stops the machine. (Code, no test.)
- [ ] Denying it stops nothing. (Code, no test.)

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Agent's stop under default rule exits 4 | Works (real daemon) | `daemon/humatad/tests/stop_access.rs::an_agents_humata_stop_under_the_default_rule_exits_denied_with_a_fix` |
| 2. Nothing stopped, no request | Works (real daemon) | `stop_access.rs::an_agents_humata_stop_under_the_default_rule_exits_denied_with_a_fix` |
| 3. Rule set to Ask | Works (real daemon) | `stop_access.rs` (`set_rule` helper); app switch not covered |
| 4. Ask puts it in the Inbox, nothing stops | Works (real daemon) | `stop_access.rs::a_rule_of_ask_puts_an_agents_stop_in_the_inbox_and_stops_nothing_meanwhile`; card scope is Fakes only: `daemon/access/tests/machine_sessions.rs::an_agent_whose_rule_says_ask_puts_the_stop_in_the_inbox_offering_that_machine_only` |
| 5. Approve in the Inbox stops, agent named | Code, no test | `daemon/humatad/src/stop/service.rs:99-125` |
| 6. Deny in the Inbox stops nothing | Code, no test | Same path, `approved = false` |
| 7. Allow stops, agent named as stopper | Works (real daemon) | `stop_access.rs::an_agent_whose_rule_allows_it_stops_its_machine_itself_included_and_is_named_as_the_stopper` |
| 8. Agent cannot start the user's stop | Works (real daemon) | `stop_access.rs::an_agent_whose_rule_allows_it_still_cannot_start_the_users_stop`; exit 5 Fakes only: `cli/tests/stop.rs::a_start_refused_for_want_of_the_user_exits_needs_user` |
| 9. Agent cannot start under default rule | Works (real daemon) | `stop_access.rs::under_the_default_rule_an_agent_cannot_start_a_stopped_machine` |
| 10. Terminal stop, no login, exits 0 | Works (real daemon) | `stop_access.rs::humata_stop_typed_outside_every_session_with_no_login_stops_the_machine` |
| 11. Terminal start, no login, exits 5 | Works (real daemon) | `stop_access.rs::humata_start_with_no_login_is_refused_and_the_machine_stays_stopped` |
| 12. User starts after login | Works (real daemon) via RPC | `stop_access.rs::an_agent_whose_rule_allows_it_still_cannot_start_the_users_stop`; `humata login` is Works (real daemon): `cli_login.rs::a_confirmed_login_makes_its_terminal_the_user_until_logout` |
| 13. Script without capability refused | Works (real daemon) | `stop_access.rs::a_script_whose_token_lacks_the_capability_is_refused_a_stop_and_nothing_stops` |
| 14. MCP stop of another session denied | Works (real daemon) | `daemon/humatad/tests/mcp.rs::an_agent_cannot_stop_another_session` |
| B1. Pending request withdrawn when machine stops | Works (real daemon) | `daemon/humatad/tests/stop_sessions.rs::a_stop_withdraws_the_agents_pending_inbox_request_as_stopped_with_its_machine` |
| B2. Stop in session without token | Works (real daemon) | `stop_access.rs::humata_stop_typed_inside_a_session_without_its_token_is_refused` (weak: asserts non-zero only) |
| B3. Unusable session token | Works (real daemon) | `stop_access.rs::a_stop_carrying_an_unusable_session_token_is_refused_not_taken_as_a_terminal` (weak: asserts non-zero only) |
| B4. Script with the capability stops | Fakes only | `daemon/access/tests/machine_sessions.rs::a_script_token_stops_sessions_only_when_it_lists_the_capability` |
| B5. Killed while stopped stays stopped | Works (real daemon) | `daemon/humatad/tests/stop_sessions.rs::a_daemon_killed_and_started_again_while_stopped_stays_stopped_and_starts_nothing` |

## Gaps and likely bugs

- Test gap: no test approves or denies an agent's stop from the Inbox (steps 5 and 6). The approve path is `daemon/humatad/src/stop/service.rs:99-125`. Its generic helper is tested only for forks.
- Test gap: B2 and B3 assert only that the exit code is not zero. A regression that failed for another reason would still pass. They should pin exit 4 or 5 and the error kind.
- Test gap: no daemon test runs a stop as a script whose token lists the capability (B4). The only check is the policy unit test.
- Test gap: the Inbox card for the agent's stop is not checked for its text or scope in a daemon test.
- No likely bug found in the stop or start paths for this case.

## Corrections to the brief

- `daemon/humatad/tests/stop_access.rs` has 11 tests, not 12.
- The brief's "script token lacking the capability" test is at `stop_access.rs:278`, and it is one of the 11.
- Step 8's exit code 5 for a user-only start is checked against a fake daemon in `cli/tests/stop.rs`, not against the real daemon.
