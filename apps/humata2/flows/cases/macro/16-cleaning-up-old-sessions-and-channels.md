# A month in, I clean up closed Desk sessions, archive and undo channel sessions, delete a channel, and empty the trash

Layer: macro. Platforms: macOS for this Mac (the host and the app); box-1 is Linux and holds one remote session. The web build shows the same lists but cannot empty the trash on a machine.

## Starting state

- This Mac is the host. box-1 is online with one session in acme.
- The Desk page has 12 closed sessions. Seven of them have a scratch folder that was not kept. Five are kept. Each row shows its last use and its folder size.
- The channel acme has 6 sessions. Three of them are finished, with their work pushed. One has an open fork.
- The sidebar is long, and it is the sidebar I want to shorten.
- The Trash on this Mac holds nothing yet.
- I hold nothing: no open dialog, no unsaved draft.

## Steps

1. **I do:** open the Desk page and click **Clean up…**.
   **I see:** the dialog "Clean up closed sessions", with "Least recently used first". Each closed session that is not kept is listed with its size. The safe ones (no unpushed work) are ticked. The footer says how many bytes the ticked ones free.
   **State and events:** the host lists the closed sessions and their folders. Nothing is deleted.
   **Status:** Works (real daemon) for the plan: `daemon/humatad/tests/desk.rs::clean_up_lists_closed_sessions_and_deletes_the_ticked_ones`. Fakes only for the dialog's order, the ticks and the bytes: `features/desk/tests/clean_up.rs::the_list_is_least_recently_used_first_with_the_safe_folders_ticked` and `features/desk/tests/clean_up.rs::freed_bytes_follow_the_ticks`.

2. **I do:** untick two sessions I want to keep, then click **Delete 5**.
   **I see:** the dialog closes. The five ticked sessions leave the Desk list. The dialog's copy says "Deleted sessions go to the trash for 30 days" (`ui/src/overlay_desk_clean_up.rs:104`).
   **State and events:** each ticked session's folder, record and conversation go to the host's trash (LD-5, DK-11). The unticked two stay. Nothing is deleted outright.
   **Status:** Works (real daemon) for the delete and its trash: `daemon/humatad/tests/desk.rs::clean_up_lists_closed_sessions_and_deletes_the_ticked_ones`. Fakes only for the dialog's confirm: `features/desk/tests/clean_up.rs::confirming_deletes_exactly_the_ticked_sessions_and_closes_the_dialog` and `desktop/tests/native/desk.rs::cleaning_up_deletes_the_ticked_closed_sessions`. The 30-day window is `daemon/humatad/tests/trash.rs` (line 116, `THIRTY_DAYS_SECONDS`).

3. **I do:** open Clean up… again, and the scratch folder of one session has unpushed commits. Tick it anyway and confirm.
   **I see:** the row names the unpushed work. Its folder is held and is not deleted. The dialog says why.
   **State and events:** a folder with unpushed or uncommitted work is not deleted by the clean-up (DK-11).
   **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::work_not_pushed_holds_the_folder_and_says_why` checks the held folder and the reason the daemon gives. The dialog's own wording is Fakes only (`features/desk/tests/clean_up.rs`).

4. **I do:** while the dialog is open, another session on the Desk is started by the agent in the background, then I confirm the clean-up.
   **I see:** nothing is deleted. The dialog shows the new list and asks me again.
   **State and events:** the plan no longer holds, so the clean-up answers with the new list (NV-27).
   **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::a_plan_that_no_longer_holds_deletes_nothing_and_answers_the_new_list`. Fakes only for the dialog: `features/desk/tests/clean_up.rs::a_desk_that_changed_under_the_dialog_deletes_nothing_and_asks_again`.

