# Archiving a review keeps its unsubmitted pull request drafts, and restoring it brings them back

Layer: macro. Platforms: macOS (the Archive dialog and the Undo toast are desktop app surfaces; Linux and web are not covered by this case).

## Starting state

- Humata 2 is open on This Mac. This Mac is the host, and its `gh` is signed in as me (fake `gh` in every headless run).
- Machines: This Mac (online). No other machine is needed.
- The channel `payments` exists, with `acme/api` as its repo.
- A review session is open for `acme/api#482` "Cache the token refresh". Its title reads "Review acme/api#482". It has one clone of the PR head, on its own branch, with no commits of mine on it, so nothing is unpushed.
- The review holds two pull request drafts that were never submitted:
  - one by the agent, on `refund.rs` line 3, "Half-even here?";
  - one by me, on `refund_test.rs` line 52.
- Neither draft is posted. The review has no earlier submit.
- The Inbox is empty. The palette is closed.

## Reproduce headless

- Plan count and the drafts through the daemon: `bazel test //apps/humata2/daemon/humatad:pr_review_test --test_arg=archive_plans_count_a_reviews_unsubmitted_drafts_until_they_are_submitted`. The test makes one agent draft, reads the plan (count 1, nothing unpushed), submits it, and reads the plan again (count 0). Evidence: the test's pass or fail line.
- The archive lifecycle and restore: `bazel test //apps/humata2/daemon/humatad:archive_test`. It covers archive, the Undo window and restore of clones, but no test in it has drafts. Evidence: pass or fail, and the test names.
- The dialog and its buttons: `bazel test //apps/humata2/desktop:native_test --test_arg=archive_drafts`. The file is `desktop/tests/native/archive_drafts.rs`. It checks for the `archive-drafts` element, `archive-confirm`, `archive-cancel`, and that the session is or isn't archived. No golden exists for this dialog (D513), so no `.new.png` is compared. If a render is wanted as evidence, the tester adds one; that is new work, not part of this run.
- The Archive flow with fakes and the drafts count: `bazel test //apps/humata2/features/session:archive_test`.
- The Undo toast timing: `bazel test //apps/humata2/features/app:nav_bugs_test --test_arg=an_archived_toast_with_undo_stays_for_the_whole_undo_window`. It uses a test clock, so no real seconds pass.
- CLI, against a throwaway `humatad2` with its own `--root` and its own `HOME`: `humata session archive <session>` plans and then archives (flags `--push-first`, `--delete-work`), and `humata session restore <session>` brings it back. The CLI cannot start a PR review or add drafts today, so the drafts must come from the daemon rig above. Evidence: CLI output and exit code. Exit 7 means preconditions; exit 4 means denied.
- Cannot be done headless: the real window and the look of the dialog; the real 10-second toast on screen; the "Frees" size (the live client always returns no size); real GitHub posting (the fake `gh` only records what would be posted). The nearest headless equivalents are the native test and the fake-clock toast test above.
- Run nothing that opens a window on the owner's screen. Use only the headless test targets above.

## Steps

1. **I do:** In the review session, ask the agent to draft a comment on `refund.rs` line 3. Then select line 52 of `refund_test.rs`, press Draft for GitHub, and type "Round half-even here too?".  
   **I see:** Two cards: "Draft for GitHub · by Claude Code" and "Draft for GitHub · by me", each with Edit and Drop. The drafts count reads 2.  
   **State and events:** Two comments with destination pull request, status Draft, stored on the session. Nothing is posted.  
   **Status:** Works (real daemon). `daemon/humatad/tests/pr_review.rs::the_agent_drafts_edits_and_drops_comments_without_reaching_github` and `daemon/humatad/tests/pr_review.rs::the_user_drafts_for_github_beside_the_agents_drafts`. The app's button is in `ui/src/comment_cards.rs:38` and `:143`, but no test here drives it through archive.

