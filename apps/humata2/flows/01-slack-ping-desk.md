# 1. A ping on Slack: look into it on the Desk

> "Hey, I got pinged on Slack. Let me make a desk and see what type of stuff this could be."

I don't know yet if this is a real task, so I don't want a channel, a branch or a board. I want a scratch place where an agent can poke around, and that cleans itself up if it turns out to be nothing.

How to read this file, and what the tags mean: [README](README.md).

## Starting state

- Humata is open on Home. This Mac's daemon is running.
- The Desk has no open sessions. `~/Humata/desk/` has no folder called `checkout-errors-from-slack`.
- `#payments` exists, with `acme/payments` cloned. devbox-1 is a linked machine and online.
- A Slack thread about checkout errors is on my clipboard.

## Steps

1. **Do:** click **Desk** in the sidebar.
   **See:** the Desk page (frame 36), with **Open · 0**, **Documents** and **Closed · scratch folders go after 7 days**, and **Sort: last used**, **Clean up…** and **New desk session** at the top.
   **Expect:** nothing changes on the daemon. The page reads the Desk list.
   **Status: Fakes only.** `desktop/tests/native/desk.rs::the_desk_page_lists_open_documents_and_closed`. The live client lists real Desk sessions (`clients/live/tests/desk.rs::a_started_session_is_on_the_desk_on_real`), but no test opens this page on a real daemon.

2. **Do:** click **New desk session**. Type the title "Checkout errors from Slack". Leave **Runs** on Claude Code, **Machine** on This Mac and the folder on **A new scratch folder**. Leave **Repos** empty. Click **Start**.
   **See:** the dialog (frame 40) shows the folder it will use, then "Starting…". The session page opens.
   **Expect:**
   - The folder `~/Humata/desk/checkout-errors-from-slack` exists, and the agent runs in it.
   - A taken name gets the next suffix (`…-2`).
   - The Desk list has one open session.
   - Event: none. Desk sessions have no event of their own, so the Desk page reloads its list instead of hearing about it.
   **Status:** the dialog is **Fakes only**: `desktop/tests/native/desk.rs::a_new_desk_session_starts_from_the_form_and_opens`. The start itself **Works** in the daemon: `daemon/humatad/tests/desk.rs::a_scratch_session_runs_its_agent_in_a_folder_named_from_its_title` and `::a_taken_name_gets_the_next_suffix`.

3. **Do:** look at the session page.
   **See:**
   - The agent's own TUI in the main tab.
   - A folder bar above the tabs (frame 37): the folder with a "scratch" chip, a machine picker, **Add repo** and **Keep folder**.
   - No **Fork** button.
   - The sidebar lists the session under Desk, not under a channel.
   **Expect:** the session has no branch and no board.
   **Status: Fakes only.** `desktop/tests/native/desk.rs::a_desk_sessions_page_has_the_folder_bar_and_no_fork` and `::the_sidebar_lists_desk_sessions_only_on_the_desk`.

4. **Do:** click into the terminal, press ⌘V, then Return.
   **See:** the thread has more than one line, so the terminal asks before it sends the paste. I accept, and the agent starts reading.
   **Expect:**
   - The text reaches the agent once, bracketed as a paste.
   - The session goes Working, then Idle or Needs input when its turn ends.
   - Event: Session updated (status), on the sessions topic.
   **Status:**
   - Typing: **Works.** `clients/live/tests/parity.rs::typed_text_reaches_the_agent_on_real` (real daemon and holder, scripted agent).
   - Paste: **Fakes only.** It's proven only in the terminal component: `terminal/pane/tests/pane.rs::a_one_line_paste_is_sent_and_bracketed_when_asked` and `::a_multi_line_paste_waits_for_the_user`. No app test pastes into a session tab. TERM owns this, and LOOK2 notes Edit › Copy does nothing while a terminal has focus.
   - Taking the message from Slack directly: **Not built, no milestone.** Copy and paste is the only way in.

5. **Do:** the agent says it needs the payments code. Click **Add repo** and type `acme/payments`.
   **See:** a chip for `acme/payments` appears in the folder bar.
   **Expect:**
   - The default branch is cloned into the folder. A Desk session gets no branch of its own.
   - Adding the same repo again changes nothing.
   - Event: none (the answer is the updated Desk session).
   **Status: Works** in the daemon: `daemon/humatad/tests/desk.rs::an_added_repo_is_cloned_and_listed_and_a_second_add_changes_nothing`. The field is **Fakes only**: `features/desk/tests/menu.rs::picking_a_machine_or_adding_a_repo_closes_its_field`.

