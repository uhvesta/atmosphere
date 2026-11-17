# 4. Writing a proposal doc with an agent

> "Hey, I'm writing a doc for a proposal."

I want to roll out a new index type on the ledger tables, and I need a short proposal for the team. The agent writes the first draft and does the digging. I read it like a page and comment on its lines as I would on a pull request, and it rewrites. There are no code changes, so no channel and no branch: this lives on the Desk.

How to read this file, and what the tags mean: [README](README.md).

## Starting state

- Humata is open. The Desk has no open sessions.
- A folder `~/Documents/proposals/` exists and isn't inside `~/Humata/`.
- `#ledger` exists and has an empty `shared/`.

## Steps

1. **Do:** on the Desk, click **New desk session**. Title it "Index rollout proposal". Pick **A folder on the machine** and choose `~/Documents/proposals/`. Click **Start**.
   **See:** the hint "Pick one. Humata never deletes it." Then the session page.
   **Expect:**
   - The agent runs in `~/Documents/proposals/`.
   - The folder must exist and sit outside Humata's folders.
   - It is never deleted, even after Close and Clean up.
   **Status: Works** in the daemon: `daemon/humatad/tests/desk.rs::a_picked_folder_is_used_where_it_is`, `::a_picked_folder_must_exist_and_sit_outside_humata` and `::a_picked_folder_is_never_deleted`. The dialog is **Fakes only**: `desktop/tests/native/desk.rs::a_new_desk_session_starts_from_the_form_and_opens`.

2. **Do:** type "Draft `rfc-index.md`: why, the plan, the risks, and a table of table sizes."
   **See:** the agent writes the file. The Desk page lists it under **Documents**.
   **Status: Works** in the daemon: `daemon/humatad/tests/desk.rs::markdown_in_the_folder_is_listed_and_copies_into_a_channels_shared_folder`. The list is **Fakes only**: `desktop/tests/native/desk.rs::the_desk_page_lists_open_documents_and_closed`.

3. **Do:** open `rfc-index.md` from **Files**, then drag its tab into a second group beside the terminal.
   **See:** the page on the right in **Preview**, with its table and diagram drawn, and the agent's terminal on the left (frame 39).
   **Status:**
   - **Works**: `desktop/tests/live_flows/review.rs::a_document_is_commented_on_rewritten_and_its_comments_followed`, steps 1–2. But this runs in a channel session, not on the Desk, and moves the tab with an app action rather than a drag.
   - On the Desk: **Fakes only**, `desktop/tests/native/document.rs::a_desk_document_opens_as_a_page_beside_the_files_panel` and `::moving_the_document_tab_draws_both_groups_and_moving_it_back_joins_them`.

4. **Do:** switch between **Preview**, **Source** and **Side by side**. Scroll the source.
   **See:** in Side by side, the page scrolls with the source while **Scrolling is linked** is ticked. Scrolling one side by hand unticks it.
   **Status: Works** for Side by side (same live test). The rest is **Fakes only**: `document.rs::the_mode_switch_changes_what_is_drawn` and `::link_scrolling_unticks_and_ticks_again`.

5. **Do:** click line 7 in the source, type "Where are these sizes from?" and click **Comment**, while the agent is mid-turn.
   **See:** the comment shows as waiting, then as sent. The page marks where it is.
   **Expect:**
   - It's typed into the terminal at once as `[comment on rfc-index.md:7]` and my text.
   - Events: Comment created (Queued), then updated (Sent) once the agent takes it in.
   **Status: Works**, same live test, step 3. The clicks are **Fakes only**: `document.rs::clicking_a_source_line_opens_the_comment_box_on_it` and `::typing_in_the_box_and_pressing_comment_sends_it_to_the_agent`.

6. **Do:** write two more comments, one on a range (shift-click its last line), and leave them as drafts. Click **Send 2 comments**.
   **See:** the drafts wait in the link bar until I send them.
   **Expect:** both arrive in one paste. Events: two Comments created (Draft), then updated (Sent).
   **Status: Works**, same live test, step 4. The range and drafts are **Fakes only**: `document.rs::shift_clicking_a_second_line_stretches_the_box_to_a_range` and `::drafts_wait_in_the_link_bar_until_send_types_them_together`.

7. **Do:** wait for the agent to rewrite the doc.
   **See:**
   - The page updates in place.
   - The sizes comment shows as addressed, with its lines before and after.
   - The comment on the diagram, which the agent removed, offers **Re-attach** or **Drop**.
   **Expect:** Events: Comment updated (Addressed); Comment updated (its lines gone).
   **Status: Works**, same live test, step 5. In the test, the test writes the new file itself, standing in for the agent.

8. **Do:** click **Resolve** on the addressed comment. Re-attach the other to the new paragraph and send it again.
   **See:** the first is resolved. The second is a draft on its new line, then sent.
   **Expect:** Events: Comment updated (Resolved); Comment updated (Draft on its new line, then Sent).
   **Status: Works**, same live test, step 6. The agent can also reply in a comment's thread and resolve it with a reply: **Works** in the daemon, `daemon/humatad/tests/comment_tools.rs::an_agent_finds_replies_to_and_resolves_a_comment_sent_to_its_session` and `::resolving_needs_a_reply_and_a_comment_that_was_sent`.

9. **Do:** on the Desk page, click **Copy to a channel…** on `rfc-index.md` and pick `#ledger`.
   **Expect:** `#ledger/shared/rfc-index.md` exists. A file already there with that name is never overwritten.
   **Status: Works** in the daemon: `daemon/humatad/tests/desk.rs::markdown_in_the_folder_is_listed_and_copies_into_a_channels_shared_folder` and `::a_copy_never_overwrites_the_channels_file`. **Fakes only** in the app: `desktop/tests/native/desk.rs::a_document_is_copied_into_a_channels_shared_folder`.

10. **Do:** send it to the team.
    **See:** I copy the Markdown out, or push it to the docs repo from a channel session (flow 2, step 13).
    **Status:** sharing a doc for others to comment on, or exporting it to another format, is **Not built, no milestone.** Humata is one person's app, so comments are between me and my agents.

11. **Do:** the team agrees. Move this Desk session into `#ledger` so its conversation goes with the work.
    **Status: Not built: ROAM** (DK-15–DK-19, Move into a channel). Flow 5 picks up from here.

## Expected outcome

`rfc-index.md` sits in `~/Documents/proposals/` and in `#ledger/shared/`. Every comment I wrote is resolved, or re-attached and sent. The Desk session can close, and the folder stays because I picked it.

## Gaps this flow shows

- The best-proven flow here (26.20) runs live in a channel session, while the spec puts it on the Desk. The Desk version has never run against a real daemon.
- Dragging tabs, clicking lines and shift-click ranges are proven only on fakes. The live test sends app actions instead.
- The live test's "agent rewriting the doc" is the test writing the file. No real agent CLI has rewritten a doc and resolved comments through Humata.
- A finished proposal can't move into the channel that will do the work (ROAM).
- There's no way to share a doc for others' comments. No milestone, by design.