5. **I do:** click Clean up… a second time with the same ticks, because my first click's answer got lost on the way.
   **I see:** the same answer as the first, not a second delete. The sessions the first one deleted are shown as gone, not as errors.
   **State and events:** the clean-up is retried with the same plan and answers what the first one did.
   **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::a_retried_clean_up_answers_what_the_first_one_did`.

6. **I do:** in acme, open each of the three finished sessions' ⋯ menu and choose **Archive**, one at a time.
   **I see:** the Archive dialog shows the archive plan, which names unpushed work and running forks if there are any. Each archived session leaves the sidebar's list and moves to Archived. The fork of the open one stays with its parent (see the branch on forks).
   **State and events:** each archive keeps the record and the conversation (LD-2). The clone folders go when the undo window closes, and what was pushed comes back on restore.
   **Status:** Works (real daemon). `daemon/humatad/tests/archive.rs::the_plan_names_unpushed_work_and_running_forks` (the plan), `daemon/humatad/tests/archive.rs::the_clones_go_when_the_window_closes_and_restore_clones_what_was_pushed` (the clones). The ⋯ menu's Archive entry is Fakes only (`features/app/tests/every_action.rs`).

7. **I do:** after the third archive, press **Undo** on the toast for the second one.
   **I see:** the second session is back in the sidebar in its place, with its conversation and its folder.
   **State and events:** the undo restores the session exactly as it was, and the other two archives stay.
   **Status:** Works (real daemon). `daemon/humatad/tests/archive.rs::undo_brings_the_session_back_exactly`. The toast with its working Undo is Fakes only: `desktop/tests/native/overlays.rs::archiving_offers_a_working_undo`.

8. **I do:** archive the one session in acme that is on box-1.
   **I see:** the session is archived on box-1. Its clones on box-1 are freed there, and the host lists it as archived.
   **State and events:** the archive runs on box-1, and the host records the result. The record and conversation are on the host.
   **Status:** Works (real daemon, remote rig with fake links). `daemon/humatad/tests/remote_archive.rs::archiving_on_box_1_frees_its_clones_there_and_lists_it_archived_on_the_host`.

9. **I do:** try to delete one archived session outright: the CLI `humata session delete <session>`, or a plain Delete if the session's menu has one.
   **I see:** the CLI refuses with "not built". The app has no control that removes a session for good (no "Delete session" label is found in `ui/src`).
   **State and events:** none. The daemon's `PlanDelete` and `DeleteSession` answer "not built".
   **Status:** Not built. `daemon/humatad/src/session_service.rs:2331` (PlanDelete) and `:2338` (DeleteSession) return `status::not_built`. No milestone was found that owns them; `OVERVIEW.md:446` notes that DeleteSession is not served. The CLI command `humata session delete` exists (`cli/src/args.rs:405`) and gets the same answer. The archive-and-delete-the-work path in the app (`ui/src/overlay_archive.rs:94`) is a different action.

10. **I do:** look at what is in the trash. I run `humata trash` in a shell, and I open the Files page's trash.
    **I see:** `humata trash` lists the five sessions' folders with their ids and when they were deleted. Whether the Files page's trash list shows session items is not confirmed (see the gaps): the Files page code draws Restore and "Empty trash…" (`ui/src/file_manager.rs`, around lines 738 and 787).
    **State and events:** the host's trash holds the five sessions' folders, records and conversations. Nothing is removed by looking.
    **Status:** Works (real daemon) for the host's trash: `daemon/humatad/tests/trash.rs::a_trashed_folder_comes_back_with_everything_in_it`. The `humata trash` listing has no CLI test that names it (Code, no test). The Files page's list of session items: Code, no test.

11. **I do:** restore one of the five from the trash.
    **I see:** the session comes back in the Desk as closed, with its folder, its conversation and its record, and its folder name is kept.
    **State and events:** the restore puts the folder back whole. If the folder name is taken, both are kept, not overwritten.
    **Status:** Works (real daemon). `daemon/humatad/tests/trash.rs::a_trashed_file_leaves_its_folder_and_comes_back_whole`, `daemon/humatad/tests/trash.rs::restoring_onto_a_path_something_else_took_keeps_both`.

12. **I do:** in acme, choose **Delete channel** from the channel page (or `humata channel delete acme` in a shell), and read the plan first.
    **I see:** the plan lists every session, item and request in acme, and the folders that will go to the trash. It changes nothing. Unpushed work is listed and blocks the delete until I tick "delete that work".
    **State and events:** `PlanDeleteChannel` answers the plan with the listing. Nothing is deleted.
    **Status:** Works (real daemon) for the plan and its block: `daemon/humatad/tests/delete_channel.rs::the_plan_lists_every_session_item_and_request_and_changes_nothing` and `daemon/humatad/tests/delete_channel.rs::unpushed_work_blocks_the_delete_until_the_user_ticks_delete_that_work`. Not built in the app: no UI or client calls `PlanDeleteChannel` or `DeleteChannel` (grep finds only the daemon's `channel_service.rs`). The CLI `humata channel delete` (`cli/src/args.rs` around line 288) has no test in `cli/tests/`.

13. **I do:** confirm the channel delete.
    **I see:** the channel's sessions stop, its requests are withdrawn, and its folder goes to the trash. The channel's sessions show as archived.
    **State and events:** the agents stop; the requests are withdrawn; the channel folder goes to the trash (CH-24, LD-7).
    **Status:** Works (real daemon). `daemon/humatad/tests/delete_channel.rs::deleting_stops_the_agents_withdraws_the_requests_and_sends_the_folder_to_trash`.

14. **I do:** change my mind and restore acme from the trash.
    **I see:** the channel comes back, with its sessions archived.
    **State and events:** the channel and its sessions come back from the trash, with the sessions archived, not running.
    **Status:** Works (real daemon). `daemon/humatad/tests/delete_channel.rs::restoring_from_the_trash_brings_the_channel_back_with_its_sessions_archived`. No UI for this restore (see gaps).

15. **I do:** delete acme again, and then choose **Empty trash** on the Storage page (or `humata trash empty` in a shell), after the plan says how much it frees.
    **I see:** the plan says what will be freed. After I confirm, the trash is empty.
    **State and events:** the trash empties only what the plan named. A plan token works once.
    **Status:** Works (real daemon) for the host's trash: `daemon/humatad/tests/trash.rs::emptying_frees_what_the_plan_said_and_a_token_works_once` and `daemon/humatad/tests/trash.rs::emptying_removes_only_what_was_planned`. The `humata trash empty` CLI is Fakes only, if covered by `cli/tests/` (no test names it). The Storage page's Empty trash is not found in `ui/src` (the only "Empty trash…" is on the Files page, `ui/src/file_manager.rs:787`).

## Branches

### A remote session's archive

- **When:** the session is on box-1 and the link is down while I archive it.
- **I do:** archive it from the host.
- **I see:** the host shows it as archived once box-1 is back, and not before. Until then it shows as pending.
- **State and events:** the archive is recorded on the host; box-1 finishes its part when it links.
- **Expected outcome:** the archive is finished once the link is back, and the host says it is pending in the meantime.
- **Status:** Works (real daemon). `daemon/humatad/tests/remote_archive.rs::an_archive_whose_link_was_lost_shows_archived_once_box_1_is_back` and `daemon/humatad/tests/remote_archive.rs::a_box_1_cut_after_it_recorded_its_archive_finishes_it_once_it_links`.

### A remote archive with work on box-1

- **When:** box-1's session has unpushed commits or changed files.
- **I do:** archive it from the host.
- **I see:** the host shows the files and unpushed commits on that session, and does not delete them.
- **State and events:** the host holds the work's pointers.
- **Expected outcome:** the archive does not drop the work.
- **Status:** Works (real daemon). `daemon/humatad/tests/remote_archive.rs::a_box_1_archive_with_work_shows_its_files_and_unpushed_commits_on_the_host`.

### Archive a session that has unpushed work

- **When:** I archive a session with unpushed commits, and I do not choose push-first or delete the work.
- **I do:** choose Archive.
- **I see:** the archive is refused and the dialog says why (the unpushed work). The options are to push first, or to archive and delete the work.
- **State and events:** nothing is archived.
- **Expected outcome:** refused with the reason; the session stays where it is.
- **Status:** Works (real daemon). `daemon/humatad/tests/archive.rs::plain_archive_is_refused_while_there_is_unpushed_work`. The "Archive and delete the work" control is in `ui/src/overlay_archive.rs:94`, Code, no test for it.

### A fork of an archived session

- **When:** I archive a session that has a fork still open.
- **I do:** archive the parent.
- **I see:** the fork moves up in the list while the parent is archived, and comes back under the parent when I restore it.
- **State and events:** the fork's place is kept.
- **Expected outcome:** the fork is not lost, and it returns to its parent on restore.
- **Status:** Works (real daemon). `daemon/humatad/tests/archive.rs::forks_move_up_while_their_parent_is_archived_and_back_on_restore`.

### An archive cut by a restart

- **When:** the host restarts in the middle of an archive.
- **I do:** restart the host.
- **I see:** the archive is either finished or undone, never half done. The session shows archived, or still running with its clones.
- **State and events:** the archive is finished at restart, or left running when it was not yet recorded.
- **Expected outcome:** one of the two whole states.
- **Status:** Works (real daemon). `daemon/humatad/tests/archive.rs::a_restart_inside_the_window_finishes_the_delete`, `daemon/humatad/tests/archive.rs::an_archive_cut_before_it_is_recorded_leaves_the_session_running`, `daemon/humatad/tests/archive.rs::an_archive_cut_at_any_point_is_finished_at_restart`.

### Deleting a channel that other machines hold

- **When:** acme is also on box-1, and box-1 is holding a session in it.
- **I do:** confirm the channel delete.
- **I see:** no plan is offered. The plan is refused up front with FailedPrecondition. The message names each machine that holds the channel, as online or offline as it really is, and the live sessions there with their uncommitted and unpushed work. The fix says to move each session I want to keep to the Desk, then remove each machine from the channel (Remove machine, or `humata channel machine rm`), reconnecting an offline one first, and plan again.
- **State and events:** nothing is planned or deleted.
- **Expected outcome:** refused before any plan, with the fix named.
- **Status:** Refused, not built. `refuse_held_elsewhere` in `daemon/humatad/src/channel_service/delete.rs` refuses in PlanDeleteChannel and again in DeleteChannel. Tested by `daemon/humatad/tests/channel_delete_remote.rs` (rig). The relayed remote delete comes after 0.2.4.

### Deleting a channel with live sessions elsewhere

- **When:** a session in acme is running on box-1 when I delete acme.
- **I do:** confirm the delete.
- **I see:** no plan is offered. The plan is refused up front, naming box-1 (online or offline) and each live session there, with the same fix as above.
- **State and events:** nothing is planned or deleted.
- **Expected outcome:** refused; the sessions keep running.
- **Status:** Refused, not built. The same `refuse_held_elsewhere` in `daemon/humatad/src/channel_service/delete.rs` covers it. `daemon/humatad/tests/channel_delete_remote.rs::a_channel_with_a_live_session_on_an_online_box_gets_no_plan_apply_would_refuse` checks that the refusal names the session and doesn't call the online box offline. The offline case is in the same file.

### The plan no longer matches

- **When:** the channel changed between the plan and the confirm (a session started, a request came in).
- **I do:** confirm the delete.
- **I see:** nothing is deleted. The plan shows what changed and asks again.
- **State and events:** the delete does nothing (NV-27).
- **Expected outcome:** nothing deleted; the new plan is shown.
- **Status:** Works (real daemon). `daemon/humatad/tests/delete_channel.rs::a_plan_that_no_longer_matches_does_nothing`.

### A delete cut by a restart

- **When:** the host restarts while acme's delete is half done.
- **I do:** restart.
- **I see:** the delete finishes after the restart, with nothing left behind.
- **State and events:** the delete resumes at start.
- **Expected outcome:** finished, not half done.
- **Status:** Works (real daemon). `daemon/humatad/tests/delete_channel.rs::a_delete_cut_short_finishes_after_a_restart` and `daemon/humatad/tests/delete_channel.rs::a_delete_cut_at_any_point_finishes_at_restart_with_nothing_left`.

### An agent tries to delete a channel

- **When:** an agent calls the delete.
- **I do:** nothing; the agent calls it.
- **I see:** refused.
- **State and events:** nothing is planned or deleted.
- **Expected outcome:** only the user can delete a channel.
- **Status:** Works (real daemon). `daemon/humatad/tests/delete_channel.rs::an_agent_cannot_plan_or_delete_a_channel`.

### Auto-delete of closed Desk sessions and the trash

- **When:** a closed Desk session's scratch folder is not kept, and seven days pass; or an item stays in the trash for 30 days.
- **I do:** nothing.
- **I see:** the session goes to the trash after 7 days (unless I kept its folder), and the trash empties its items after 30 days.
- **State and events:** the trash sweeps 30-day-old items when it is used and once at daemon start (`daemon/humatad/src/trash_service.rs:8-9`).
- **Expected outcome:** the 7-day and 30-day rules hold, and keeping a folder stops the 7-day one.
- **Status:** Works (real daemon) for the 7 days: `daemon/humatad/tests/desk.rs::a_closed_scratch_folder_goes_to_the_trash_after_its_days`. For keeping: `daemon/humatad/tests/desk.rs::keeping_a_folder_takes_away_its_delete_time`. For the 30 days: `daemon/humatad/tests/trash.rs` (line 116). The brief's "45-day opt-in" is not in the spec or the decisions (see corrections).

### A trashed folder that another item took the name of

- **When:** I restore a session whose folder name is taken now.
- **I do:** restore.
- **I see:** both are kept; the restored one gets a new name.
- **State and events:** no overwrite.
- **Expected outcome:** nothing is overwritten.
- **Status:** Works (real daemon). `daemon/humatad/tests/trash.rs::restoring_onto_a_path_something_else_took_keeps_both`.

### A retried trash empty

- **When:** I empty the trash with the same plan token twice.
- **I do:** empty twice.
- **I see:** the second one is refused; nothing else is removed.
- **State and events:** the token works once.
- **Expected outcome:** nothing removed twice.
- **Status:** Works (real daemon). `daemon/humatad/tests/trash.rs::emptying_frees_what_the_plan_said_and_a_token_works_once`.

## Expected outcomes

- [ ] The clean-up lists closed sessions, least recently used first, with the safe ones ticked and the bytes it frees.
- [ ] Confirming deletes exactly the ticked sessions, to the trash, and nothing else.
- [ ] A folder with unpushed work is held, and the dialog says why.
- [ ] A clean-up whose plan changed deletes nothing and shows the new list.
- [ ] A retried clean-up answers what the first one did.
- [ ] Archiving a session keeps its record and conversation, and undo brings it back exactly.
- [ ] A session on box-1 is archived there and shows archived on the host.
- [ ] A session cannot be deleted outright in the app (not built).
- [ ] A channel's delete plan changes nothing and shows the work that blocks it.
- [ ] A channel delete stops its agents, withdraws its requests, and sends its folder to the trash.
- [ ] The channel comes back from the trash with its sessions archived.
- [ ] Empty trash frees what the plan named, and only that.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Clean up… lists closed sessions | Works (real daemon) | `daemon/humatad/tests/desk.rs::clean_up_lists_closed_sessions_and_deletes_the_ticked_ones`; Fakes only for dialog: `features/desk/tests/clean_up.rs::the_list_is_least_recently_used_first_with_the_safe_folders_ticked` |
| 2. Delete the ticked ones to the trash | Works (real daemon); Fakes only for confirm | `daemon/humatad/tests/desk.rs::clean_up_lists_closed_sessions_and_deletes_the_ticked_ones`; `features/desk/tests/clean_up.rs::confirming_deletes_exactly_the_ticked_sessions_and_closes_the_dialog` |
| 3. Unpushed work held and explained | Works (real daemon) | `daemon/humatad/tests/desk.rs::work_not_pushed_holds_the_folder_and_says_why` |
| 4. Plan changed under the dialog | Works (real daemon); Fakes only for dialog | `daemon/humatad/tests/desk.rs::a_plan_that_no_longer_holds_deletes_nothing_and_answers_the_new_list`; `features/desk/tests/clean_up.rs::a_desk_that_changed_under_the_dialog_deletes_nothing_and_asks_again` |
| 5. Retried clean-up | Works (real daemon) | `daemon/humatad/tests/desk.rs::a_retried_clean_up_answers_what_the_first_one_did` |
| 6. Archive three channel sessions | Works (real daemon); ⋯ entry Fakes only | `daemon/humatad/tests/archive.rs::the_plan_names_unpushed_work_and_running_forks`; `daemon/humatad/tests/archive.rs::the_clones_go_when_the_window_closes_and_restore_clones_what_was_pushed` |
| 7. Undo one archive | Works (real daemon); toast Fakes only | `daemon/humatad/tests/archive.rs::undo_brings_the_session_back_exactly`; `desktop/tests/native/overlays.rs::archiving_offers_a_working_undo` |
| 8. Archive a box-1 session | Works (real daemon, fake links) | `daemon/humatad/tests/remote_archive.rs::archiving_on_box_1_frees_its_clones_there_and_lists_it_archived_on_the_host` |
| 9. Delete an archived session outright | Not built | `daemon/humatad/src/session_service.rs:2331`, `:2338`; `OVERVIEW.md:446`; no milestone |
| 10. Trash listed | Works (real daemon) for host trash; Code, no test for session items in the Files trash list | `daemon/humatad/tests/trash.rs::a_trashed_folder_comes_back_with_everything_in_it`; `ui/src/file_manager.rs` |
| 11. Restore from trash | Works (real daemon) | `daemon/humatad/tests/trash.rs::a_trashed_file_leaves_its_folder_and_comes_back_whole` |
| 12. Channel delete plan | Works (real daemon) for daemon; Not built in app; Code, no test for CLI | `daemon/humatad/tests/delete_channel.rs::the_plan_lists_every_session_item_and_request_and_changes_nothing`; `daemon/humatad/tests/delete_channel.rs::unpushed_work_blocks_the_delete_until_the_user_ticks_delete_that_work` |
| 13. Channel delete stops agents, withdraws, trashes | Works (real daemon) | `daemon/humatad/tests/delete_channel.rs::deleting_stops_the_agents_withdraws_the_requests_and_sends_the_folder_to_trash` |
| 14. Restore channel from trash | Works (real daemon); no UI | `daemon/humatad/tests/delete_channel.rs::restoring_from_the_trash_brings_the_channel_back_with_its_sessions_archived` |
| 15. Empty trash after plan | Works (real daemon); Storage page Not built | `daemon/humatad/tests/trash.rs::emptying_frees_what_the_plan_said_and_a_token_works_once` |
| B1. Remote archive, link lost | Works (real daemon) | `daemon/humatad/tests/remote_archive.rs::an_archive_whose_link_was_lost_shows_archived_once_box_1_is_back` |
| B2. Remote archive with work | Works (real daemon) | `daemon/humatad/tests/remote_archive.rs::a_box_1_archive_with_work_shows_its_files_and_unpushed_commits_on_the_host` |
| B3. Archive with unpushed work refused | Works (real daemon); "delete the work" control Code, no test | `daemon/humatad/tests/archive.rs::plain_archive_is_refused_while_there_is_unpushed_work` |
| B4. Fork of an archived parent | Works (real daemon) | `daemon/humatad/tests/archive.rs::forks_move_up_while_their_parent_is_archived_and_back_on_restore` |
| B5. Archive cut by restart | Works (real daemon) | `daemon/humatad/tests/archive.rs::an_archive_cut_at_any_point_is_finished_at_restart` |
| B6. Channel held by other machines | Refused up front, not built | `refuse_held_elsewhere` in `daemon/humatad/src/channel_service/delete.rs` |
| B7. Channel with live sessions elsewhere | Refused up front, not built | `refuse_held_elsewhere` in `daemon/humatad/src/channel_service/delete.rs` |
| B8. Plan no longer matches | Works (real daemon) | `daemon/humatad/tests/delete_channel.rs::a_plan_that_no_longer_matches_does_nothing` |
| B9. Channel delete cut by restart | Works (real daemon) | `daemon/humatad/tests/delete_channel.rs::a_delete_cut_at_any_point_finishes_at_restart_with_nothing_left` |
| B10. Agent cannot delete a channel | Works (real daemon) | `daemon/humatad/tests/delete_channel.rs::an_agent_cannot_plan_or_delete_a_channel` |
| B11. 7-day Desk and 30-day trash auto-delete | Works (real daemon) | `daemon/humatad/tests/desk.rs::a_closed_scratch_folder_goes_to_the_trash_after_its_days`; `daemon/humatad/tests/trash.rs` (line 116) |
| B12. Restore name taken | Works (real daemon) | `daemon/humatad/tests/trash.rs::restoring_onto_a_path_something_else_took_keeps_both` |
| B13. Retried empty trash | Works (real daemon) | `daemon/humatad/tests/trash.rs::emptying_frees_what_the_plan_said_and_a_token_works_once` |

## Gaps and likely bugs

**Not built**
- Deleting one session outright (`PlanDelete`, `DeleteSession`, `PlanDropRepo`, `DropRepo`): `daemon/humatad/src/session_service.rs:2331-2386`. No milestone found; `OVERVIEW.md:446` says DeleteSession is not served.
- Deleting a channel with live sessions on other machines, and a channel that other machines hold. Both are refused up front, naming the machines and sessions, with the fix: `refuse_held_elsewhere` in `daemon/humatad/src/channel_service/delete.rs`. The relayed delete comes after 0.2.4.
- The app's channel delete: no UI or client calls `PlanDeleteChannel` or `DeleteChannel`. The daemon and CLI have it, with no CLI test.
- The Storage page's Empty trash, and any UI to restore a deleted channel or session. Only the Files page's trash is in `ui/src/file_manager.rs`.

**Likely bugs**
- None confirmed by reading alone. The Files page's trash list (`ui/src/file_manager.rs`) is built for files, and it is not confirmed that it shows the session and channel items the host's trash holds. To confirm: delete a session, then open the Files page's trash.

**Code, no test**
- The app's "Archive and delete the work" control (`ui/src/overlay_archive.rs:94`).
- `humata channel delete` in the CLI (`cli/src/args.rs` around line 288; `cli/src/commands/looking.rs:185`).
- `humata trash empty` in the CLI (no test names it in `cli/tests/`).

**Corrections to the brief**
- "Auto-delete opt-in (45 days?)" is not in the spec or the decisions. The rules are: a closed Desk session whose scratch folder is not kept goes to the trash after 7 days (DK-11, `daemon/humatad/tests/desk.rs::a_closed_scratch_folder_goes_to_the_trash_after_its_days`); keeping the folder stops that (`keeping_a_folder_takes_away_its_delete_time`); the trash empties its items after 30 days (LD-7, `daemon/humatad/src/trash_service.rs:8-9`; `daemon/humatad/tests/trash.rs:116`). It is a default on, not an opt-in, for the 7 days; keeping is the opt-out.
- "A retried clean up answers what the first did" is covered by `daemon/humatad/tests/desk.rs::a_retried_clean_up_answers_what_the_first_one_did`, which is a real-daemon test.
- "Trash restore and empty" for channels: the restore is daemon-only (`daemon/humatad/tests/delete_channel.rs`); the app has no UI for it.
- "Deleting a channel with running sessions" is two cases: running sessions on this machine are stopped by the delete (tested); running sessions on other machines are refused as not built.
