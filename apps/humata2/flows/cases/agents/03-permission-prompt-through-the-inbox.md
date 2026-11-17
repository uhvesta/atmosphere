# A permission prompt waits in the Inbox, and the answer reaches the hook

Layer: agents. Platforms: macOS, Linux.

## Starting state

- Humata 2 runs on this Mac. The channel `acme/api` has a session, `Fix login`, whose agent tab runs Claude Code.
- At daemon start the daemon wrote its hook settings into its own state folder. The Claude `PermissionRequest` group holds a blocking hook with a timeout of 3600 seconds, not async. Codex and Copilot have their hooks too.
- Permission asks wait in the Inbox. This is the default (Settings › Agents, D199).
- The Inbox is empty. The app is focused on `Fix login`.
- The agent's shell is in the session folder. Nothing is held yet.

## Steps

1. **I do:** Claude wants to run a shell command. I do not type anything. Claude runs its `PermissionRequest` hook, which is `humata hook claude permission`.

   **I see:** Claude waits at its prompt. The hook holds, so Claude shows no prompt of its own.

   **State and events:** The hook sends the payload to the daemon. The daemon raises one card and keeps the hold open. It sends a keepalive every 15 seconds (`daemon/humatad/src/session_service/hooks.rs:30`). The hook is written with a timeout of 3600 seconds (`daemon/agent_hooks/src/lib.rs:290-296`).

   **Status:** Works (real daemon). `daemon/humatad/tests/hooks.rs::the_daemon_writes_a_blocking_claude_hook_that_holds_for_an_hour` checks the timeout and that the hook is not async.

2. **I do:** I look at the Inbox.

   **I see:** A card that says the agent wants to run Bash, with the whole command shown, and a description. The card offers Allow and Deny.

   **State and events:** One Inbox request, of kind session, with the command as its preview. It is keyed by the session and the ask, so the same ask makes the same card.

   **Status:** Works (real daemon). `daemon/humatad/tests/hooks.rs::an_inbox_approval_reaches_the_waiting_hook` checks the kind, the command and a non-empty action title. The test uses a shell in place of Claude, with the real `humata` binary.

3. **I do:** I press Allow.

   **I see:** The hook prints `hookSpecificOutput` with `decision.behavior` set to `allow`, and exits 0. Claude runs the command.

   **State and events:** The card leaves the Inbox as approved. The hook's output is the Claude shape from `render_permission` (`daemon/agent_hooks/src/lib.rs:219-229`).

   **Status:** Works (real daemon). `daemon/humatad/tests/hooks.rs::an_inbox_approval_reaches_the_waiting_hook`. The macOS headless test `desktop/tests/live_inbox.rs::an_agents_permission_ask_waits_in_the_inbox_and_allow_reaches_its_hook` also runs the app with a shell agent. Neither runs real Claude.

4. **I do:** On a second Claude ask, I press Deny and type the note "run it against staging".

   **I see:** The hook prints `decision.behavior` set to `deny`, with the note in `message`. Claude shows the note to the model.

   **State and events:** The card leaves the Inbox as denied, and the note is kept on the request.

   **Status:** Works (real daemon). `daemon/humatad/tests/hooks.rs::an_inbox_deny_reaches_the_hook_with_its_note`.

5. **I do:** A Copilot agent asks to run `rm -rf build`. I press Allow.

   **I see:** The hook prints the Copilot shape, `{"behavior":"allow"}`, which is not wrapped in `hookSpecificOutput`.

   **State and events:** One card, with the command `rm -rf build`.

   **Status:** Works (real daemon). `daemon/humatad/tests/hooks.rs::each_agent_gets_the_answer_in_its_own_shape`. Only the Copilot allow shape is checked there. The Copilot deny shape is covered by a pure test, `daemon/agent_hooks/tests/payloads.rs::copilot_reads_a_flat_behavior` (Fakes only).

6. **I do:** A Codex agent asks to run `cargo publish --dry-run`. I press Deny with a note.

   **I see:** The hook prints the Claude-style shape, with `decision.behavior` set to `deny` and the note in `message`.

   **State and events:** The card leaves the Inbox as denied.

   **Status:** Fakes only. `daemon/agent_hooks/tests/payloads.rs::claude_and_codex_read_allow_and_deny_from_hook_specific_output` checks the shape. No daemon test holds a Codex hook. Codex also needs its hooks trusted once (see the branch "Codex hooks not trusted yet").

