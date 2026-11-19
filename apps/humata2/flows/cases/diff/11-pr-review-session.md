# A pull request review shows what is reviewed, steps across both PRs, and keeps the agent's drafts clearly mine to send

Layer: Diff and review. Platforms: macOS desktop.

Covers DS-52, DS-53, DS-54, DS-55, DS-59 in `workflow/milestones/DIFFVIEW-spec.md`,
and spec §8 items 4 (where Submit review lives) and 8 (the draft count wording).
Mockup: `review` §1. Frames: 67 `67-pr-review-dialog`, 68 `68-pr-review-session`,
68b and 68c (full screen and after Show the terminal).

## Starting state

- Machine `box-1` runs the session "Refunds" in channel #payments.
- The review covers two pull requests, `acme/api` #482 and `acme/ledger` #91. Both are open. Each has a branch that is cloned on `box-1`.
- The app is open with the session "Refunds" beside the terminal. The Diff panel is open on the review.
- The agent, Copilot, has left two drafts on `acme/api` #482 and two on `acme/ledger` #91, for four drafts in all. One of the four drafts is mine, written in the panel, so the count is 3 by agent and 1 by me.
- I hold the mouse. No sheet or dialog is open.

## Reproduce headless

- Fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=pr_session` runs `desktop/tests/native/pr_session.rs`. That covers the header, the drafts, Edit and Drop, the focus, the bottom-up file list, and frame 68 (`pr_review_session_frame_68`).
- Fakes, review features: `bazel test //apps/humata2/features/review:github_drafts_test` and `:github_draft_cards_test` cover the draft rules and the draft card rules under `features/review/tests/`.
- Real daemon, live PR flow: `bazel test //apps/humata2/desktop:live_flows_test --test_filter=pr_review` runs `desktop/tests/live_flows/pr_review.rs` against a real `humatad2` with a fake `gh`. Its five tests cover starting a review of two PRs, the agent's focus reaching the view, drafts posted as one review per PR, new commits, and a review started on another machine.
- Not headless: the blue outline on a closed card (it is a frame check), and the look of the header at a real narrow width under the trackpad. The native tests draw the panel at fixed sizes.
- No window is opened on the owner's screen.

## Steps

