# Reading other channels follows the rule the user sets: Allow lists at once, Ask waits in the Inbox, Never is refused

Layer: agents. Platforms: macOS, Linux, web (the Inbox card).

## Starting state

- This Mac runs humatad2. This Mac is the only machine.
- Channel `payments` holds `acme/api`. A session "Fix login" runs there, and its agent is mine.
- Channel `search` holds a session "Search spike". I am not in it.
- The app is open on `payments`, on the Agent access tab.
- Reading other channels (the global read rule, `global_read`) is at its default, Allow (D127.4).
- My agent has its session token in its shell, so every `humata` call it makes is for its own session.
- The Inbox is empty.

## Steps

1. **I do:** the agent runs `humata --json channels` with the rule at its default.
   **I see:** the list of channels comes back at once. Each channel shows its name, folder, repos, machines and live session count.
   **State and events:** no Inbox request is made. The CLI exits 0.
   **Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::an_agent_lists_channels_at_once_while_the_rule_is_left_at_its_default`. The test calls ListChannels over gRPC with the agent's token. Nothing in the tests runs the MCP `channels_list` at the default rule.

2. **I do:** in the app, I open `payments`, pick Agent access, and set "Reading other channels" to Ask me.
   **I see:** the row reads Ask me. After the app reloads from the daemon, it still reads Ask me.
   **State and events:** the daemon stores `global_read` on `payments` as Ask.
   **Status:** Works (real daemon). `desktop/tests/live_inbox.rs::the_agent_access_tab_sets_the_channels_rule_on_the_daemon`.

3. **I do:** the agent runs `humata --json channels`.
   **I see:** no list. The CLI prints a pending error with the request id, and it returns at once. It does not hang.
   **State and events:** one Inbox request is open for the agent's session, for `global_read`. The call has no result yet. The CLI exits 12. Over MCP the same call returns outcome `pending` with a request id.
   **Status:** Works (real daemon). `daemon/humatad/tests/matrix.rs::a_cli_call_that_asks_ends_pending_and_never_hangs_without_wait` checks exit 12 and the request id. `daemon/humatad/tests/mcp.rs::an_agent_lists_the_channels_once_the_user_approves` checks the pending outcome over MCP.

4. **I do:** I look at the Inbox.
   **I see:** the request appears in the global Inbox and on its card. The card names the requester's session and channel, gives the title, and names the rule that sent the call to me (Reading other channels, Ask me). It shows a preview. Allow and Deny are offered. Always is not, because this capability has no target.
   **State and events:** the request is Pending, with a preview and no Always option.
   **Status:** Works (real daemon). `desktop/tests/live_inbox.rs::a_request_allowed_from_the_requesters_session_returns_the_agents_waiting_call` counts the request, names the rule and opens the card. The preview and the missing Always are checked by `daemon/humatad/tests/inbox.rs::a_deferred_card_has_a_preview_and_offers_no_always_allow_without_a_target`.

5. **I do:** I press Allow once.
   **I see:** the card leaves the Inbox. The agent's call returns the channel list. A second wait on the same request returns the same result.
   **State and events:** the request is Approved, with result type `humata.v1.ListChannelsResponse`, and the result is kept. No grant is made, so the next `humata channels` asks again.
   **Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::an_approved_deferred_call_runs_and_its_result_is_waited_for` and `daemon/humatad/tests/inbox.rs::approving_a_deferred_call_once_makes_no_grant_so_the_next_call_asks_again`.

6. **I do:** I set "Reading other channels" to Never. The agent runs `humata --json channels` again.
   **I see:** the call is refused at once with a denied error. The error names the rule and says I can change it in Agent access. No card appears.
   **State and events:** no Inbox request. The CLI exits 4.
   **Status:** Code, no test. The check is in `daemon/humatad/src/channel_service.rs:1018-1027`, which asks the guard for `GlobalRead` on `Target::Global`. The Never decision comes from `daemon/access/src/policy.rs:807`. No test sets `GlobalRead` to Never. The nearest tests use other capabilities: `daemon/humatad/tests/matrix.rs::the_matrix_holds_for_an_agent_over_the_cli` (its Never row runs `session stop`) and `daemon/humatad/tests/access.rs::explain_follows_the_rule_as_it_stands` (Git push, Never).

7. **I do:** with the rule at Ask or Never, I (the user, from my own shell) run `humata channels`.
   **I see:** the list comes back with no question.
   **State and events:** the user's call is not checked against the agent rules.
   **Status:** Code, no test for Ask or Never. The user path at the default rule is tested by `daemon/humatad/tests/inbox.rs::the_user_lists_channels_without_asking`.

