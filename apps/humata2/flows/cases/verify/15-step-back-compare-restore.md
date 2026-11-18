# Stepping back to an old version reads it as it was, and Restore adds a version and keeps every other one

Layer: verify (whole-flow, Desk). Platforms: macOS first; Linux where the host runs; the web build shares the same UI.

This case covers flow 04 steps 11 to 13 (read an old version, Compare with current, Restore as new version). Continues from H-1: the same proposal folder, with three versions. No macro case covers stepping back or restoring.

## Starting state

- Humata is open on Home. This Mac's daemon is running.
- The Desk session "Index rollout proposal" is open, agent Claude Code on this Mac. Its folder is `<humata home>/desk/index-rollout-proposal/`, a kept folder with three versions of `rfc-index.md`: v1 by claude ("Draft"), v2 by you ("Before review"), v3 by claude ("Rewrite risks").
- `rfc-index.md` is open on a Markdown tab in Preview, showing v3 (the current file). Two comments that I wrote earlier are on v3, both sent and both recorded when they were made.
- The file on disk equals v3. Nothing is unsaved.
- I hold nothing on the clipboard.

## Reproduce headless

- Daemon, real `humatad2` in a temp folder, real git, fake agent. Run:
  `bazel test //apps/humata2/daemon/humatad:doc_history_test //apps/humata2/daemon/humatad:doc_version_diff_test //apps/humata2/daemon/humatad:doc_comments_test //apps/humata2/daemon/humatad:desk_roam_test //apps/humata2/daemon/humatad:desk_versions_remote_test --test_output=all`
  Evidence: the pass lines for `an_old_version_reads_as_it_was`, `restoring_saves_unsaved_changes_first_and_keeps_every_version`, `a_saved_version_then_an_edit_is_one_removed_and_one_added_line`, `a_desk_session_moved_to_another_machine_keeps_its_versions_and_their_comments` and `versions_of_a_desk_folder_on_a_box_are_listed_saved_read_and_restored_there`.
- Git rules, real git in a temp folder: `bazel test //apps/humata2/daemon/sessions:sessions_unit_test --test_arg=doc_repo --test_output=errors`. Evidence: the log for `restore_changes_only_that_file`, `restore_saves_unsaved_changes_first` and `restoring_the_text_the_file_already_has_is_refused`.
- Reducers on fakes: `bazel test //apps/humata2/features/session:history_test --test_output=errors`. Evidence: the log for `stepping_back_and_forward_walks_the_versions_and_keeps_the_mode`, `an_old_version_is_read_only_until_back_to_current`, `restoring_adds_a_version_and_lands_on_current`, `restoring_with_unsaved_changes_saves_them_first` and `compare_with_current_loads_the_daemons_diff_of_the_version_and_the_file`.
- Native view on fakes: `bazel test //apps/humata2/desktop:native_test --test_arg=doc_versions --test_output=errors`. Evidence: a `.new.png` render if a view differs. The labels are not checked by these tests.
- Live app on a real daemon: there is no live test for the History strip, the version bar, Compare or Restore. The nearest live check is `bazel test //apps/humata2/desktop:live_flows_test --test_arg=desk_document`, which covers comments on the document page.
- The test stands in for the agent: it runs `git commit` in the session's folder. Nothing touches `~/.claude`, `~/.codex`, `~/.copilot` or a git config.
- What cannot be done headless: see the real window and the real agent CLI. Nearest headless equivalents: the daemon tests and the native render.

## Steps

1. **I do:** click ◀ on the History strip twice, from v3 to v1.  
   **I see:** the page shows v1. The bar reads "v1 of 3", and the page is read-only. The bar has Compare with current, Restore as new version and Back to current. The comments show as they were when v1 was made.  
   **State and events:** no write. The daemon reads v1 as it was. The tab's mode stays the same.  
   **Status:** Fakes only for the stepping and the read-only page: `desktop/tests/native/doc_versions.rs::stepping_back_shows_an_old_version_that_takes_no_comment` and `features/session/tests/history.rs::stepping_back_and_forward_walks_the_versions_and_keeps_the_mode`. Works (real daemon) for v1 as it was: `daemon/humatad/tests/doc_history.rs::an_old_version_reads_as_it_was`. The comments as recorded: `daemon/humatad/tests/doc_comments.rs::an_agents_commit_records_the_files_comments_and_later_changes_leave_them`.

2. **I do:** click a line on v1 and try to type a comment.  
   **I see:** no comment box opens on v1. Sending, resolving and archiving are not offered.  
   **State and events:** nothing is written.  
   **Status:** Fakes only: `features/session/tests/history.rs::an_old_version_is_read_only_until_back_to_current`.

