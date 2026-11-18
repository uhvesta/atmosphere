# Review: the Changes panel, bases, line comments, marks, push, archive with unpushed work

Layer: explored · macOS · harness: `//apps/humata2/desktop:native_test` (filters `review`, `review_switches`, `push`, `archive_drafts`, `views`) and `//apps/humata2/desktop:live_flows_test` (filters `review`, `pr_review`); features in `//apps/humata2/features/review`, `//apps/humata2/features/session`

Phase 1 (map from code and snapshots, no run). Every step below is read from the reducers, the views and the existing tests. Steps marked PENDING RUN need a Bazel run to make the evidence.

## Setup

- Native tests use the fake daemon and the `design` fixture: session `ledger` (repos `ledger` on branch `ledger-schema`, and `api`), with 3 files in 2 repos, +184 −24.
- Live flows use the real app on live clients with a throwaway git root, seeded by `live_flows/review.rs` helpers (`shown`, `draft`, `pick_base`, `push_outcome`).
- Snapshot pictures are the `design/` PNGs (65 is the archive dialog) and the `native/*.view.txt` accessibility trees.
- Evidence folder: `<scratchpad>/verify/evidence/X-G/`.

## Steps

### 1. Open the Changes panel on a session with changes
- **Screen:** Right column "Changes" (panel toolbar: Inbox, Diff, Files, Board, Journal, Info). Header "Diff against [main ⌄]", "3 files · 2 repos", "+184 −24". One group per repo: `ledger` with a "Push & PR" button and branch `ledger-schema`, `api` with "Push & PR" and branch `ledger-schema` and "PR #1". File rows show a line count and `+N −N`. Footer "0 draft comments" (design/65 shows "1 draft comment" and "Send 1 comment").
- **Offered:** the base chip (opens the base menu), Expand panel, Close panel, a row per file (select), Open in editor on the selected file, Push & PR per repo, Fold per file (the group's Fold).
- **Action:** none (open the session with changes).
- **Result:** the selected file's diff shows in the same panel below the list.
- **Evidence:** `native/changes-pushed.view.txt`, `native/archive-unpushed.view.txt`, `panels/changes-light.txt`, `design/65-archive-unpushed.png` (panel beside the terminal).
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=push::each_repo_group_pushes` (expect pass; the tree shows both groups).

### 2. Pick what the diff is against (base chip, Compare with)
- **Screen:** the chip reads "Against main" (or "Against [branch]" or "Uncommitted only" etc.). The menu lists: "Default branch (origin/HEAD)" (`diff-base-origin-main`), "Uncommitted only" (`diff-base-uncommitted`), "Since I last reviewed" (`diff-base-since-reviewed`), "Since the fork from its parent" (`diff-base-since-fork`, only when the session is a fork), "Another session's branch" (`diff-base-other-brin` in the fixture), a "Branches · as of last fetch" filter field, and "Branch or commit…" (type a branch or commit). Per repo, the menu is "Comparing per repo" with "Against main" and "Pull request's base" for a PR review.
- **Offered:** pick a preset, filter branches, type a branch or commit name, Esc to close.
- **Action:** click the chip, then pick "Since I last reviewed" or "Uncommitted only".
- **Result:** the panel reruns the diff against that base; the menu closes; the chip changes. A session that is not a fork has no "Since the fork" line (`diff-base-since-fork` absent).
- **Evidence:** native test `review.rs` `the_base_menu_picks_what_the_diff_is_against_and_mark_reviewed_starts_it_over`; live `review.rs` `pick_base`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=review::the_base_menu` (expect pass).

### 3. A base with no changes
- **Screen:** an empty diff with one of: "No changes yet", "Nothing uncommitted", "No changes since you last reviewed", "No changes since the fork", "No changes against that branch", "No changes against the pull request's base", "No changes against each repository's base". Marker `diff-none-against-base` (the empty line for the chosen base) rather than `diff-no-changes-yet`.
- **Offered:** change the base chip.
- **Action:** pick a base with no changes.
- **Result:** the empty-state text names the base, not generic "no changes".
- **Evidence:** `panels` snapshot not captured for this state; text read from `ui/src/panel_changes.rs` lines 921–931.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=review::the_base_menu` (asserts `diff-none-against-base`).

### 4. Mark reviewed starts the since-reviewed base over
- **Screen:** "Mark reviewed" button in the Diff panel (`panel_changes.rs` lines 1144–1152).
- **Offered:** Mark reviewed; the base menu's "Since I last reviewed".
- **Action:** Mark reviewed, then open the base menu and pick "Since I last reviewed".
- **Result:** the since-reviewed base now compares from the mark; the diff shows no changes from before the mark (empty state "No changes since you last reviewed").
- **Evidence:** native `review.rs` `the_base_menu_picks...` (asserts the mark then the empty since-reviewed state).
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=review::the_base_menu`.
- **CLI:** `humata diff <SESSION> mark-reviewed` (PENDING RUN, see replay in the views record); it takes no paths and refuses them.

### 5. Read failure keeps the last diff
- **Screen:** on a refresh failure, the last diff stays with a line "Couldn't refresh. Showing the last diff read." and "Try again" (`diff-refresh-failed`, `diff-retry`). On a first-read failure: "Couldn't read the changes." with "Try again" (`diff-read-failed`) and no diff.
- **Offered:** Try again.
- **Action:** a refresh fails, then Try again succeeds.
- **Result:** the notice goes; the rows are the new read.
- **Evidence:** native `review.rs` `a_failed_refresh_keeps_the_last_diff_with_try_again` and `a_failed_first_read_names_the_error_with_try_again`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=review::a_failed`.

### 6. Click a diff line to open a comment box under it
- **Screen:** clicking a diff line opens a box (`doc-comment-box`) directly under that line, sized to its card. Shift-click a second line to stretch the box to a range (lines 41–43 in the test).
- **Offered:** type the comment in the box and close it (close control not confirmed from the code: PENDING RUN to read the box's buttons in `design/50-diff-comment-box.png`).
- **Action:** click a new-side line, type "Name this error.", close.
- **Result:** the comment is typed as a draft for `src/client.rs:42`. The box closes. Removed lines go on the old side (test `a_removed_line_takes_its_comment_on_the_old_side`).
- **Evidence:** native `review.rs` `clicking_a_diff_line...`, `shift_clicking...`, `a_removed_line...`; design `design/50-diff-comment-box.png`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=review::clicking_a_diff_line`.

### 7. Drafted comments show in the panel and send together
- **Screen:** each draft is a card under its line with Edit (`doc-edit-comment-<id>`). Footer shows "Send N comment(s)" (`send-comments`) and the count "Drafts: N (M by agent, K by you)" in the PR case.
- **Offered:** Edit a card's text, Send comments (sends all drafts at once to the session's agent), per-card controls.
- **Action:** draft a comment, then click Send.
- **Result:** the fake comments client gets the typed comment once; the drafts go to `sendable`; "Send comments" disappears.
- **Evidence:** native `review.rs` `drafted_comments_show_in_the_panel_and_send_together`, `a_draft_can_be_edited_from_its_card`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=review::drafted_comments`.
- **CLI:** `humata comment add <SESSION> <owner/repo>:<path>:<first>-<last> "<text>"` always writes a terminal-destination comment (PENDING RUN to confirm); `humata comment send <SESSION> [IDS]`; `humata comment ls`; `humata comment rm <ID>`.

### 8. Push one repo group and name its pull request
- **Screen:** "Push & PR" on the group header; while running "Pushing…"; on success "Pushed" with "PR #N" (`changes-pushed-api`); on failure "Push failed" with "Try again" (`changes-push-failed-api`).
- **Offered:** Push & PR per repo (`changes-push-<repo>`), Try again on failure.
- **Action:** click the `api` group's Push & PR.
- **Result:** only `api` is pushed; `ledger` stays "Push & PR". One push call is recorded.
- **Evidence:** `native/changes-pushed.view.txt` and `.png`; design `design/65-archive-unpushed.png`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=push` (both tests).
- **CLI:** `humata session push <SESSION> --repo <owner/repo> [--pr] [--message "..."]`.