8. **I do:** with the rule at Ask, the agent in `search` runs `humata session read <a session in payments>`.
   **I see:** the call is refused at once, not held. The message says an agent can search and read only its own channel's sessions, and it names `payments` as another channel.
   **State and events:** no Inbox request. The CLI exits 4.
   **Status:** Not built. Asking for another channel is a seam, not a feature. The code says so at `daemon/humatad/src/history_service.rs:331-339` (SEAM SESSIONS_SEARCH_OTHER). `apps/humata2/DECISIONS.md:1354` says that until that capability lands, asking for another channel is refused, not Asked. The refusal itself is tested by `daemon/humatad/tests/history_tools.rs::an_agent_reads_its_channels_sessions_and_is_refused_another_channels`. The access policy already asks for these reads (`daemon/access/tests/refusals.rs::another_channel_is_read_with_global_read_and_otherwise_refused`), but no surface calls the policy for them.

9. **I do:** the agent runs `humata session read <a session in its own channel>`. The rule setting does not matter for its own channel.
   **I see:** the turns come back in order. If there are more, a page token comes with them.
   **State and events:** no Inbox request. The read is limited by channel only (`daemon/humatad/src/history_service.rs:478-491`). It does not call the rule guard.
   **Status:** Works (real daemon). `daemon/humatad/tests/history_tools.rs::a_read_gives_a_sessions_turns_in_order` and `daemon/humatad/tests/history_tools.rs::an_agent_reads_its_channels_sessions_and_is_refused_another_channels` (the sibling read returns two turns). Independence from the rule setting is seen in the code only.

10. **I do:** the agent asks for another session's turns through the ListTurns RPC, with the rule at Ask.
    **I see:** the request is held in the Inbox, and the turns come back once I allow it.
    **State and events:** an Inbox request for `ChannelRead` is open, and the turns come back after Allow.
    **Status:** Not built for agents. The Ask path works over gRPC (`daemon/humatad/tests/inbox.rs::an_agent_listing_another_sessions_turns_waits_in_the_inbox_when_the_rule_asks`). But no MCP tool and no CLI command calls ListTurns. The app client uses it for the fork dialog (`clients/live/src/daemon.rs:533`). No milestone names an agent turns tool.

11. **I do:** the agent runs `humata conversation <session>`.
    **I see:** an error that says the ReadConversation call "isn't built into this humatad2 yet". The fix line says to update Humata.
    **State and events:** nothing is read.
    **Status:** Not built, with no milestone. `daemon/humatad/src/session_service.rs:2446-2455` returns not_built. `cli/src/commands/sessions.rs:50-66` calls it, and `cli/src/args.rs:110` defines the command. `api/CLI.md:117` marks the row "stale". The working way to read a conversation is step 9, `humata session read`.

12. **I do:** the agent runs `humata access why <its session> global-read`, or calls the MCP tool `access_explain`.
    **I see:** the answer says what the rule decides for that session and capability, and where it comes from (the rule, or a default). It also says whether the Mac is away.
    **State and events:** nothing changes.
    **Status:** Code, no test for the MCP tool (`daemon/humatad/src/mcp.rs:962-983`) or the CLI (`cli/src/commands/inbox.rs:300-310`). The daemon's explain is tested over gRPC with Git push: `daemon/humatad/tests/access.rs::explain_follows_the_rule_as_it_stands`.

13. **I do:** the agent runs `humata access set global-read allow` with its own token.
    **I see:** refused. The error says that only the user can change a rule.
    **State and events:** the rule stays as it was.
    **Status:** Works (real daemon). `daemon/humatad/tests/access.rs::an_agent_cannot_change_rules` (PermissionDenied with the user-only kind; the rule is unchanged).

## Branches

### The user denies the deferred call

- **When:** after step 4, I press Deny instead of Allow.
- **I do:** Deny on the card.
- **I see:** the agent's call ends with a denied error. No channel list comes back.
- **Expected outcome:** the request is Denied, the result is empty, and the list never runs.
- **Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::a_denied_deferred_call_never_runs`.

### The daemon restarts while the call waits

- **When:** after step 3, the request is still Pending and the daemon restarts before I answer.
- **I do:** restart the daemon, or restart the Mac.
- **I see:** the card is gone from the Inbox. The request shows as withdrawn, with the reason "call ended". An ask-ahead for network that I made at the same time still waits.
- **Expected outcome:** the deferred request is Withdrawn, the list never runs, and the unrelated ask-ahead stays Pending.
- **Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::after_a_restart_a_pending_deferred_request_is_withdrawn_and_an_ask_ahead_still_waits`.