3. **I do:** click Compare with current.  
   **I see:** the diff view, with v1 as the base and the current file on the other side. Its title names the file and "v1 against the current file".  
   **State and events:** the daemon diffs v1 against the file on disk. The compare panel opens. Nothing is written.  
   **Status:** Works (real daemon) for the diff: `daemon/humatad/tests/doc_version_diff.rs::a_saved_version_then_an_edit_is_one_removed_and_one_added_line`. Fakes only for the action: `features/session/tests/history.rs::compare_with_current_loads_the_daemons_diff_of_the_version_and_the_file`. The diff view itself is covered under REVIEW. If v1 read the same as the current file, the view would say so (`ui/src/doc_history.rs:625`): Code, no test.

4. **I do:** click ▶ once to v2, then click Restore as new version.  
   **I see:** the strip should read "v4 of 4 · You · Restored v2", with v3 still in the list. (Actual: it reads "Current" (Likely bug 2).)  
   **State and events:** unsaved changes, if any, are saved first (none here). Only `rfc-index.md` is written, and a new version is committed by you with the note "Restored v2". No version is removed or rewritten. The session is told (Session updated).  
   **Status:** Works (real daemon) for the restore: `daemon/humatad/tests/doc_history.rs::restoring_saves_unsaved_changes_first_and_keeps_every_version` and `daemon/sessions/src/doc_repo.rs::restore_changes_only_that_file`. Fakes only for the button: `desktop/tests/native/doc_versions.rs::restoring_an_old_version_adds_a_version_and_returns_to_current` and `features/session/tests/history.rs::restoring_adds_a_version_and_lands_on_current`. Likely bug for the label (Gaps, item 2).

5. **I do:** open History ▾.  
   **I see:** v4 by you with "Restored v2" at the top. v3, v2 and v1 are still there, each unchanged. The file on disk now equals v2's text.  
   **State and events:** four versions in the folder's repository. Only `rfc-index.md` changed in the last commit.  
   **Status:** Works (real daemon): `daemon/humatad/tests/doc_history.rs::restoring_saves_unsaved_changes_first_and_keeps_every_version` and `daemon/sessions/src/doc_repo.rs::restore_changes_only_that_file`. The popup's drawing: Code, no test.

6. **I do:** read the two comments on the page.  
   **I see:** both comments are still shown on the current file, re-anchored to their passages. Neither is marked addressed, because the restore was my change.  
   **State and events:** a comment's status stays sent. Its lines follow the text they were on.  
   **Status:** Code, no test. I searched the daemon's tests and `features/session/tests` for a restore followed by a check on the comments, and found none. The watcher that marks a comment addressed on a change is `daemon/humatad/src/review/comments.rs:657-660`, and GAPS item 42 says it also fires on the user's own edit. Whether a restore reaches that watcher is not verified here.

7. **I do:** click Back to current, then restart this Mac's daemon.  
   **I see:** the page shows the current file with "Current", not read-only. After the restart, History still lists v4, v3, v2 and v1, and the two comments are still there.  
   **State and events:** nothing is written by Back to current. The restart reads the versions and the comments from the folder and the store.  
   **Status:** Fakes only for Back to current: `features/session/tests/history.rs::an_old_version_is_read_only_until_back_to_current`. Works (real daemon) for a version that survives a stopped daemon: `daemon/humatad/tests/doc_comments.rs::a_version_made_while_the_daemon_was_stopped_is_recorded_when_it_starts`. No test reopens the History strip after a restart, so the list and the comments on screen after a restart are unproven.

## Branches

### Restore when the file already has that text

- **When:** I step to v3, which is the text the file already has, and click Restore as new version.
- **I do:** click Restore as new version.
- **I see:** nothing is added. The restore is refused with a reason: the file already holds that version.
- **Expected:** no commit, and the strip does not move.
- **Status:** Fakes only for the reducer: `features/session/tests/history.rs::restoring_the_text_the_file_already_has_changes_nothing`. Works (real daemon, real git) for the refusal: `daemon/sessions/src/doc_repo.rs::restoring_the_text_the_file_already_has_is_refused`.

### Restore with unsaved edits

- **When:** I edit `rfc-index.md` in my own editor, then step to v2 and click Restore as new version.
- **I do:** click Restore as new version.
- **I see:** the unsaved edit becomes its own version first, with the note "Saved before restoring v2". Then "Restored v2" is added as the next version.
- **Expected:** both versions exist, in that order, and nothing is lost.
- **Status:** Works (real daemon): `daemon/humatad/tests/doc_history.rs::restoring_saves_unsaved_changes_first_and_keeps_every_version`. Fakes only for the reducer: `features/session/tests/history.rs::restoring_with_unsaved_changes_saves_them_first`.

### Move the Desk session to devbox-1 from its bar

