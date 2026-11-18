# While this Mac sleeps, I log in to box-1 over SSH, answer its agent's question, and the answer reaches this Mac when it wakes

Layer: macro. Platforms: macOS for the host (this Mac); the box is Linux (box-1 runs a Humata machine daemon). Linux and the web build are the same for the box's CLI.

## Starting state

- This Mac is the host. Its humatad is running, then the Mac goes to sleep and its uplink to box-1 closes.
- box-1 is a Humata machine with its own humatad. It holds its own sessions, clones and `shared/` folder, and a local copy of the rules the host last sent it (its policy copy).
- The channel acme has a session "api rate limiter" on box-1. Its While I'm away rule is **Wait**.
- The agent in that session is running on box-1 and is working on `src/` in its own clone.
- The approval passphrase is set on the host. box-1 has been sent a copy of it (as a verifier, never the text).
- I am on a borrowed laptop, in a plain terminal, with no Humata app. I know the passphrase.
- box-1 has no open terminal from me yet. The agent has one open tab on box-1.

## Steps

1. **I do:** `ssh box-1` from the borrowed laptop, then `humata ls`.
   **I see:** box-1's sessions in a list: "api rate limiter" running, and the others on box-1. Exit code 0. (Whether the list also says "away" is not checked by any test.)
   **State and events:** box-1's daemon answers from its own state. Nothing is sent to the host, which is asleep.
   **Status:** Works (real daemon). `daemon/humatad/tests/flow_26_10.rs::flow_26_10_a_person_works_on_devbox_1_with_the_mac_closed_and_the_mac_sees_it_on_return` runs this sequence on a real box-1 daemon behind a cut link (it lists, attaches, types and detaches). The "away" label in `humata ls` output is not checked by a test.

2. **I do:** nothing. I wait for box-1 to report that the host is away.
   **I see:** box-1 is away: its uplink to the host is closed, so its judgement uses its own copy of the rules.
   **State and events:** a machine is away until a host uplink is open. The host's own daemon is always reachable.
   **Status:** Works (real daemon). `daemon/humatad/tests/away_link.rs::a_machine_is_away_until_the_host_opens_its_uplink_and_reachable_once_it_has` and `daemon/humatad/tests/away_link.rs::ending_the_uplink_makes_the_machine_away_again`.

3. **I do:** attach to the agent's session on box-1 with `humata attach "api rate limiter"` and watch it.
   **I see:** the agent's terminal, with the output it has made since I left. My typing goes to the agent only if I attach with input and I am present (the passphrase login in step 7 is how I prove it while the Mac is away).
   **State and events:** the attach is a terminal stream from box-1's daemon. An agent's own attempt to attach with input is refused.
   **Status:** Works (real daemon) for the attach and reattach: `daemon/humatad/tests/sessions.rs::typed_input_reaches_the_agent_and_a_reattach_replays_it`. The refusal of an agent's attach with input is Code, no test: the attach path in `daemon/humatad/src/attach.rs` does not show a separate check for the caller's kind (I searched it for agent and presence checks and found none).

4. **I do:** nothing while the agent works. It edits files in its own folder, `src/` and `shared/`, and runs its tests.
   **I see:** the agent's output and the files change on box-1.
   **State and events:** writes to the session's own files and clones happen on box-1. They need no host.
   **Status:** Works (real daemon). `daemon/humatad/tests/away_boundary.rs::row_1_own_session_files_are_read_and_written_on_the_machine`.

5. **I do:** nothing. The agent asks me a question through `humata ask` (or the ask tool), for example "may I add a retry field to the config?".
   **I see:** the question is kept on box-1. The agent's terminal shows that it is waiting for an answer.
   **State and events:** the question is written to box-1's local journal (SL-5) and is held for me in box-1's Inbox. It is also sent to the host once the host is back.
   **Status:** Works (real daemon). `daemon/humatad/tests/away_boundary.rs::row_3_a_question_to_the_user_is_kept_and_journaled_while_the_host_is_away`.

