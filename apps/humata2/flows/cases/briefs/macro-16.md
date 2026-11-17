# Brief macro-16: Cleaning up old sessions and channels
Output: `apps/humata2/flows/cases/macro/16-cleaning-up-old-sessions-and-channels.md`

Scenario: a month in, the sidebar is long. I open "Clean up…", tick closed desk sessions, archive three channel sessions (and undo one), try to delete a session outright, delete a whole channel, and empty the Trash.

Steps: Clean up lists closed sessions and deletes the ticked ones; a plan that no longer holds deletes nothing and shows the new list; archive with Undo; work not pushed holds a folder and says why; channel delete plan; Trash restore and empty.

Branches: SessionService.PlanDelete / DeleteSession / DropRepo not_built (session_service.rs ~2330–2385); a retried clean up answers what the first did; deleting a channel with running sessions; auto-delete opt-in (45 days?) — check; a remote session's archive.

Pointers: daemon/humatad/tests/desk.rs (clean up tests), archive.rs, remote_archive.rs, delete_channel.rs, trash.rs; features/desk/tests/clean_up.rs; desktop/tests/native/desk.rs `cleaning_up_deletes_the_ticked_closed_sessions`, overlays.rs `archiving_offers_a_working_undo`; daemon/humatad/src/channel_service/delete.rs.
