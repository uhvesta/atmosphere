# An ask-ahead grant covers only what was approved, and revoking it makes the next ask wait

Layer: agents. Platforms: macOS, Linux.

## Starting state

- Humata 2 runs on this Mac. The channel `acme/api` has two sessions: `Fix login`, which runs my agent, and `Review docs`, which has a `docs/` folder of specs.
- I am the agent of `Fix login`. I hold its session token and call Humata through MCP.
- Reading another session's files is an Ask-me capability by default (`daemon/access/src/capability.rs:210`).
- I have no grants. The Inbox is empty. The person is at the app, focused on `Fix login`.

## Steps

1. **I do:** Before a long job, I call the MCP tool `inbox_request` with `capability` set to `files_read_other_session`, `target` set to `{session: Review docs, repo: acme/docs, path: docs}`, and a reason that says what the job needs.

   **I see:** My call waits. The Inbox shows a card with the kind of action, the target and the reason, and the choices Allow, Always allow for that folder, and Deny.

   **State and events:** One ask-ahead request is pending. Nothing is granted yet (IN-26).

   **Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::a_folder_grant_covers_what_is_under_it_and_not_its_parent` asks ahead for a folder and answers the card with Allow. The card text is not asserted.

2. **I do:** The person presses Allow.

   **I see:** My call returns `decision` set to `approved`, with a `grant_id`.

   **State and events:** One grant is made for my session and the folder (IN-14, IN-22). Allow on an ask-ahead makes the same grant as Always allow.

   **Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::an_approved_ask_ahead_covers_the_same_ask_and_nothing_wider` checks the grant's requester, target and the request that made it. The MCP form, with a network target, is `daemon/humatad/tests/mcp.rs::an_agent_asks_ahead_for_access_and_gets_the_grant_or_the_users_no`.

3. **I do:** I ask ahead again for `docs/specs`, which is under the granted folder.

   **I see:** `decision` is `approved` at once, with no card.

   **State and events:** The grant covers the folder and everything under it.

   **Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::a_folder_grant_covers_what_is_under_it_and_not_its_parent`.

4. **I do:** I read `docs/specs/a.md` with `file_read`.

   **I see:** The file's text, with no card.

   **State and events:** The read is checked against the grant's folder in the access engine (`need_files`, `daemon/access/src/policy.rs:611-625`).

   **Status:** Fakes only. `daemon/access/tests/grants.rs::always_let_covers_the_folder_asked_for_and_nothing_beside_it` checks the folder rule in the engine. No test reads a file over MCP with a grant in place.

5. **I do:** I ask ahead for the whole `Review docs` folder (`path` set to empty).

   **I see:** My call waits, and a new card is raised.

   **State and events:** The grant covered `docs`, not its parent.

   **Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::a_folder_grant_covers_what_is_under_it_and_not_its_parent` checks that the parent is pending.

6. **I do:** I write `docs/plan.md` in my own clone with `file_write`.

   **I see:** The write is applied, with no card.

   **State and events:** A write into my own clone is judged as owned (`daemon/access/src/policy.rs:725-727`). No grant and no ask-ahead apply.

   **Status:** Works (real daemon). `daemon/humatad/tests/matrix.rs::the_matrix_holds_for_an_agent_over_mcp` runs `file_write` into the agent's own clone and expects `applied`.

7. **I do:** The person revokes the grant, from the app or with `humata access revoke <grant>`. Then I try the same revoke myself.

   **I see:** The grant is gone from the list. My own revoke is refused as user-only.

   **State and events:** Revoking is user-only.

   **Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::a_revoked_grant_makes_the_next_ask_wait_for_the_user`.

8. **I do:** I ask ahead for `docs/` again.

   **I see:** My call waits, and a new card is raised. The old grant does not apply.

   **Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::a_revoked_grant_makes_the_next_ask_wait_for_the_user`.