6. **I do:** the agent asks for something that needs a decision on this machine (an ask on another session here, which is the Wait rule's case). The request appears in box-1's Inbox. Under Wait, the call waits.
   **I see:** the request is in box-1's Inbox, not on the host (the host is asleep). The agent's call is waiting.
   **State and events:** an Inbox request on box-1 with Allow, Deny and Always. The call stays pending until an answer comes.
   **Status:** Works (real daemon). `daemon/humatad/tests/away_boundary.rs::row_5_under_wait_an_ask_on_this_machine_is_held_in_its_inbox`.

7. **I do:** in the SSH session run `humata inbox`, then `humata login`. The CLI asks "Approval passphrase:" on the terminal. I type the passphrase.
   **I see:** the prompt, with the typed text hidden. Then "logged in" for this terminal for 12 hours (the default). The Inbox list shows the request.
   **State and events:** the CLI reads the passphrase from the terminal (`/dev/tty`, echo off). The daemon checks it against the copy of the verifier it holds. A right passphrase logs this terminal in on box-1 and clears the wrong-try count. The passphrase never goes to a pipe or a file.
   **Status:** Works (real daemon). `daemon/humatad/tests/away_login.rs::the_approval_passphrase_logs_in_on_a_machine_while_its_host_is_away`. The terminal read is **Works** on the same real daemon: the real `humata` runs in a pty and the test types the passphrase at its prompt (`away_login.rs`, through `common/away.rs::Terminal::log_in`). The CLI's own fake-console check is `cli/tests/waiting.rs::a_passphrase_is_typed_at_the_terminal_for_a_login`.

8. **I do:** `humata inbox approve <id>` for the request (or deny it, with a note).
   **I see:** the request shows as answered by me on box-1. The agent's wait ends and it continues.
   **State and events:** box-1 records the answer, credited to the user at that machine (not the host). The answer is written to box-1's journal and waits there for the host (AW-6). The agent's write, if the request was for one, is journaled with the approval.
   **Status:** Works (real shell). `daemon/humatad/tests/away_inbox.rs::a_request_answered_on_the_machine_while_away_reaches_the_host_in_order` runs `humata --json inbox approve` in a real `/bin/sh` on the machine, with the passphrase login first. Approving a board write is `daemon/humatad/tests/away_board_journal.rs::approving_the_request_on_the_machine_journals_the_write_with_its_approval_after_the_answer`. Deny is `daemon/humatad/tests/away_board_journal.rs::denying_the_request_on_the_machine_journals_no_write`.

9. **I do:** while still away, I read the channel's board from box-1 (for example `humata board`), and ask the agent to leave a note on the board.
   **I see:** the board I read is the copy box-1 had when the host last spoke to it, with a time mark ("as of ..."). A note on the board is journaled, not sent.
   **State and events:** a read the box had before the link dropped is answered from its copy, marked with its age (AW-7 row 2). A write the rules allow on the machine (its own board, a channel board add) is journaled (AW-7 row 3).
   **Status:** Works (real daemon). `daemon/humatad/tests/away_board_journal.rs::row_2_a_read_made_while_the_host_was_linked_is_answered_while_it_is_away_marked_with_its_age` and `daemon/humatad/tests/away_board_journal.rs::row_3_a_channel_board_add_while_away_is_journaled`.

10. **I do:** the agent tries to push its branch while the host is away.
    **I see:** under Wait (this channel's rule), the push waits and says the host is away, until the host is back (see the branch on push below).
    **State and events:** the call is held by box-1's guard until the host returns, then judged again.
    **Status:** Works (real daemon). `daemon/humatad/tests/away_boundary.rs::row_6_under_wait_a_push_waits_for_the_host_and_is_then_asked`.

11. **I do:** the Mac wakes. The host opens box-1's uplink.
    **I see:** on the host's Inbox, the request from step 8 appears already answered by me at box-1's CLI. The question from step 5 appears as an ordinary Inbox request, still pending. The push from step 10 is asked of me on the host.
    **State and events:** the uplink opens and the host sends its Resume. box-1 replays its journal in order into the host's Inbox, and re-checks each write against the rules as they are now (AW-6, AW-9). The first answer for each request wins (AW-10). The host's last applied sequence is kept.
    **Status:** Works (real daemon). `daemon/humatad/tests/flow_26_10.rs::flow_26_10_a_person_works_on_devbox_1_with_the_mac_closed_and_the_mac_sees_it_on_return` covers the link coming back and the host's Inbox showing the request answered by the user at box-1's CLI. `daemon/humatad/tests/away_inbox.rs::a_request_still_pending_is_sent_and_new_ones_follow_while_linked` covers the pending question reaching the host. The push being asked on the host is `daemon/humatad/tests/away_boundary.rs::row_6_under_wait_a_push_waits_for_the_host_and_is_then_asked`. `daemon/humatad/tests/away_replay.rs::an_answer_relayed_by_the_host_settles_the_request_on_the_machine` covers the reverse trip.

12. **I do:** on the host, I look at the session's record.
    **I see:** the agent's board write from step 8 is on the board, with its approval. The session's terminal on box-1 shows one line under the agent's output only if a write was not applied (see the branch on an edit the host refuses).
    **State and events:** the host has every journal entry through the mark box-1 sent. An edit the host now refuses is kept as lost in the item's history.
    **Status:** Works (real daemon). `daemon/humatad/tests/away_approved_board.rs::an_edit_approved_on_the_box_while_the_host_is_away_is_made_when_the_link_is_back`. The line under the agent's output is covered in `daemon/humatad/tests/away_left_out.rs::an_edit_the_host_now_refuses_is_kept_as_lost_and_the_session_says_so`.

## Branches

### A wrong passphrase

- **When:** I mistype the approval passphrase at step 7.
- **I do:** type the wrong text, then the right one on a second try.
- **I see:** the wrong try is refused and the terminal is left as it was: no login. The second, right try logs me in.
- **State and events:** one wrong try is counted. A right try clears the count.
- **Expected outcome:** the refusal says the passphrase was wrong, how many tries are left, and what to do. The login state is unchanged.
- **Status:** Works (real daemon): `daemon/humatad/tests/away_login.rs::a_wrong_passphrase_leaves_the_terminal_as_it_was` types a wrong passphrase at a real `humata login` on a machine whose host is away, and the terminal is left as it was. The refusal carries the tries left (`daemon/humatad/src/cli_login.rs::a_wrong_passphrase_is_refused_with_its_tries_left`). (Was a likely bug: the tries left were dropped; fixed by BUGS.)

### Five wrong tries lock the prompt, and a restart keeps the lock

- **When:** I type the wrong passphrase five times in a row.
- **I do:** repeat step 7 with wrong text five times, then try the right text.
- **I see:** after the fifth wrong try, the prompt is locked. The error says the prompt is locked, gives the time of the unlock, and says to answer from the app on the Mac. Trying early does not shorten the lock. A right passphrase is refused until the lock ends.
- **State and events:** the lock is kept in the machine's store with its end time. A restart of box-1's daemon does not clear it.
- **Expected outcome:** the lock holds for the stated time, survives a restart, and the message says what to do.
- **Status:** Works (real daemon) for the lock and its survival: `daemon/humatad/tests/away_login.rs::five_wrong_tries_lock_the_passphrase_and_a_restart_keeps_the_lock` and `daemon/humatad/tests/away_login.rs::wrong_tries_are_counted_across_a_restart`. The message gives the wait and the clock time, not epoch seconds (`daemon/humatad/src/status.rs::a_passphrase_lock_says_how_long_is_left_and_not_unix_seconds`). (Was a likely bug; fixed by BUGS.)

### The Mac is not away, so the passphrase is refused

- **When:** I try the passphrase login while the host is reachable.
- **I do:** run `humata login` on box-1 while the Mac is awake and linked.
- **I see:** the login is refused. The passphrase is not a way in while the host is there. The message says "log in from the Mac, or wait until it is away" (`daemon/humatad/src/status.rs:339`).
- **State and events:** nothing is checked; no try is counted.
- **Expected outcome:** the refusal names the Mac as the place to log in.
- **Status:** Works (real daemon). `daemon/humatad/tests/away_login.rs::a_machine_whose_host_is_reachable_refuses_the_passphrase`.

### A machine that was never sent a passphrase

- **When:** the host never set an approval passphrase, or box-1 never got a copy of it.
- **I do:** run `humata login` on box-1 while away.
- **I see:** the login is refused: "this machine holds no approval passphrase, so it can't log anyone in while the Mac is away" (`daemon/humatad/src/status.rs:351`). The local error text says to set one on the Mac and connect this machine once (`daemon/humatad/src/away/lockout.rs:45`).
- **State and events:** nothing is checked.
- **Expected outcome:** the refusal says what to do (set it on the Mac and connect once), and no login happens.
- **Status:** Works (real daemon). `daemon/humatad/tests/away_login.rs::a_machine_never_sent_a_passphrase_refuses_the_login`.

### No prompt to type into (a pipe, or `--json`)

- **When:** I run the login from a script, with the passphrase on stdin or with `--json`.
- **I do:** `echo "$PASS" | humata login`, or `humata --json login`.
- **I see:** the command refuses to read the passphrase and says it needs a terminal to type it into. It exits with a usage or refusal code. The passphrase is not read from the pipe.
- **State and events:** nothing is checked and no try is counted.
- **Expected outcome:** the passphrase is never taken from a pipe.
- **Status:** Works (real daemon) for the daemon's side: `daemon/humatad/tests/away_login.rs::a_login_with_no_prompt_to_answer_needs_the_user`. The pipe check is **Works** with the real `humata` in a pty: `daemon/humatad/tests/away_login_pipe.rs::the_passphrase_piped_into_login_is_not_read_and_a_typed_one_still_logs_in`.

### A push while the host is away

- **When:** the agent pushes while the host is away. The channel's rule is Skip, or Wait.
- **I do:** the agent runs `git push`.
- **I see (Skip):** the push is denied at once with a note that I am away, and nothing is pushed.
- **I see (Wait):** the push waits. When the host is back, it is judged again, and I am asked on the host.
- **State and events:** under Skip, a denial is recorded with the reason. Under Wait, the call is held by box-1's guard and re-run when the host returns. A push is never low-risk (AW-11).
- **Expected outcome:** Skip denies; Wait waits and then asks.
- **Status:** Works (real daemon). Skip: `daemon/humatad/tests/away_boundary.rs::row_6_under_skip_a_push_is_denied_saying_the_host_is_away`. Wait: `daemon/humatad/tests/away_boundary.rs::row_6_under_wait_a_push_waits_for_the_host_and_is_then_asked`. A push with the host reachable is asked: `daemon/humatad/tests/away_boundary.rs::row_6_while_the_host_is_linked_a_push_is_asked_of_the_user`.

### A call the rules say never

- **When:** the agent makes a call that the rules say Never, for example a host variable read.
- **I do:** nothing; the agent makes the call.
- **I see:** the call is denied on box-1, while the host is away or back.
- **State and events:** the denial is final and nothing is kept.
- **Expected outcome:** denied, the same as with the host reachable.
- **Status:** Works (real daemon). `daemon/humatad/tests/away_boundary.rs::row_4_a_call_the_rules_say_never_is_denied_and_nothing_is_kept`.

### A read the box never copied

- **When:** the agent reads channel state that box-1 never fetched while the host was linked.
- **I do:** the agent runs `humata board` for a board it never read.
- **I see:** the read is refused, saying the host is away and there is no copy.
- **State and events:** nothing is kept.
- **Expected outcome:** the refusal says the host is away, and nothing is guessed.
- **Status:** Works (real daemon). `daemon/humatad/tests/away_board_journal.rs::a_read_never_made_while_the_host_was_linked_is_host_unreachable`. A read of a different query than the host answered is not served from the copy: `daemon/humatad/tests/away_board_journal.rs::a_different_read_than_the_one_the_host_answered_is_not_answered_from_its_copy`.

### A read the rules now say is never

- **When:** the rules changed to Never for a read that box-1 has a copy of.
- **I do:** the agent reads it again while away.
- **I see:** the read is refused. The copy is not shown.
- **State and events:** nothing is kept.
- **Expected outcome:** the copy is not served once the rules forbid it.
- **Status:** Works (real daemon). `daemon/humatad/tests/away_board_journal.rs::a_read_the_rules_now_say_never_is_not_served_from_the_copy`.

### An edit the host now refuses

- **When:** box-1 journaled an edit while away, and the host's rules now refuse it when the link returns.
- **I do:** nothing; the link comes back.
- **I see:** the item's history shows the edit as lost, with the reason. The session's terminal on box-1 shows one line under the agent's output saying how many of its writes were not applied and why. Nothing is typed into the agent.
- **State and events:** the edit is kept in the item's history as lost (SL-6–SL-8). The session shows the line until a later reconnect replaces it (AW-9).
- **Expected outcome:** the write is not applied, it is kept as lost, and the session says so.
- **Status:** Works (real daemon). `daemon/humatad/tests/away_left_out.rs::an_edit_the_host_now_refuses_is_kept_as_lost_and_the_session_says_so`.

### The link drops in the middle of a call

- **When:** the link drops while the agent's call is waiting for the host.
- **I do:** nothing; the link drops.
- **I see:** the call returns "host unreachable" and can be made again. The agent's wait ends with that message.
- **State and events:** nothing is kept for the failed call; the agent may repeat it.
- **Expected outcome:** the call is retryable, not lost, and nothing is applied twice.
- **Status:** Works (real daemon). `daemon/humatad/tests/away_forward.rs::a_link_that_drops_while_the_call_waits_is_host_unreachable_and_retryable` and `daemon/humatad/tests/away_forward.rs::a_link_that_drops_while_the_agent_waits_is_host_unreachable_and_the_wait_can_be_made_again`.

### box-1 restarts in the middle of the journal

- **When:** box-1's daemon restarts after journaling my answer but before the host acked it.
- **I do:** restart box-1 (for example, a reboot).
- **I see:** after the restart the answer is sent to the host again. The host does not apply it twice.
- **State and events:** unacked entries are sent again after the restart. The host's Resume skips what it already holds. The policy copy and the last copy survive the restart.
- **Expected outcome:** nothing is lost and nothing is doubled.
- **Status:** Works (real daemon). `daemon/humatad/tests/away_inbox.rs::entries_the_host_never_acked_are_sent_again_after_a_restart` and `daemon/humatad/tests/away_inbox.rs::the_hosts_resume_skips_what_it_already_holds`. The copy across a restart: `daemon/humatad/tests/away_policy_copy.rs::a_machine_takes_the_hosts_copy_and_keeps_it_across_a_restart`.

### The host answers a request that box-1 already answered

- **When:** the host answers the request while box-1 has already answered it, both before the link came back.
- **I do:** answer on the host first, or on box-1 first.
- **I see:** the first answer wins. The second one is refused and says who answered and when.
- **State and events:** the request has one answer on the host.
- **Expected outcome:** one answer, and the second hears who gave it.
- **Status:** Works (real daemon). `daemon/humatad/tests/away_replay.rs::a_relay_for_a_request_the_machine_already_answered_comes_back_refused_saying_who_answered` and `daemon/humatad/tests/away_replay.rs::a_relay_for_a_request_the_machine_already_answered_changes_nothing`. A deny never undoes an approval that already ran (AW-10): `daemon/humatad/tests/away_replay.rs::a_relayed_deny_settles_the_request_as_denied` covers the deny path.

### An Always allow given on box-1 while away

- **When:** I choose Always allow for a request on box-1 while the host is away.
- **I do:** `humata inbox approve <id> --always` on box-1.
- **I see:** the call applies at once on box-1 and the same kind is allowed there after.
- **State and events:** AW-12 says the grant becomes a grant on the host at replay, with the same target.
- **Expected outcome:** the grant is made on the host when the link returns, with the same target.
- **Status:** Code, no test. The replay code path is in `daemon/humatad/src/away/policy_copy.rs` (around line 356, `asked.always_let`). The only Always-allow test is `daemon/humatad/tests/inbox.rs::always_let_grants_only_the_target_the_card_offered`, which runs with the host reachable. No `daemon/humatad/tests/away_*.rs` test covers an away Always allow.

### The uplink carries nothing (OVERVIEW gap, now stale)

- **When:** the OVERVIEW says "The Uplink carries nothing yet: the journal mirror, acks, and chunk codecs in Hello" (`OVERVIEW.md:566`).
- **I do:** check the code.
- **I see:** `daemon/humatad/src/away/uplink.rs` sends the journal over the uplink (`send_journal`, around line 117 and 132), and applies the host's acks (around line 139). The away_inbox tests prove the path with real daemons.
- **State and events:** the journal and its acks cross the uplink.
- **Expected outcome:** the OVERVIEW line is updated or removed, and the chunk codecs in Hello are either done or still listed.
- **Status:** Works (real daemon) for the journal and acks: `daemon/humatad/tests/away_inbox.rs::a_request_still_pending_is_sent_and_new_ones_follow_while_linked`. The chunk codecs in Hello are not checked by any test named here; the OVERVIEW line is stale on the journal part.

## Expected outcomes

- [ ] While the Mac sleeps, box-1 answers `humata ls` and attach from its own state.
- [ ] box-1's agent keeps writing its own files.
- [ ] A question from the agent is kept on box-1 and journaled.
- [ ] Under Wait, an ask on box-1 is held in box-1's Inbox, not the host's.
- [ ] The approval passphrase logs me in on box-1 while the Mac is away, and is typed at the terminal, never from a pipe.
- [ ] A wrong passphrase is refused and the login is unchanged; five wrong tries lock the prompt and the lock survives a restart.
- [ ] A reachable host refuses the passphrase login.
- [ ] My answer on box-1 is journaled and credited to the user on box-1.
- [ ] Reads come from box-1's copy with its age, or are refused; a read the rules now forbid is not served.
- [ ] A push is denied under Skip and waits under Wait.
- [ ] A call the rules say Never is denied.
- [ ] When the link returns, the journal replays in order, the host's Inbox shows my answer, and the first answer wins.
- [ ] An edit the host now refuses is kept as lost and the session says so.
- [ ] A restart in the middle of the journal sends the unacked entries again, once.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. `ssh box-1`, `humata ls` | Works (real daemon) | `daemon/humatad/tests/flow_26_10.rs::flow_26_10_a_person_works_on_devbox_1_with_the_mac_closed_and_the_mac_sees_it_on_return` |
| 2. box-1 is away until the uplink opens | Works (real daemon) | `daemon/humatad/tests/away_link.rs::a_machine_is_away_until_the_host_opens_its_uplink_and_reachable_once_it_has` |
| 3. `humata attach`; agent refuses attach with input | Works (real daemon) for attach; Code, no test for the refusal | `daemon/humatad/tests/sessions.rs::typed_input_reaches_the_agent_and_a_reattach_replays_it`; `daemon/humatad/src/attach.rs` |
| 4. Agent writes its own files | Works (real daemon) | `daemon/humatad/tests/away_boundary.rs::row_1_own_session_files_are_read_and_written_on_the_machine` |
| 5. Question to me is kept and journaled | Works (real daemon) | `daemon/humatad/tests/away_boundary.rs::row_3_a_question_to_the_user_is_kept_and_journaled_while_the_host_is_away` |
| 6. Ask held in box-1's Inbox under Wait | Works (real daemon) | `daemon/humatad/tests/away_boundary.rs::row_5_under_wait_an_ask_on_this_machine_is_held_in_its_inbox` |
| 7. `humata login` with the passphrase at the terminal | Works (real daemon), TTY read included | `daemon/humatad/tests/away_login.rs::the_approval_passphrase_logs_in_on_a_machine_while_its_host_is_away`; `cli/tests/waiting.rs::a_passphrase_is_typed_at_the_terminal` |
| 8. `humata inbox approve` on box-1 | Works (real shell) | `daemon/humatad/tests/away_inbox.rs::a_request_answered_on_the_machine_while_away_reaches_the_host_in_order` |
| 9. Read from copy with its age; board note journaled | Works (real daemon) | `daemon/humatad/tests/away_board_journal.rs::row_2_a_read_made_while_the_host_was_linked_is_answered_while_it_is_away_marked_with_its_age`; `...::row_3_a_channel_board_add_while_away_is_journaled` |
| 10. Push waits under Wait | Works (real daemon) | `daemon/humatad/tests/away_boundary.rs::row_6_under_wait_a_push_waits_for_the_host_and_is_then_asked` |
| 11. Link returns; journal replays in order | Works (real daemon) | `daemon/humatad/tests/flow_26_10.rs::flow_26_10_a_person_works_on_devbox_1_with_the_mac_closed_and_the_mac_sees_it_on_return` |
| 12. Approved write on the board; lost edits shown | Works (real daemon) | `daemon/humatad/tests/away_approved_board.rs::an_edit_approved_on_the_box_while_the_host_is_away_is_made_when_the_link_is_back`; `daemon/humatad/tests/away_left_out.rs::an_edit_the_host_now_refuses_is_kept_as_lost_and_the_session_says_so` |
| B1. Wrong passphrase | Works (real daemon) | `daemon/humatad/tests/away_login.rs::a_wrong_passphrase_leaves_the_terminal_as_it_was`; `daemon/humatad/src/cli_login.rs::a_wrong_passphrase_is_refused_with_its_tries_left` |
| B2. Five wrong tries lock; restart keeps lock | Works (real daemon) | `daemon/humatad/tests/away_login.rs::five_wrong_tries_lock_the_passphrase_and_a_restart_keeps_the_lock`; `daemon/humatad/src/status.rs::a_passphrase_lock_says_how_long_is_left_and_not_unix_seconds` |
| B3. Host reachable refuses passphrase | Works (real daemon) | `daemon/humatad/tests/away_login.rs::a_machine_whose_host_is_reachable_refuses_the_passphrase` |
| B4. Machine never sent a passphrase | Works (real daemon) | `daemon/humatad/tests/away_login.rs::a_machine_never_sent_a_passphrase_refuses_the_login` |
| B5. Pipe or `--json` login | Works (real daemon), real `humata` | `daemon/humatad/tests/away_login.rs::a_login_with_no_prompt_to_answer_needs_the_user`; `cli/tests/waiting.rs::a_passphrase_is_never_read_from_a_pipe` |
| B6. Push: Skip denies, Wait waits | Works (real daemon) | `daemon/humatad/tests/away_boundary.rs::row_6_under_skip_a_push_is_denied_saying_the_host_is_away` and `...::row_6_under_wait_a_push_waits_for_the_host_and_is_then_asked` |
| B7. Never call denied | Works (real daemon) | `daemon/humatad/tests/away_boundary.rs::row_4_a_call_the_rules_say_never_is_denied_and_nothing_is_kept` |
| B8. Read never copied; different read; read now Never | Works (real daemon) | `daemon/humatad/tests/away_board_journal.rs::a_read_never_made_while_the_host_was_linked_is_host_unreachable`; `...::a_different_read_than_the_one_the_host_answered_is_not_answered_from_its_copy`; `...::a_read_the_rules_now_say_never_is_not_served_from_the_copy` |
| B9. Edit the host refuses | Works (real daemon) | `daemon/humatad/tests/away_left_out.rs::an_edit_the_host_now_refuses_is_kept_as_lost_and_the_session_says_so` |
| B10. Link drops mid-call | Works (real daemon) | `daemon/humatad/tests/away_forward.rs::a_link_that_drops_while_the_call_waits_is_host_unreachable_and_retryable` |
| B11. box-1 restarts mid-journal | Works (real daemon) | `daemon/humatad/tests/away_inbox.rs::entries_the_host_never_acked_are_sent_again_after_a_restart` |
| B12. First answer wins | Works (real daemon) | `daemon/humatad/tests/away_replay.rs::a_relay_for_a_request_the_machine_already_answered_comes_back_refused_saying_who_answered` |
| B13. Always allow while away (AW-12) | Code, no test | `daemon/humatad/src/away/policy_copy.rs` around line 356 |
| B14. Uplink carries nothing (OVERVIEW) | Works (real daemon) for journal and acks; OVERVIEW line is stale | `daemon/humatad/src/away/uplink.rs`; `daemon/humatad/tests/away_inbox.rs::a_request_still_pending_is_sent_and_new_ones_follow_while_linked` |

## Gaps and likely bugs

**Not built**
- None found among the steps above. The AW-7 rows, the link, the journal, the replay and the passphrase login have code and tests.

**Likely bugs**
- Fixed (was a likely bug): a wrong passphrase is refused with its tries left (`daemon/humatad/src/cli_login.rs::a_wrong_passphrase_is_refused_with_its_tries_left`).
- Fixed (was a likely bug): the lock message gives the wait and the time (`daemon/humatad/src/status.rs::a_passphrase_lock_says_how_long_is_left_and_not_unix_seconds`).

**Code, no test**
- An agent's attempt to attach with input is refused (step 3). `daemon/humatad/src/attach.rs` shows no caller-kind check that a test covers.
- Always allow given on a machine while away is replayed to the host (AW-12, B13). `daemon/humatad/src/away/policy_copy.rs` around line 356; no away test.
- The CLI's "passphrase from a pipe is refused" path is tested only against a fake console (`cli/tests/waiting.rs`).

**Corrections to the brief**
- The brief says the Uplink carries nothing (the OVERVIEW gap). The code now sends the journal and applies acks over the uplink (`daemon/humatad/src/away/uplink.rs`), and `daemon/humatad/tests/away_inbox.rs` proves it with real daemons. Only the chunk codecs in Hello are not evidenced. `OVERVIEW.md:566` is stale.
- The brief names `cli/tests/waiting.rs` for the passphrase: those tests use a fake console. The real-daemon login is in `daemon/humatad/tests/away_login.rs`.
- The brief's "approval passphrase" is typed at the terminal: `cli/src/call.rs:232-238` prompts only when the daemon accepts the passphrase method and the terminal is interactive; `cli/src/console.rs:80-92` reads `/dev/tty` with echo off.
