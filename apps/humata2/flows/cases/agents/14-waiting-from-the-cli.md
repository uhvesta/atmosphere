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
   **Status:** Fakes only for the typed-at-a-terminal prompt: `cli/tests/waiting.rs::a_passphrase_is_typed_at_the_terminal`. The daemon side is Works (real daemon): `daemon/humatad/tests/away_login.rs::the_approval_passphrase_logs_in_on_a_machine_while_its_host_is_away`.

5. **I do:** pipe a passphrase into the command: `echo "correct horse" | humata inbox approve rq_1` on `box-1`.
   **I see:** the command exits 5 with the kind `presence_required`. It does not read the pipe.
   **State and events:** no answer is sent with a passphrase from a pipe.
   **Status:** Fakes only. `cli/tests/waiting.rs::a_passphrase_is_never_read_from_a_pipe`.

6. **I do:** the agent, from its shell tab in `fix-retry`, runs `humata inbox approve rq_1` with its session token.
   **I see:** the command fails. The agent is told that only the user can answer, and no presence prompt is offered.
   **State and events:** the daemon refuses the answer for an agent with the kind `user_only` (exit 5). Nothing is answered.
   **Status:** Works (real daemon) for the refusal: `daemon/humatad/tests/inbox.rs::an_agent_cannot_answer_its_own_request`. The CLI's rule against offering presence in a session's terminal is Fakes only: `cli/tests/waiting.rs::presence_is_never_offered_from_a_sessions_terminal`.

7. **I do:** run `humata --dry-run --machine this-mac trash empty`.
   **I see:** the command prints the plan: how many items and how many bytes it would free. It prints a plan token. It says nothing was changed.
   **State and events:** only the plan call is made. The empty call is not made, and the trash is unchanged.
   **Status:** Fakes only for the CLI flags: `cli/tests/waiting.rs::dry_run_prints_the_plan_and_changes_nothing`. The daemon's plan and its token are Works (real daemon): `daemon/humatad/tests/trash.rs::emptying_frees_what_the_plan_said_and_a_token_works_once`.

8. **I do:** run `humata --plan <token> --machine this-mac trash empty`, with the token from step 7.
   **I see:** the command empties exactly the items the plan named, and exits 0.
   **State and events:** the empty call carries the plan token. The token works once.
   **Status:** Fakes only for the CLI: `cli/tests/waiting.rs::plan_does_what_the_printed_plan_said`. The daemon side is Works (real daemon): `daemon/humatad/tests/trash.rs::emptying_removes_only_what_was_planned`.

9. **I do:** run `humata --machine this-mac trash empty` with no flags, in a pipe, so there is no one to answer "Continue?".
   **I see:** the command refuses. It prints the plan, then says to use `--dry-run` and `--plan`, or `--yes`. It exits 2.
   **State and events:** no empty call is made.
   **Status:** Fakes only. `cli/tests/waiting.rs::a_change_with_no_one_to_confirm_it_is_refused`. The exit 2 is not in the CLI.md §7 table, which lists no case for it.

10. **I do:** run `humata --yes --machine this-mac trash empty`.
    **I see:** the plan is printed, the question is skipped, and the empty runs. It exits 0.
    **State and events:** the empty call carries the plan's token. `--yes` never skips presence.
    **Status:** Fakes only. `cli/tests/waiting.rs::yes_skips_the_question_and_uses_the_plans_token`.

11. **I do:** run `humata --machine this-mac trash empty` in a terminal, and at "Continue?" I type n.
    **I see:** nothing is emptied. The command prints nothing else.
    **State and events:** no empty call is made. The trash is unchanged.
    **Status:** Likely bug. The test `cli/tests/waiting.rs::answering_no_changes_nothing` passes only because it expects exit 130; see Likely bug 1. The "nothing changed" part is Fakes only.

12. **I do:** the agent runs `humata --json access ask-ahead network_reach network:api.example.com:443 --reason "file the follow-ups"`, with no wait.
    **I see:** the command exits 12 with the kind `pending` and the request id `rq_9`. It does not wait.
    **State and events:** the request is held in the Inbox. Its `wait_seconds` is 0.
    **Status:** Fakes only for the CLI: `cli/tests/waiting.rs::json_doesnt_wait_and_exits_12_with_the_request`. The agent's MCP path gives the same pending outcome: `daemon/humatad/tests/mcp.rs::an_approved_request_is_waited_for_with_inbox_wait` (first call).

13. **I do:** the same command with `--wait 30`.
    **I see:** the command holds for up to 30 seconds. If no one answers, it exits 12.
    **State and events:** the daemon is asked to hold the answer for 30 seconds.
    **Status:** Fakes only. `cli/tests/waiting.rs::wait_asks_the_daemon_to_hold_the_answer_that_long`.

14. **I do:** run the same request in a terminal, with no `--wait`, and I approve it in the app while it waits.
    **I see:** the command keeps waiting, prints "Waiting for an answer (request rq_9). Ctrl-C stops waiting; the request stays.", and exits 0 once I approve.
    **State and events:** one wait stream is held until the answer comes. It is not asked again.
    **Status:** Fakes only for the CLI: `cli/tests/waiting.rs::a_terminal_waits_until_the_answer_comes`. The daemon's side of the same wait is Works (real daemon): `daemon/humatad/tests/inbox.rs::the_users_reply_ends_the_agents_wait`.