| # | I do | I see | State and events | Status |
|---|---|---|---|---|
| 1 | Start a review of `acme/api` #482 and `acme/ledger` #91 from the PR review dialog. | The session opens with both PRs cloned. The agent is told about the review. | Two clones are made, one per head. The review is started on `box-1`. | Works (real daemon). `desktop/tests/live_flows/pr_review.rs::starting_a_review_of_two_pull_requests_clones_both_heads_and_tells_the_agent`. |
| 2 | Read the Diff panel's header. | It names the pull requests and the repos. The spec's example reads "PR #482 against main · 2 repos". The header has no base to pick, because a pull request's base is GitHub's. (DS-52) | `header` with `review` set (`ui/src/panel_changes.rs:690-846`). | Fakes only. `desktop/tests/native/pr_session.rs::the_diff_names_the_pull_request_and_each_repository` (`ui/changes/src/panel_changes/review_header.rs::review_line`). |
| 3 | Narrow the panel so the header is short of room. | The title ellipsizes before ⤢ and ✕. Both stay. (DS-52) | `panel_changes.rs:818-846` lays the header out. | Fakes only. `desktop/tests/native/pr_session.rs::a_narrow_review_panel_keeps_full_screen_and_close`. |
| 4 | Read the footer's count. | "Drafts: 4 (3 by agent, 1 by you)". The count matches the drafts. (DS-55, §8 item 8) | The count comes from `github_draft_counts` (`ui/src/panel_changes.rs:755-765`). The id is `review-drafts-3-1`. The mockup's wording is "4 drafts · 3 by Copilot, 1 by you", so the agent is not named. | Fakes only. Built (D861, RV-1): the footer reads "4 drafts · 3 by claude, 1 by you" (the agent's command; bare "4 drafts" if it can't fit). `pr_session.rs::a_draft_for_github_has_edit_and_drop_and_the_footer_counts_the_drafts`; unit `review_header.rs::the_count_names_who_wrote_the_drafts_when_it_fits`, `::a_count_that_does_not_fit_falls_back_to_the_bare_total`. |
| 5 | Look in the footer for **Submit review…**. | There is no Submit review in the footer, beside the terminal or in full screen. (DS-55) | The footer draws the drafts count and **Send N comments**. Submit review is not in it (`panel_changes.rs:745-790`). | Fakes only. Built (DV-18, RV-1): `desktop/tests/native/pr_session.rs::submit_review_in_the_footer_opens_the_sheet`; `diff_view.rs::a_pull_request_review_footer_has_submit_review`. Both run beside the terminal; full screen shares the footer but has no test. |
| 6 | Look at the session header while the Diff panel is open. | **Submit review** is in the session bar at the top. It is still there while the panel shows. (§8 item 4) | `ui/src/pages.rs:316-330` draws it whenever `session.pull_request_review` is set and the session is not Desk. There is no check for the Diff panel. §8 item 4 says it is hidden while the panel shows, so the screen has one blue action. | Fakes only. Built (RV-1): `desktop/tests/native/pr_session.rs::the_session_bar_hides_submit_review_while_the_diff_shows`. |
| 7 | Look for a file bar that steps across both PRs: "1 of 6 ‹ ›". | There is no such bar. The file header shows one file in one repo. (DS-53) | No file bar across PRs. The ⌥⌘↓ and ⌥⌘↑ keys are not bound (spec §3). | Fakes only. Built (DV-15): `desktop/tests/native/diff_keys.rs::the_inline_review_steps_files_across_its_pull_requests`; `features/review/tests/stepping.rs::next_and_previous_file_walk_the_files_across_repos_and_stop_at_the_ends`. |
| 8 | Read an agent's draft card. | The card says "Draft for GitHub · by" and the agent's name, with **Edit** and **Drop**. (DS-54) | The title comes from `comment_cards.rs:341` (`Draft for GitHub · by {agent}`). Edit and Drop are in the card. | Fakes only. Built, as D856 words it: "<agent> [Draft] line N" with Edit and a faded Drop (`ui/changes/src/comment_cards.rs::agent_github_draft`). `desktop/tests/native/pr_session.rs::a_draft_for_github_has_edit_and_drop_and_the_footer_counts_the_drafts`. |
| 9 | Look at a closed agent draft's outline. | The outline is grey, like any closed card. It is blue only when the card is open. (DS-54) | The card's border is blue only when `open` (`ui/src/comment_cards.rs:82-90`, the border at :86-87). | Code, no test. Settled by D856 and D861: an agent's draft keeps a plain grey line with the blue [Draft] chip; only the open box is outlined (`comment_cards.rs`). Frame 68 holds it. The old rule (blue outline on every draft) is dropped. |
| 10 | Click **Edit** on a draft, change the text, and close the box. | The draft opens in the box. Closing keeps the draft, changed. (DS-54) | The edit sets the card to open. The draft is kept when the box closes. | Fakes only. `desktop/tests/native/pr_session.rs::edit_opens_the_draft_in_the_box_and_closing_keeps_it`. |
| 11 | Click **Drop** on a draft. | The draft goes from the list. The footer's count goes down. (DS-54, DS-55) | `drop` removes the draft; the count is re-read. | Fakes only. `desktop/tests/native/pr_session.rs::drop_removes_the_draft_and_the_footer_follows`. |
| 12 | The agent focuses a line in a file. | The line is marked and scrolled into view. A line below the fold is brought up. (DS-59) | `review_code` (`ui/src/panel_changes.rs:948`) marks the focus and scrolls to it. | Fakes only. `desktop/tests/native/pr_session.rs::the_line_the_agent_focuses_is_marked` and `::a_focus_below_the_fold_is_scrolled_into_view`. |
| 13 | The agent focuses a line while I am still, on the real daemon. | The focus reaches the view. Undo puts my view back. (DS-59) | The focus travels from the agent through the daemon to the view. | Works (real daemon). `desktop/tests/live_flows/pr_review.rs::the_agents_focus_and_note_reach_the_view_when_the_person_is_still_and_undo_puts_it_back`. |
| 14 | Look at the file list across both PRs. | The files are listed bottom-up, the newest first, each under its repo. (DS-59) | The list is built per repo (`review_repo_head`, `ui/src/panel_changes.rs:1043`). | Fakes only. `desktop/tests/native/pr_session.rs::files_are_listed_bottom_up`. |

## Branches

### I scroll while the agent focuses a line

