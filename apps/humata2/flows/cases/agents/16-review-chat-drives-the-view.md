# The review chat moves my view, leaves notes and orders the review, and I can undo its moves or turn its controls off

Layer: agents. Platforms: macOS for the review panel and the chat (the app); Linux where the host runs. The daemon side is the same on both. The app's half of the chat (the view following the agent, the Undo strip, Back, and the switches in the Info panel) is not built yet, so those steps are marked as such.

## Starting state

- This Mac runs the host daemon. The machine is `this-mac`.
- A review session `review-api-42` is open on `this-mac`, with the repo `acme/api`. Its diff is against uncommitted changes, and `refund.rs` has one changed line.
- The review panel shows the diff of `refund.rs`, with my cursor on line 1.
- The review session's agent runs in its terminal tab. That tab is the review chat (PR-22). The agent holds its session token.
- The agent's three switches are on: Focus, Notes and Order. That is the default.
- Nothing has been noted or ordered yet.

## Steps

1. **I do:** type "where is the retry logic?" into the review chat, and press Enter.
   **I see (expected):** the agent answers in the chat terminal, and it says where the retry logic is.
   **State and events:** the agent's reply is text in its own terminal. The daemon records nothing for it.
   **Status:** Code, no test. The review session's terminal is the agent's (PR-22), but no test runs a real agent that answers a question in the chat.

2. **I do:** nothing. The agent calls `review_focus` on `refund.rs`, lines 3 to 4.
   **I see (daemon):** the call is applied. The review's open file is `refund.rs`, and lines 3 to 4 are the view's visible lines.
   **State and events:** the view moves. A change is recorded with an undo id, and the change is sent on the feed that the app watches.
   **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off` (the feed gets the undo id) and `daemon/humatad/tests/mcp.rs::the_review_chats_agent_shows_the_user_a_place_and_reads_it_back` (the MCP call).

3. **I do:** look at the review panel and the chat.
   **I see (expected, PR-24):** the panel moves to `refund.rs` at lines 3 to 4, and the chat shows a line "● review_focus refund.rs:3" with an Undo button. The move waits if I am scrolling, typing or selecting.
   **State and events:** the app applies the change it was sent.
   **Status:** Not built (PR milestone, B2 and C2). No app crate calls `WatchReviewChanges` or `FocusReview` (grep of `clients/` and `features/`). `workflow/milestones/PR.md` lists the chat follower as open.

4. **I do:** nothing. The agent calls `review_context` to read what I see.
   **I see (agent):** the open file `refund.rs`, the visible lines, and my selection as text. The selection is quoted.
   **State and events:** a read only. Nothing changes.
   **Status:** Works (real daemon). `daemon/humatad/tests/mcp.rs::the_review_chats_agent_shows_the_user_a_place_and_reads_it_back` (the selection comes back as `["2"]`), and `daemon/humatad/tests/review.rs::the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off` (the selection is quoted).

5. **I do:** nothing. The agent calls `review_focus` on `README.md`, which the diff does not show.
   **I see (agent):** the call fails. It says only files in the diff can be shown.
   **State and events:** the view does not move.
   **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off` (FailedPrecondition) and `daemon/humatad/tests/mcp.rs::the_review_chats_agent_shows_the_user_a_place_and_reads_it_back` (isError).

6. **I do:** nothing. The agent calls `review_note` on `refund.rs` at the hunk that holds line 2, with the text "why 2?".
   **I see (agent):** the reply carries a note id. In the app, the note should sit under line 2 marked with the agent's name (PR-23).
   **State and events:** one note exists, written by the agent, anchored in its clone, on a file the last diff showed. It is never posted.
   **Status:** Works (real daemon) for the note: `daemon/humatad/tests/mcp.rs::the_review_chats_agent_leaves_notes_and_orders_the_review`. The display under the line with the agent's name is Not built (PR milestone; the comments model has no agent author yet, `workflow/milestones/PR.md` pick 2).

7. **I do:** nothing. The agent calls `review_note_edit` to change the note's text to "why a digit?", and `review_note_drop` on a second note it made.
   **I see (agent):** the edit is applied, and the drop reports `dropped: true`. The notes list then has one note with the new text.
   **State and events:** the edited note keeps its id. The dropped note is gone.
   **Status:** Works (real daemon). `daemon/humatad/tests/mcp.rs::the_review_chats_agent_leaves_notes_and_orders_the_review`.