2. **I do:** Open the session's Info panel and press Archive.  
   **I see:** Nothing has been archived yet. The daemon's plan says 2 drafts were never submitted, and nothing is unpushed.  
   **State and events:** The plan is read, not used up. No archive is recorded, and the session stays open.  
   **Status:** Works (real daemon) for the count. `daemon/humatad/tests/pr_review.rs::archive_plans_count_a_reviews_unsubmitted_drafts_until_they_are_submitted` checks the count (1 in that test) and that nothing is unpushed. The count of 2 runs the same code (`daemon/humatad/src/session_service/archive.rs:451-452`). Nothing tests 2 directly.

3. **I do:** Nothing more; the dialog is shown.  
   **I see:** The Archive dialog, titled "Archive “Review acme/api#482”?", with the usual line about the conversation and the summary. Below it the drafts line. Then the buttons.  
   **State and events:** The archive is not started. The dialog waits for me.  
   **Status:** Fakes only. `desktop/tests/native/archive_drafts.rs::unsubmitted_drafts_ask_before_archiving` and `features/session/tests/archive.rs::archive_asks_first_when_a_review_has_unsubmitted_drafts`. Both run on fake clients, not a live daemon.

4. **I do:** Read the drafts line.
   **I see (per D513 and PR-17):** "2 drafts on the pull request were never submitted and are kept with the archived review."
   **I actually see in code:** "2 drafts on the pull request were never submitted; archiving drops them."  
   **State and events:** None. The line is a label.  
   **Status:** Likely bug. `ui/src/overlay_archive.rs:51-55` writes "archiving drops them", which contradicts the D513 ruling, "kept with the archived review". No test reads this text (D513 says no golden covers the dialog), so the wording passes unnoticed. To confirm: open the dialog with one draft and read the line; the text ends "archiving drops it."

5. **I do:** Look at the buttons, with nothing unpushed.  
   **I see:** Exactly two buttons, Cancel and Archive. No "Archive and delete the work" and no "Commit, push and open a PR, then archive".  
   **State and events:** None.  
   **Status:** Code, no test. The unpushed box and its two buttons are behind `if unpushed` (`ui/src/overlay_archive.rs:58` and the `else` branch that builds `archive-confirm`, about lines 119-130). The native test checks that `archive-confirm` is present, not that the other two are absent.

6. **I do:** Press Cancel.  
   **I see:** The dialog closes. The session is still open in the sidebar and in the Info panel. The drafts are still there.  
   **State and events:** No archive call goes to the daemon. The drafts count is unchanged.  
   **Status:** Fakes only. `desktop/tests/native/archive_drafts.rs::cancel_keeps_a_session_with_unsubmitted_drafts` checks that the dialog closes and the session isn't archived. The reducer only clears the dialog (`features/session/src/lib.rs`, the `CancelArchive` arm). No test reads the drafts count after Cancel.

7. **I do:** Open Archive again and press Archive in the dialog.  
   **I see:** The dialog closes. The session is gone from the open list and shows as archived. A toast appears (step 9).  
   **State and events:** One archive is recorded with choice ARCHIVE (nothing is unpushed). The archive is recorded and the session is archived.  
   **Status:** Fakes only. `desktop/tests/native/archive_drafts.rs::archive_goes_ahead_once_confirmed_over_the_drafts` checks the session is archived after the confirm.

8. **I do:** Nothing. Check the drafts after the archive.  
   **I see:** The archived review's drafts are still in the store. Nothing shows them, because the review is archived.  
   **State and events:** The two draft comments keep status Draft and their ids. No comment is removed.  
   **Status:** Code, no test. The archive path never touches the comments. Searching `daemon/humatad/src/session_service/archive.rs` finds only the count (lines 451-452). Its only effect on the review is `self.tell.ended(&id)` (line 1101), which forgets the PR head check (`daemon/humatad/src/review/pull_requests.rs:103-108`). Comments are removed only by Drop (`daemon/humatad/src/review/comments.rs:209`, which removes a comment for good) and by the store's delete (`daemon/store/src/comments.rs:150`). No test asserts the drafts survive an archive.