When I am moving the view, and the agent focuses a line.
I see the view stay where I put it, and the focus is marked without moving me. (The spec does not say what happens; the live test covers only the still case.)
Expected: a focus while I scroll does not pull the view away from me.
**Status:** Code, no test. `review_header.rs::reveal_focus` scrolls once per focus. The still case is step 13. No test covers a focus while I scroll.

### I edit a draft and close the box without saving

When I open a draft in the box, change the text, and close the box.
I see the draft as it was before I opened it, or with my change kept. The test says the draft is kept when the box closes.
Expected: the draft is still there, and it is still mine to send.
**Status:** Fakes only. `desktop/tests/native/pr_session.rs::edit_opens_the_draft_in_the_box_and_closing_keeps_it`.

## Expected outcomes

- [ ] The header names the review, each pull request, and the repo count (DS-52).
- [ ] A narrow header keeps ⤢ and ✕ (DS-52).
- [ ] The footer counts the drafts by author, and the count matches the drafts (DS-55).
- [ ] Each agent draft says "Draft for GitHub", and has Edit and Drop (DS-54).
- [ ] An agent draft is outlined in blue, so it reads as not sent (DS-54).
- [ ] Submit review is in the footer, beside the terminal and in full screen, and not in the session bar while the panel shows (DS-55, §8 item 4).
- [ ] A file bar steps across every PR of the review (DS-53).
- [ ] The agent's focus is marked and scrolled into view (DS-59).
- [ ] Files are listed bottom-up (DS-59).

## Status table

Counts per tag: Works (real shell) 0; Works (real daemon) 2; Fakes only 12;
Code, no test 2; Not built 0; Likely bug 0. Total 16 rows: 14 steps and 2
branches.

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Start a review | Works (real daemon) | `live_flows/pr_review.rs::starting_a_review_of_two_pull_requests_clones_both_heads_and_tells_the_agent` |
| 2 Header names the review | Fakes only | `pr_session.rs::the_diff_names_the_pull_request_and_each_repository` |
| 3 Narrow header | Fakes only | `pr_session.rs::a_narrow_review_panel_keeps_full_screen_and_close` |
| 4 Footer count | Fakes only | `pr_session.rs::a_draft_for_github_has_edit_and_drop_and_the_footer_counts_the_drafts`; unit tests in `review_header.rs` |
| 5 Submit review in footer | Fakes only | `pr_session.rs::submit_review_in_the_footer_opens_the_sheet` (not run in full screen) |
| 6 Session bar hides Submit review | Fakes only | `pr_session.rs::the_session_bar_hides_submit_review_while_the_diff_shows` |
| 7 File bar across PRs | Fakes only | `diff_keys.rs::the_inline_review_steps_files_across_its_pull_requests` |
| 8 Draft card label, Edit, Drop | Fakes only | `pr_session.rs::a_draft_for_github_has_edit_and_drop_and_the_footer_counts_the_drafts` |
| 9 Outline on a closed draft | Code, no test | settled by D856, D861: plain grey line, no blue outline |
| 10 Edit | Fakes only | `pr_session.rs::edit_opens_the_draft_in_the_box_and_closing_keeps_it` |
| 11 Drop | Fakes only | `pr_session.rs::drop_removes_the_draft_and_the_footer_follows` |
| 12 Focus marked and scrolled | Fakes only | `pr_session.rs::the_line_the_agent_focuses_is_marked`, `::a_focus_below_the_fold_is_scrolled_into_view` |
| 13 Focus reaches the view | Works (real daemon) | `live_flows/pr_review.rs::the_agents_focus_and_note_reach_the_view_when_the_person_is_still_and_undo_puts_it_back` |
| 14 Files bottom-up | Fakes only | `pr_session.rs::files_are_listed_bottom_up` |
| Scroll while focus lands | Code, no test | `review_header.rs::reveal_focus` |
| Edit, close, keep | Fakes only | `pr_session.rs::edit_opens_the_draft_in_the_box_and_closing_keeps_it` |

## Gaps and likely bugs

- Nothing is Not built or a Likely bug in this case after the audit. RV-1, DV-15, DV-18 and D856, D861 landed.
- Settled: the agent draft's blue outline (step 9) is dropped by D856 and D861; the footer wording names the agent by its command, not "agent" (§8 item 8).
- Gap in tests: Submit review in the full-screen footer, a focus landing while I scroll, and the narrow footer count.
- The step narratives above (no Submit review in the footer, no file bar, "Drafts: 4 (3 by agent…)") describe the 2026-11-18 code; only the Status cells and the counts were re-audited.