### 9. Push fails, then succeeds on retry (live)
- **Screen:** the group shows "Push failed" and a retry control.
- **Action:** the first click's push fails (fake), then the second click succeeds.
- **Result:** `push_outcome(..., false)` then `true`; the group reads "Pushed … PR #N".
- **Evidence:** live `review.rs` `the_diff_panel_base_menu_line_comments_and_push_work_by_clicking` (steps 710–888).
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=review::the_diff_panel_base_menu`.

### 10. Archive a session with unpushed work asks first (D513-adjacent, unpushed branch)
- **Screen:** "Archive “Ledger schema migration”?" card with "Archiving stops Claude Code on devbox-1 and keeps the conversation, the record, the scrollback and a summary of the diff. It frees about 2.4 GB once the Undo window passes." An orange box lists per repo, e.g. "ledger: 3 uncommitted files on uhvesta/ledger-schema, never pushed." and "api: 1 commit on uhvesta/ledger-reads not pushed.", then "Archiving deletes that work." Buttons: Cancel, "Archive and delete the work" (red outline), "Commit, push and open a PR, then archive" (primary). Forks line: "Its 2 forks keep running; they move up in the tree." when there are forks.
- **Offered:** Cancel, Archive and delete the work, Commit, push and open a PR, then archive.
- **Action:** Archive from the session menu.
- **Result:** the dialog opens; nothing is archived yet (`archive-sheet`).
- **Evidence:** `design/65-archive-unpushed.png`; `native/archive-unpushed.view.txt` (tree of the panel; the modal is not in that tree, the PNG is the proof).
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=review::archiving_unpushed_work_asks_first`.