9. **I do:** Read the toast after the archive.  
   **I see:** A toast reading "Archived “Review acme/api#482”." with an Undo action. There is no "Frees" part, because the live client returns no size. It stays for the Undo window, 10 seconds.  
   **State and events:** The toast carries the restore action for the session.  
   **Status:** Fakes only. `features/app/tests/nav_bugs.rs::an_archived_toast_with_undo_stays_for_the_whole_undo_window` checks that the toast stays for 9 seconds and goes at 10. It sends the delegate directly, not a real archive, so the text is not checked.

10. **I do:** Press Undo within 10 seconds.
    **I see:** The toast shows working, then closes. The session comes back in the sidebar, where it was.
    **State and events:** The app calls the daemon's restore for the session (`features/app/src/toast.rs:66-91`).
    **Status:** Code, no test for the Undo click on an archive toast. The daemon restore is proven for ordinary sessions: `daemon/humatad/tests/archive.rs::the_clones_go_when_the_window_closes_and_restore_clones_what_was_pushed`. No test restores a review.

11. **I do:** Open the restored review's Info panel and the review session.
    **I see:** "Drafts: 2" again, with the two cards, Edit and Drop on each.
    **State and events:** The review is loaded from the store on its first use after the restart, or from memory otherwise (`daemon/humatad/src/review.rs:239-261` reads the review record and its comments).
    **Status:** Code, no test. Nothing restores a review and then checks its drafts.

12. **I do:** Press Submit review, pick Comment, write a summary and submit.
    **I see:** One review posts for the PR with both drafts. Both cards show as posted, with their links.
    **State and events:** Both drafts move from Draft to Posted with a `posted_url`.
    **Status:** Code, no test after an archive and restore. Submit works on an open review (`daemon/humatad/tests/pr_review.rs::submitting_posts_one_review_per_pr_at_the_reviewed_head_with_its_drafts`), but no test submits after archive and restore.

13. **I do:** On a second review with no drafts and nothing unpushed, press Archive.
    **I see:** No dialog. The session archives at once, and the toast appears.
    **State and events:** One archive is recorded with choice ARCHIVE.
    **Status:** Fakes only. `desktop/tests/native/archive_drafts.rs::no_drafts_archive_at_once_with_no_drafts_line` checks that no drafts line shows and the session is archived. `features/session/tests/archive.rs::archive_goes_ahead_when_a_review_has_no_unsubmitted_drafts` checks only that no dialog opens.

14. **I do:** On a review with 2 drafts and one unpushed commit in its clone, press Archive.
    **I see:** The dialog shows the drafts line, then the unpushed box ("acme/api: ..." with "Archiving deletes that work."), then three buttons: Cancel, "Archive and delete the work", and "Commit, push and open a PR, then archive".
    **State and events:** The choice sent is COMMIT_PUSH_PR_THEN_ARCHIVE when I press the third button, and ARCHIVE_AND_DELETE_WORK for the second (`clients/live/src/daemon.rs:677-685`). The drafts are kept either way.
    **Status:** Code, no test for the combined dialog. The drafts line and the unpushed box are both drawn (`ui/src/overlay_archive.rs:49-57` and `:58-107`). The daemon refuses a plain archive with unpushed work (`daemon/humatad/tests/archive.rs::plain_archive_is_refused_while_there_is_unpushed_work`), which is why the dialog never offers one. No test covers both at once. Step 4's wording error also applies here.

15. **I do:** Delete the review (the delete action on the session).
    **I see:** An error saying the call is not built.
    **State and events:** Nothing changes. The drafts are not dropped by a delete, because the delete does nothing.
    **Status:** Not built. `daemon/humatad/src/session_service/archive.rs:913` answers `status::not_built(method)` for DeleteSession and PlanDelete, and `daemon/humatad/src/session_service.rs:2690` calls it. The only tracked note is in `workflow/milestones/ASK.md` (T3: "archive/delete RPCs aren't built"). No milestone names session delete directly.

