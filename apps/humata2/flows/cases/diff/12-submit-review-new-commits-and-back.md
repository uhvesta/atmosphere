# I submit one review for every PR at once, see new commits as they land, and go back with one click

Layer: Diff and review. Platforms: macOS desktop.

Covers DS-56, DS-57, DS-58 in `workflow/milestones/DIFFVIEW-spec.md`. Mockup:
`review` §1 and §2. Frame 69 (`69-pr-submit-review`), with the session in frame 68.

## Starting state

- Machine `box-1` runs the review session "Refunds" in channel #payments. It reviews `acme/api` #482 and `acme/ledger` #91, both open, both cloned.
- The Diff panel is open beside the terminal. The agent has left drafts on both PRs. The person (me) has one draft of my own.
- The app is open on the session. Nothing else is open.
- The fake `gh` in the live test accepts a review post for each PR. In the native test, the fakes do the same.

## Reproduce headless

- Native, the sheet and the bar: `bazel test //apps/humata2/desktop:native_test --test_filter=pr_review` runs `desktop/tests/native/pr_review.rs`. It covers the sheet's verdicts, Submit, a failed post, Cancel, the header button, and frame 69 (`pr_submit_review_frame_69`).
- Native, the clone bar and the archive: `bazel test //apps/humata2/desktop:native_test --test_filter=pr_session` runs `desktop/tests/native/pr_session.rs::update_the_clone_sends_the_update_and_clears_the_notice` and `::a_merged_pull_request_offers_archive`.
- Real daemon with a fake `gh`: `bazel test //apps/humata2/desktop:live_flows_test --test_filter=pr_review` runs `desktop/tests/live_flows/pr_review.rs`. Its tests cover one review per PR with my draft in it, new commits with Update the clone, a merged PR with Archive, and a review started on another machine.
- Not headless: the real GitHub post, and the look of the bar on a real PR. The fake `gh` answers in the test.
- No window is opened on the owner's screen.

## Steps

