# An agent on a box keeps working while the host is away, and each call says what happens

Layer: agents. Platforms: macOS, Linux, web.

## Starting state

- The host is this Mac. It is asleep, so the host is away.
- `box-1` is a machine. Its own `humatad2` runs, and it holds the host's last copy of the rules for channel `Payments`: the "While I'm away" rule is Wait, and the rules and grants are the ones from the last link.
- Session `ledger` runs on `box-1` with Claude Code. Its tab 0 has the session token in `HUMATA_SESSION_TOKEN`. Its clone is on `box-1`.
- `ledger` owns one board item on `Payments`, "Fix login", with a known revision.
- The agent read the `Payments` channel list while the host was linked, earlier today. The read's answer is on `box-1`.
- Nothing is in the host's Inbox from `box-1` yet. The app on this Mac is closed.
- I have the approval passphrase, and I can log in on `box-1` over SSH.

## Steps

1. **I do:** The agent writes `notes/plan.md` in its clone, then reads it back.

   **I see:** Both calls succeed. The file is there, with the text the agent wrote.

   **State and events:** The write and read happen on `box-1`. The host is not needed.

   **Status:** Works (real daemon). `daemon/humatad/tests/away_boundary.rs::row_1_own_session_files_are_read_and_written_on_the_machine`, with the link away and reachable.

2. **I do:** The agent calls the MCP tool `inbox_ask` with the question "Which schema should the migration use?".

   **I see:** The call returns as pending. On `box-1`, `humata inbox` lists the question. The host shows nothing yet.

   **State and events:** The question is kept in `box-1`'s own journal (AW-3). It is not sent up, because the host is away.

   **Status:** Works (real daemon). `daemon/humatad/tests/away_boundary.rs::row_3_a_question_to_the_user_is_kept_and_journaled_while_the_host_is_away`.

3. **I do:** The agent reads another session's files, `ledger` reads `brin`, in a channel whose rule is Never for that capability.

   **I see:** A denial with the Never kind, and nothing is held.

   **State and events:** The rule says Never, so the call is denied, with or without the host. Nothing is journaled, so nothing is left for the host.

   **Status:** Works (real daemon). `daemon/humatad/tests/away_boundary.rs::row_4_a_call_the_rules_say_never_is_denied_and_nothing_is_kept`.

4. **I do:** With the rule set to Wait, the agent runs `humata access ask-ahead` for `brin`'s files, or types into `brin`.

   **I see:** The call waits. On `box-1`, `humata inbox` shows a pending request for each. The agent's tab shows the wait.

   **State and events:** Each request is held in `box-1`'s Inbox. Nothing is sent to the host until it is back.

   **Status:** Works (real daemon). `daemon/humatad/tests/away_boundary.rs::row_5_under_wait_an_ask_on_this_machine_is_held_in_its_inbox`.

5. **I do:** The rule is changed to Skip. The agent asks ahead for the same files.

   **I see:** The call is refused at once, with the user-away kind. Nothing is held.

   **State and events:** Under Skip, the machine refuses the ask. Nothing is journaled.

   **Status:** Works (real daemon). `daemon/humatad/tests/away_boundary.rs::row_5_under_skip_an_ask_on_this_machine_is_denied_saying_the_user_is_away`. The test checks the error kind, not the wording.

6. **I do:** Under Skip, the agent runs `humata session push` for `main`.

   **I see:** A refusal at once, with the host-unreachable kind, exit 8. Nothing is held.

   **State and events:** The push needs the host, and Skip denies it at once. Nothing is journaled.

   **Status:** Works (real daemon). `daemon/humatad/tests/away_boundary.rs::row_6_under_skip_a_push_is_denied_saying_the_host_is_away`.

7. **I do:** The rule is set to Wait. The agent runs `humata session push` for `main` again.

   **I see:** The call waits. When the host links again, the push shows in the host's Inbox as a pending request, and the agent's call stays held until I answer it.

   **State and events:** The call waits for the host, is judged again when it comes back, and is then journaled to the host as an Inbox request.

   **Status:** Works (real daemon). `daemon/humatad/tests/away_boundary.rs::row_6_under_wait_a_push_waits_for_the_host_and_is_then_asked`.

8. **I do:** The push waits past its deadline, while the host is still away.

   **I see:** The call ends with the host-unreachable kind, exit 8. The agent can call again.

   **State and events:** Nothing is kept for the host.

   **Status:** Works (real daemon). `daemon/humatad/tests/away_boundary.rs::row_6_a_held_call_whose_deadline_passes_while_away_is_host_unreachable_and_keeps_nothing`.