8. **I do:** nothing. The agent calls `review_prepare` with a title, an order (`refund.rs` first), and a note on line 2. The review chat is open.
   **I see (agent):** the reply says `applied: true`. The order is saved with the review.
   **State and events:** the order and the note are saved as part of the review. The change applies at once, without an offer, because the user asked for it in the chat (D59).
   **Status:** Works (real daemon). `daemon/humatad/tests/mcp.rs::the_review_chats_agent_leaves_notes_and_orders_the_review` and `daemon/humatad/tests/review.rs::a_plan_in_an_open_review_chat_orders_the_review_and_leaves_its_notes`. The check for the Order switch is at `daemon/humatad/src/review.rs:1106`.

9. **I do:** look at the Diff panel.
   **I see (expected, RV-32):** the files are in the agent's order, and the group title is the agent's.
   **State and events:** the app shows the saved order.
   **Status:** Not built (PR milestone, C2; VIEWS P8 for the order in the Diff panel). No app crate calls the chat's order.

10. **I do:** ask the chat "what notes did you leave?"
    **I see (agent):** the agent lists its notes, grouped by file, each with its anchor and whether it is outdated.
    **State and events:** a read only.
    **Status:** Works (real daemon) for the list: `daemon/humatad/tests/review.rs::list_comments_gives_the_review_chats_notes_by_file` (this test calls the comment service, not the MCP tool). The MCP `comments_list` tool is dispatched in `daemon/humatad/src/mcp.rs`, but no MCP test calls it.

11. **I do:** click Undo on the focus line in the chat.
    **I see (expected, PR-24):** the view goes back to where it was before the agent's focus. The chat line shows it as undone.
    **State and events:** the change is undone. Its undo id is recorded, and the view is as it was.
    **Status:** Works (real daemon) for the undo: `daemon/humatad/tests/review.rs::the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off` (after the undo, no open file). The Undo button is Not built (PR C2).

12. **I do:** open the review session's Info panel and turn off "Agent may: Focus".
    **I see (expected, PR-25):** the switch is off. The agent's next `review_focus` is refused with a message that the user turned it off.
    **State and events:** the switches are saved with the review. Focus is denied with the source `ReviewSwitchOff`.
    **Status:** Works (real daemon) for the daemon side: `daemon/humatad/tests/review.rs::the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off` (PermissionDenied with `ReviewSwitchOff`). The Info panel switches are Not built: the Info panel shows only "What this agent may do", and no app crate calls `SetAgentSwitches`.

13. **I do:** turn Notes off, too.
    **I see (agent):** its `review_note` call is refused, and the reply says the user turned notes off.
    **State and events:** no note is written.
    **Status:** Works (real daemon). `daemon/humatad/tests/mcp.rs::the_review_chats_agent_leaves_notes_and_orders_the_review` (isError after notes are switched off).

14. **I do:** push a new commit that changes the lines the note quotes, and open the review again.
    **I see (agent and app):** the note shows as outdated. A note whose lines only moved stays and moves with them.
    **State and events:** each diff marks notes outdated where their quoted lines changed, and moves the others (RV-34, PR-26).
    **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::a_later_diff_marks_notes_whose_lines_changed_outdated_and_moves_the_rest`. The app's outdated mark is not checked by that test.

15. **I do:** restart the host while the review has its view, notes, order and switches.
    **I see:** after the restart, the view, the notes, the order and the switches are all still there. Undo still undoes the same move, and a repeated key does not make a second note.
    **State and events:** the review is saved. Note ids are never reused.
    **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::undo_and_a_repeated_key_act_the_same_after_a_restart` and `daemon/humatad/tests/review.rs::the_agents_notes_are_kept_through_a_restart_and_never_reuse_an_id`.

## Branches

### The chat never reviews on its own