- **When:** after step 5, from the folder bar's machine picker, I pick devbox-1.
- **I do:** pick devbox-1.
- **I see:** the app does not move the session. (The flow's expected result is that the folder, its versions and their comments go with it.)
- **Expected:** the session, its folder, its versions and their comments are on devbox-1, and History there lists, reads and restores the versions.
- **Status:** Not built in the app (flow 01 step 6, no milestone; the client answers "not in this release", `clients/live/src/desk.rs:453-455`). Works (real daemon) at the daemon level: `daemon/humatad/tests/desk_roam.rs::a_desk_session_moved_to_another_machine_keeps_its_versions_and_their_comments`.

### The folder lives on a box

- **When:** the Desk session's folder is on devbox-1, and I open History there.
- **I do:** step back, save and restore a version from this Mac.
- **I see:** the same versions as on the machine that holds the folder.
- **Expected:** the versions are listed, saved, read and restored on the box.
- **Status:** Works (real daemon): `daemon/humatad/tests/desk_versions_remote.rs::versions_of_a_desk_folder_on_a_box_are_listed_saved_read_and_restored_there`.

## Expected outcomes

- [ ] Stepping back to an old version shows it as it was, read-only, and it takes no comment.
- [ ] Compare with current shows the daemon's diff of that version against the file on disk.
- [ ] Restore as new version adds a version, saves unsaved edits first, and changes only `rfc-index.md`.
- [ ] No version is removed or rewritten by a restore.
- [ ] After a restore, the strip shows the new version's number and "Restored vN". Not yet true (Likely bug 2).
- [ ] The comments are re-anchored after a restore, and none is marked addressed. Not verified by any test.
- [ ] Versions and their comments survive a daemon restart and a move to another machine.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. ◀ twice shows v1 read-only | Fakes only; Works (real daemon) for the content | `desktop/tests/native/doc_versions.rs::stepping_back_shows_an_old_version_that_takes_no_comment`; `daemon/humatad/tests/doc_history.rs::an_old_version_reads_as_it_was` |
| 2. Commenting on v1 is refused | Fakes only | `features/session/tests/history.rs::an_old_version_is_read_only_until_back_to_current` |
| 3. Compare with current shows the diff | Works (real daemon) for the diff; Fakes only for the action | `daemon/humatad/tests/doc_version_diff.rs::a_saved_version_then_an_edit_is_one_removed_and_one_added_line`; `features/session/tests/history.rs::compare_with_current_loads_the_daemons_diff_of_the_version_and_the_file` |
| 4. Restore as new version adds v4 | Works (real daemon); Likely bug for the label | `daemon/humatad/tests/doc_history.rs::restoring_saves_unsaved_changes_first_and_keeps_every_version` |
| 5. History lists v4 with v1 to v3 kept | Works (real daemon) for data; Code, no test for the popup | `daemon/sessions/src/doc_repo.rs::restore_changes_only_that_file` |
| 6. Comments re-anchor, none addressed after a restore | Code, no test | searched: no test of a restore's effect on comments |
| 7. Back to current, then a restart | Fakes only (Back to current); Works (real daemon) (restart) | `features/session/tests/history.rs::an_old_version_is_read_only_until_back_to_current`; `daemon/humatad/tests/doc_comments.rs::a_version_made_while_the_daemon_was_stopped_is_recorded_when_it_starts` |
| Branch: restore the text already there | Fakes only (reducer); Works (real daemon, real git) (refusal) | `features/session/tests/history.rs::restoring_the_text_the_file_already_has_changes_nothing`; `daemon/sessions/src/doc_repo.rs::restoring_the_text_the_file_already_has_is_refused` |
| Branch: restore with unsaved edits | Works (real daemon); Fakes only (reducer) | `daemon/humatad/tests/doc_history.rs::restoring_saves_unsaved_changes_first_and_keeps_every_version` |
| Branch: move to devbox-1 from the bar | Not built (app); Works (real daemon) (daemon) | `clients/live/src/desk.rs:453-455`; `daemon/humatad/tests/desk_roam.rs::a_desk_session_moved_to_another_machine_keeps_its_versions_and_their_comments` |
| Branch: folder on a box | Works (real daemon) | `daemon/humatad/tests/desk_versions_remote.rs::versions_of_a_desk_folder_on_a_box_are_listed_saved_read_and_restored_there` |

## Gaps and likely bugs

Not built:

- Moving a Desk session from its folder bar to another machine. Flow 01 step 6 has no milestone. The daemon can move it (`daemon/humatad/tests/desk_roam.rs`), and the app client refuses (`clients/live/src/desk.rs:453-455`).

Likely bugs:

1. **The strip reads "Current" after a restore, not "v4 of 4 · You · Restored v2".** `features/session/src/history.rs:491-500` sets `Position::Current` after a restore, and `ui/src/doc_history.rs:228-234` shows "v N of M" only while a version is being viewed. The same happens after a save (H-1, step 4). Confirm: restore v2 and read the strip label. The same fix as H-1 item 2 covers both.

Gaps in proof:

- No live test clicks the History strip, the version bar, Compare with current or Restore as new version. Every such check is on fakes or the daemon alone.
- Whether a restore re-anchors comments without marking them addressed is not tested (step 6). The related watcher is `daemon/humatad/src/review/comments.rs:657-660`, which GAPS item 42 says marks a comment addressed on the user's own edit. Confirm with a restore on a file that has a sent comment, then read the comment's status.
- The "same as current" note (`ui/src/doc_history.rs:625`) has no test.