## Branches

### The plan can't read the review

- **When:** the store can't read the review record or its comments when Archive is pressed.
- **I do:** Press Archive.
- **I see:** No dialog and no archive. An error line under the session names the failure and what to do.
- **Expected outcome:** Nothing is archived, and no draft is lost or hidden.
- **Status:** Code, no test. `daemon/humatad/src/session_service/archive.rs:451-452` passes the error up with `?`, and `features/session/src/lib.rs` (the `ArchivePlanned` `Err` arm) sets the session's error.

### Archive fails after I press Archive in the dialog

- **When:** the daemon refuses or fails the archive after Confirm.
- **I do:** Press Archive in the dialog.
- **I see:** The dialog stays open with the error. The session stays open.
- **Expected outcome:** The session is not archived, and its drafts are as they were.
- **Status:** Fakes only. `features/session/tests/archive.rs::failed_archive_keeps_the_open_session` checks the session stays open and not archived. It has no drafts, so the drafts part is Code, no test.

### The review lives on another machine

- **When:** the review session's clones are on a box, not This Mac.
- **I do:** Press Archive.
- **I see:** The dialog reads the count from that box's daemon (`daemon/humatad/src/session_service/archive_remote.rs:308-322` relays the plan).
- **Expected outcome:** The count is the box's drafts, and the same dialog applies.
- **Status:** Not built (PR A7). The flow doc says a review on another machine is unproven. The relay code exists, but no test archives a review on a box.

### The daemon stops in the middle of an archive

- **When:** the daemon stops after the archive is recorded but before the summary or clones are done.
- **I do:** Start the daemon again.
- **I see:** The archive is finished at restart, and the review's drafts are still there.
- **Expected outcome:** The archive finishes, and no draft is lost.
- **Status:** Code, no test for drafts. `daemon/humatad/tests/archive.rs::an_archive_cut_short_is_finished_at_restart` proves the archive finishes, but it has no drafts.

### I press Cancel while the archive is being sent

- **When:** the archive call is in flight and I press Cancel.
- **I do:** Press Cancel.
- **I see:** Cancel does nothing; the dialog stays until the archive finishes.
- **Expected outcome:** The archive goes ahead once, and no half-cancelled state.
- **Status:** Code, no test. Cancel is disabled while `sending` is set (`ui/src/overlay_archive.rs:94`), and `features/session/src/lib.rs` (the `CancelArchive` arm) ignores it while sending.

## Expected outcomes