9. **I do:** The agent moves its own item "Fix login" to In progress, against the revision it saw. It also adds an item to the `Payments` board.

   **I see:** Both writes succeed. Each answer says the write was journaled, not applied on the host.

   **State and events:** Both writes are kept in `box-1`'s journal, with the revision the agent saw. Each is replayed when the host is back.

   **Status:** Works (real daemon). `daemon/humatad/tests/away_board_journal.rs::row_3_a_channel_board_add_while_away_is_journaled` and `away_board_journal.rs::a_move_of_its_own_item_while_away_is_journaled_against_the_revision_it_saw`.

10. **I do:** The agent lists the `Payments` channel, the list it read earlier while the host was linked.

    **I see:** Through MCP, the agent gets the same list it got before, with a mark that says when the host answered it. Through the `humata` CLI, the list arrives with no mark.

    **State and events:** The read is answered from `box-1`'s copy of the host's last answer. The mark is the host's response time, and it stays the same after a restart.

    **Status:** Works (real daemon) for the copy and its mark, over gRPC: `daemon/humatad/tests/away_board_journal.rs::row_2_a_read_made_while_the_host_was_linked_is_answered_while_it_is_away_marked_with_its_age`. The CLI prints the age (`cli/tests/boards.rs::a_read_from_a_machines_copy_keeps_its_age`, fakes). No test checks the MCP output itself.

11. **I do:** The agent reads `Payments` for a second channel it never read while the host was linked.

    **I see:** A refusal with the host-unreachable kind, exit 8.

    **State and events:** Nothing is made up. The machine has no copy of that read, so it refuses it.

    **Status:** Works (real daemon). `daemon/humatad/tests/away_board_journal.rs::a_read_never_made_while_the_host_was_linked_is_host_unreachable`.

12. **I do:** This step uses a different setup. `box-1` is linked to the host, but the host has never sent it the rules. I start a session on `box-1`.

    **I see:** The start is refused, with the failed-precondition code. No session is stored, and no tab is started.

    **State and events:** Without a copy of the rules, the machine does not know whether to contain agents, so it starts nothing. A start that waits for the rules goes on once they arrive.

    **Status:** Works (real daemon). `daemon/humatad/tests/away_contain_copy.rs::a_start_before_the_hosts_rules_arrive_is_refused_and_nothing_is_started`. The waiting start is `away_contain_copy.rs::a_start_that_waits_goes_on_once_the_hosts_rules_arrive`.

## Branches

### The link drops while a call waits

**When:** The host link drops while `ledger`'s forwarded call is waiting for the host.

**I do:** Nothing. The link drops.

**I see:** The agent's call ends with the host-unreachable kind. The error says it can be retried, and the retry keeps the same key.

**State and events:** The first try may have reached the host, so the key is kept. A retry with the same key is the same call.

**Expected outcome:** The call ends with a retry the agent can use. No call is lost or made twice.

**Status:** Works (real daemon). `daemon/humatad/tests/away_forward.rs::a_link_that_drops_while_the_call_waits_is_host_unreachable_and_retryable`.

### The agent stops waiting

**When:** `ledger`'s agent stops waiting on a request the host holds, while the host is linked.

**I do:** Stop the agent's wait.

**I see:** Nothing for the agent, which has stopped waiting.

**State and events:** The machine sends the host a cancel for the forwarded wait, with the same call id.

**Expected outcome:** The host learns that the wait ended.

**Status:** Works (real daemon). `daemon/humatad/tests/away_forward.rs::an_agent_that_stops_waiting_has_its_forwarded_wait_cancelled`.

### The link returns, and an answer came in on box-1 first

**When:** While the host is away, I answer a held request on `box-1` with `humata inbox approve`, using the passphrase. Then the host links again, and it has its own answer for the same request.

**I do:** Log in on `box-1` with the passphrase, approve the request there. Later the host answers the same request.

**I see:** On the host, the request shows as answered on `box-1`, credited to me on that machine. The host's later answer is refused, and it says who answered first.

**State and events:** The machine's journal replays in order, one entry per request and answer. The host's acknowledgement ends the resend. The first answer wins.

**Expected outcome:** The first answer stands. Nothing is undone by the later answer.

**Status:** Works (real daemon). `daemon/humatad/tests/away_replay.rs::a_relay_for_a_request_the_machine_already_answered_changes_nothing` and `away_replay.rs::a_relay_for_a_request_the_machine_already_answered_comes_back_refused_saying_who_answered`. The journal order and the resend are in `daemon/humatad/tests/away_inbox.rs::a_request_answered_on_the_machine_while_away_reaches_the_host_in_order` and `away_inbox.rs::entries_the_host_never_acked_are_sent_again_after_a_restart`.

