# A command that needs an answer asks for presence once, waits or exits 12, and a declined change changes nothing

Layer: agents. Platforms: macOS and Linux for the CLI. The app's own prompts are macOS only. Most CLI results below come from fakes; the daemon side of each is named where a real daemon test exists.

## Starting state

- This Mac runs the host daemon. The machine is `this-mac`, and `box-1` is linked and online.
- In the Inbox, the agent in session `fix-retry` has a pending request `rq_1`: an ask-ahead for network reach to `api.example.com:443`, with the reason "fetch the schema".
- The trash on `this-mac` holds files. A plan for emptying it is made on demand.
- I have two terminals. One is a plain terminal with no `humata login`. The other is a plain terminal where I have run `humata login` and approved it in the app.
- The agent has a shell tab in `fix-retry`, and its session token is in that tab's environment.
- The app is open on this Mac.

## Steps

1. **I do:** in the plain terminal with no login, run `humata inbox approve rq_1`.
   **I see:** the command exits 5. It prints that the presence check failed, and it says to run `humata login` in a terminal or to confirm in the Humata app, then run the command again.
   **State and events:** the daemon refuses the answer with the kind `presence_required`. Nothing is answered, and `rq_1` stays pending.
   **Status:** Works (real daemon) for the refusal: `daemon/humatad/tests/auth.rs::the_release_daemon_refuses_a_call_with_no_credential`. The CLI's own exit 5 and message are Fakes only, in `cli/tests/waiting.rs::presence_asked_twice_is_reported`.

2. **I do:** in the plain terminal, run `humata login`. The app shows the request to log in, and I choose Allow.
   **I see:** the login succeeds, and the terminal is my login until `humata logout` or for up to 12 hours.
   **State and events:** a login is bound to this terminal session only. Its token is never printed.
   **Status:** Works (real daemon) for the daemon side: `daemon/humatad/tests/cli_login.rs::a_confirmed_login_makes_its_terminal_the_user_until_logout`. That test answers the login as the app would, through the daemon's CLI-auth service. The app's card for a pending login is Not built (see Gaps).

