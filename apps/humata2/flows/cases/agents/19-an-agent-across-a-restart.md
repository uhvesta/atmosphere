# An agent keeps its tab, its Inbox and its conversation across a daemon restart and a Mac reboot

Layer: agents. Platforms: macOS, Linux, web.

## Starting state

- Humata 2 runs on this Mac with channel `Payments` and session `ledger` (Claude Code). The agent is in its turn, so the sidebar shows Working.
- The agent's tab 0 has its own session token in `HUMATA_SESSION_TOKEN`. Its holder keeps the shell and the agent running.
- Settings › Agents › Ask for permission in the Inbox is on, so Claude's permission hook raises a card.
- Three things are pending for `ledger`:
  - an ask-ahead for network reach to `example.com:443`, which the agent sent earlier and nobody has answered;
  - a deferred call: the agent's `channel.list` call is waiting for my approval in the Inbox, and the agent is waiting on it;
  - a permission hook that is holding Claude's `Bash` tool call. Claude is waiting on the hook's answer.
- Earlier, I approved a folder read for `ledger`, and I revoked an older grant. Both are in the Inbox history.
- The Inbox also holds one unanswered question from another session.
- The app is open on `ledger`.

## Steps

1. **I do:** The daemon crashes, and I start `humatad2` again (or an upgrade restarts it). The holder for `ledger`'s tab keeps running.

   **I see:** The app reconnects. `ledger` still shows Working, and its tab shows the same agent output with no new "agent up" line. The agent did not start again.

   **State and events:** The daemon reattaches the live holder. The tab's session token keeps working, because the token is tied to the holder and is saved across the restart.

   **Status:** Works (real daemon). `daemon/humatad/tests/mcp.rs::a_tab_that_outlives_a_restart_keeps_its_token` restarts the daemon with the holder alive, and then asks `WhoAmI` with the tab's token.

2. **I do:** In the agent's tab, run `humata whoami`.

   **I see:** The agent is `ledger`, in its channel. The call is not refused.

   **State and events:** The token resolves to the same session (`daemon/humatad/src/identity.rs`, the session-token branch).

   **Status:** Works (real daemon). The same MCP test checks `WhoAmI` over gRPC, not the `humata` command.

3. **I do:** Nothing. I open the Inbox after the restart.

   **I see:** The deferred `channel.list` card is no longer pending. It shows as withdrawn, with the reason that the call ended. The agent's call ends too.

   **State and events:** The request is saved as withdrawn with reason `CallEnded`. Nothing is left that could run the call. The restore code withdraws any deferred request that was still pending (`daemon/humatad/src/inbox/kept.rs:207-210`).

   **Status:** Works (real daemon) for the state. `daemon/humatad/tests/inbox.rs::after_a_restart_a_pending_deferred_request_is_withdrawn_and_an_ask_ahead_still_waits`. The text the agent gets back from its call is not asserted by any test.

4. **I do:** Nothing. I look at the ask-ahead for `example.com:443`.

   **I see:** It is still pending, with Allow and Deny. Its card is the same as before the restart.

   **State and events:** The ask-ahead was not a live call, so the restart does not end it. The restored request is still pending.

   **Status:** Works (real daemon). The same test asserts `Pending` after the restart. Approving it after a restart is not tested.

5. **I do:** Nothing. Claude's permission hook was holding a `Bash` tool call when the daemon restarted.

   **I see:** Claude shows its own permission prompt in the tab. The Humata card for that tool call is gone from the Inbox.

   **State and events:** A pending permission card is never saved. It lives only in memory (`daemon/humatad/src/inbox/permission.rs:154-186` adds the entry without a commit). When the daemon dies, the hook's stream ends. `humata hook` then gets no answer and prints nothing, and it exits 0 (`cli/src/commands/hook.rs:46-66`). Claude falls back to its own prompt.

   **Status:** Code, no test. No test restarts the daemon while a hook is holding a card. The nearest tests are `daemon/humatad/tests/hooks.rs::with_the_daemon_gone_the_hook_prints_nothing_and_exits_0` (the daemon was dead before the hook started) and `hooks.rs::a_hook_that_stops_holding_withdraws_its_card_and_it_cant_be_answered` (the hook let go, and the daemon stayed up).

6. **I do:** While the daemon is down, Claude runs another `Bash` call, so its permission hook runs.

   **I see:** The hook prints nothing and exits 0. Claude shows its own prompt.

   **State and events:** The hook finds no daemon to hold it, so it gives up at once. No card is raised, and the hook never fails or blocks the agent. Its wait is an hour at most (`cli/src/commands/hook.rs:22`), so a daemon that stops answering without closing the stream is a separate case that no test covers.

   **Status:** Works (real daemon). `daemon/humatad/tests/hooks.rs::with_the_daemon_gone_the_hook_prints_nothing_and_exits_0`.