| # | I do | I see | State and events | Status |
|---|---|---|---|---|
| 1 | Click **Submit review** in the session bar. | A sheet opens. It lists both PRs, each with its drafts, and says who wrote each. There is an optional summary. Frame 69. (DS-56) | `SubmitAction::Open` (`ui/src/pages.rs:316-330` sends it). The sheet is `ui/src/overlay_submit_review.rs`. | Fakes only. `desktop/tests/native/pr_review.rs::a_review_session_opens_submit_review_from_its_header` and `::pr_submit_review_frame_69`. |
| 2 | Read the drafts on each PR. | Each PR lists its own drafts, with **Edit** and **Drop**. The agent's summary is shown, and I can add another. (DS-56) | The sheet keeps one list per PR (`drafts_are_listed_per_pull_request_with_a_verdict_each`). | Fakes only. `desktop/tests/native/pr_review.rs::drafts_are_listed_per_pull_request_with_a_verdict_each` and `::the_agents_summary_shows_and_another_can_be_added`. |
| 3 | Pick a verdict for each PR: **Comment**, **Approve** or **Request changes**. | Each PR has its own verdict. The chosen one is marked. (DS-56) | `verdicts` (`ui/src/overlay_submit_review.rs:444-476`), ids `submit-verdict-{pr}-{verdict}`. | Fakes only. `desktop/tests/native/pr_review.rs::drafts_are_listed_per_pull_request_with_a_verdict_each`. |
| 4 | Click **Cancel**. | The sheet closes. Nothing is posted. The drafts stay. (DS-56) | Cancel sends no post. | Fakes only. `desktop/tests/native/pr_review.rs::cancel_closes_submit_review_without_posting`. |
| 5 | Pick a verdict on each PR, then click **Submit to GitHub**. | One review goes to each PR: `acme/api` #482 and `acme/ledger` #91. Each carries its own drafts. Nothing is sent before this click. (DS-56) | Two posts, one per PR, through the host's `gh`. My draft is posted with the agent's, in its own review. | Works (real daemon). `desktop/tests/live_flows/pr_review.rs::a_note_made_a_draft_and_the_persons_own_draft_post_in_one_review_per_pull_request`. Also `pr_review.rs::submit_posts_one_review_per_pull_request` on fakes. |
| 6 | Submit, and make one PR's post fail. | That PR's card shows the error. Its drafts stay on the list. (DS-56) | The failure is kept on the card; the drafts are not dropped. | Fakes only. `desktop/tests/native/pr_review.rs::a_failed_post_shows_its_error_on_its_card_and_keeps_its_drafts`. |
| 7 | A plan for the review cannot be made. | The sheet says why, and submits nothing. | The plan is not sent. | Fakes only. `desktop/tests/native/pr_review.rs::a_plan_that_cannot_be_made_says_why_and_submits_nothing`. |
| 8 | New commits land on `acme/api` #482 while the review is open. | A bar under the terminal says "2 new commits on the PR branch", with **Update the clone**. (DS-57) | `review_notices` (`ui/src/pages.rs:500-580`) draws `pr-new-commits` from `pr_heads.notice`. The bar sums new commits over open PRs. | Works (real daemon). `desktop/tests/live_flows/pr_review.rs::new_commits_update_the_clone_and_a_merged_pull_request_offers_archive`. Also `desktop/tests/native/pr_session.rs::update_the_clone_sends_the_update_and_clears_the_notice` on fakes. |
| 9 | Click **Update the clone**. | The bar goes. The clone is at the new head. While the update runs, the button is hidden. (DS-57) | `HeadsAction::UpdateClone` (`pages.rs:500-580`, the `updating` flag hides the button). | Works (real daemon). The same live test as step 8 runs the update. |
| 10 | The PR branch is force-pushed. | The bar says "The PR branch was force-pushed", with **Update the clone**. (DS-57) | The bar text is `pages.rs` (`(0, _)` arm). The fake in `pr_session.rs` always sends `force_pushed: false` (`pr_session.rs:270`), so the force-push text is not tested in the UI. The daemon detects force pushes in `daemon/humatad/tests/pr_refresh.rs`. | Code, no test. `ui/src/pages.rs::review_notices` has the "The PR branch was force-pushed" arm (`(0, _)`). The native helper in `pr_session.rs` always sends `force_pushed: false`, so no UI test reaches it. The daemon detects force pushes in `daemon/humatad/tests/pr_refresh.rs`. |
| 11 | A PR is merged. | A bar says "acme/ledger#91 was merged. Archive this review when you are done with it.", with **Archive**. (DS-57) | `review_notices` draws `pr-ended` for a closed or merged PR (`pages.rs:500-580`). Archive sends `HeadsAction::Archive`. | Works (real daemon). `desktop/tests/live_flows/pr_review.rs::new_commits_update_the_clone_and_a_merged_pull_request_offers_archive`. Also `pr_session.rs::a_merged_pull_request_offers_archive` on fakes. |
| 12 | Look for a Back button in the session's top bar. | There is no ‹ button at the start of the bar. The bar starts with the channel's name and the session's title. (DS-58) | `session_header` (`ui/src/pages.rs:129-200`) draws crumbs only. The top bar's Back (`ui/src/shell.rs:187-216`) is drawn only off sessions (`shell.rs:507`). | Fakes only. Built (DV-18, D848): the session header starts with the 24 pt ghost Back, tooltip "Back  ⌘[" (`ui/src/pages.rs`, id `session-back`). `desktop/tests/native/diff_full_screen.rs::a_session_page_has_a_back_button` checks it exists. |
| 13 | Press ⌘[ in the session. | The app goes back to where I was before this session. | `cmd-[` is bound to `Back` (`ui/src/keys.rs:204`). | Code, no test. `cmd-[` is bound to `Back` (`ui/core/src/keys.rs`) and is in the ⌘/ sheet. No test presses ⌘[ in a session. |
| 14 | Open a page that is not a session, and look at its top bar. | A ‹ button at the start, with the tooltip "Back  ⌘[". It is disabled when there is nowhere to go. (DS-58) | `top_bar` (`ui/src/shell.rs:187-216`) draws it with `can_go_back`. | Code, no test. `ui/src/shell.rs` (`back`) draws it with the tooltip "Back  ⌘[", disabled when there is nowhere to go. No test. |

## Branches

### Submit fails, and I submit again

When a post fails (step 6), and I press **Submit to GitHub** again.
I see the error stay on the failed PR's card until the next post. The drafts stay on the list, so nothing I wrote is lost.
Expected: the second submit posts only what has not been posted. (The test checks that the drafts stay; it does not check a second post.)
**Status:** Fakes only. `desktop/tests/native/pr_review.rs::a_failed_post_shows_its_error_on_its_card_and_keeps_its_drafts`.

### A session with no pull request

When the session reviews no PR at all.
I see no **Submit review** in the session bar.
Expected: the header has no Submit review button.
**Status:** Fakes only. `desktop/tests/native/pr_review.rs::a_session_that_reviews_no_pull_request_has_no_submit_review`.

### A force-push comes with new commits

When the PR branch is force-pushed and it also has new commits, so `new_commits` is more than zero.
I see only the commit count, "2 new commits on the PR branch". The force-push is not named.
Expected: the bar says both things, so I know the history was rewritten and not only added to.
Why: `review_notices` (`ui/src/pages.rs:500-580`) matches on `(commits, rewritten)`. The `(n, _)` arm ignores `rewritten`, and the force-push text is used only when there are no new commits (`(0, _)`).
**Status:** Fakes only. Fixed by REVIEWUI batch 3: `review_notices` says "The PR branch was force-pushed with 2 new commits"; the notice is a `Status` with its words as its label (`pr_session.rs::a_force_push_with_new_commits_names_both`).

## Expected outcomes

- [ ] Submit review opens one sheet for every PR of the review, with each PR's drafts and verdict (DS-56).
- [ ] Nothing is sent until Submit to GitHub is clicked (DS-56).
- [ ] Submit to GitHub posts one review per PR, with the drafts for that PR (DS-56).
- [ ] Cancel closes the sheet and posts nothing (DS-56).
- [ ] A failed post shows its error on its card and keeps its drafts (DS-56).
- [ ] New commits on a PR show as a bar under the terminal, with Update the clone (DS-57).
- [ ] A force-push shows as a force-push, with or without new commits (DS-57).
- [ ] A merged or closed PR offers Archive (DS-57).
- [ ] A session has a ‹ Back control at the start of its top bar, with the tooltip "Back  ⌘[" (DS-58).
- [ ] ⌘[ goes back to where I was, in a session and on other pages (DS-58).

## Status table

Counts per tag: Works (real shell) 0; Works (real daemon) 4; Fakes only 11;
Code, no test 2; Not built 0; Likely bug 0. Total 17 rows: 14 steps and 3
branches.

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Submit review opens the sheet | Fakes only | `pr_review.rs::a_review_session_opens_submit_review_from_its_header`, `::pr_submit_review_frame_69`; the footer button by `pr_session.rs::submit_review_in_the_footer_opens_the_sheet` |
| 2 Drafts and summary per PR | Fakes only | `pr_review.rs::drafts_are_listed_per_pull_request_with_a_verdict_each`, `::the_agents_summary_shows_and_another_can_be_added` |
| 3 Verdicts | Fakes only | `pr_review.rs::drafts_are_listed_per_pull_request_with_a_verdict_each` |
| 4 Cancel | Fakes only | `pr_review.rs::cancel_closes_submit_review_without_posting` |
| 5 Submit posts one review per PR | Works (real daemon) | `live_flows/pr_review.rs::a_note_made_a_draft_and_the_persons_own_draft_post_in_one_review_per_pull_request` |
| 6 Failed post keeps drafts | Fakes only | `pr_review.rs::a_failed_post_shows_its_error_on_its_card_and_keeps_its_drafts` |
| 7 Plan cannot be made | Fakes only | `pr_review.rs::a_plan_that_cannot_be_made_says_why_and_submits_nothing` |
| 8 New commits bar | Works (real daemon) | `live_flows/pr_review.rs::new_commits_update_the_clone_and_a_merged_pull_request_offers_archive` |
| 9 Update the clone | Works (real daemon) | the same live test; `pr_session.rs::update_the_clone_sends_the_update_and_clears_the_notice` |
| 10 Force-push text | Fakes only | `pr_session.rs::a_force_push_with_new_commits_names_both` (force-push alone and with commits) |
| 11 Merged PR offers Archive | Works (real daemon) | `live_flows/pr_review.rs::new_commits_update_the_clone_and_a_merged_pull_request_offers_archive`; `pr_session.rs::a_merged_pull_request_offers_archive` |
| 12 Back in the session header | Fakes only | `diff_full_screen.rs::a_session_page_has_a_back_button` |
| 13 ⌘[ in a session | Code, no test | `ui/core/src/keys.rs` (`cmd-[`) |
| 14 Back on other pages | Code, no test | `ui/src/shell.rs` (`back`) |
| Submit fails, then again | Fakes only | `pr_review.rs::a_failed_post_shows_its_error_on_its_card_and_keeps_its_drafts` |
| No pull request, no Submit | Fakes only | `pr_review.rs::a_session_that_reviews_no_pull_request_has_no_submit_review` |
| Force-push with new commits | Fakes only | `pr_session.rs::a_force_push_with_new_commits_names_both` |

## Gaps and likely bugs

- Fixed (REVIEWUI batch 3): a force-push that also adds commits names both (`ui/src/pages.rs::review_notices`; `pr_session.rs::a_force_push_with_new_commits_names_both`).
- Cleared: Back in the session header (DV-18, D848). Submit review is in the footer and hidden from the session bar while the Diff shows (RV-1, case 11).
- Code, no test: the force-push text on its own (the helper in `pr_session.rs` always sends `force_pushed: false`); ⌘[ in a session; the top bar's Back on other pages.
- Not proven live: the bar on a real GitHub PR, and a force-push on a real PR. The live test uses the fake `gh`.
- The step narratives for 1, 12 and 13 describe the 2026-11-18 code; only the Status cells and the counts were re-audited.