### An edit the host now refuses

**When:** While the host is away, `ledger` edits its item. Meanwhile the host's rule for that item is changed to Never. Then the host links again.

**I do:** Link `box-1` to the host.

**I see:** The item is as it was before the edit. Its history shows the edit as lost. The session shows one summary line, which the app draws under the agent's terminal. Nothing is typed into the agent.

**State and events:** The write is checked again against the host's rules at replay (AW-9). The host keeps the summary until the next link.

**Expected outcome:** The edit is kept as lost, the session says so, and nothing is typed into the agent.

**Status:** Works (real daemon) for the data. `daemon/humatad/tests/away_left_out.rs::an_edit_the_host_now_refuses_is_kept_as_lost_and_the_session_says_so`. The summary line under the terminal is drawn by the app and is not tested here.

### An approved board edit while away

**When:** The rule for `ledger`'s own board asks. The link is cut. The agent's edit waits as a request in `box-1`'s Inbox. I approve it there with the passphrase.

**I do:** Approve the request on `box-1`. Then link the host.

**I see:** The item on the host has changed, and the host's copy of the request shows Approved.

**State and events:** The approved write is journaled with its approval. Replay checks it against the host's mirror of the request.

**Expected outcome:** The approved edit is made once the link is back.

**Status:** Works (real daemon). `daemon/humatad/tests/away_approved_board.rs::an_edit_approved_on_the_box_while_the_host_is_away_is_made_when_the_link_is_back`. The other direction, approving the mirror on the host after the link is back, is `away_approved_board.rs::a_mirror_approved_on_the_host_after_the_link_is_back_makes_the_edit`.

### A comment or view draft while away

**When:** `ledger` writes a comment or a view draft while the host is away.

**I do:** Make the call.

**I see:** A refusal with the host-unreachable kind. Nothing is kept.

**State and events:** Drafts need the host, and AW-7 says they are refused while away, with nothing kept.

**Expected outcome:** Refused. Nothing is saved on `box-1`.

**Status:** Works (real daemon). `daemon/humatad/tests/away_board_journal.rs::a_view_draft_while_away_is_host_unreachable_and_nothing_is_kept`.

### The agent reads a copy with the CLI

**When:** The agent runs `humata` in its tab, not MCP, and reads a board list while the host is away.

**I do:** Run `humata` to list `Payments` items.

**I see:** The list, with no mark that says when the host answered it.

**State and events:** The machine answers from its copy and sends the age in the `humata-stale-as-of` header. The CLI's transport keeps only the body (`cli/src/live.rs:110-117`), so the header never reaches the output.

**Expected outcome:** AW-7 says a read from the copy is marked with the time it was made. The CLI should show that mark, as the MCP tool does.

**Status:** Fakes only (fixed): `cli/tests/boards.rs::a_read_from_a_machines_copy_keeps_its_age` and `cli/tests/boards.rs::a_live_read_carries_no_age`. The CLI keeps the `humata-stale-as-of` header and prints `stale_as_of` (`cli/src/live.rs`, `cli/src/output.rs`). The daemon's header is proven by `daemon/humatad/tests/away_board_journal.rs::row_2_a_read_made_while_the_host_was_linked_is_answered_while_it_is_away_marked_with_its_age`. No test runs the real CLI against an away machine. (Was a likely bug; fixed by BUGS.)

## Expected outcomes