3. **I do:** run `humata inbox approve rq_1` again in the logged-in terminal.
   **I see:** the command exits 0. The request shows as approved in the Inbox.
   **State and events:** the grant is made, the request moves to Approved, and the agent's wait ends.
   **Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::the_users_reply_ends_the_agents_wait` covers the approval and the end of the agent's wait. The CLI command itself is only fake-tested in `cli/tests/waiting.rs::answering_yes_goes_ahead`.

4. **I do:** the host is away (its link is down), and I log in on `box-1` with `humata login`. The command asks "Approval passphrase:" and I type the passphrase.
   **I see:** the passphrase is not echoed. The login succeeds on `box-1`, and `humata inbox approve` then works there.
   **State and events:** the passphrase is checked on the machine against the hash its policy copy holds. A right passphrase makes a login bound to this terminal session.
   **Status:** Works (real daemon, real shell): `daemon/humatad/tests/away_login.rs::the_approval_passphrase_logs_in_on_a_machine_while_its_host_is_away` runs the real `humata login` in a pty of its own on a machine whose host is away and types the passphrase at its prompt.

5. **I do:** pipe a passphrase into the command: `echo "correct horse" | humata inbox approve rq_1` on `box-1`.
   **I see:** the command exits 5 with the kind `presence_required`. It does not read the pipe.
   **State and events:** no answer is sent with a passphrase from a pipe.
   **Status:** Works (real daemon, real shell): `daemon/humatad/tests/away_login_pipe.rs::the_passphrase_piped_into_login_is_not_read_and_a_typed_one_still_logs_in`. The fake test is `cli/tests/waiting.rs::a_passphrase_is_never_read_from_a_pipe`.

6. **I do:** the agent, from its shell tab in `fix-retry`, runs `humata inbox approve rq_1` with its session token.
   **I see:** the command fails. The agent is told that only the user can answer, and no presence prompt is offered.
   **State and events:** the daemon refuses the answer for an agent with the kind `user_only` (exit 5). Nothing is answered.
   **Status:** Works (real daemon) for the refusal: `daemon/humatad/tests/inbox.rs::an_agent_cannot_answer_its_own_request`. The CLI's rule against offering presence in a session's terminal is Fakes only: `cli/tests/waiting.rs::presence_is_never_offered_from_a_sessions_terminal`.

7. **I do:** run `humata --dry-run --machine this-mac trash empty`.
   **I see:** the command prints the plan: how many items and how many bytes it would free. It prints a plan token. It says nothing was changed.
   **State and events:** only the plan call is made. The empty call is not made, and the trash is unchanged.
   **Status:** Works (real daemon): `daemon/humatad/tests/cli_files.rs::a_dry_run_empties_nothing_and_its_printed_plan_empties_the_trash` runs the real `humata --dry-run trash empty` and the trash keeps its item.

8. **I do:** run `humata --plan <token> --machine this-mac trash empty`, with the token from step 7.
   **I see:** the command empties exactly the items the plan named, and exits 0.
   **State and events:** the empty call carries the plan token. The token works once.
   **Status:** Works (real daemon): `daemon/humatad/tests/cli_files.rs::a_dry_run_empties_nothing_and_its_printed_plan_empties_the_trash` passes the printed token to `--plan`, and the trash is emptied. That the token works once is the daemon's `trash.rs::emptying_frees_what_the_plan_said_and_a_token_works_once`.

9. **I do:** run `humata --machine this-mac trash empty` with no flags, in a pipe, so there is no one to answer "Continue?".
   **I see:** the command refuses. It prints the plan, then says to use `--dry-run` and `--plan`, or `--yes`. It exits 2.
   **State and events:** no empty call is made.
   **Status:** Works (real daemon): `daemon/humatad/tests/cli_files.rs::emptying_the_trash_with_no_one_to_confirm_it_is_refused_and_keeps_it`. The exit 2 is not in the CLI.md §7 table, which lists no case for it.

10. **I do:** run `humata --yes --machine this-mac trash empty`.
    **I see:** the plan is printed, the question is skipped, and the empty runs. It exits 0.
    **State and events:** the empty call carries the plan's token. `--yes` never skips presence.
    **Status:** Works (real daemon): `daemon/humatad/tests/cli_files.rs::trash_empty_frees_what_it_listed` empties the trash with `--yes` and no question. The fake test is `cli/tests/waiting.rs::yes_skips_the_question_and_uses_the_plans_token`.

11. **I do:** run `humata --machine this-mac trash empty` in a terminal, and at "Continue?" I type n.
    **I see:** nothing is emptied. The command prints nothing else.
    **State and events:** no empty call is made. The trash is unchanged.
    **Status:** Fakes only (fixed): `cli/tests/waiting.rs::answering_no_changes_nothing` answers n and the run exits `declined` (not 130, interrupted), says so on stderr and calls nothing. The question is the CLI's own and never reaches the daemon. (Was a likely bug; fixed by BUGS.)

12. **I do:** the agent runs `humata --json access ask-ahead network_reach network:api.example.com:443 --reason "file the follow-ups"`, with no wait.
    **I see:** the command exits 12 with the kind `pending` and the request id `rq_9`. It does not wait.
    **State and events:** the request is held in the Inbox. Its `wait_seconds` is 0.
    **Status:** Works (real daemon): `daemon/humatad/tests/cli_waiting.rs::json_doesnt_wait_and_exits_pending_naming_the_request_in_the_inbox`. The agent's MCP path gives the same pending outcome: `daemon/humatad/tests/mcp.rs::an_approved_request_is_waited_for_with_inbox_wait`.

13. **I do:** the same command with `--wait 30`.
    **I see:** the command holds for up to 30 seconds. If no one answers, it exits 12.
    **State and events:** the daemon is asked to hold the answer for 30 seconds.
    **Status:** Works (real daemon): `daemon/humatad/tests/cli_waiting.rs::a_wait_that_runs_out_leaves_the_request_pending`.

14. **I do:** run the same request in a terminal, with no `--wait`, and I approve it in the app while it waits.
    **I see:** the command keeps waiting, prints "Waiting for an answer (request rq_9). Ctrl-C stops waiting; the request stays.", and exits 0 once I approve.
    **State and events:** one wait stream is held until the answer comes. It is not asked again.
    **Status:** Works (real daemon) for a held wait: `daemon/humatad/tests/cli_waiting.rs::a_wait_ends_with_the_users_yes` waits with `--wait` until the user approves. The terminal's wait with no flag is Fakes only: `cli/tests/waiting.rs::a_terminal_waits_until_the_answer_comes`.

15. **I do:** in the same wait, I press ⌃C.
    **I see:** the command stops waiting and exits 130 with the kind `interrupted`. The request stays pending. `humata inbox show rq_9` still shows it.
    **State and events:** the wait ends. The request is not changed.
    **Status:** Works (real daemon): `daemon/humatad/tests/cli_waiting.rs::ctrl_c_stops_waiting_and_the_request_stays` sends SIGINT to the real `humata` while it waits; it exits 130 and the request stays pending.

16. **I do:** I deny `rq_9` in the app, and the waiting command's terminal is still open.
    **I see:** the command exits 4 with the kind `denied`.
    **State and events:** the request is denied. The wait ends with the denial.
    **Status:** Works (real daemon): `daemon/humatad/tests/cli_waiting.rs::a_wait_ends_denied_with_the_users_no`.

## Branches

### Presence is asked a second time after the first answer

- **When:** the daemon asks for presence again on the retry, because the first proof is not accepted.
- **I do:** run `humata --json inbox approve rq_1` with the proof the daemon asked for.
- **I see:** the command exits 5 with `presence_required`. It does not loop.
- **Expected outcome:** the CLI sends one retry at most, and reports the second refusal.
- **Status:** Fakes only. `cli/tests/waiting.rs::presence_asked_twice_is_reported`.

### A typed passphrase is sent, but the daemon never checks it on an answer

- **When:** the host is away and the answer is refused for want of a passphrase. The CLI prompts, and the passphrase goes in the answer's `presence` field.
- **I do:** type the passphrase at the prompt.
- **I see:** the answer is refused again with `presence_required`, and the CLI exits 5.
- **Expected outcome:** a right passphrase is checked and the answer goes through, or the prompt is not offered.
- **Status:** Fakes only (fixed): `cli/tests/waiting.rs::an_answer_refused_for_a_passphrase_is_not_retried_with_one`. An answer refused for a passphrase is not retried with one; the CLI says to log in with it first (SEC-13). A passphrase is typed only for a login (`cli/tests/waiting.rs::a_passphrase_is_typed_at_the_terminal_for_a_login`). (Was a likely bug; fixed by BUGS.)

### The agent waits with `inbox_wait` after an approved request

- **When:** the agent asked, the request is pending, and the user approves it.
- **I do:** the agent calls the MCP tool `inbox_wait` with the request id.
- **I see:** the wait ends with the approved result. The agent does not have to ask again.
- **Expected outcome:** the agent gets the answer and the grant it made.
- **Status:** Works (real daemon). `daemon/humatad/tests/mcp.rs::an_approved_request_is_waited_for_with_inbox_wait`. The MCP tool is served by the server's wait path (`daemon/mcp/src/server.rs:393`), not by `fn call`.

### There is no CLI command `humata inbox wait`

- **When:** an agent or a script wants to wait for a request by id, with no command that made it.
- **I do:** run `humata inbox wait <request>`.
- **I see:** the CLI rejects the command as unknown.
- **Expected outcome:** the wait is done by the `--wait` flag on the command that made the request, or by the MCP tool `inbox_wait`.
- **Status:** Not built (no milestone). `cli/src/args.rs` `InboxCommand` has Show, Approve, Deny, Reply, ApproveAll and DenyAll, and no Wait. `api/CLI.md` §5.4 does not list it.

### An agent answers its own request through MCP

- **When:** the agent calls the MCP tool `inbox_answer`.
- **I do:** the agent calls the tool by name.
- **I see:** the call is refused as user only, before it reaches the daemon.
- **Expected outcome:** the agent can't answer any request, its own included.
- **Status:** Fakes only. The refusal is in the MCP crate (`daemon/mcp/src/user_only.rs`), and `daemon/mcp/tests/refusals.rs:16` lists `inbox_answer`. The daemon-level refusal is Works (real daemon): `daemon/humatad/tests/inbox.rs::an_agent_cannot_answer_its_own_request`.

### A passphrase login approves on a machine while the host is away

- **When:** the host is away and the machine's policy holds the request.
- **I do:** log in on the machine with the passphrase, then run `humata inbox approve`.
- **I see:** the approval is journaled on the machine. Its answer comes after the request in the journal.
- **Expected outcome:** the approval is kept on the machine and replayed when the host returns.
- **Status:** Works (real daemon, real shell). `daemon/humatad/tests/away_boundary.rs::row_7_while_away_only_answering_with_the_passphrase_and_reading_the_copy_are_open_to_the_user` runs `"$HUMATA" --json inbox approve` in a real shell after the passphrase login.

### Five wrong passphrases lock the prompt

- **When:** the typed passphrase is wrong five times in a row.
- **I do:** type a wrong passphrase at the prompt five times.
- **I see:** the prompt is locked for 15 minutes. The command says so and names when it ends.
- **Expected outcome:** the lock holds across a restart.
- **Status:** Works (real daemon). `daemon/humatad/tests/away_login.rs::five_wrong_tries_lock_the_passphrase_and_a_restart_keeps_the_lock`.

### The wait ends without an answer

- **When:** the wait stream ends before any answer: the daemon restarts, or the link drops.
- **I do:** run a command that waits, and restart the host while it waits.
- **I see:** the command exits 70 with the kind `internal`. Its message says that humata and humatad may be different versions and to update Humata.
- **Expected outcome:** the command says the wait ended, that the request stays pending, and how to wait again.
- **Status:** Works (real daemon): `daemon/humatad/tests/cli_waiting.rs::a_wait_that_runs_out_leaves_the_request_pending` ends a wait with no answer as pending, naming the request. The fake test is `cli/tests/waiting.rs::a_wait_that_ends_unanswered_is_pending_not_a_version_mismatch`. (Was a likely bug; fixed by BUGS.)

### A presence prompt is refused from a session's terminal

- **When:** a command that needs presence runs in a session's shell, where anything typed may come from the agent.
- **I do:** run it from the session's shell.
- **I see:** the CLI does not ask for a passphrase or a confirmation. It exits 5 with the kind `presence_required`, and it says to run the command from a terminal outside Humata's sessions.
- **Expected outcome:** nothing is changed. The refusal is the CLI's, and no prompt is shown.
- **Status:** Works (real daemon) for the refusal: `daemon/humatad/tests/cli_waiting.rs::an_agent_cannot_approve_its_own_request_from_the_cli`, where the agent's `humata inbox approve` fails and the request stays. That no prompt is offered is Fakes only: `cli/tests/waiting.rs::presence_is_never_offered_from_a_sessions_terminal`.

### The app's "Allow or Don't Allow" card for a command

- **When:** the command needs presence, the host is reachable, and I am at the Mac.
- **I do:** run `humata inbox approve rq_1` from a plain terminal.
- **I see (expected, CLI.md §3):** the app shows "`humata inbox approve` in Terminal wants to approve…" with Allow and Don't Allow, and the command waits for the answer.
- **I see (code):** the daemon refuses with `presence_required` (step 1). It never sends the ConfirmInApp method for an answer, and the app has no client for it.
- **Expected outcome:** the card appears, and the command is sent again on Allow.
- **Status:** Not built (no milestone). The only ConfirmInApp path is for login (`daemon/humatad/src/cli_login.rs`), and no app crate calls the pending-login service.

## Expected outcomes

- [ ] A command refused for want of presence exits 5 with `presence_required` and says how to fix it.
- [x] A passphrase is read only from a terminal that can read it, never from a pipe (`daemon/humatad/tests/away_login_pipe.rs::the_passphrase_piped_into_login_is_not_read_and_a_typed_one_still_logs_in`).
- [ ] A session's terminal is never offered a presence prompt.
- [x] `--dry-run` prints a plan and a token, and changes nothing (`daemon/humatad/tests/cli_files.rs::a_dry_run_empties_nothing_and_its_printed_plan_empties_the_trash`).
- [ ] `--plan <token>` runs exactly what the printed plan named, and the token works once.
- [x] A destructive command with no one to confirm refuses before it changes anything (`daemon/humatad/tests/cli_files.rs::emptying_the_trash_with_no_one_to_confirm_it_is_refused_and_keeps_it`).
- [ ] `--yes` skips "Continue?" and never skips presence.
- [ ] Declining "Continue?" changes nothing.
- [x] `--json` without `--wait` does not wait, and exits 12 with the request id (`daemon/humatad/tests/cli_waiting.rs::json_doesnt_wait_and_exits_pending_naming_the_request_in_the_inbox`).
- [x] `--wait N` holds the answer for N seconds, then exits 12 if none came (`daemon/humatad/tests/cli_waiting.rs::a_wait_that_runs_out_leaves_the_request_pending`).
- [ ] A terminal wait holds one stream until the answer comes.
- [ ] A wait that ends without an answer says so and says how to wait again.
- [x] A denied request exits 4 with the kind `denied` (`daemon/humatad/tests/cli_waiting.rs::a_wait_ends_denied_with_the_users_no`).
- [x] ⌃C stops waiting with exit 130, and the request stays pending (`daemon/humatad/tests/cli_waiting.rs::ctrl_c_stops_waiting_and_the_request_stays`).
- [x] An agent cannot answer any request, its own included (`daemon/humatad/tests/cli_waiting.rs::an_agent_cannot_approve_its_own_request_from_the_cli`; `daemon/humatad/tests/inbox.rs::an_agent_cannot_answer_its_own_request`).

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Answer refused for want of presence | Works (real daemon) | `auth.rs::the_release_daemon_refuses_a_call_with_no_credential`; CLI exit is Fakes only |
| 2. `humata login` approved in the app | Works (real daemon) | `cli_login.rs::a_confirmed_login_makes_its_terminal_the_user_until_logout`; app card Not built |
| 3. Approve after login | Works (real daemon) | `inbox.rs::the_users_reply_ends_the_agents_wait` |
| 4. Typed passphrase while away | Works (real daemon, real shell) | `daemon/humatad/tests/away_login.rs::the_approval_passphrase_logs_in_on_a_machine_while_its_host_is_away` |
| 5. Passphrase from a pipe refused | Works (real daemon, real shell) | `daemon/humatad/tests/away_login_pipe.rs::the_passphrase_piped_into_login_is_not_read_and_a_typed_one_still_logs_in` |
| 6. Agent cannot answer, no presence offered | Works (real daemon) | `inbox.rs::an_agent_cannot_answer_its_own_request` |
| 7. Dry run changes nothing | Works (real daemon) | `daemon/humatad/tests/cli_files.rs::a_dry_run_empties_nothing_and_its_printed_plan_empties_the_trash` |
| 8. `--plan` does the printed plan | Works (real daemon) | `daemon/humatad/tests/cli_files.rs::a_dry_run_empties_nothing_and_its_printed_plan_empties_the_trash`; token once: `trash.rs::emptying_frees_what_the_plan_said_and_a_token_works_once` |
| 9. No one to confirm, refused | Works (real daemon) | `daemon/humatad/tests/cli_files.rs::emptying_the_trash_with_no_one_to_confirm_it_is_refused_and_keeps_it` |
| 10. `--yes` skips the question | Works (real daemon) | `daemon/humatad/tests/cli_files.rs::trash_empty_frees_what_it_listed` |
| 11. Declining changes nothing | Fakes only (fixed) | `cli/tests/waiting.rs::answering_no_changes_nothing` |
| 12. `--json` exits 12 with the request | Works (real daemon) | `daemon/humatad/tests/cli_waiting.rs::json_doesnt_wait_and_exits_pending_naming_the_request_in_the_inbox` |
| 13. `--wait N` holds N seconds | Works (real daemon) | `daemon/humatad/tests/cli_waiting.rs::a_wait_that_runs_out_leaves_the_request_pending` |
| 14. Terminal waits for the answer | Works (real daemon) with `--wait`; no-flag terminal wait Fakes only | `daemon/humatad/tests/cli_waiting.rs::a_wait_ends_with_the_users_yes`; `waiting.rs::a_terminal_waits_until_the_answer_comes` |
| 15. ⌃C exits 130 | Works (real daemon) | `daemon/humatad/tests/cli_waiting.rs::ctrl_c_stops_waiting_and_the_request_stays` |
| 16. Denied exits 4 | Works (real daemon) | `daemon/humatad/tests/cli_waiting.rs::a_wait_ends_denied_with_the_users_no` |
| Branch: presence asked twice | Fakes only | `waiting.rs::presence_asked_twice_is_reported` |
| Branch: typed passphrase on an answer | Fakes only (fixed) | `cli/tests/waiting.rs::an_answer_refused_for_a_passphrase_is_not_retried_with_one`; `cli/tests/waiting.rs::a_passphrase_is_typed_at_the_terminal_for_a_login` |
| Branch: agent's `inbox_wait` | Works (real daemon) | `mcp.rs::an_approved_request_is_waited_for_with_inbox_wait` |
| Branch: `humata inbox wait` | Not built (no milestone) | `args.rs` `InboxCommand` has no Wait |
| Branch: agent's MCP `inbox_answer` | Fakes only (refusal); daemon Works | `daemon/mcp/tests/refusals.rs:16`; `inbox.rs::an_agent_cannot_answer_its_own_request` |
| Branch: passphrase login while away | Works (real daemon, real shell) | `away_boundary.rs::row_7_while_away_only_answering_with_the_passphrase_and_reading_the_copy_are_open_to_the_user` |
| Branch: five wrong passphrases | Works (real daemon) | `away_login.rs::five_wrong_tries_lock_the_passphrase_and_a_restart_keeps_the_lock` |
| Branch: wait ends unanswered | Works (real daemon) | `daemon/humatad/tests/cli_waiting.rs::a_wait_that_runs_out_leaves_the_request_pending`; `cli/tests/waiting.rs::a_wait_that_ends_unanswered_is_pending_not_a_version_mismatch` |
| Branch: session's terminal, no presence | Works (real daemon) for the refusal; no prompt Fakes only | `daemon/humatad/tests/cli_waiting.rs::an_agent_cannot_approve_its_own_request_from_the_cli`; `waiting.rs::presence_is_never_offered_from_a_sessions_terminal` |
| Branch: app's Allow or Don't Allow card | Not built (no milestone) | `cli_login.rs` only; no app client |

## Gaps and likely bugs

- **Not built:** the app's "Allow or Don't Allow" card for a command that needs presence (CLI.md §3). The daemon never sends `ConfirmInApp` for an answer, and no app crate calls the pending-login service. No milestone found.
- **Not built:** `humata inbox wait`. The wait is the `--wait` flag, or the MCP tool `inbox_wait`.
- **Fixed (was likely bug 1):** a declined "Continue?" exits `declined`, not interrupted (`cli/tests/waiting.rs::answering_no_changes_nothing`).
- **Fixed (was likely bug 2):** the CLI asks for the passphrase only at `humata login` (`cli/tests/waiting.rs::an_answer_refused_for_a_passphrase_is_not_retried_with_one`).
- **Fixed (was likely bug 3):** a wait that ends unanswered exits `pending` with the request (`cli/tests/waiting.rs::a_wait_that_ends_unanswered_is_pending_not_a_version_mismatch`).
- **Test gap:** every CLI result in this case is a fake. The real terminal signal for ⌃C is not tested by any real-process test.
- **Test gap:** the first step's "presence asked in the app, then the call is sent again" has no real test. The `waiting.rs` test that names it uses a fake that refuses with `ConfirmInApp`, which the daemon does not send for an answer.