15. **I do:** in the same wait, I press ⌃C.
    **I see:** the command stops waiting and exits 130 with the kind `interrupted`. The request stays pending. `humata inbox show rq_9` still shows it.
    **State and events:** the wait ends. The request is not changed.
    **Status:** Fakes only. `cli/tests/waiting.rs::ctrl_c_stops_waiting_with_130` and `cli/tests/exit_codes.rs::ctrl_c_while_waiting_exits_130` both call the command with an interrupt that is already fired. The real terminal signal is not tested.

16. **I do:** I deny `rq_9` in the app, and the waiting command's terminal is still open.
    **I see:** the command exits 4 with the kind `denied`.
    **State and events:** the request is denied. The wait ends with the denial.
    **Status:** Fakes only. `cli/tests/waiting.rs::a_denied_request_is_an_error_with_its_kind`.

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
- **Status:** Likely bug 2. See Gaps.

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
- **Status:** Likely bug 3. The test `cli/tests/waiting.rs::a_wait_that_ends_unanswered_is_an_error` only checks that the exit is not 0.

### A presence prompt is refused from a session's terminal

- **When:** a command that needs presence runs in a session's shell, where anything typed may come from the agent.
- **I do:** run it from the session's shell.
- **I see:** the CLI does not ask for a passphrase or a confirmation. It exits 5 with the kind `presence_required`, and it says to run the command from a terminal outside Humata's sessions.
- **Expected outcome:** nothing is changed. The refusal is the CLI's, and no prompt is shown.
- **Status:** Fakes only for the CLI: `cli/tests/waiting.rs::presence_is_never_offered_from_a_sessions_terminal`. The daemon never offers a presence prompt to an agent at all (`daemon/access/src/verdict.rs`, the agent's principal), and `daemon/humatad/tests/inbox.rs::an_agent_cannot_answer_its_own_request` covers that.

### The app's "Allow or Don't Allow" card for a command

- **When:** the command needs presence, the host is reachable, and I am at the Mac.
- **I do:** run `humata inbox approve rq_1` from a plain terminal.
- **I see (expected, CLI.md §3):** the app shows "`humata inbox approve` in Terminal wants to approve…" with Allow and Don't Allow, and the command waits for the answer.
- **I see (code):** the daemon refuses with `presence_required` (step 1). It never sends the ConfirmInApp method for an answer, and the app has no client for it.
- **Expected outcome:** the card appears, and the command is sent again on Allow.
- **Status:** Not built (no milestone). The only ConfirmInApp path is for login (`daemon/humatad/src/cli_login.rs`), and no app crate calls the pending-login service.

## Expected outcomes

- [ ] A command refused for want of presence exits 5 with `presence_required` and says how to fix it.
- [ ] A passphrase is read only from a terminal that can read it, never from a pipe.
- [ ] A session's terminal is never offered a presence prompt.
- [ ] `--dry-run` prints a plan and a token, and changes nothing.
- [ ] `--plan <token>` runs exactly what the printed plan named, and the token works once.
- [ ] A destructive command with no one to confirm refuses before it changes anything.
- [ ] `--yes` skips "Continue?" and never skips presence.
- [ ] Declining "Continue?" changes nothing.
- [ ] `--json` without `--wait` does not wait, and exits 12 with the request id.
- [ ] `--wait N` holds the answer for N seconds, then exits 12 if none came.
- [ ] A terminal wait holds one stream until the answer comes.
- [ ] A wait that ends without an answer says so and says how to wait again.
- [ ] A denied request exits 4 with the kind `denied`.
- [ ] ⌃C stops waiting with exit 130, and the request stays pending.
- [ ] An agent cannot answer any request, its own included.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Answer refused for want of presence | Works (real daemon) | `auth.rs::the_release_daemon_refuses_a_call_with_no_credential`; CLI exit is Fakes only |
| 2. `humata login` approved in the app | Works (real daemon) | `cli_login.rs::a_confirmed_login_makes_its_terminal_the_user_until_logout`; app card Not built |
| 3. Approve after login | Works (real daemon) | `inbox.rs::the_users_reply_ends_the_agents_wait` |
| 4. Typed passphrase while away | Fakes only | `waiting.rs::a_passphrase_is_typed_at_the_terminal`; daemon Works in `away_login.rs::the_approval_passphrase_logs_in_on_a_machine_while_its_host_is_away` |
| 5. Passphrase from a pipe refused | Fakes only | `waiting.rs::a_passphrase_is_never_read_from_a_pipe` |
| 6. Agent cannot answer, no presence offered | Works (real daemon) | `inbox.rs::an_agent_cannot_answer_its_own_request` |
| 7. Dry run changes nothing | Fakes only | `waiting.rs::dry_run_prints_the_plan_and_changes_nothing`; daemon `trash.rs::emptying_frees_what_the_plan_said_and_a_token_works_once` |
| 8. `--plan` does the printed plan | Fakes only | `waiting.rs::plan_does_what_the_printed_plan_said`; daemon `trash.rs::emptying_removes_only_what_was_planned` |
| 9. No one to confirm, refused | Fakes only | `waiting.rs::a_change_with_no_one_to_confirm_it_is_refused` |
| 10. `--yes` skips the question | Fakes only | `waiting.rs::yes_skips_the_question_and_uses_the_plans_token` |
| 11. Declining changes nothing | Likely bug | `waiting.rs::answering_no_changes_nothing` expects exit 130 |
| 12. `--json` exits 12 with the request | Fakes only | `waiting.rs::json_doesnt_wait_and_exits_12_with_the_request`; MCP real in `mcp.rs::an_approved_request_is_waited_for_with_inbox_wait` |
| 13. `--wait N` holds N seconds | Fakes only | `waiting.rs::wait_asks_the_daemon_to_hold_the_answer_that_long` |
| 14. Terminal waits for the answer | Fakes only | `waiting.rs::a_terminal_waits_until_the_answer_comes`; daemon `inbox.rs::the_users_reply_ends_the_agents_wait` |
| 15. ⌃C exits 130 | Fakes only | `waiting.rs::ctrl_c_stops_waiting_with_130`; `exit_codes.rs::ctrl_c_while_waiting_exits_130` |
| 16. Denied exits 4 | Fakes only | `waiting.rs::a_denied_request_is_an_error_with_its_kind` |
| Branch: presence asked twice | Fakes only | `waiting.rs::presence_asked_twice_is_reported` |
| Branch: typed passphrase on an answer | Likely bug | `call.rs:232-238`; `inbox.rs:807` ignores `presence` |
| Branch: agent's `inbox_wait` | Works (real daemon) | `mcp.rs::an_approved_request_is_waited_for_with_inbox_wait` |
| Branch: `humata inbox wait` | Not built (no milestone) | `args.rs` `InboxCommand` has no Wait |
| Branch: agent's MCP `inbox_answer` | Fakes only (refusal); daemon Works | `daemon/mcp/tests/refusals.rs:16`; `inbox.rs::an_agent_cannot_answer_its_own_request` |
| Branch: passphrase login while away | Works (real daemon, real shell) | `away_boundary.rs::row_7_while_away_only_answering_with_the_passphrase_and_reading_the_copy_are_open_to_the_user` |
| Branch: five wrong passphrases | Works (real daemon) | `away_login.rs::five_wrong_tries_lock_the_passphrase_and_a_restart_keeps_the_lock` |
| Branch: wait ends unanswered | Likely bug | `call.rs:164`, `error.rs:292`; test checks only "not OK" |
| Branch: session's terminal, no presence | Fakes only | `waiting.rs::presence_is_never_offered_from_a_sessions_terminal` |
| Branch: app's Allow or Don't Allow card | Not built (no milestone) | `cli_login.rs` only; no app client |

## Gaps and likely bugs

- **Not built:** the app's "Allow or Don't Allow" card for a command that needs presence (CLI.md §3). The daemon never sends `ConfirmInApp` for an answer, and no app crate calls the pending-login service. No milestone found.
- **Not built:** `humata inbox wait`. The wait is the `--wait` flag, or the MCP tool `inbox_wait`.
- **Likely bug 1:** a declined "Continue?" exits 130 with the kind `interrupted` and an empty message. `cli/src/error.rs:151` maps `NotConfirmed` to `plain("interrupted", exit::INTERRUPTED, String::new())`. CLI.md §7 lists 130 as "Interrupted" only, so a script reads a refusal as a Ctrl-C. To confirm: run a destructive command with `--machine` in a terminal and answer n. Expect exit 130 and no message.
- **Likely bug 2:** the CLI types a passphrase for an answer, and the daemon never reads it. `daemon/humatad/src/inbox.rs:807-822` (`answer_request`) does not look at `request.presence`. Only `daemon/humatad/src/cli_login.rs:381` reads it, for login. `DECISIONS.md` D48 item 6 says the answer's presence fields are ignored for now. So the typed passphrase is sent, not checked, and refused again. The daemon asks for a passphrase only from a terminal whose login was confirmed in the app, while the host is away (`daemon/access/src/policy.rs`, `user_only_while_away`). To confirm: while the host is away, from such a terminal, run `humata inbox approve` and type the passphrase. Expect a second `presence_required`, even though the passphrase is right.
- **Likely bug 3:** a wait that ends without an answer maps to `DaemonError::Ended`. `cli/src/error.rs:292` turns that into exit 70 `internal` with "humata and humatad may be different versions". That message is wrong here. The request is still pending. To confirm: restart the host during `humata --wait 60 ...`. The test `cli/tests/waiting.rs::a_wait_that_ends_unanswered_is_an_error` should check the exit and the kind, not only "not OK".
- **Test gap:** every CLI result in this case is a fake. The real terminal signal for ⌃C is not tested by any real-process test.
- **Test gap:** the first step's "presence asked in the app, then the call is sent again" has no real test. The `waiting.rs` test that names it uses a fake that refuses with `ConfirmInApp`, which the daemon does not send for an answer.
