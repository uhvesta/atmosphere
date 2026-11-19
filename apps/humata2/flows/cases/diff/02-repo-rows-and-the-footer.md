# Each repo is one row I can fold, and the footer only offers what I can do

Layer: Diff and review. Platforms: macOS desktop.

## Starting state

- Machine `box-1`, channel `#payments`, session "Refunds" on branch `refunds-fix`, with two repos in it: `acme/api` (3 changed files, +40 −12) and `acme/ledger` (2 changed files, +8 −1, compared with `release/2.4`).
- The Diff panel is open beside the terminal. No file is open.
- No drafts for GitHub. The session is not a pull request review.
- The summary row above the list reads "5 files · 2 repos" with +48 −13 at the right.

## Reproduce headless

- Fakes: `bazel test //apps/humata2/features/review/...` with `--test_filter=collapse` or `--test_filter=toggling` (the fold reducer: toggling a repo twice, one session's folds not touching another's, folding the repo that holds the open file keeps the file selected).
- Fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=each_repo_group_pushes_its_own_branch` (`desktop/tests/native/push.rs`: each repo has its own Push & PR).
- Fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=a_failed_push_shows_on_its_group_with_try_again`.
- Fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=a_draft_for_github_has_edit_and_drop` (the footer with a draft, `desktop/tests/native/pr_session.rs`), and `--test_filter=drop_removes_the_draft` (the footer after the draft is dropped).
- Fakes: `bazel test //apps/humata2/features/app/...` with `--test_filter=session_sheet` or `--test_filter=sheet` (`features/app/tests/session_add_repo.rs`: the add-repo sheet opens for the session and a picked repo is cloned in it).
- Real daemon: `bazel test //apps/humata2/desktop:live_flows_test --test_filter=session_add_repo` (`desktop/tests/live_flows/session_add_repo.rs::a_repo_added_to_a_running_session_is_cloned_in_it_joins_the_channel_and_is_not_told_to_the_agent`).
- Not headless: the chevron's hover fill and the footer's visual weight. The spec's "one blue action per screen" is checked by eye only. No windows on the owner's screen.

## Steps