- [ ] Writes to the agent's own files work with the host away or linked.
- [ ] A question to me is kept and journaled, and the host sees nothing until it is back.
- [ ] A call the rules say Never is denied, and nothing is kept.
- [ ] An ask under Wait is held in `box-1`'s Inbox, and the agent waits.
- [ ] An ask under Skip is refused at once, and nothing is journaled.
- [ ] A push under Skip is refused at once. A push under Wait waits for the host, then is asked.
- [ ] A held call whose deadline passes ends with host-unreachable, and nothing is kept.
- [ ] Board writes the rules allow are journaled against the revision the agent saw.
- [ ] A read the machine has a copy of is answered from it, with its age (through MCP; see the CLI bug).
- [ ] A read the machine never had is refused with host-unreachable.
- [ ] A start before `box-1` has the host's rules is refused, and nothing starts.
- [ ] When the link drops during a call, the call ends with a retry the agent can use.
- [ ] The first answer to a request wins, wherever it came from.
- [ ] An edit the host refuses on replay is kept as lost, and the session says so.
- [x] The CLI shows the age of a copied read (`cli/tests/boards.rs::a_read_from_a_machines_copy_keeps_its_age`).

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. Own files read and written | Works (real daemon) | `daemon/humatad/tests/away_boundary.rs::row_1_own_session_files_are_read_and_written_on_the_machine` |
| 2. Question to me, kept and journaled | Works (real daemon) | `away_boundary.rs::row_3_a_question_to_the_user_is_kept_and_journaled_while_the_host_is_away` |
| 3. Never call denied | Works (real daemon) | `away_boundary.rs::row_4_a_call_the_rules_say_never_is_denied_and_nothing_is_kept` |
| 4. Ask under Wait held in the box's Inbox | Works (real daemon) | `away_boundary.rs::row_5_under_wait_an_ask_on_this_machine_is_held_in_its_inbox` |
| 5. Ask under Skip denied | Works (real daemon) | `away_boundary.rs::row_5_under_skip_an_ask_on_this_machine_is_denied_saying_the_user_is_away` (kind only) |
| 6. Push under Skip denied | Works (real daemon) | `away_boundary.rs::row_6_under_skip_a_push_is_denied_saying_the_host_is_away` |
| 7. Push under Wait waits, then asked | Works (real daemon) | `away_boundary.rs::row_6_under_wait_a_push_waits_for_the_host_and_is_then_asked` |
| 8. Held call past its deadline | Works (real daemon) | `away_boundary.rs::row_6_a_held_call_whose_deadline_passes_while_away_is_host_unreachable_and_keeps_nothing` |
| 9. Board writes journaled | Works (real daemon) | `away_board_journal.rs::row_3_a_channel_board_add_while_away_is_journaled`; `a_move_of_its_own_item_while_away_is_journaled_against_the_revision_it_saw` |
| 10. Read answered from the copy, with age | Works (real daemon) over gRPC; MCP output untested; CLI Fakes only | `daemon/humatad/tests/away_board_journal.rs::row_2_a_read_made_while_the_host_was_linked_is_answered_while_it_is_away_marked_with_its_age`; `cli/tests/boards.rs::a_read_from_a_machines_copy_keeps_its_age` |
| 11. Read never made while linked | Works (real daemon) | `away_board_journal.rs::a_read_never_made_while_the_host_was_linked_is_host_unreachable` |
| 12. Start before the rules arrive refused (linked, no copy yet) | Works (real daemon) | `away_contain_copy.rs::a_start_before_the_hosts_rules_arrive_is_refused_and_nothing_is_started` |
| Branch: link drops while waiting | Works (real daemon) | `away_forward.rs::a_link_that_drops_while_the_call_waits_is_host_unreachable_and_retryable` |
| Branch: agent stops waiting | Works (real daemon) | `away_forward.rs::an_agent_that_stops_waiting_has_its_forwarded_wait_cancelled` |
| Branch: relink, first answer wins | Works (real daemon) | `away_replay.rs::a_relay_for_a_request_the_machine_already_answered_changes_nothing`; `away_inbox.rs::entries_the_host_never_acked_are_sent_again_after_a_restart` |
| Branch: edit the host refuses | Works (real daemon) for data; summary line is UI, not tested | `away_left_out.rs::an_edit_the_host_now_refuses_is_kept_as_lost_and_the_session_says_so` |
| Branch: approved board edit | Works (real daemon) | `away_approved_board.rs::an_edit_approved_on_the_box_while_the_host_is_away_is_made_when_the_link_is_back` |
| Branch: comment or view draft | Works (real daemon) | `away_board_journal.rs::a_view_draft_while_away_is_host_unreachable_and_nothing_is_kept` |
| Branch: CLI read with age | Fakes only (fixed) | `cli/tests/boards.rs::a_read_from_a_machines_copy_keeps_its_age`; `cli/tests/boards.rs::a_live_read_carries_no_age` |

## Gaps and likely bugs

**Not built**

- None found for these rows. AWAY is marked closed in `workflow/milestones/AWAY.md`, and its open lines are about the tests, not the features.

**Likely bugs**

- Fixed (was a likely bug): the CLI prints the age of a copied read (`cli/tests/boards.rs::a_read_from_a_machines_copy_keeps_its_age`).
- Under Skip, a push is refused with the host-unreachable kind, and an ask is refused with the user-away kind. AW-4 and AW-7 say the refusal should say the user or host is away. The tests check the kinds, not the words, so the words are untested.