## Expected outcomes

- [ ] With the rule at its default, `humata channels` returns the list at once and makes no Inbox request.
- [ ] With the rule at Ask, the call returns pending with a request id, the CLI exits 12, and nothing runs yet.
- [ ] The card names the requester, the channel, the rule that asked and a preview. It offers Allow and Deny, and not Always.
- [ ] Allow returns the channel list to the waiting call and makes no grant.
- [ ] Deny ends the call as denied, and the list never runs.
- [ ] A restart withdraws the pending deferred request and leaves unrelated asks pending.
- [ ] With the rule at Never, the agent's call is refused at once and the error names the rule.
- [ ] An agent cannot change a rule.
- [ ] An agent reading a session in another channel is refused at once, not asked, until the seam lands.
- [ ] An agent reads turns of a session in its own channel, in order.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Default rule: list channels at once | Works (real daemon) | `daemon/humatad/tests/inbox.rs::an_agent_lists_channels_at_once_while_the_rule_is_left_at_its_default` |
| 2. Set the rule to Ask in Agent access | Works (real daemon) | `desktop/tests/live_inbox.rs::the_agent_access_tab_sets_the_channels_rule_on_the_daemon` |
| 3. Ask: the call is pending, exit 12 | Works (real daemon) | `daemon/humatad/tests/matrix.rs::a_cli_call_that_asks_ends_pending_and_never_hangs_without_wait`; `daemon/humatad/tests/mcp.rs::an_agent_lists_the_channels_once_the_user_approves` |
| 4. The card with a preview, no Always | Works (real daemon) | `desktop/tests/live_inbox.rs::a_request_allowed_from_the_requesters_session_returns_the_agents_waiting_call`; `daemon/humatad/tests/inbox.rs::a_deferred_card_has_a_preview_and_offers_no_always_allow_without_a_target` |
| 5. Allow: the result is waited for, no grant | Works (real daemon) | `daemon/humatad/tests/inbox.rs::an_approved_deferred_call_runs_and_its_result_is_waited_for`; `...::approving_a_deferred_call_once_makes_no_grant_so_the_next_call_asks_again` |
| 6. Never: refused at once | Code, no test | `daemon/humatad/src/channel_service.rs:1018-1027`; `daemon/access/src/policy.rs:807` |
| 7. User lists under Ask or Never | Code, no test | Default-rule case only: `daemon/humatad/tests/inbox.rs::the_user_lists_channels_without_asking` |
| 8. Agent reads another channel's session under Ask | Not built | `daemon/humatad/src/history_service.rs:331-339`; `apps/humata2/DECISIONS.md:1354` |
| 9. Agent reads its own channel's session | Works (real daemon) | `daemon/humatad/tests/history_tools.rs::a_read_gives_a_sessions_turns_in_order` |
| 10. Agent lists turns under Ask (ListTurns) | Not built | No MCP tool or CLI command; gRPC only: `daemon/humatad/tests/inbox.rs::an_agent_listing_another_sessions_turns_waits_in_the_inbox_when_the_rule_asks` |
| 11. `humata conversation` | Not built | `daemon/humatad/src/session_service.rs:2446-2455`; `api/CLI.md:117` |
| 12. Explain what the rule decides | Code, no test | `daemon/humatad/src/mcp.rs:962-983`; gRPC only: `daemon/humatad/tests/access.rs::explain_follows_the_rule_as_it_stands` |
| 13. Agent cannot change a rule | Works (real daemon) | `daemon/humatad/tests/access.rs::an_agent_cannot_change_rules` |
| Branch: user denies | Works (real daemon) | `daemon/humatad/tests/inbox.rs::a_denied_deferred_call_never_runs` |
| Branch: restart while waiting | Works (real daemon) | `daemon/humatad/tests/inbox.rs::after_a_restart_a_pending_deferred_request_is_withdrawn_and_an_ask_ahead_still_waits` |

## Gaps and likely bugs

Not built:

- Agent reads of another channel's session under Ask (step 8). Only a refusal exists. The seam is `SESSIONS_SEARCH_OTHER` (`apps/humata2/DECISIONS.md:1354`, `daemon/humatad/src/history_service.rs:331-339`). Milestone: FIND (T4b names the seam).
- No agent tool or CLI command lists another session's turns through ListTurns (step 10). No milestone.
- `humata conversation` calls ReadConversation, which is not built (step 11, `daemon/humatad/src/session_service.rs:2446-2455`). No milestone. `api/CLI.md:117` says "stale".

Likely bugs:

- None found in this case.