1. **I do:** read the top of the Diff list.
   **I see:** the summary row: "5 files · 2 repos", a small blue text button "Add repo…" at the left, and "+48 −13" at the right (DS-6 says this row goes; DS-77: it is status I can't act on).
   **State and events:** the row is drawn in `panel_changes.rs:413-450` with id `changes-add-repo`. The totals are the sum over all repos.
   **Status:** Code, no test. The summary row is gone (DV-16, DS-6); nothing asserts its absence. Frames 11 and 85 hold the panel as snapshots.

2. **I do:** click "Add repo…" in the summary row.
   **I see:** the add-repo sheet opens for "Refunds". I pick a repo from the library or from GitHub, and the sheet closes after a pick.
   **State and events:** `SessionAddRepoAction::Open(session)` (`ui/src/overlay_session_add_repo.rs`). The clone is made in the session, and the daemon is not told the agent about it.
   **Status:** Fakes only for the sheet and the clone (`features/app/tests/session_add_repo.rs::the_sheet_opens_for_the_session_and_closes_its_menu`, `::a_picked_repo_is_cloned_in_the_session_and_the_sheet_closes`). Works (real daemon) for the clone (`desktop/tests/live_flows/session_add_repo.rs::a_repo_added_to_a_running_session_is_cloned_in_it_joins_the_channel_and_is_not_told_to_the_agent`). **Add repo…** now sits in the footer (`changes-add-repo`), not a summary row.

3. **I do:** look at the first repo, `acme/api`.
   **I see:** two rows, not one. The first row: a chevron ⌄, a folder icon, "api" (the name with no owner, DS-5 "name without a leading /"), the branch "refunds-fix" in dim text, then "+40 −12". The second row, indented: "vs origin/main ⌄" (the base chip) and a "Push & PR" button at the right. DS-5 wants one row and the branch only when it differs from the session's.
   **State and events:** `repo_head` (`ui/src/panel_changes.rs:1425-1470`) draws the two rows. The branch is always the session's branch passed in (`:1450`). The base chip is `repo_base` (`:1491`). `Push & PR` is `push_control` (`:1787`).
   **Status:** Fakes only. Built (DV-10, DV-16): one row per repo with the chevron, name, the clone's branch only when it differs, the "against X" words (D848) and the totals. `diff_view.rs::each_repo_shows_its_own_clones_branch`, `::repo_totals_stay_inside_their_row_at_any_width`.

4. **I do:** click the chevron ⌄ beside `acme/api`.
   **I see:** nothing. The chevron looks like a control, and its file rows stay open.
   **State and events:** the chevron is `icon(IconName::ChevronDown, ...)` in a `row()` with no `id` and no `on_click` (`panel_changes.rs:1446`). Nothing sends `ReviewAction::ToggleRepo`. The reducer would fold the repo (`features/review/src/lib.rs:926-932`), but no UI sends it.
   **Status:** Fakes only. Likely bug cleared: the chevron is now the button `repo-toggle-{index}` and sends `ToggleRepo` (`review_header.rs::repo_head`). `diff_view.rs::a_collapsed_repo_hides_its_files_and_opens_again`.

5. **I do:** click the chevron again, or check the repo's files after folding.
   **I see:** the files of `acme/api` do not hide, because the list never reads the fold. Nothing folds.
   **State and events:** `ReviewState.collapsed` (`features/review/src/lib.rs:226`) is not read by `panel_changes.rs`. A fold in the reducer hides nothing on screen.
   **Status:** Fakes only. Built: the list reads `ReviewState.collapsed`; the same test clicks the chevron twice and sees the files hide and return, with the other repo untouched.

6. **I do:** fold a repo through the reducer, as the tests do (no screen sends it).
   **I see:** the reducer folds and unfolds a repo for this session only. Folding in one session leaves another session's folds alone. With no session open, a toggle changes nothing.
   **State and events:** `ReviewAction::ToggleRepo { repo }` keys `(session, repo)` (`features/review/src/lib.rs:926-932`).
   **Status:** Fakes only. `features/review/tests/collapse.rs::toggling_a_repository_twice_returns_to_the_first_state`, `::a_collapse_in_one_session_leaves_another_sessions_collapsed_repositories_alone`, `::with_no_open_session_a_toggle_changes_nothing`.

7. **I do:** pick a file in `acme/ledger` while it is folded, or step onto it with ⌥⌘↓.
   **I see:** this cannot happen, because nothing folds in the UI. A picked file opens its code under the list as in case 01.
   **State and events:** DS-78 says picking a file in a folded repo opens that repo. Not built with the list (DV-15).
   **Status:** Fakes only, on the reducer. Built (DV-15): `features/review/tests/stepping.rs::stepping_onto_a_file_in_a_collapsed_repo_opens_the_repo`. A file in a folded repo has no row to click, so stepping is the way in; no UI test drives it.

8. **I do:** collapse the repo that holds the open file, from a test harness.
   **I see:** the open file stays selected, so the code stays open.
   **State and events:** `saved.selected` is kept (`features/review/src/lib.rs:102-108`).
   **Status:** Fakes only. `features/review/tests/collapse.rs::collapsing_the_repository_of_the_selected_file_keeps_the_selection`. DS-78 is Built; both halves are proven on the reducer.

9. **I do:** click "Push & PR" under `acme/ledger`.
   **I see:** the button label becomes "Pushing…" and then the group names its pull request. The `acme/api` group is left as it was. A "Push & PR" button sits under both repos, even when nothing is unpushed (DS-10 wants it only when the plan reports unpushed work).
   **State and events:** `ReviewAction::Push { repo }` (`ui/src/panel_changes.rs:1787-1800`). The push's state is in `review.pushes` keyed by session and repo.
   **Status:** Fakes only. `desktop/tests/native/push.rs::each_repo_group_pushes_its_own_branch_and_names_its_pull_request`; the menu is `diff_view.rs::push_and_pr_opens_one_menu_listing_every_repo`. Not built: hiding Push & PR… when nothing is unpushed; no push plan is held in state, so it shows whenever a diff is listed (DS-10, no open task).

10. **I do:** push `acme/api` and the push fails (for example, the remote refuses).
    **I see:** the failure appears on its group, with "Try again" beside it, and the other group is unchanged.
    **State and events:** the failure is in `review.pushes` (RV-2).
    **Status:** Fakes only. `desktop/tests/native/push.rs::a_failed_push_shows_on_its_group_with_try_again`.

11. **I do:** look at the footer with no drafts.
    **I see:** "0 draft comments" at the left, and nothing at the right. The footer shows even though it has no action to offer (DS-10).
    **State and events:** `panel_changes.rs:745-792`. The text is always drawn; "Send N comments" appears only when `sendable > 0` (`when_some(... filter sendable > 0)`).
    **Status:** Code, no test. Settled by D850: the footer always shows in a session: Push & PR… and Add repo… at the left, and "Send to <agent>" at the right, disabled and grey when nothing waits. No test asserts the empty footer.

12. **I do:** add a draft comment on a line, then look at the footer.
    **I see:** the footer reads "1 draft comment" at the left and a blue "Send 1 comment" button at the right. Send is the one blue action (DS-11 is partly built: Push & PR and the counts are also accent blue today).
    **State and events:** `CommentsAction::SendAll(session)` on the Send button (`panel_changes.rs:770-790`).
    **Status:** Fakes only. `desktop/tests/native/pr_session.rs::a_draft_for_github_has_edit_and_drop_and_the_footer_counts_the_drafts` (a PR review: "N drafts · …"); `::a_review_footer_offers_send_as_a_secondary_left_of_submit`. In the plain Diff the blue "Send to <agent>" lights when a comment waits.

13. **I do:** drop the draft.
    **I see:** the footer goes back to "0 draft comments" with no action.
    **State and events:** the footer follows the drafts.
    **Status:** Fakes only. `pr_session.rs::drop_removes_the_draft_and_the_footer_follows`.

14. **I do:** look for Add repo… after the change the spec wants (DS-6, §8 item 3).
    **I see:** today it is in the summary row. The spec says it moves to the footer's left, beside Push & PR…, as a quiet text button, and the footer always shows in a session.
    **State and events:** the footer has no Add repo button yet.
    **Status:** Code, no test. Built (DV-16): Add repo… is a quiet text button in the footer beside Push & PR… (`panel_changes/mod.rs::changes`); frame 98's footer keeps only Add repo….

## Branches

### A review on a pull request: the repo header is different

When I open a pull request review session (PR #482 in `acme/api`).
I see the repo header from `review_repo_head` (`ui/src/panel_changes.rs:1045-1080`): a chevron drawn as ⌄ or ›, the name, "#482" in accent, and the branch. The chevron is drawn from the `open` flag. Nothing in that function sends a fold.
**Status:** Likely bug. The inline PR review's repo row still draws a chevron with no action: `review_header.rs::review_repo_head` (called from `panel_changes/mod.rs::changes` for a review). The Diff panel's chevron works; this one does not. No open task owns it.

### I restart the app with a repo folded

When I fold `acme/api` (through a test), quit, and reopen.
I see the repo unfolded. The spec says folds are per session and not kept across a restart (DS-74).
**State and events:** `ReviewState.collapsed` is "Not saved (DV-7)" (`features/review/src/lib.rs:225`). `ReviewSaved` keeps only `selected` and per-file `folded` (`:102-110`).
**Status:** Code, no test for the restart. `ReviewState.collapsed` is not in `ReviewSaved`, so a restart opens every repo (DS-74). The reducer's per-session state is in the Fakes tests above.

### The session has one repo

When the session has one repo and the summary row says "3 files · 1 repo".
I see the same summary row and Add repo (DS-6 says the row goes).
**Status:** Code, no test. With one repo the row and footer draw as for several; no summary row.

### An add-repo sheet is refused

When I pick a repo the daemon refuses.
I see the sheet stays open and says why.
**Status:** Fakes only. `features/app/tests/session_add_repo.rs::a_refused_repo_keeps_the_sheet_open_and_says_why`.

## Expected outcomes

- [ ] Each repo is one row: a working chevron that folds its files, its name without a leading slash, and its totals (DS-5).
- [ ] The branch shows only when it differs from the session's; the base shows only when it differs from the header's (DS-5).
- [ ] The summary row is gone, and Add repo… is in the footer's left (DS-6, §8 item 3).
- [ ] The footer shows only when it has an action (DS-10).
- [ ] Push & PR is offered only when the plan reports unpushed commits or uncommitted files, or while no plan is known (DS-10).
- [ ] Each repo's push pushes that repo only, and a failed push shows on its group.
- [ ] A fold is per session and not kept across a restart (DS-74).
- [ ] Folding the repo that holds the open file keeps the file open (DS-78).

## Status table

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Summary row | Code, no test | gone (DV-16); `panel_changes/mod.rs::changes` |
| 2 Add repo sheet and clone | Fakes only (sheet); Works (real daemon) (clone) | `features/app/tests/session_add_repo.rs`; `desktop/tests/live_flows/session_add_repo.rs` |
| 3 One row per repo | Fakes only | `diff_view.rs::each_repo_shows_its_own_clones_branch`; `review_header.rs::repo_head` |
| 4 Chevron folds | Fakes only | `diff_view.rs::a_collapsed_repo_hides_its_files_and_opens_again` (Likely bug cleared) |
| 5 Fold hides files | Fakes only | same test; `features/review/src/lib.rs::ReviewState::collapsed` |
| 6 Reducer folds per session | Fakes only | `features/review/tests/collapse.rs` (three tests) |
| 7 Pick or step onto a file in a folded repo | Fakes only | `features/review/tests/stepping.rs::stepping_onto_a_file_in_a_collapsed_repo_opens_the_repo` |
| 8 Fold keeps the open file | Fakes only | `features/review/tests/collapse.rs::collapsing_the_repository_of_the_selected_file_keeps_the_selection` |
| 9 Push & PR per repo | Fakes only | `desktop/tests/native/push.rs::each_repo_group_pushes_its_own_branch_and_names_its_pull_request`; "only when unpushed" not built (no push plan in state) |
| 10 Failed push | Fakes only | `push.rs::a_failed_push_shows_on_its_group_with_try_again` |
| 11 Footer with no drafts | Code, no test | D850: Send is disabled, not hidden; `panel_changes/mod.rs::changes` footer |
| 12 Footer with a draft | Fakes only | `pr_session.rs::a_draft_for_github_has_edit_and_drop_and_the_footer_counts_the_drafts` |
| 13 Drop the draft | Fakes only | `pr_session.rs::drop_removes_the_draft_and_the_footer_follows` |
| 14 Add repo in the footer | Code, no test | `panel_changes/mod.rs::changes` (`changes-add-repo`); frame 98 |
| Branch: PR review repo header | Likely bug | `review_header.rs::review_repo_head` (chevron has no action); no open task |
| Branch: restart with a fold | Code, no test | `features/review/src/lib.rs::ReviewSaved` has no `collapsed` |
| Branch: one repo | Code, no test | `panel_changes/mod.rs::changes` |
| Branch: refused repo | Fakes only | `session_add_repo.rs::a_refused_repo_keeps_the_sheet_open_and_says_why` |

Counts (18 rows): Works (real shell) 0; Works (real daemon) 0 as a row's first tag (step 2 is Fakes only, and its clone is also proven on the real daemon); Fakes only 12; Code, no test 5; Not built 0; Likely bug 1 (the PR review's repo chevron).

## Gaps and likely bugs

- Likely bug: the inline PR review's repo row draws a chevron with no action (`ui/changes/src/panel_changes/review_header.rs::review_repo_head`). The Diff panel's own chevron works (cleared). No open task owns this; a REVIEWUI follow-up should either make it fold (`ToggleRepo`) or draw no chevron.
- Not built: Push & PR… hides when nothing is unpushed (DS-10). `RepoDiff` has no ahead-of-remote signal and no push plan is held in state; the rule's "no plan known" case is what runs. No open task.
- Settled by D850: the footer always shows, with "Send to <agent>" disabled when nothing waits (the spec's "footer shows only when it has an action" was replaced).
- Gap in tests: no test asserts the empty footer or that the summary row is gone, no UI test steps onto a file in a folded repo, and a restart with a fold is untested.
- The step narratives above predate DV-10 and DV-16; only the Status lines and the table were re-audited on 2026-11-19.