- [ ] Archiving a review with unsubmitted drafts opens the dialog and does not archive at once.
- [ ] The dialog offers only Cancel and Archive when nothing is unpushed.
- [ ] The drafts line says the drafts are kept with the archived review.
- [ ] Cancel leaves the session open and its drafts unchanged.
- [ ] Archive keeps the drafts in the store (no comment is removed).
- [ ] The Undo toast restores the session, and restore brings the review's drafts back.
- [ ] A restored review's drafts can be submitted and post once.
- [ ] Archiving with no drafts and nothing unpushed goes straight ahead, with no dialog.
- [ ] With drafts and unpushed work both, the dialog shows the drafts line, the unpushed box and its buttons.
- [ ] Deleting the review is the only thing that drops the drafts (delete is not built).

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Two drafts (agent and me) | Works (real daemon) | `daemon/humatad/tests/pr_review.rs::the_agent_drafts_edits_and_drops_comments_without_reaching_github`, `::the_user_drafts_for_github_beside_the_agents_drafts` |
| 2. Plan counts unsubmitted drafts | Works (real daemon) | `daemon/humatad/tests/pr_review.rs::archive_plans_count_a_reviews_unsubmitted_drafts_until_they_are_submitted` |
| 3. Dialog asks first | Fakes only | `desktop/tests/native/archive_drafts.rs::unsubmitted_drafts_ask_before_archiving`; `features/session/tests/archive.rs::archive_asks_first_when_a_review_has_unsubmitted_drafts` |
| 4. Drafts line says drafts are kept | Likely bug | `ui/src/overlay_archive.rs:51-55` says "archiving drops them" |
| 5. Only Cancel and Archive with nothing unpushed | Code, no test | `ui/src/overlay_archive.rs:58`, `:119-130` |
| 6. Cancel leaves everything | Fakes only | `desktop/tests/native/archive_drafts.rs::cancel_keeps_a_session_with_unsubmitted_drafts` (drafts count not read) |
| 7. Archive goes ahead | Fakes only | `desktop/tests/native/archive_drafts.rs::archive_goes_ahead_once_confirmed_over_the_drafts` |
| 8. Drafts stay in the store | Code, no test | `daemon/humatad/src/session_service/archive.rs:1101` (only forgets the head check); `daemon/humatad/src/review/pull_requests.rs:103-108` |
| 9. Undo toast text and timing | Fakes only | `features/app/tests/nav_bugs.rs::an_archived_toast_with_undo_stays_for_the_whole_undo_window` (delegate sent directly) |
| 10. Undo restores the session | Code, no test | `features/app/src/toast.rs:66-91`; daemon restore generally: `daemon/humatad/tests/archive.rs::the_clones_go_when_the_window_closes_and_restore_clones_what_was_pushed` |
| 11. Restore brings drafts back | Code, no test | `daemon/humatad/src/review.rs:239-261` |
| 12. Submit after restore posts both | Code, no test | Submit on an open review only: `daemon/humatad/tests/pr_review.rs::submitting_posts_one_review_per_pr_at_the_reviewed_head_with_its_drafts` |
| 13. No drafts, nothing unpushed: straight through | Fakes only | `desktop/tests/native/archive_drafts.rs::no_drafts_archive_at_once_with_no_drafts_line`; `features/session/tests/archive.rs::archive_goes_ahead_when_a_review_has_no_unsubmitted_drafts` |
| 14. Drafts and unpushed both: dialog covers both | Code, no test | `ui/src/overlay_archive.rs:49-107`; `clients/live/src/daemon.rs:677-685` |
| 15. Delete the review drops the drafts | Not built | `daemon/humatad/src/session_service/archive.rs:913`; `daemon/humatad/src/session_service.rs:2690` |
| Branch: plan can't read the review | Code, no test | `daemon/humatad/src/session_service/archive.rs:451-452` |
| Branch: archive fails after Confirm | Fakes only | `features/session/tests/archive.rs::failed_archive_keeps_the_open_session` (no drafts) |
| Branch: review on another machine | Not built | PR A7; `daemon/humatad/src/session_service/archive_remote.rs:308-322` |
| Branch: daemon stops mid-archive | Code, no test | `daemon/humatad/tests/archive.rs::an_archive_cut_short_is_finished_at_restart` (no drafts) |
| Branch: Cancel while sending | Code, no test | `ui/src/overlay_archive.rs:94` |

Counts: steps 15 (Works 2, Fakes only 5, Code no test 6, Likely bug 1, Not built 1). Branches 5 (Code no test 3, Fakes only 1, Not built 1).

## Gaps and likely bugs

- Not built: deleting a session. `DeleteSession` and `PlanDelete` answer `not_built` (`daemon/humatad/src/session_service/archive.rs:913`), so the one thing that should drop the drafts cannot be shown. Only a note in `workflow/milestones/ASK.md` (T3) tracks it.
- Not built: a review on another machine (PR A7). Its archive count is relayed but untested.
- Likely bug: `ui/src/overlay_archive.rs:51-55` tells the person that archiving drops the drafts. D513's ruling says they are kept with the archived review, and the code keeps them (no path removes them). To confirm: open the dialog on a review with one draft and read the line. The fix is the wording, not the archive path.
- No test reads the drafts after an archive, after a restore, or after a submit that follows a restore. These are Code, no test, so a green run proves none of them.
- No test asserts the dialog's buttons when nothing is unpushed, or the combined dialog when drafts and unpushed work both exist.
