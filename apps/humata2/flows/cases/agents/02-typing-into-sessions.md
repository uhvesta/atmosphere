# An agent's typing goes where it may, and only the user opens shells

Layer: agents. Platforms: macOS (this Mac), Linux (`box-1`).

## Starting state

- Humata 2 runs on this Mac. The channel `acme/api` has two sessions: `Fix login`, which runs an agent, and `Review docs`, which has a shell tab.
- The shell of `Review docs` is at a prompt. Nothing is typed into it yet.
- The agent of `Fix login` holds its session token and calls Humata through MCP (`session_type`, `inbox_wait`).
- The rule for typing into another session is the default, which is Ask me (`daemon/access/src/capability.rs:225`).
- The Inbox is empty. The app is focused on the `Fix login` session.

## Steps

1. **I do:** I call the MCP tool `session_type` with `session` set to `Fix login` (my own session) and `text` set to `printf ok > typed.txt` followed by Enter.

   **I see:** The result has `typed` set to true and `queued_id` set to null. `typed.txt` appears in the session's folder with `ok` in it.

   **State and events:** The call is judged as my own session, so no Inbox request is raised (`daemon/access/src/policy.rs:823-824`). The text reaches the shell as raw bytes. The tool's description and `api/MCP.md:251` say it is one bracketed paste. The code does not do that (see Gaps).

   **Status:** Works (real daemon). `daemon/humatad/tests/mcp.rs::an_agent_types_into_its_own_session_through_mcp`.

2. **I do:** I call `session_type` with `session` set to `Review docs` and `text` set to `echo hello` followed by Enter.

   **I see:** The call does not return text. The reply is pending with a request id. I call `inbox_wait` with that id, and the call waits. The Inbox shows a card that names `Review docs` and the text. The card offers Allow and Deny. For this kind of action it also offers Always allow for that session (`policy.rs:823-834`, `need_exactly`). The person presses Allow. My wait ends with `typed` set to true. `hello` appears in the shell of `Review docs`.

   **State and events:** One Inbox request is created, with the session as the target. Typing waits until it is approved. After approval the text is typed once.

   **Status:** Works (real daemon). `daemon/humatad/tests/terminal.rs::an_agents_typing_into_another_session_that_a_rule_sends_to_ask_runs_once_approved` sets the rule to Ask, approves the request, and sees `hello` in the other tab. The test drives the gRPC call directly; the MCP pending reply for `session_type` is not tested on its own.

3. **I do:** I try to open a shell tab in my session, or to paste an image into it. There is no MCP tool for either, so I call the daemon's `OpenShellTab` or `PasteImage` with my session token.

   **I see:** Refused as the user's alone. The error is `user_only` with exit code 5. The message says the user must do it, not an agent.

   **State and events:** `TerminalInput` is a user-only capability (`daemon/access/src/capability.rs:241`). No Inbox request is raised, because an agent cannot ask for this.

   **Status:** Works (real daemon). `daemon/humatad/tests/mcp.rs::opening_a_shell_or_pasting_is_the_users_alone`. The test uses the gRPC calls, not the CLI. The agent's `humata shell <machine>` path is not tested.

4. **I do:** The shell in `Review docs` has run `exit 3`. I call `session_type` with `session` set to `Review docs` and the text `echo hi`. The person has already allowed typing into that session, so no new request is raised.

   **I see:** An error of kind `not_found`. The message says the tab's shell exited before the text reached it. The fix says to open a new tab in the session and type again. Nothing is typed.

   **State and events:** The tab's writable state is Exited. Nothing reaches the holder.

   **Status:** Works (real daemon). `daemon/humatad/tests/terminal.rs::typing_into_a_shell_that_exited_is_refused`. The test sends the same `Type` call as the user, with no token. The agent path runs the same code after the access gate, so this is strong evidence but not an agent-token run.

## Branches

### The person denies the request

**When:** The person presses Deny on the card from step 2, with a note.