7. **I do:** On a Claude ask, I look for Always allow.

   **I see:** There is no Always allow on a permission card. The card shows only Allow and Deny.

   **State and events:** If an Always answer is sent anyway, the daemon refuses it with `invalid_field` (`daemon/humatad/src/inbox/permission.rs:233-245`). The fix says to change the rule in the agent's own settings. The code comment in `daemon/agent_hooks/src/lib.rs:238` says Always stays deferred.

   **Status:** Not built, no milestone. The refusal in `permission.rs` has no test. No test sends Always to a permission card.

8. **I do:** Two Claude hooks hold the same ask, one after the other. I press Allow once.

   **I see:** One card. Both hooks print Allow and exit 0.

   **State and events:** The second hold joins the first card (`raise_permission` in `daemon/humatad/src/inbox/permission.rs:154-190`).

   **Status:** Works (real daemon). `daemon/humatad/tests/hooks.rs::the_same_ask_held_twice_is_one_card_and_one_answer_reaches_both`.

## Branches

### The hook stops holding

**When:** The agent's own timeout kills the hook, or the person answers in the agent's own prompt.

**I do:** Close the hook's stream while its card is pending.

**I see:** The card is withdrawn with reason `call_ended`. It can no longer be answered.

**Expected:** The card leaves the Inbox, and an answer after that is refused.

**Status:** Works (real daemon). `daemon/humatad/tests/hooks.rs::a_hook_that_stops_holding_withdraws_its_card_and_it_cant_be_answered`. The macOS test `desktop/tests/live_inbox.rs::an_agents_permission_ask_waits_in_the_inbox_and_allow_reaches_its_hook` also covers the person answering in the agent's prompt.

### A payload that is not an ask

**When:** Claude sends a `Stop` event, or non-JSON, or an unknown agent name, or Copilot sends a sandbox bypass.

**I do:** The hook runs with that payload.

**I see:** No card. The hook prints nothing and exits 0, so the agent's own prompt decides.

**Expected:** No decision, and no card.

**Status:** Works (real daemon). `daemon/humatad/tests/hooks.rs::a_payload_that_isnt_an_ask_gets_no_decision_and_no_card`. The parser has pure tests: `daemon/agent_hooks/tests/payloads.rs::what_is_not_a_permission_ask_raises_no_card` and `daemon/agent_hooks/tests/payloads.rs::a_copilot_sandbox_bypass_raises_no_card` (Fakes only).

### Only the session's own agent may hold a hook

**When:** A caller with no agent identity sends a `hook` request.

**I do:** Send a `Hook` request with no session token.

**I see:** Refused as an invalid field, saying only a session's agent holds a permission hook. Nothing is raised in the Inbox.

**Expected:** Only an agent identity may hold a hook. The card is raised for that agent's own session.

**Status:** Works (real daemon). `daemon/humatad/tests/hooks.rs::only_the_sessions_own_agent_may_hold_a_hook`. The test uses a caller with no token. It does not test another session's agent token, though `daemon/humatad/src/session_service/hooks.rs:41-52` raises the card only for the caller.

### Answer all leaves permission cards alone

**When:** I press Answer all on a list that includes a permission card.

**I do:** Answer all, with approve set.

**I see:** The permission card stays pending. The hook is still held.

**Status:** Works (real daemon). `daemon/humatad/tests/hooks.rs::answer_all_leaves_permission_cards_alone`.

### The daemon is gone, or asks are kept in the agent's prompt

**When:** The daemon is not running while a hook fires, or Settings › Agents › Ask for permission in the Inbox is off (D199).

**I see:** The hook prints nothing and exits 0, so the agent's own prompt asks. With asks off, no card is raised, and turning the setting on raises cards again with no restart.

**Status:** Works (real daemon). `daemon/humatad/tests/hooks.rs::with_the_daemon_gone_the_hook_prints_nothing_and_exits_0` kills the daemon and runs the real hook command. `daemon/humatad/tests/hooks.rs::with_inbox_asks_off_the_hook_gets_no_decision_and_no_card` covers the setting, and `daemon/humatad/tests/hooks.rs::asks_go_to_the_inbox_by_default` the default. The CLI's refusal path is `cli/tests/hook.rs::a_refusal_or_a_daemon_error_prints_nothing_and_exits_0` (Fakes only).

### Codex hooks not yet trusted

**When:** Codex's hooks have not yet been heard by the daemon.

**I do:** Open Settings › Agents.

**I see:** A step asks the person to trust Codex's hooks. A turn report from such a session clears it.

**Status:** Works (real daemon). `daemon/humatad/tests/codex_trust.rs::codexs_hooks_need_trust_until_one_of_them_is_heard`.

### A hook gets no answer in time

**When:** Nobody answers the card before the daemon's hold limit.

**I see:** The daemon ends the hold with no decision, before the agent's own timeout. The hook prints nothing.

