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
   **See:** the dialog asks nothing about a folder and shows no path. Then the session page, with "Goes 7 days after close" in the folder bar.
   **Expect:**
   - Humata makes `~/Humata/desk/index-rollout-proposal`, and the agent runs in it.
   - The folder is a git repo of its own with no commit yet. Humata writes only that repo's `.git/config`, never the global git config.
   **Status:** the start **Works** in the daemon: `daemon/humatad/tests/desk.rs::a_scratch_session_runs_its_agent_in_a_folder_named_from_its_title`. The folder's own repo with no version **Works** in the daemon: `daemon/humatad/tests/doc_versions.rs::a_new_desk_folder_is_a_repository_with_no_version_yet`. The dialog starts a session on a real daemon (**Works**): `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel`. That the dialog asks nothing about the folder and shows no path is **Fakes only**: `desktop/tests/native/desk.rs::a_new_desk_session_starts_from_the_form_and_opens` (its frame 40 snapshot).

2. **Do:** type "Draft `rfc-index.md`: why, the plan, the risks, and a table of table sizes."
   **See:** the agent writes the file. The Desk page lists it under **Documents**.
   **Status: Works** in the daemon: `daemon/humatad/tests/desk.rs::markdown_in_the_folder_is_listed_and_copies_into_a_channels_shared_folder`. The list **Works**: `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel` (a file in the session's folder is listed under Documents on the Desk page). Fake-client version: `desktop/tests/native/desk.rs::the_desk_page_lists_open_documents_and_closed`.

3. **Do:** open `rfc-index.md` from **Files**, then drag its tab into a second group beside the terminal.
   **See:** the page on the right in **Preview**, with its table and diagram drawn, and the agent's terminal on the left (frame 39).
   **Status:**
   - **Works**: `desktop/tests/live_flows/review.rs::a_document_is_commented_on_rewritten_and_its_comments_followed`, steps 1–2. But this runs in a channel session, not on the Desk, and moves the tab with an app action.
   - Dragging it into a second group **Works**: `desktop/tests/live_flows/document.rs::a_documents_page_is_clicked_through_comments_drafts_rewrite_resolve_and_reattach` drags the tab onto the drop zone beside the terminal (a channel session). The driver finds the zone after the drag starts, since it is drawn only while dragging: `testing/flow/tests/drag.rs`.
   - On the Desk **Works**: `desktop/tests/live_flows/desk_document.rs::a_desk_documents_page_is_clicked_through_comments_drafts_rewrite_resolve_and_reattach` runs steps 3 to 8 by clicks and a tab drag on a Desk session, reading the document from the session's own folder (D382, the SessionFolder file root). Fake-client version: `desktop/tests/native/document.rs::a_desk_document_opens_as_a_page_beside_the_files_panel` and `::moving_the_document_tab_draws_both_groups_and_moving_it_back_joins_them`.

4. **Do:** switch between **Preview**, **Source** and **Side by side**. Scroll the source.
   **See:** in Side by side, the page scrolls with the source while **Scrolling is linked** is ticked. Scrolling one side by hand unticks it.
   **Status: Works.** Clicking Preview, Source and Side by side, and ticking Scrolling is linked off and on: `desktop/tests/live_flows/document.rs::a_documents_page_is_clicked_through_comments_drafts_rewrite_resolve_and_reattach`. Scrolling the page by hand unticks it, in the same test and on the Desk in `desktop/tests/live_flows/desk_document.rs`: the driver turns the wheel over the page (D384). Fake-client: `desktop/tests/native/document.rs::scrolling_the_page_by_hand_unticks_link_scrolling` and `::link_scrolling_unticks_and_ticks_again`.

5. **Do:** click line 7 in the source, type "Where are these sizes from?" and click **Comment**, while the agent is mid-turn.
   **See:** the comment shows as waiting, then as sent. The page marks where it is.
   **Expect:**
   - It's typed into the terminal at once as `[comment on rfc-index.md:7]` and my text.
   - Events: Comment created (Queued), then updated (Sent) once the agent takes it in.
   **Status: Works**, with the clicks: `desktop/tests/live_flows/document.rs::a_documents_page_is_clicked_through_comments_drafts_rewrite_resolve_and_reattach` (click line 7, type, click Comment; it reaches the agent and shows Sent). Also `review.rs::a_document_is_commented_on_rewritten_and_its_comments_followed`, step 3. On the Desk too: see step 3.

6. **Do:** write two more comments, one on a range (shift-click its last line), and leave them as drafts. Click **Send 2 comments**.
   **See:** the drafts wait in the link bar until I send them.
   **Expect:** both arrive in one paste. Events: two Comments created (Draft), then updated (Sent).
   **Status: Works**, with the clicks: `desktop/tests/live_flows/document.rs::a_documents_page_is_clicked_through_comments_drafts_rewrite_resolve_and_reattach` (shift-click stretches the box to a range; both drafts wait, then Send 2 comments types both). On the Desk too: see step 3.

7. **Do:** wait for the agent to rewrite the doc.
   **See:**
   - The page updates in place.
   - The sizes comment shows as addressed, with its lines before and after.
   - The comment on the diagram, which the agent removed, offers **Re-attach** or **Drop**.
   **Expect:** Events: Comment updated (Addressed); Comment updated (its lines gone).
   **Status: Works**, same live test, step 5, and `desktop/tests/live_flows/document.rs::a_documents_page_is_clicked_through_comments_drafts_rewrite_resolve_and_reattach`. In the tests, the test writes the new file itself, standing in for the agent.

8. **Do:** click **Resolve** on the addressed comment. Re-attach the other to the new paragraph and send it again.
   **See:** the first is resolved. The second is a draft on its new line, then sent.
   **Expect:** Events: Comment updated (Resolved); Comment updated (Draft on its new line, then Sent).
   **Status: Works**, same live test, step 6, and with the clicks (Resolve, Re-attach, Send 1 comment) in `desktop/tests/live_flows/document.rs::a_documents_page_is_clicked_through_comments_drafts_rewrite_resolve_and_reattach`. The agent can also reply in a comment's thread and resolve it with a reply: **Works** in the daemon, `daemon/humatad/tests/comment_tools.rs::an_agent_finds_replies_to_and_resolves_a_comment_sent_to_its_session` and `::resolving_needs_a_reply_and_a_comment_that_was_sent`.

9. **Do:** ask the agent: "Commit the draft as a version each time you finish a rewrite."
   **See:** the agent runs `git commit` in its terminal. The History strip on the tab shows "v1 of 1", by claude, and the folder bar says "Kept · 1 version".
   **Expect:**
   - A commit in the folder's own repo, made with plain git. No version is made on its own, per turn or otherwise.
   - The folder is kept: Close no longer puts it on a 7-day clock.
   **Status: Works** in the daemon. A plain `git commit` is the agent's version: `daemon/sessions/src/doc_repo.rs::a_plain_git_commit_is_the_agents_version`. The session hears about it without anyone listing: `daemon/humatad/tests/doc_comments.rs::an_agents_commit_tells_the_session_without_anyone_listing`. It counts its versions: `daemon/humatad/tests/doc_history.rs::the_desk_session_counts_its_versions_as_they_are_saved`. A version keeps the folder: `daemon/humatad/tests/doc_versions.rs::a_saved_version_keeps_the_folder_past_its_days`. The History strip is **Fakes only**: `desktop/tests/native/doc_versions.rs::the_strip_shows_on_a_desk_markdown_tab_with_versions` and `features/session/tests/history.rs::an_agent_commit_moves_doc_head_and_the_versions_are_listed_again`. The "Kept · 1 version" chip is drawn (`ui/src/desk_bar.rs`, `kept_chip`), but no test checks it.

10. **Do:** after the agent's next rewrite and commit, change a sentence myself. Click **Save version**, type "Before review" and save.
    **See:** "Current · unsaved changes" until I save, then "v3 of 3". **History ▾** lists v3 by **You**, v2 and v1 by claude, each with its time, message and lines added and removed.
    **Expect:**
    - One commit holding every change in the folder outside its clones, marked as mine.
    - With nothing changed, **Save version** is disabled and says so.
    **Status: Works** in the daemon:
    - Saving commits every change outside the clones as mine: `daemon/sessions/src/doc_repo.rs::save_commits_every_change_as_yours_and_never_the_clones` and `daemon/humatad/tests/doc_versions.rs::clones_in_the_folder_are_left_out_of_its_versions`.
    - Nothing changed is refused, and a change shows as unsaved: `daemon/humatad/tests/doc_history.rs::saving_with_nothing_changed_is_refused_and_a_change_shows_as_unsaved`.
    - The list is newest first, with who made each version and its lines added and removed: `::a_file_lists_its_versions_newest_first_with_who_made_each`.

    The strip and Save version are **Fakes only**: `desktop/tests/native/doc_versions.rs::save_version_needs_a_change_since_the_newest_version` and `features/session/tests/history.rs::save_version_is_off_without_a_change_and_on_after_one`.

11. **Do:** click ◀ twice.
    **See:** v1, read-only, in the same mode, with a bar naming it and **Compare with current**, **Restore as new version** and **Back to current**. Its comments show as they were when v1 was made.
    **Expect:** nothing can be commented, sent, resolved or archived on v1. The current file's comments are untouched.
    **Status:** an old version reads as it was, and its comments were recorded when it was made. Both **Work** in the daemon: `daemon/humatad/tests/doc_history.rs::an_old_version_reads_as_it_was` and `daemon/humatad/tests/doc_comments.rs::an_agents_commit_records_the_files_comments_and_later_changes_leave_them`. Stepping back, read-only, in the same mode is **Fakes only**: `desktop/tests/native/doc_versions.rs::stepping_back_shows_an_old_version_that_takes_no_comment`, `features/session/tests/history.rs::stepping_back_and_forward_walks_the_versions_and_keeps_the_mode` and `::an_old_version_is_read_only_until_back_to_current`.

12. **Do:** click **Compare with current**.
    **See:** the diff view, with v1 as the base and the current file on the other side.
    **Status:** the daemon's diff of a version against the file **Works** in the daemon: `daemon/humatad/tests/doc_version_diff.rs::a_saved_version_then_an_edit_is_one_removed_and_one_added_line`. Compare with current is **Fakes only**: `desktop/tests/native/doc_versions.rs::compare_with_current_asks_the_diff_view_for_the_shown_version` and `features/session/tests/history.rs::compare_with_current_loads_the_daemons_diff_of_the_version_and_the_file`. The diff view itself **Works** (REVIEW).

13. **Do:** step to v2 and click **Restore as new version**.
    **See:** "v4 of 4 · You · Restored v2". v3 is still in the list.
    **Expect:**
    - Unsaved changes, if any, are saved first as their own version ("Saved before restoring v2").
    - Only `rfc-index.md` changes. No commit is removed or rewritten.
    - Its comments re-anchor; none becomes addressed, since the restore is my change.
    **Do (other case):** restart the Mac, or move the session to devbox-1. Every version and its comments are still there (DV-10).
    **Status:** the restore **Works** in the daemon. It saves unsaved changes first and keeps every version: `daemon/humatad/tests/doc_history.rs::restoring_saves_unsaved_changes_first_and_keeps_every_version`. It changes only that file: `daemon/sessions/src/doc_repo.rs::restore_changes_only_that_file`. The button is **Fakes only**: `desktop/tests/native/doc_versions.rs::restoring_an_old_version_adds_a_version_and_returns_to_current` and `features/session/tests/history.rs::restoring_with_unsaved_changes_saves_them_first`. No test checks that the comments re-anchor after a restore.
    Other case:
    - A version made while the daemon was stopped is recorded when it starts: **Works** in the daemon, `daemon/humatad/tests/doc_comments.rs::a_version_made_while_the_daemon_was_stopped_is_recorded_when_it_starts`.
    - A move to another machine keeps the versions and their comments: **Works** in the daemon, `daemon/humatad/tests/desk_roam.rs::a_desk_session_moved_to_another_machine_keeps_its_versions_and_their_comments`.
    - Versions on that machine are listed, saved, read and restored there: **Works** in the daemon, `daemon/humatad/tests/desk_versions_remote.rs::versions_of_a_desk_folder_on_a_box_are_listed_saved_read_and_restored_there`.

14. **Do:** ask the agent: "What changed between v2 and v3, and why?"
    **See:** it answers from the versions.
    **Expect:** it calls `doc_versions_list` and `doc_version_read`. Reading its own folder needs no rule; another session's folder would ask under **Read another session's files**.
    **Status: Works** in the daemon: `daemon/humatad/tests/mcp_doc_versions.rs::a_desk_agent_lists_and_reads_the_versions_of_its_own_folders_file` and `::another_sessions_versions_need_the_read_rule`. No real agent CLI has answered from them yet.

15. **Do:** on the Desk page, click **Copy to a channel…** on `rfc-index.md` and pick `#ledger`.
    **Expect:** `#ledger/shared/rfc-index.md` exists. A file already there with that name is never overwritten.
    **Status: Works.** `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel` copies the document from the Desk page into a channel's `shared/`. The daemon side: `daemon/humatad/tests/desk.rs::markdown_in_the_folder_is_listed_and_copies_into_a_channels_shared_folder` and `::a_copy_never_overwrites_the_channels_file`. Fake-client version: `desktop/tests/native/desk.rs::a_document_is_copied_into_a_channels_shared_folder`.

16. **Do:** send it to the team.
    **See:** I copy the Markdown out, or push it to the docs repo from a channel session (flow 2, step 13).
    **Status:** sharing a doc for others to comment on, or exporting it to another format, is **Not built, no milestone.** Humata is one person's app, so comments are between me and my agents.

17. **Do:** the team agrees. Move this Desk session into `#ledger` so its conversation goes with the work.
    **Expect:** the folder's versions and their comment records come along, and the History strip keeps working in the channel (DV-10).
    **Status: Not built: ROAM** (DK-15–DK-19, Move into a channel). Flow 5 picks up from here. Carrying the versions **Works** in the daemon: `daemon/humatad/tests/desk_move.rs::a_desk_session_moved_into_a_channel_keeps_its_versions`. No test checks the History strip in the channel afterwards.

## Expected outcome

`rfc-index.md` sits in `~/Humata/desk/index-rollout-proposal/`, with every version I or the agent saved, and in `#ledger/shared/`. Every comment I wrote is resolved, or re-attached and sent. The Desk session can close, and the folder stays because it holds versions.

## Gaps this flow shows

- The best-proven flow here (26.20) runs live in a channel session, while the spec puts it on the Desk. The Desk's part has run against a real daemon too: the Documents list and the copy (`desktop/tests/live_flows/desk.rs`), and the document page with its comments (`desktop/tests/live_flows/desk_document.rs`).
- The live test's "agent rewriting the doc" is the test writing the file. No real agent CLI has rewritten a doc and resolved comments through Humata.
- A finished proposal can't move into the channel that will do the work (ROAM).
- There's no way to share a doc for others' comments. No milestone, by design.
- Versions (DOCV, DV-1–DV-13) are proven in the daemon and on the fake clients. No live app test clicks through the History strip yet, and the "Kept · N versions" chip has no test.
- Starting a Desk session in a picked folder still works in the daemon (step 1's old path), but no new session can pick one (D451).