**I do:** I wait on `inbox_wait`.

**I see:** My call ends with error kind `denied`. The error carries the person's note, and the fix says not to ask again for the same thing. Nothing is typed into `Review docs`.

**Expected:** Nothing is typed. I am told the person's note.

**Status:** Code, no test. The denial path is shared: `daemon/humatad/src/inbox.rs` (`settled`, around line 1724) builds the denied error. `daemon/humatad/tests/mcp.rs::an_agent_asks_ahead_for_access_and_gets_the_grant_or_the_users_no` shows the `denied` kind through the MCP wait, but for `inbox_request`. No test denies a typing request.

### The approval arrives after I stopped waiting

**When:** I cancel my MCP call while the card from step 2 is still pending.

**I do:** Send `notifications/cancelled` for that call.

**I see:** The card leaves the Inbox as withdrawn, with the reason that the agent stopped waiting. Allow, if pressed afterwards, is refused as withdrawn.

**Expected:** The request is withdrawn, and nothing is typed after the cancel.

**Status:** Code, no test. The cancel path is `daemon/mcp/src/server.rs` (`cancel`, lines 276-296, then `withdraw_later`). `daemon/humatad/tests/mcp.rs::a_question_the_agent_stops_waiting_on_is_withdrawn` proves it for `inbox_ask`. `daemon/humatad/tests/inbox.rs::a_withdrawn_request_cannot_be_answered` proves the refusal of a late answer. Neither drives a typing request.

### The same keyed input is sent twice

**When:** I send the same `session_type` request twice on one MCP connection, with the same request id.

**I do:** Send the request, then send it again with the same id.

**I see:** The second reply matches the first. The text is typed once.

**Expected:** The text is typed once. A repeat gets the first answer.

**Status:** Code, no test. The key is the same on both sends (`daemon/mcp/tests/waiting.rs::a_client_retry_of_the_same_request_gets_the_same_inbox_request`, a fake daemon). The daemon keeps the result in memory (`daemon/humatad/src/terminal.rs:694-697`, `Remembered` at 1258), but no test sends the same `Type` key twice. The holder's keyed input, `daemon/holder/tests/keyed_input.rs::a_keyed_input_types_once_and_a_repeat_types_nothing`, is used for review comment batches only (`terminal.rs:241`), not for `session_type`.

### A restarted MCP server sends the same request again

**When:** The MCP server restarts and the agent resends a request with the same id.

**I do:** Resend the request after a restart of my MCP server.

**I see:** The text is typed a second time, as a new intent.

**Expected:** Per D29, a restarted server makes new keys, so a new intent is typed.

**Status:** Fakes only. `daemon/mcp/tests/waiting.rs::a_restarted_server_makes_new_keys_for_the_same_request_ids`. This is by design, so `session_type` is not safe to resend after an MCP restart. Not proven live.

### I ask for `when` set to `turn_end`

**When:** I pass `when` set to `turn_end`, which the tool schema offers (`daemon/mcp/src/tools.rs:532-534`).

**I do:** Call `session_type` with `when` set to `turn_end`.

**I see:** An error of kind `peer_version_unsupported`. The fix says "update Humata on this machine; until then, do this from the app".

**Expected:** The text is not typed now, and I am told the mode is not available. The fix should not suggest an update that cannot bring the mode.

**Status:** Not built, no milestone. `daemon/humatad/src/terminal.rs:911-914` returns `not_built`. `api/MCP.md:251` documents the `turn_end` mode, and `SPEC.md` SE-23 calls the wait mode optional (D126). The message's fix text is in `daemon/humatad/src/status.rs:131-135`.

### Always allow for another session's typing

**When:** On the card from step 2, the person ticks Always allow and presses Allow.

**I do:** Call `session_type` to `Review docs` again, then to a third session.

**I see:** The second call to `Review docs` is typed at once, with no card. The call to the third session raises a new card.

**Expected:** The grant covers only `Review docs` (IN-22: "Note, question or typing into another session: that session").