- **When:** the agent is asked to mark the diff as reviewed, or to give its own verdict.
- **I do:** the agent calls the diff's mark-reviewed.
- **I see:** the call is refused as user only. The agent gives no verdict.
- **Expected outcome:** only I decide what I have reviewed, and the review is not marked.
- **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::the_sessions_agent_may_read_and_focus_but_not_change_the_users_review_state`. The same test checks that the agent can read its own diff and the review context, and can focus.

### A plan outside an open review chat is a draft

- **When:** the agent calls `review_prepare` before I have a review chat open.
- **I do:** nothing.
- **I see (agent):** the plan is a draft view, not applied. It is offered to me later, as a view (VW-2).
- **Expected outcome:** the order does not change until the chat is open, and the draft waits for my answer.
- **Status:** Works (real daemon). `daemon/humatad/tests/mcp.rs::the_review_chats_agent_leaves_notes_and_orders_the_review` (state `draft`, then `applied: true` once the chat reports its view).

### Notes go only on files the diff shows

- **When:** the agent leaves a note on a file the last diff did not show.
- **I do:** the agent calls `review_note` on that file.
- **I see:** the note is refused with FailedPrecondition.
- **Expected outcome:** no note is kept for a file outside the diff.
- **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::the_agents_notes_are_kept_through_a_restart_and_never_reuse_an_id` (the first check in that test).

### The Order switch is off

- **When:** I turn Order off, and the agent plans the review.
- **I do:** the agent calls `review_prepare` with an order.
- **I see:** the call is refused, and the reply says the user turned it off.
- **Expected outcome:** the order does not change.
- **Status:** Code, no test. The check is `daemon/humatad/src/review.rs:1106` (`require_use(ReviewOrder)`). The tests above switch Focus and Notes off, not Order.

### The agent hears of notes on every session it may list

- **When:** the agent's session may read another session's notes under its rules.
- **I do:** a note is made in that other session.
- **I see:** the agent's feed carries the note.
- **Expected outcome:** the agent hears only about sessions it may list.
- **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::an_agent_hears_of_notes_on_every_session_it_may_list_them_for`.

### The feed tells of each note change, but not again after a restart

- **When:** a note is made, changed, dropped or undone, and then the host restarts.
- **I do:** make a note change, restart, and read the feed.
- **I see:** each change is told once. After the restart, the old changes are not told again.
- **Expected outcome:** the feed's position survives the restart.
- **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::the_feed_tells_of_each_note_made_changed_dropped_and_undone_but_not_again_after_a_restart`.

### Resolve, reopen or reattach a note

- **When:** I want to resolve a note the agent made, reopen it, or reattach it to a new line.
- **I do:** use Resolve, Reopen or Reattach on the note.
- **I see:** the call fails with "not built" for a note.
- **Expected outcome:** the note keeps its state until these are built.
- **Status:** Not built (no milestone). `daemon/humatad/src/review.rs:981` (Reattach), `:998` (Resolve) and `:1007` (Reopen) return `not_built` when the id is a note, not a user comment.

### The agent's notes are switched off

- **When:** I turn Notes off after the agent has left notes.
- **I do:** turn Notes off, then read the notes.
- **I see:** the agent's next note is refused.
- **Expected outcome:** no new note is written while the switch is off.
- **Status:** Works (real daemon) for the refusal: `daemon/humatad/tests/review.rs::notes_are_the_agents_and_the_user_can_turn_them_off` (PermissionDenied, and no note is kept) and `daemon/humatad/tests/mcp.rs::the_review_chats_agent_leaves_notes_and_orders_the_review`. Keeping notes the agent left before the switch is off is not tested: that test makes no note first.

## Expected outcomes