### 11. Cancel keeps the session
- **Screen:** dialog closes; session stays open in the tree.
- **Action:** Cancel.
- **Result:** not archived; no toast.
- **Evidence:** live `review.rs` `archiving_unpushed_work_asks_first_and_undo_brings_the_session_back` (cancel step).
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=review::archiving_unpushed_work`.

### 12. Commit, push and open a PR, then archive; Undo restores
- **Screen:** after the archive, the toast "Undo" button (`overlay_toasts.rs`). The sidebar then shows the session under "Archived" (`channel.rs` line 299).
- **Offered:** Undo on the toast.
- **Action:** the primary button, then Undo within the window.
- **Result:** the work is pushed, the session is archived, Undo brings it back to the tree.
- **Evidence:** live test above; design `design/66-archived-session.png`, `design/53-comment-archived-toast.png`.
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=review::archiving_unpushed_work`.

### 13. Archive with unsubmitted PR drafts asks first (D513)
- **Screen:** same dialog shape but no unpushed box, since nothing is unpushed. Line: "N draft(s) on the pull request(s) was/were never submitted; archiving drops it/them." (`archive-drafts`). Buttons: Cancel and "Archive" (`archive-confirm`). The drafts line appears only when the count is above 0.
- **Offered:** Cancel, Archive.
- **Action:** Archive on a review with 1 unsubmitted draft.
- **Result:** the dialog opens and the session is not archived.
- **Evidence:** native `archive_drafts.rs`; `features/session/tests/archive.rs` `archive_asks_first_when_a_review_has_unsubmitted_drafts`; daemon `daemon/humatad/tests/pr_review.rs` `archive_plans_count_a_reviews_unsubmitted_drafts_until_they_are_submitted`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=archive_drafts`.

### 14. Cancel with drafts keeps the session and its drafts; Archive with drafts goes ahead
- **Screen:** after Cancel the session shows; after Archive the session is archived.
- **Action:** Cancel, then Archive.
- **Result:** Cancel: not archived. Archive: archived (`archive_goes_ahead_once_confirmed_over_the_drafts`). The line still says "archiving drops them": see Surprises.
- **Evidence:** native `archive_drafts.rs`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=archive_drafts` (cancel and confirm tests).
- **PENDING RUN:** after Archive, check whether the drafts survive: restore the review and list drafts with `humata review drafts <SESSION>` (see the PR review record). Expected per D513: drafts kept with the archived review. Observed: not run yet.

### 15. No drafts and nothing unpushed: Archive at once
- **Screen:** no dialog; the session is archived; toast with Undo.
- **Action:** Archive.
- **Result:** archived immediately.
- **Evidence:** native `archive_drafts.rs` `no_drafts_archive_at_once_with_no_drafts_line`; `features/session/tests/archive.rs` `archive_goes_ahead_when_a_review_has_no_unsubmitted_drafts`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=archive_drafts::no_drafts`.

## Not reachable headless

- The real push over SSH to a remote box and its PR creation: only the fake push runs in native tests; live push goes through `clients/live` and needs `gh` on the host (live `review.rs` uses a fake GitHub behind the daemon).
- The Archive disk size estimate ("about 2.4 GB") comes from the fake plan; the real figure is a daemon measurement.

## Surprises

- **The archive drafts line says archiving drops the drafts.** `ui/src/overlay_archive.rs` line ~49–58 writes "archiving drops it/them". DECISIONS.md D513 (ruling 2026-11-16) says the opposite: archiving keeps the drafts with the archived review and they come back on restore; the line should say "kept with the archived review". The daemon's archive path (`daemon/humatad/src/session_service/archive.rs`) only counts drafts in the plan; nothing in that file drops them. So the UI text contradicts the decision and probably the behaviour. Verify in phase 2 (PENDING RUN in step 14).
- **Drafts-only dialog button says "Archive"** with no word about the drafts' fate beyond the line above; the red "Archive and delete the work" only appears with unpushed work. A person could read "Archive" as safe while the line says drafts are dropped.
- **Archive with unpushed work offers "Archive and delete the work"** in red as a second choice; the primary action pushes first. Fine, but the Cancel button is the only neutral way out.
- **Base chip names are not the CLI names.** The chip says "Against main"; the CLI wants `--against main|uncommitted|last-reviewed|fork|pr|<session>`. `--against last-turn` is refused with "this daemon can't compare against a turn yet" even though CLI.md names it.
- **Mark reviewed marks the whole diff.** The CLI refuses paths ("no paths: this daemon marks the whole diff reviewed"); the panel has no per-file mark.
- **Comment destination is not chosen on the CLI.** `humata comment add` always sends the destination as the session terminal; the GitHub draft destination only comes from a PR review (see the PR review record).
- **Empty-state text depends on the base** and is not the same generic line in every case. Good, but it means a test on "No changes yet" alone misses the other bases.
