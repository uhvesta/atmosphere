# 4. Writing a proposal doc with an agent

> "Hey, I'm writing a doc for a proposal."

I want to roll out a new index type on the ledger tables, and I need a short proposal for the team. The agent writes the first draft and does the digging. I read it like a page and comment on its lines as I would on a pull request, and it rewrites. There are no code changes, so no channel and no branch: this lives on the Desk.

How to read this file, and what the tags mean: [README](README.md).

## Starting state

- Humata is open. The Desk has no open sessions.
- `~/Humata/desk/` has no folder called `index-rollout-proposal`.
- `#ledger` exists and has an empty `shared/`.

## Steps

1. **Do:** on the Desk, click **New desk session**. Title it "Index rollout proposal". Leave **Runs** on Claude Code and **Machine** on This Mac. Click **Start**.
   **See:** the dialog asks nothing about a folder and shows no path. Then the session page, with "Not kept · goes 7 days after close" in the folder bar.
   **Expect:**
   - Humata makes `~/Humata/desk/index-rollout-proposal`, and the agent runs in it.
   - The folder is a git repo of its own with no commit yet. Humata writes only that repo's `.git/config`, never the global git config.
   **Status:** the start **Works** in the daemon: `daemon/humatad/tests/desk.rs::a_scratch_session_runs_its_agent_in_a_folder_named_from_its_title`. The dialog with no folder choice, and the repo, are **Not built: DOCV** (DK-14, DV-1). Today's dialog is **Fakes only**: `desktop/tests/native/desk.rs::a_new_desk_session_starts_from_the_form_and_opens`.

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

9. **Do:** ask the agent: "Commit the draft as a version each time you finish a rewrite."
   **See:** the agent runs `git commit` in its terminal. The History strip on the tab shows "v1 of 1", by claude, and the folder bar says "Kept · 1 version".
   **Expect:**
   - A commit in the folder's own repo, made with plain git. No version is made on its own, per turn or otherwise.
   - The folder is kept: Close no longer puts it on a 7-day clock.
   **Status: Not built: DOCV** (DV-2, DV-4, DV-5).

10. **Do:** after the agent's next rewrite and commit, change a sentence myself. Click **Save version**, type "Before review" and save.
    **See:** "Current · unsaved changes" until I save, then "v3 of 3". **History ▾** lists v3 by **You**, v2 and v1 by claude, each with its time, message and lines added and removed.
    **Expect:**
    - One commit holding every change in the folder outside its clones, marked as mine.
    - With nothing changed, **Save version** is disabled and says so.
    **Status: Not built: DOCV** (DV-2, DV-3, DV-5).

11. **Do:** click ◀ twice.
    **See:** v1, read-only, in the same mode, with a bar naming it and **Compare with current**, **Restore as new version** and **Back to current**. Its comments show as they were when v1 was made.
    **Expect:** nothing can be commented, sent, resolved or archived on v1. The current file's comments are untouched.
    **Status: Not built: DOCV** (DV-6, DV-12).

12. **Do:** click **Compare with current**.
    **See:** the diff view, with v1 as the base and the current file on the other side.
    **Status: Not built: DOCV** (DV-7). The diff view itself **Works** (REVIEW).

13. **Do:** step to v2 and click **Restore as new version**.
    **See:** "v4 of 4 · You · Restored v2". v3 is still in the list.
    **Expect:**
    - Unsaved changes, if any, are saved first as their own version ("Saved before restoring v2").
    - Only `rfc-index.md` changes. No commit is removed or rewritten.
    - Its comments re-anchor; none becomes addressed, since the restore is my change.
    **Do (other case):** restart the Mac, or move the session to devbox-1. Every version and its comments are still there (DV-10).
    **Status: Not built: DOCV** (DV-8, DV-10).

14. **Do:** ask the agent: "What changed between v2 and v3, and why?"
    **See:** it answers from the versions.
    **Expect:** it calls `doc_versions_list` and `doc_version_read`. Reading its own folder needs no rule; another session's folder would ask under **Read another session's files**.
    **Status: Not built: DOCV** (DV-9).

15. **Do:** on the Desk page, click **Copy to a channel…** on `rfc-index.md` and pick `#ledger`.
    **Expect:** `#ledger/shared/rfc-index.md` exists. A file already there with that name is never overwritten.
    **Status: Works** in the daemon: `daemon/humatad/tests/desk.rs::markdown_in_the_folder_is_listed_and_copies_into_a_channels_shared_folder` and `::a_copy_never_overwrites_the_channels_file`. **Fakes only** in the app: `desktop/tests/native/desk.rs::a_document_is_copied_into_a_channels_shared_folder`.

16. **Do:** send it to the team.
    **See:** I copy the Markdown out, or push it to the docs repo from a channel session (flow 2, step 13).
    **Status:** sharing a doc for others to comment on, or exporting it to another format, is **Not built, no milestone.** Humata is one person's app, so comments are between me and my agents.

17. **Do:** the team agrees. Move this Desk session into `#ledger` so its conversation goes with the work.
    **Expect:** the folder's versions and their comment records come along, and the History strip keeps working in the channel (DV-10).
    **Status: Not built: ROAM** (DK-15–DK-19, Move into a channel). Flow 5 picks up from here. Carrying versions is **Not built: DOCV** (DV-10).

## Expected outcome

`rfc-index.md` sits in `~/Humata/desk/index-rollout-proposal/`, with every version I or the agent saved, and in `#ledger/shared/`. Every comment I wrote is resolved, or re-attached and sent. The Desk session can close, and the folder stays because it holds versions.

## Gaps this flow shows

- The best-proven flow here (26.20) runs live in a channel session, while the spec puts it on the Desk. The Desk version has never run against a real daemon.
- Dragging tabs, clicking lines and shift-click ranges are proven only on fakes. The live test sends app actions instead.
- The live test's "agent rewriting the doc" is the test writing the file. No real agent CLI has rewritten a doc and resolved comments through Humata.
- A finished proposal can't move into the channel that will do the work (ROAM).
- There's no way to share a doc for others' comments. No milestone, by design.
- Versions aren't built (DOCV, DV-1–DV-13): no Desk folder is a git repo yet, and nothing records comments per version.
- Starting a Desk session in a picked folder still works in the daemon (step 1's old path), but no new session can pick one (D451).