**Status:** Code, no test. The offer is `need_exactly` in `daemon/access/src/policy.rs:823-834`, and the grant is made at `daemon/humatad/src/inbox.rs:517-535`. No test types under a grant.

## Expected outcomes

- [ ] My own session accepts `session_type` at once, and no Inbox request is raised.
- [ ] Typing into another session waits for the person, and the text is typed once after Allow.
- [ ] Opening a shell or pasting an image is refused to an agent, as the user's alone.
- [ ] Typing into a shell that has exited is refused, and nothing is typed.
- [ ] A denied request types nothing, and the agent is told why.
- [ ] A request whose wait was cancelled is withdrawn, and a late Allow is refused.
- [ ] A repeat with the same key types the text once.
- [ ] `when` set to `turn_end` is refused as not built.
- [ ] Always allow for one session covers that session only.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Own session, typed at once | Works (real daemon) | `daemon/humatad/tests/mcp.rs::an_agent_types_into_its_own_session_through_mcp` |
| 2. Other session waits for Allow | Works (real daemon) | `daemon/humatad/tests/terminal.rs::an_agents_typing_into_another_session_that_a_rule_sends_to_ask_runs_once_approved` |
| 3. Open a shell or paste is refused | Works (real daemon) | `daemon/humatad/tests/mcp.rs::opening_a_shell_or_pasting_is_the_users_alone` |
| 4. Typing into an exited shell is refused | Works (real daemon) | `daemon/humatad/tests/terminal.rs::typing_into_a_shell_that_exited_is_refused` |
| Branch: person denies | Code, no test | `daemon/humatad/src/inbox.rs` (settled) |
| Branch: approval after withdrawal | Code, no test | `daemon/humatad/tests/mcp.rs::a_question_the_agent_stops_waiting_on_is_withdrawn` (inbox_ask only) |
| Branch: same keyed input twice | Code, no test | `daemon/mcp/tests/waiting.rs::a_client_retry_of_the_same_request_gets_the_same_inbox_request` (fake) |
| Branch: MCP restart resends | Fakes only | `daemon/mcp/tests/waiting.rs::a_restarted_server_makes_new_keys_for_the_same_request_ids` |
| Branch: `when` set to `turn_end` | Not built (no milestone) | `daemon/humatad/src/terminal.rs:911-914` |
| Branch: Always allow for a session | Code, no test | `daemon/access/src/policy.rs:823-834` |

## Gaps and likely bugs

- Not built: queued typing at turn end (`when` set to `turn_end`). `daemon/humatad/src/terminal.rs:911-914`, `api/MCP.md:251`, SPEC SE-23 (D126). No milestone.
- Likely bug: the tool promises a bracketed paste, but the text is typed raw. `daemon/mcp/src/tools.rs:1080` says "as one bracketed paste", and `api/MCP.md:251` says the same. `SPEC.md` SE-23 says whole messages use serialized bracketed paste. The code types raw bytes: `daemon/humatad/src/terminal.rs:920` passes `request.text.as_bytes()` to `type_into`, and the comment at `terminal.rs:1158-1159` says "raw, with no bracketed paste around it". The test at step 1 depends on the raw form, since its text ends in Enter. To confirm: read the bytes the holder writes for a `session_type` text that contains a newline, and look for `ESC [ 200 ~`.
- Likely bug: the idempotency memory is lost on a daemon restart. `daemon/humatad/src/terminal.rs:694-697` (`recall`) reads an in-memory map, and `Remembered` (line 1258) is not saved. `api/GRPC.md:486` says operations are idempotent by key. To confirm: send a `Type` with key K, restart the daemon, and send `Type` with K again. The text is typed twice.
- Likely bug: the `turn_end` refusal tells the person to update Humata, which cannot bring a mode that no release has. `daemon/humatad/src/status.rs:131-135`.
- Code, no test: the denial, the withdrawal, the repeat and the Always grant for typing (branches above).