- [ ] The agent can read my review and my selection, and can move my view to a file and line in the diff.
- [ ] A move to a file the diff does not show is refused.
- [ ] The agent's notes are the agent's, anchored in its clone, and only on files the diff shows.
- [ ] A plan in an open review chat applies at once, with its order and notes.
- [ ] A plan outside an open review chat is a draft, and waits for my answer.
- [ ] The agent can never mark a diff reviewed, or change my switches.
- [ ] Turning Focus or Notes off refuses the agent's call, and the reply says I turned it off.
- [ ] A later diff marks notes outdated where their lines changed, and moves the rest.
- [ ] Undo puts the view back, and a repeated key makes the same change once.
- [ ] After a restart, the view, notes, order, switches and undo all survive.
- [ ] The feed tells each note change once, and not again after a restart.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Ask the chat a question | Code, no test | PR-22; no test with a real agent answering |
| 2. Agent `review_focus` moves the view | Works (real daemon) | `review.rs::the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off`; `mcp.rs::the_review_chats_agent_shows_the_user_a_place_and_reads_it_back` |
| 3. The app's panel and Undo strip follow | Not built (PR, B2 and C2) | no app crate calls `WatchReviewChanges` or `FocusReview` |
| 4. Agent `review_context` reads my selection | Works (real daemon) | `mcp.rs::the_review_chats_agent_shows_the_user_a_place_and_reads_it_back` |
| 5. Focus on a file outside the diff is refused | Works (real daemon) | `review.rs::the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off` |
| 6. Agent `review_note` | Works (real daemon); display Not built | `mcp.rs::the_review_chats_agent_leaves_notes_and_orders_the_review` |
| 7. `review_note_edit` and `review_note_drop` | Works (real daemon) | `mcp.rs::the_review_chats_agent_leaves_notes_and_orders_the_review` |
| 8. `review_prepare` applies in an open chat | Works (real daemon) | `mcp.rs::the_review_chats_agent_leaves_notes_and_orders_the_review`; `review.rs::a_plan_in_an_open_review_chat_orders_the_review_and_leaves_its_notes` |
| 9. The Diff panel shows the order | Not built (PR, C2; VIEWS P8) | no app crate calls the chat's order |
| 10. Agent lists its notes by file | Works (real daemon) via gRPC | `review.rs::list_comments_gives_the_review_chats_notes_by_file`; MCP `comments_list` has no test |
| 11. Undo of a focus | Works (real daemon); Undo button Not built (PR, C2) | `review.rs::the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off` |
| 12. Focus switch off | Works (real daemon); Info panel Not built (PR-25) | `review.rs::the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off` |
| 13. Notes switch off | Works (real daemon) | `mcp.rs::the_review_chats_agent_leaves_notes_and_orders_the_review` |
| 14. A later diff marks notes outdated | Works (real daemon) | `review.rs::a_later_diff_marks_notes_whose_lines_changed_outdated_and_moves_the_rest` |
| 15. A restart keeps the review | Works (real daemon) | `review.rs::undo_and_a_repeated_key_act_the_same_after_a_restart`; `review.rs::the_agents_notes_are_kept_through_a_restart_and_never_reuse_an_id` |
| Branch: chat never reviews | Works (real daemon) | `review.rs::the_sessions_agent_may_read_and_focus_but_not_change_the_users_review_state` |
| Branch: plan outside a chat is a draft | Works (real daemon) | `mcp.rs::the_review_chats_agent_leaves_notes_and_orders_the_review` |
| Branch: notes only on files in the diff | Works (real daemon) | `review.rs::the_agents_notes_are_kept_through_a_restart_and_never_reuse_an_id` |
| Branch: Order switch off | Code, no test | `review.rs:1106` |
| Branch: agent hears of notes it may list | Works (real daemon) | `review.rs::an_agent_hears_of_notes_on_every_session_it_may_list_them_for` |
| Branch: feed not repeated after restart | Works (real daemon) | `review.rs::the_feed_tells_of_each_note_made_changed_dropped_and_undone_but_not_again_after_a_restart` |
| Branch: Resolve, Reopen, Reattach a note | Not built (no milestone) | `review.rs:981`, `:998`, `:1007` |
| Branch: Notes switch off, earlier notes kept | Works (real daemon) for the refusal; keeping earlier notes Code, no test | `review.rs::notes_are_the_agents_and_the_user_can_turn_them_off` |

## Gaps and likely bugs

- **Not built (PR milestone, open, lane PR):** the app's half of the review chat. That is the chat follower that moves my view (B2, C2), the Undo strip and Back, the Info panel switches "Agent may: Focus, Notes, Order" (PR-25), the agent's note display with its name (pick 2), and the Order in the Diff panel. No app crate calls `WatchReviewChanges`, `UndoReviewChange`, `FocusReview`, `SetAgentSwitches` or the chat's order.
- **Not built (no milestone):** Resolve, Reopen and Reattach for notes (`daemon/humatad/src/review.rs:981`, `:998`, `:1007`). PR-26 says notes persist, and it does not name a resolve.
- **Test gap:** the Order switch has no test that turns it off.
- **Test gap:** the MCP `comments_list` tool has no test. The list is tested only through the gRPC comment service.
- **Test gap:** the note and the app's view moving are not tested together. The only live flow for review, `desktop/tests/live_flows/review.rs`, tests the diff and the user's comments, not the chat.
- **Likely bugs:** none found in the daemon code for this case. The open items above are missing features, not wrong behaviour.