9. **I do:** I watch the feed, as the person and as my agent's session.

   **I see:** Each grant and each revoke appears in the feed, for the access topic and for the agent.

   **Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::the_feed_tells_of_the_grant_an_approval_makes_and_its_revoking`.

10. **I do:** I call `humata access grants`.

    **I see:** Only the grants made for my session.

    **Status:** Works (real daemon, gRPC). `daemon/humatad/tests/inbox.rs::an_agent_lists_only_its_own_sessions_grants` calls `ListGrants` with my token. The CLI command itself is not run in a test.

11. **I do:** The daemon restarts while the grant stands.

    **I see:** The grant still covers the same ask after the restart.

    **Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::a_restart_keeps_the_inbox_and_the_grants_still_standing`.

## Branches

### Approve all never approves an ask-ahead, and a deferred call approved once makes no grant

**When:** The person uses Answer all, with approve, on a list with an ask-ahead in it. Or I make a deferred call that the person approves once.

**I see:** The ask-ahead stays pending and is listed as skipped. For the deferred call, the call runs, and my next one asks again. No grant is listed in either case.

**Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::approving_all_never_approves_an_ask_ahead` and `daemon/humatad/tests/inbox.rs::approving_a_deferred_call_once_makes_no_grant_so_the_next_call_asks_again`.

### Always allow with a target the card did not offer

**When:** The person, or a client, sends Always allow with a target other than the one offered.

**I see:** Refused, with `invalid_field` on `answer.always.target`. The request stays pending.

**Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::always_let_grants_only_the_target_the_card_offered`.

### Revoking a grant that is not there

**I see:** Not found, for a grant.

**Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::revoking_a_grant_that_isnt_there_is_not_found`.

### A grant's scope in the engine: one session, forks, moves, session end, Never

**When:** A fork of `Fix login` asks for the same folder. Or the session moves, or ends. Or the channel's rule for file reads is set to Never.

**I see:** A fork gets no grant. A moved session keeps its grants. Grants end with their session. A Never rule suspends grants until it changes back.

**Status:** Fakes only. `daemon/access/tests/grants.rs::a_grant_belongs_to_one_session_and_forks_get_none`, `daemon/access/tests/grants.rs::a_moved_session_keeps_its_grants`, `daemon/access/tests/grants.rs::grants_end_with_their_session` and `daemon/access/tests/grants.rs::a_never_rule_suspends_grants_until_it_changes_back`. These are pure engine tests, not a live daemon.

### A write into another session's clone

**When:** I write a file into `Review docs`'s clone with `file_write`.

**I see:** Refused, with the refusal kind NotItsFolder. No card is raised, because no write grant exists to ask for.

**Expected:** Refused at once. IN-22 has no write row.

**Status:** Code, no test. `daemon/access/src/policy.rs:728-729` returns `NotItsFolder`. No test covers this refusal.

### A reply to an access ask

**When:** An answer with text is sent for an ask-ahead request.

**I see:** Refused, with `invalid_field`. The fix says to allow it, allow it with Always allow, or deny it.

**Status:** Code, no test. `daemon/humatad/src/inbox.rs:509-513` returns the refusal. No test sends a reply to an access request.

### The Always tick and the Reply editor in the app

**When:** The person ticks "Allow this for the rest of the session" on a card, or presses Reply.

**I see:** Allow sends an Always answer for the offered target. Reply opens a box that says "Write a reply…", but the box is static text: nothing can be typed, so Send reply denies with an empty reply.

**Expected:** The grant is the one the card offered (IN-14).

**Status:** Likely bug for the Reply editor: `ui/src/request_card.rs:339-350` draws the draft as plain text, and nothing in `ui/src` sends `InboxAction::ReplyText`. The tick is tested at the reducer by `features/inbox/tests/answers.rs::a_ticked_allow_sends_always_with_the_rule`. The Reply button is wired at `ui/src/request_card.rs:420` and `features/inbox/src/lib.rs:148-154`. No reducer test sends `StartReply` or `SendReply`.

## Expected outcomes

- [ ] An ask-ahead waits for the person, and Allow makes a grant with its id.
- [ ] A grant covers the folder asked for and everything under it, and nothing above it or beside it.
- [ ] Writes into my own clone need no ask.
- [ ] A revoke makes the next ask wait. The agent cannot revoke.
- [ ] Approve all never approves an ask-ahead, and an Always answer for a target not offered is refused.
- [ ] The feed tells of each grant and each revoke. An agent lists only its own grants.
- [ ] Grants survive a daemon restart.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Ask ahead waits | Works (real daemon) | `daemon/humatad/tests/inbox.rs::a_folder_grant_covers_what_is_under_it_and_not_its_parent` |
| 2. Allow makes a grant | Works (real daemon) | `daemon/humatad/tests/inbox.rs::an_approved_ask_ahead_covers_the_same_ask_and_nothing_wider` |
| 3. Grant covers the folder below | Works (real daemon) | `daemon/humatad/tests/inbox.rs::a_folder_grant_covers_what_is_under_it_and_not_its_parent` |
| 4. Read under the grant | Fakes only | `daemon/access/tests/grants.rs::always_let_covers_the_folder_asked_for_and_nothing_beside_it` |
| 5. Parent asks again | Works (real daemon) | `daemon/humatad/tests/inbox.rs::a_folder_grant_covers_what_is_under_it_and_not_its_parent` |
| 6. Write into own clone | Works (real daemon) | `daemon/humatad/tests/matrix.rs::the_matrix_holds_for_an_agent_over_mcp` |
| 7. Revoke, agent refused | Works (real daemon) | `daemon/humatad/tests/inbox.rs::a_revoked_grant_makes_the_next_ask_wait_for_the_user` |
| 8. Next ask waits | Works (real daemon) | `daemon/humatad/tests/inbox.rs::a_revoked_grant_makes_the_next_ask_wait_for_the_user` |
| 9. Feed tells of grant and revoke | Works (real daemon) | `daemon/humatad/tests/inbox.rs::the_feed_tells_of_the_grant_an_approval_makes_and_its_revoking` |
| 10. Agent lists its own grants | Works (real daemon, gRPC) | `daemon/humatad/tests/inbox.rs::an_agent_lists_only_its_own_sessions_grants` |
| 11. Restart keeps grants | Works (real daemon) | `daemon/humatad/tests/inbox.rs::a_restart_keeps_the_inbox_and_the_grants_still_standing` |
| Branch: Approve all, deferred call once | Works (real daemon) | `daemon/humatad/tests/inbox.rs::approving_all_never_approves_an_ask_ahead` |
| Branch: Always with a target not offered | Works (real daemon) | `daemon/humatad/tests/inbox.rs::always_let_grants_only_the_target_the_card_offered` |
| Branch: revoke a missing grant | Works (real daemon) | `daemon/humatad/tests/inbox.rs::revoking_a_grant_that_isnt_there_is_not_found` |
| Branch: engine scope (forks, moves, end, Never) | Fakes only | `daemon/access/tests/grants.rs::grants_end_with_their_session` |
| Branch: write into another clone | Code, no test | `daemon/access/src/policy.rs:728-729` |
| Branch: reply to an access ask | Code, no test | `daemon/humatad/src/inbox.rs:509-513` |
| Branch: Always tick and Reply editor | Likely bug (Reply box takes no typing) | `ui/src/request_card.rs:420`, `features/inbox/src/lib.rs:148-154` |

## Gaps and likely bugs

- Not built, no milestone: an ask-ahead for a write into a folder. IN-22 has no write row, and `daemon/access/src/policy.rs:728-729` refuses another session's writes outright. A write ask-ahead was the first idea for this case, so steps 4 and 6 use a read and an own-clone write instead.
- Code, no test: the write refusal into another clone and the reply to an access ask.
- Likely bug: the Reply box takes no typing. `ui/src/request_card.rs:339-350` draws the draft as text, and `ReplyText` (`features/inbox/src/lib.rs:150`) has no sender in `ui/src`. Confirm by pressing Reply… and typing.
- Documentation, not behaviour: `OVERVIEW.md:458` says session-only grants and the Reply editor are disabled. The code wires the Reply button and sends `StartReply` and `SendReply` (`features/inbox/src/lib.rs:148-154`). The Always tick is labelled for the rest of the session (`ui/src/request_card.rs:151`), and the grant is made for the requesting session (IN-14, step 2). The Reply button is wired, but the box cannot be typed into, so the OVERVIEW entry is half right.
- No MCP tool lists grants or revokes. Agents use `humata access grants` (CLI.md). Revoke stays out of agent reach by design (`api/MCP.md:337`).