7. **I do:** Open the Inbox.

   **I see:** The settled requests are where they were: the folder read is Approved, the other request is Approved, and the older grant is still revoked. The question from the other session is still pending.

   **State and events:** Each request, answer and grant is saved before anyone hears of it. The grants are rebuilt exactly as narrow as they were, and a revoked grant stays revoked.

   **Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::a_restart_keeps_the_inbox_and_the_grants_still_standing`.

8. **I do:** Restart the whole Mac. The holders and the agent die with it. When the Mac is back, Humata's daemon starts at login.

   **I see:** The app opens on `ledger`. The session is running again, and its agent is resumed in its conversation. The earlier transcript is still there.

   **State and events:** The daemon starts each session's agent with `--resume` and the same conversation id. The hook state from before the reboot is cleared (D169: a resume forgets what the hooks said).

   **Status:** Works (real daemon, fake agent). `desktop/tests/live_flows.rs::after_a_reboot_the_session_is_running_again_in_its_conversation` (the daemon and every holder are killed, then the app reopens). The agent is a fake that prints `agent up --resume`. `daemon/humatad/tests/sessions.rs::a_lost_holder_is_resumed_in_its_conversation_after_a_restart` covers the same resume after the holder alone is lost.

9. **I do:** In the resumed tab, run `humata whoami`.

   **I see:** It works. The token in the tab's environment is new, and the token from before the reboot is refused.

   **State and events:** A session tab brought back by a start gets a fresh token each time. The old holder's token is dropped with the old holder (`daemon/humatad/src/restart.rs:340-400`).

   **Status:** Code, no test. The restart test in step 1 keeps the holder alive, so it never reaches the fresh-token path.

10. **I do:** Nothing. The reboot happened while `ledger` was partway through forking a session, with its code copied and its agent not yet started.

    **I see:** After the start, the fork is gone. Its folder is removed, and the parent session is unchanged.

    **State and events:** An operation cut before its agent starts is undone at the next start. One cut after its agent started is kept.

    **Status:** Works (real daemon). `daemon/humatad/tests/fork.rs::a_fork_cut_before_its_agent_starts_is_undone_at_restart` and `fork.rs::a_fork_cut_after_its_agent_started_is_kept_at_restart`. The host's power-cut suite, `daemon/humatad/tests/power_cut_host.rs`, runs the same fork at each fault point. Its tests are generated by the `cut_tests!` macro, and `a_local_fork_with_the_hosts_power_cut_loses_nothing_part_1` is one of them (fake box).

11. **I do:** Nothing. The reboot happened while `ledger` was moving to another machine. The move had copied the folder, and the agent had moved or had not.

    **I see:** If the agent had not moved, the move is undone and `ledger` stays on this Mac. If the agent had moved, the move is finished, and `ledger` runs on the other machine.

    **State and events:** A move cut after the copy is undone. A move cut after the agent moved is finished at the next start.

    **Status:** Works (real daemon). `daemon/humatad/tests/move_session.rs::a_move_cut_short_after_the_copy_is_undone_at_restart` and `move_session.rs::a_move_cut_short_after_the_agent_moved_is_finished_at_restart`. The host-side power cut is in `daemon/humatad/tests/power_cut_host.rs::a_move_with_the_hosts_power_cut_loses_nothing_part_1`.

## Branches

### The agent was mid-turn at the reboot

**When:** The Mac reboots while Claude is partway through a turn.

**I do:** Reopen the Mac and the app.

**I see:** `ledger` is running again in its conversation, with the earlier transcript. The status shows Working until Claude's start hook reports idle.

**State and events:** The resume starts Claude with `--resume`. The interrupted turn is not run again. Nothing re-sends the text that was in progress, so the agent waits for the next input.

**Expected outcome:** The conversation resumes. The interrupted turn is not repeated, and no text is typed into the agent on its own.

**Status:** Works (real daemon) for the resume, through `daemon/humatad/tests/sessions.rs::a_lost_holder_is_resumed_in_its_conversation_after_a_restart`. Code, no test for "the interrupted turn is not repeated".

### The conversation was lost in the reboot

**When:** The conversation file for `ledger` is gone when the daemon starts.

**I do:** Open `ledger`.

**I see:** The session shows Exited, with a recovery line: "Humata couldn't resume this session's agent. Try Resume again". The start report names the session.

**State and events:** The daemon sets the session to ResumeFailed. It starts nothing in its place (`daemon/humatad/src/session_service.rs:1724`, `clients/live/src/map.rs:192-197`). Start fresh is offered, and it begins a new conversation with a summary of the saved terminal output.

**Expected outcome:** The session says it could not resume and offers Resume and start fresh. No new agent starts on its own.

**Status:** Works (real daemon). `daemon/humatad/tests/sessions.rs::a_reboot_that_lost_the_conversation_says_the_resume_failed` (the status and the report). `daemon/humatad/tests/start_fresh.rs::a_failed_resume_starts_fresh_on_a_new_conversation_and_hears_the_summary` (the fallback). The app's card and its buttons Work (real daemon): `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt`, `desktop/tests/live_flows/restart.rs::start_fresh_on_the_couldnt_resume_card_starts_a_new_conversation_that_hears_the_summary`. Its exact words are checked only by the design goldens (`desktop/tests/design/session.rs`, `desktop/tests/design/resume.rs`); behaviour tests don't assert wording.

### A handover was cut after the new agent launched

**When:** A fork was handed over to a different agent. The daemon stopped after the launch and before the handover was marked done.

**I do:** Restart the daemon.

**I see:** The fork is kept. The new agent receives the handover text once, not twice.

**State and events:** The fork is kept because its agent started. The handover is typed only once, after the restart.

**Expected outcome:** The handover appears in the fork's conversation exactly once.

**Status:** Works (real daemon). `daemon/humatad/tests/fork.rs::a_handover_cut_after_the_launch_is_typed_once_after_a_restart`.

## Expected outcomes

- [ ] After a daemon restart, the agent's tab keeps running, and its token still works from inside the tab.
- [ ] The deferred call is withdrawn with reason `CallEnded`. The ask-ahead is still pending.
- [ ] A hook that loses its daemon prints nothing and exits 0. Claude then shows its own prompt.
- [ ] The Inbox keeps the settled requests and the grants, and a revoked grant stays revoked.
- [ ] After a Mac reboot, the session runs again in its conversation.
- [ ] A session tab brought back by a start gets a new token, and the old token is refused.
- [ ] A fork cut before its agent starts is undone. One cut after its agent started is kept.
- [ ] A move cut after the copy is undone. One cut after the agent moved is finished.
- [ ] An interrupted turn is not repeated after a reboot.
- [ ] A lost conversation shows "couldn't resume", with a way forward and no agent started on its own.
- [ ] A handover cut after launch is typed once.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. Daemon restarts, tab keeps its token | Works (real daemon) | `daemon/humatad/tests/mcp.rs::a_tab_that_outlives_a_restart_keeps_its_token` |
| 2. `humata whoami` in the tab | Works (real daemon) | same test, over gRPC `WhoAmI` |
| 3. Deferred call withdrawn | Works (real daemon) | `daemon/humatad/tests/inbox.rs::after_a_restart_a_pending_deferred_request_is_withdrawn_and_an_ask_ahead_still_waits`; agent's error text untested |
| 4. Ask-ahead still pending | Works (real daemon) | same test; approving after a restart is untested |
| 5. Hook holding card during a daemon restart | Code, no test | `daemon/humatad/src/inbox/permission.rs:154-186`; `cli/src/commands/hook.rs:44-66` |
| 6. Hook fired while the daemon is down | Works (real daemon) | `daemon/humatad/tests/hooks.rs::with_the_daemon_gone_the_hook_prints_nothing_and_exits_0` |
| 7. Inbox and grants survive | Works (real daemon) | `daemon/humatad/tests/inbox.rs::a_restart_keeps_the_inbox_and_the_grants_still_standing` |
| 8. Session resumes after a Mac reboot | Works (real daemon, fake agent) | `desktop/tests/live_flows.rs::after_a_reboot_the_session_is_running_again_in_its_conversation` |
| 9. Fresh token after a reboot | Code, no test | `daemon/humatad/src/restart.rs:340-400` |
| 10. Fork cut midway | Works (real daemon) | `daemon/humatad/tests/fork.rs::a_fork_cut_before_its_agent_starts_is_undone_at_restart`; `power_cut_host.rs::a_local_fork_with_the_hosts_power_cut_loses_nothing_part_1` (macro-generated) |
| 11. Move cut midway | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_move_cut_short_after_the_copy_is_undone_at_restart`; `move_session.rs::a_move_cut_short_after_the_agent_moved_is_finished_at_restart` |
| Branch: mid-turn at reboot, resume | Works (real daemon) | `daemon/humatad/tests/sessions.rs::a_lost_holder_is_resumed_in_its_conversation_after_a_restart` |
| Branch: mid-turn at reboot, turn not repeated | Code, no test | none |
| Branch: conversation lost | Works (real daemon) | `daemon/humatad/tests/sessions.rs::a_reboot_that_lost_the_conversation_says_the_resume_failed`; `start_fresh.rs::a_failed_resume_starts_fresh_on_a_new_conversation_and_hears_the_summary` |
| Branch: "Couldn't resume" card | Works (real daemon); words by goldens | `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt`; `desktop/tests/design/resume.rs` |
| Branch: handover cut after launch | Works (real daemon) | `daemon/humatad/tests/fork.rs::a_handover_cut_after_the_launch_is_typed_once_after_a_restart` |

## Gaps and likely bugs

**Not built**

- None found for these flows. Start fresh, the power-cut suites, and the move and fork recovery are built.

**Likely bugs**

- None confirmed. Two things need a look:
  - The agent's `channel.list` call ends with the withdrawn request, but no test asserts what the agent is told. Check the error text the agent gets, so it knows to ask again.
  - A reboot gives the resumed tab a new token. An agent that had kept the old token in a long-running process outside the tab will be refused with an identity mismatch. The refusal text should say to use the tab's new environment, or restart the agent. Check the text the refusal gives.