**Status:** Fakes only. `daemon/humatad/src/session_service/hooks.rs::nobody_answering_ends_the_hold_with_no_decision_before_the_agent_gives_up` checks the order of the two limits with a channel, not a live daemon.

### Live checks with real agents

**When:** A real Claude Code, Codex or Copilot run fires its hook.

**Status:** Fakes only, and the live checks are owed. Every test above uses a shell script in place of the agent, with recorded payloads. `workflow/milestones/HOOKS.md:45` lists the live checks as owed. Not proven live.

## Expected outcomes

- [ ] The Claude hook is blocking, with a one-hour timeout.
- [ ] The card shows the whole command, and Allow or Deny reaches the waiting hook in that agent's shape.
- [ ] A Deny carries the person's note to the agent.
- [ ] One ask held twice makes one card, and both holds get the answer.
- [ ] A hook that stops holding withdraws its card, which can no longer be answered.
- [ ] A payload that is not an ask makes no card and gives no decision.
- [ ] With the daemon gone, or asks in the agent's prompt, the hook prints nothing and exits 0.
- [ ] Always allow is not offered on a permission card, and an Always answer is refused.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Blocking hook holds for an hour | Works (real daemon) | `daemon/humatad/tests/hooks.rs::the_daemon_writes_a_blocking_claude_hook_that_holds_for_an_hour` |
| 2. Card shows the command | Works (real daemon) | `daemon/humatad/tests/hooks.rs::an_inbox_approval_reaches_the_waiting_hook` |
| 3. Allow reaches the hook | Works (real daemon) | `daemon/humatad/tests/hooks.rs::an_inbox_approval_reaches_the_waiting_hook` |
| 4. Deny with a note | Works (real daemon) | `daemon/humatad/tests/hooks.rs::an_inbox_deny_reaches_the_hook_with_its_note` |
| 5. Copilot shape | Works (real daemon) | `daemon/humatad/tests/hooks.rs::each_agent_gets_the_answer_in_its_own_shape` |
| 6. Codex shape and deny | Fakes only | `daemon/agent_hooks/tests/payloads.rs::claude_and_codex_read_allow_and_deny_from_hook_specific_output` |
| 7. Always on a permission card | Not built (no milestone) | `daemon/humatad/src/inbox/permission.rs:233-245` |
| 8. Same ask held twice | Works (real daemon) | `daemon/humatad/tests/hooks.rs::the_same_ask_held_twice_is_one_card_and_one_answer_reaches_both` |
| Branch: hook stops holding | Works (real daemon) | `daemon/humatad/tests/hooks.rs::a_hook_that_stops_holding_withdraws_its_card_and_it_cant_be_answered` |
| Branch: payload is not an ask | Works (real daemon) | `daemon/humatad/tests/hooks.rs::a_payload_that_isnt_an_ask_gets_no_decision_and_no_card` |
| Branch: only the session's agent holds | Works (real daemon) | `daemon/humatad/tests/hooks.rs::only_the_sessions_own_agent_may_hold_a_hook` |
| Branch: Answer all | Works (real daemon) | `daemon/humatad/tests/hooks.rs::answer_all_leaves_permission_cards_alone` |
| Branch: daemon gone, or asks in the agent's prompt | Works (real daemon) | `daemon/humatad/tests/hooks.rs::with_the_daemon_gone_the_hook_prints_nothing_and_exits_0` |
| Branch: Codex hooks not trusted | Works (real daemon) | `daemon/humatad/tests/codex_trust.rs::codexs_hooks_need_trust_until_one_of_them_is_heard` |
| Branch: no answer in time | Fakes only | `daemon/humatad/src/session_service/hooks.rs::nobody_answering_ends_the_hold_with_no_decision_before_the_agent_gives_up` |
| Branch: live checks with real agents | Fakes only (live checks owed) | `workflow/milestones/HOOKS.md:45` |

## Gaps and likely bugs

- Not built: Always allow on a permission card (step 7). `daemon/agent_hooks/src/lib.rs:238` says it "stays deferred". No milestone.
- Fakes only: the Codex shape (step 6), the hook's limit in time, and the payloads' parser. None runs with a real Codex.
- Owed: live checks with real Claude Code, Codex and Copilot runs (`workflow/milestones/HOOKS.md:45`). This is verification, not a missing feature.
- Known gap: the Codex trust step clears when any Codex hook is heard, so a heard turn hook does not prove the permission hook is trusted (`workflow/milestones/HOOKS.md:58`). Documented by the milestone, not yet fixed.
- Code, no test: the refusal of an Always answer on a permission card (`daemon/humatad/src/inbox/permission.rs:238`).
- Likely bug: none found in the code read.