6. **Do:** pick devbox-1 in the folder bar's machine picker, to dig where the logs are.
   **See:** the session moves to devbox-1 and keeps its conversation.
   **Expect:** the folder and conversation are on devbox-1, and the agent is running there.
   **Status: Not built, no milestone.**
   - The fake moves it: `desktop/tests/native/desk.rs::a_desk_session_moves_to_another_machine_from_its_bar`.
   - A real daemon won't even start a Desk session off This Mac: `daemon/humatad/tests/desk.rs::another_machine_is_refused_and_starts_nothing`.
   - The live client answers "not in this release" (`clients/live/src/desk.rs`).
   - D182 deferred this.

7. **Do:** the agent writes `notes.md`. On the Desk page, click **Copy to a channel…** on it and pick `#payments`.
   **See:** `notes.md` is listed under **Documents**, and a note says where it was copied.
   **Expect:**
   - `#payments/shared/notes.md` exists.
   - If a file with that name is already there, the copy is refused and the channel's file stays as it was.
   **Status: Works** in the daemon: `daemon/humatad/tests/desk.rs::markdown_in_the_folder_is_listed_and_copies_into_a_channels_shared_folder` and `::a_copy_never_overwrites_the_channels_file`. **Fakes only** in the app: `desktop/tests/native/desk.rs::a_document_is_copied_into_a_channels_shared_folder`.

8. **Do:** it's a real bug. From the session menu, pick **Move into a channel…**, choose `#payments`, check the branch it will get and accept. (Or the agent proposes the move, and I accept it in the Inbox, frame 88.)
   **See:** the session leaves the Desk and shows under `#payments`, with the same conversation, now on its own branch.
   **Expect:** Session created in `#payments`. The Desk session goes.
   **Status: Not built: ROAM** (DK-15–DK-19, frame 89). There's no menu item, dialog or Inbox card. Today: start a channel session and paste the notes in (flow 2).

9. **Do:** if it's nothing, pick **Close session** from the session menu.
   **See:** the session moves under **Closed**, with 7 days left. **Keep** and **Reopen** are on its row.
   **Expect:**
   - The agent stops, and the folder stays, due for the trash in 7 days.
   - **Keep** takes the delete time away.
   - **Reopen** resumes the same conversation in the same folder.
   - Reopening is refused once the folder is gone.
   **Status: Works.** `clients/live/tests/desk.rs::closing_and_reopening_on_real` and `::a_kept_folder_has_no_days_left_on_real`; `daemon/humatad/tests/desk.rs::closing_stops_the_agent_and_keeps_the_folder_for_seven_days`, `::reopening_resumes_the_conversation_in_the_same_folder` and `::reopening_is_refused_when_the_folder_is_gone`. The screen is **Fakes only**: `desktop/tests/native/desk.rs::closing_a_desk_session_lists_it_under_closed` and `::keeping_and_reopening_a_closed_session`.

10. **Do:** weeks later, click **Clean up…**, tick the old closed sessions and click delete.
    **See:** the dialog lists closed sessions (frame 55). A session with unpushed work is named, with why it's kept.
    **Expect:**
    - Only the ticked sessions are deleted.
    - If the list changed since the dialog opened, nothing is deleted and the new list is shown.
    - A picked folder is never deleted.
    **Status: Works.** `clients/live/tests/desk.rs::clean_up_deletes_only_the_ticked_session_on_real` and `::a_stale_clean_up_deletes_nothing_on_real`; `daemon/humatad/tests/desk.rs::work_not_pushed_holds_the_folder_and_says_why` and `::cleaning_up_a_picked_session_leaves_its_folder`. The dialog is **Fakes only**: `desktop/tests/native/desk.rs::cleaning_up_deletes_the_ticked_closed_sessions`.

## Expected outcome

Either the work lives on as a `#payments` session with the same conversation (step 8, not built), or the Desk session is closed and its scratch folder is gone after 7 days. `#payments/shared/` keeps the notes either way.

## Gaps this flow shows

- A Desk session can't become a channel session. That's the natural next step here, and it's ROAM.
- A Desk session only runs on This Mac against a real daemon. Moving one has no milestone.
- Desk sessions have no events. Every Desk screen reloads instead of following changes. If code is to come from events, the Desk needs some.
- Pasting into a session tab has no app test, and copy is broken while a terminal has focus (TERM).
- Every Desk screen is proven only on fakes. The client and daemon work.
- Nothing takes a chat message straight into Humata. No milestone, and not in the spec.
