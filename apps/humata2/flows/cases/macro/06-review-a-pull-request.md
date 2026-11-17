# A teammate's pull request is reviewed with an agent: drafts, marks that survive a force push, and one submit

Layer: macro. Platforms: macOS, Linux, web. (The keys shown are the macOS ones; Linux uses `Ctrl`.)

## Starting state

- Humata 2 is open on This Mac. This Mac is the host, and its `gh` is signed in as me.
- Machines: This Mac (online). No other machine is needed for this case.
- The channel `payments` exists, with `acme/api` as its repo.
- A teammate has asked me to review `acme/api#482`, "Cache the token refresh". It has three commits. Its head branch is in `acme/api` itself, not a fork.
- No session reviews `acme/api#482` yet. This matters for step 1 (see D197 in `DECISIONS.md`).
- The Inbox is empty. The palette is closed.

## Steps

1. **I do:** Press ⌘K and paste `https://github.com/acme/api/pull/482`.
   **I see:** The palette offers "Open pull request #482". It does not offer "Review this PR", because no session here reviews this PR yet.
   **State and events:** The palette shows only what it can act on. No review is started.
   **Status:** Works (real daemon). `desktop/tests/live_flows/find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds`, step 3, pastes a PR URL into the palette of an app on a live host and finds no Review entry; on fakes, `features/palette/tests/review_pull_gate.rs::a_pull_request_no_session_reviews_offers_no_review`. The daemon can resolve the URL too: `daemon/humatad/tests/pr_review.rs::a_pull_request_resolves_from_its_url_and_from_owner_repo_number` (with a fake `gh`).

2. **I do:** Once a session reviews the pull request, pick "Review this PR" in the palette. In the dialog it opens, pick This Mac and the agent, and press Start.
   **I see:** The "Review a pull request" dialog (frame 67) resolves the PR and shows its title, branches and size. Start opens the new review session.
   **State and events:** The dialog would call the daemon to start the review.
   **Status:** Not built (PR milestone C1, C5, D1; D197). No dialog exists. Today the palette offers no review entry for this PR, so nothing is started from the app.

3. **I do:** Nothing. The daemon makes the review.
   **I see:** A `pr-reviews` channel exists with one session on the PR's head branch. The session's clone is on its own branch name.
   **State and events:** The channel and session are recorded. The session is bound to the PR and its push is not allowed yet.
   **Status:** Works (real daemon). `daemon/humatad/tests/pr_review.rs::starting_a_review_makes_the_pr_reviews_channel_and_one_session_on_every_prs_head` and `daemon/humatad/tests/pr_review.rs::a_second_review_reuses_the_channel_and_clones_a_fork_pr_from_its_pull_ref`. The app has no client for this call (`clients/src` has no pull request client; PR milestone B1), so the sidebar's `pr-reviews` entry is Not built (PR B1, C1).

4. **I do:** Nothing. The review session opens with its agent terminal.
   **I see:** The agent's first message names the PR's URL, title, base and head, and my context ("Focus on the refresh race"). It then waits for my first question.
   **State and events:** The review chat is told each PR and my context. It does not review on its own.
   **Status:** Works (real daemon). `daemon/humatad/tests/pr_review.rs::the_review_chat_is_told_each_pr_and_the_users_context`. The review chat as an app view has no client (PR milestone B2, C2).

5. **I do:** Ask the chat: "Where does the refresh lock get taken?"
   **I see:** The agent answers in its terminal. Its focus moves the view to `src/auth/refresh.rs` at the line it names. The view changes only when the agent asks for it, and each change can be undone.
   **State and events:** The view change is recorded as a change to the review view. The change has an Undo.
   **Status:** Not built (PR milestone C2). The daemon side is built and tested: `daemon/humatad/tests/review.rs::the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off`. No app client watches those changes (no `WatchReviewChanges` call in `clients/`), so the view does not move on my screen.

6. **I do:** Ask the agent to draft a comment on line 40 of that file for GitHub.
   **I see:** The agent's draft appears as a card under its drafts, with the author marked as the agent. Nothing is posted to GitHub.
   **State and events:** A draft comment with destination `pull_request` is stored, status draft. Its author is the agent.
   **Status:** Works (real daemon). `daemon/humatad/tests/pr_review.rs::the_agent_drafts_edits_and_drops_comments_without_reaching_github` (fake `gh`, which records that nothing was posted). The draft card is Not built in the app (PR milestone C6, D2).

7. **I do:** Select line 52 in the same file, press Draft for GitHub beside Comment, and type "Why not hold the lock across the await?"
   **I see:** My draft appears beside the agent's, under the same file, with my name on it.
   **State and events:** The draft is stored as mine, not the agent's, and it is also a draft, not posted.
   **Status:** Works (real daemon). `daemon/humatad/tests/pr_review.rs::the_user_drafts_for_github_beside_the_agents_drafts`. The button "Draft for GitHub" is Not built in the app (PR milestone C6).

8. **I do:** Ask the chat for a note on a point, then press Make it a draft on the note.
   **I see:** The note becomes a draft in the same place. One Undo puts the note back.
   **State and events:** The note's id is kept. Its destination changes from a note to a pull request draft.
   **Status:** Works (real daemon). `daemon/humatad/tests/pr_review.rs::a_note_of_the_review_chat_becomes_a_draft_in_place`. The button is Not built in the app (PR milestone C6).

9. **I do:** Mark the commits reviewed up to the head, on the PR's commit list.
   **I see:** All three commits show as reviewed, with the time and the head I reviewed.
   **State and events:** Each mark records when, which head, and who. The snapshot of what I was shown is pinned.
   **Status:** Works (real daemon). `daemon/humatad/tests/pr_commits.rs::mark_reviewed_marks_every_commit_up_to_head_and_pins_its_snapshot` and `daemon/humatad/tests/pr_commits.rs::marking_records_when_which_head_and_who`. The PR commit list is Not built in the app (PR milestone C2, D2). The local diff's Mark reviewed works in the app (`features/review/tests/diffs.rs::since_i_last_reviewed_shows_only_what_changed_after_mark_reviewed`, Fakes only).

10. **I do:** Nothing. The teammate force-pushes a rebase that keeps the same patch for two commits.
    **I see:** The two commits keep their reviewed mark. The head change is reported once, as one force push, on the feed.
    **State and events:** The mark moves to the same patch under the new commit id. The force push is recorded as one change.
    **Status:** Works (real daemon). `daemon/humatad/tests/pr_commits.rs::a_force_push_carries_a_mark_to_the_same_patch` and `daemon/humatad/tests/pr_review.rs::a_force_push_is_reported_as_one`.

11. **I do:** Nothing. The teammate also changes the code in the third commit.
    **I see:** That commit is unreviewed again. The other two stay reviewed.
    **State and events:** The changed patch has no mark. Its earlier mark stays in the history.
    **Status:** Works (real daemon). `daemon/humatad/tests/pr_commits.rs::a_changed_patch_after_a_force_push_is_unreviewed`.

12. **I do:** Press Refresh on the PR.
    **I see:** New commits appear before the next poll, and the head change is shown.
    **State and events:** The head is fetched now. A second refresh at the same time asks GitHub once only.
    **Status:** Works (real daemon). `daemon/humatad/tests/pr_refresh.rs::refreshing_now_finds_new_commits_before_the_next_poll` and `daemon/humatad/tests/pr_refresh.rs::two_refreshes_at_once_ask_github_once`. The Refresh button and the head-change notice are Not built in the app (PR milestone C4).

13. **I do:** Press Update the clone.
    **I see:** The clone fetches the new head. The agent is told which files changed. Lines of my drafts that changed show as outdated. The rest stay as they were.
    **State and events:** The clone's head moves. Changed drafts are marked outdated. The agent receives the list of changed files.
    **Status:** Works (real daemon). `daemon/humatad/tests/pr_review.rs::updating_the_clone_fetches_the_head_tells_the_agent_and_outdates_changed_lines`. The button is Not built in the app (PR milestone C4).

14. **I do:** Open the review again and look only at what changed since my last mark.
    **I see:** The commits since my mark are listed as new. The others stay as reviewed.
    **State and events:** The mark on each commit is read again from the daemon.
    **Status:** Not built (PR milestone C4, D2). The daemon keeps the marks (`daemon/humatad/tests/pr_commits.rs::a_second_review_of_the_pr_sees_its_marks`, Works real daemon), but the app's PR view that lists the new commits is not built.

15. **I do:** Press Submit review. The plan opens.
    **I see:** The plan lists my drafts and the agent's drafts per PR. A draft on a line outside the PR's diff is set apart with a note. The summary and a verdict are shown for me to choose.
    **State and events:** The plan has a single-use token. Nothing is posted yet.
    **Status:** Works (real daemon). `daemon/humatad/tests/pr_review.rs::the_submit_plan_lists_drafts_per_pr_and_sets_apart_those_outside_the_diff`. The Submit review screen (frame 69) is Not built (PR milestone C3, D3).

16. **I do:** Pick "Comment" as the verdict, write a summary and press Submit review. Then press Submit review again by mistake.
    **I see:** One review posts for the PR, at the head I reviewed, with its drafts. Each draft shows as posted, with its link. The second press posts nothing.
    **State and events:** The posted review is written down before the call and marked posted after it. A second submit with the same plan is a no-op.
    **Status:** Works (real daemon). `daemon/humatad/tests/pr_review.rs::submitting_posts_one_review_per_pr_at_the_reviewed_head_with_its_drafts` and `daemon/humatad/tests/pr_review.rs::a_submit_plan_posts_only_once`.

## Branches

### The pull request cannot be fetched

- **When:** the PR does not exist, or I cannot see it.
- **I do:** Paste `acme/api#999` in the palette and open it.
- **I see:** The PR is not found. The message says what to do, for example to check the number and the repo name.
- **Expected outcome:** No session is started and nothing is written.
- **Status:** Works (real daemon). `daemon/humatad/tests/pr_review.rs::a_pull_request_that_cant_be_fetched_says_why_and_what_to_do`. The dialog that would show it is Not built (PR milestone C1).

### The lookup is rate limited

- **When:** GitHub says the rate limit is reached.
- **I do:** Paste the PR again straight away.
- **I see:** The message says to wait until the limit resets, and gives the time it resets.
- **Expected outcome:** The error says to wait. It does not tell me to add a token.
- **Status:** Works (real daemon). `daemon/humatad/tests/pr_review.rs::a_rate_limited_lookup_says_to_wait_until_the_limit_resets`.

### The review session tries to push

- **When:** the agent in the review session tries to push a branch.
- **I do:** Nothing, or I allow it for that session.
- **I see:** The push is refused and the refusal says why. It runs only after I let this session push.
- **Expected outcome:** Only I can let a review push. The agent's push waits until then.
- **Status:** Works (real daemon). `daemon/humatad/tests/pr_review.rs::a_review_session_cant_push_until_the_user_lets_it` and `daemon/humatad/tests/pr_review.rs::only_the_user_can_let_a_review_push`. The screen that asks me to allow it is not checked here.

### Posting fails for one PR of several

- **When:** GitHub refuses one PR's review, for example because a drafted line is not in its diff.
- **I do:** Read the failure, fix or drop that draft, and submit again.
- **I see:** The PR that failed is named, with what to fix. The other PRs in the same submit are posted. That PR's drafts stay as drafts.
- **Expected outcome:** Only the PR that failed keeps its drafts. Nothing is posted twice.
- **Status:** Works (real daemon). `daemon/humatad/tests/pr_review.rs::a_failed_post_keeps_that_prs_drafts_and_says_which_pr_failed`.

### Crash right after posting

- **When:** the daemon stops after GitHub has the review but before Humata has marked it posted.
- **I do:** Start the daemon again.
- **I see:** The restart finds the review on GitHub and marks the drafts posted. It does not post them again.
- **Expected outcome:** One review on GitHub, and the drafts show as posted.
- **Status:** Works (real daemon). `daemon/humatad/tests/pr_review.rs::a_review_posted_just_before_a_crash_is_found_on_github_not_posted_again`. When GitHub never lists the review, the drafts stay unconfirmed and are not posted again: `daemon/humatad/tests/pr_review.rs::a_review_github_never_lists_is_left_unconfirmed_not_posted_again`.

### This Mac is stopped while the review is open

- **When:** I press Stop all on This Mac, or the machine is stopped.
- **I do:** Stop the machine, and wait for a new PR head on GitHub.
- **I see:** The review stops following the PR's heads, so no new head appears while it is stopped. When it starts again, it follows the PR again and shows the new head.
- **Expected outcome:** A stopped machine asks GitHub for nothing. A submit cut short by the stop is settled when the machine starts.
- **Status:** Works (real daemon). `daemon/humatad/tests/pr_stopped.rs::a_stopped_machine_stops_following_its_reviews_and_its_start_follows_them_again` and `daemon/humatad/tests/pr_stopped.rs::a_submit_cut_short_on_a_stopped_machine_is_settled_at_its_start`.

### The review chat tries to mark my commits reviewed

- **When:** the agent is asked, or decides, to mark commits as reviewed for me.
- **I do:** Nothing.
- **I see:** The agent cannot change my review state. It can read the marks and focus the view.
- **Expected outcome:** Only I mark commits. The agent reads them.
- **Status:** Works (real daemon). `daemon/humatad/tests/pr_commits.rs::only_the_user_marks_commits_the_agent_reads_them` and `daemon/humatad/tests/review.rs::the_sessions_agent_may_read_and_focus_but_not_change_the_users_review_state`.

### A draft is made for a PR outside the review

- **When:** the agent or I draft a comment on a PR that this review does not cover.
- **I do:** Draft for a PR that is not in the review.
- **I see:** The draft is refused. The refusal says the PR is not part of this review.
- **Expected outcome:** Nothing is stored for that PR.
- **Status:** Works (real daemon). `daemon/humatad/tests/pr_review.rs::a_draft_for_a_pr_outside_the_review_is_refused`.

### I undo a view change the agent made

- **When:** the agent moved the view and I do not want that.
- **I do:** Press Undo on the view change.
- **I see:** The view goes back to where it was.
- **Expected outcome:** The change is undone and the chat keeps its notes.
- **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off`. The Undo button in the app is Not built (PR milestone C2).

### A note's lines change under it

- **When:** the author's new head changes the lines a note points to.
- **I do:** Look at the note after Update the clone.
- **I see:** The note is marked outdated. Notes on lines that did not change stay where they were.
- **Expected outcome:** Outdated notes are shown as outdated, and the rest keep their place.
- **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::a_later_diff_marks_notes_whose_lines_changed_outdated_and_moves_the_rest`.

### A comment with no destination is sent to CreateComment

- **When:** a client sends a comment with no destination at all.
- **I do:** Nothing a person can do from the app. The app never sends an empty destination.
- **I see:** Nothing in the app. A client gets the error "CommentService.CreateComment for a pull request is not built".
- **Expected outcome:** The error should say what is wrong with the request, and should not name a pull request.
- **Status:** Likely bug. `daemon/humatad/src/review.rs:849` to `:855` only accepts Terminal, PullRequest or Note destinations. Any other value, including an unset one, reaches the `not_built` at `review.rs:851`, and the message says "for a pull request". A pull request draft does not reach it: `daemon/humatad/tests/pr_review.rs::the_user_drafts_for_github_beside_the_agents_drafts` sends a PullRequest destination and succeeds. `clients/live/src/comments.rs:147` treats an unset destination as a terminal comment, so the two sides disagree. Confirm by sending a `CreateCommentRequest` with no destination.

## Expected outcomes

- [ ] A pasted PR with no reviewing session offers no review entry, and starting nothing.
- [ ] Starting a review creates one `pr-reviews` channel and one session on the PR's head, and the session's push is refused until I allow it.
- [ ] The review chat is told the PR and my context, and it never reviews on its own.
- [ ] A view change the chat makes can be undone, and the agent cannot change my review state.
- [ ] Agent drafts and my drafts sit side by side, and nothing is posted until I submit.
- [ ] A marked commit keeps its mark through a force push that keeps its patch, and loses it when the patch changes.
- [ ] Refresh finds new commits, and Update the clone outdates only the drafts whose lines changed.
- [ ] Submit posts one review per PR at the reviewed head, and a second submit posts nothing.
- [ ] A failed post names its PR and keeps only that PR's drafts.
- [ ] A crash after posting does not post the review again.
- [ ] A stopped machine asks GitHub for nothing until it starts again.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Paste the PR, no review offered | Works (real daemon) | `desktop/tests/live_flows/find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds` (step 3) |
| 2. Review this PR dialog and Start | Not built | PR milestone C1, C5, D1; D197 |
| 3. Daemon makes the channel and session | Works (real daemon) | `daemon/humatad/tests/pr_review.rs::starting_a_review_makes_the_pr_reviews_channel_and_one_session_on_every_prs_head` |
| 4. Chat is told the PR and my context | Works (real daemon) | `daemon/humatad/tests/pr_review.rs::the_review_chat_is_told_each_pr_and_the_users_context` |
| 5. Chat moves the view | Not built | PR milestone C2; daemon side `daemon/humatad/tests/review.rs::the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off` |
| 6. Agent drafts, not posted | Works (real daemon) | `daemon/humatad/tests/pr_review.rs::the_agent_drafts_edits_and_drops_comments_without_reaching_github` |
| 7. My draft beside the agent's | Works (real daemon) | `daemon/humatad/tests/pr_review.rs::the_user_drafts_for_github_beside_the_agents_drafts` |
| 8. Note becomes a draft in place | Works (real daemon) | `daemon/humatad/tests/pr_review.rs::a_note_of_the_review_chat_becomes_a_draft_in_place` |
| 9. Mark commits reviewed | Works (real daemon) | `daemon/humatad/tests/pr_commits.rs::mark_reviewed_marks_every_commit_up_to_head_and_pins_its_snapshot` |
| 10. Force push keeps marks for the same patch | Works (real daemon) | `daemon/humatad/tests/pr_commits.rs::a_force_push_carries_a_mark_to_the_same_patch` |
| 11. Changed patch is unreviewed | Works (real daemon) | `daemon/humatad/tests/pr_commits.rs::a_changed_patch_after_a_force_push_is_unreviewed` |
| 12. Refresh finds new commits | Works (real daemon) | `daemon/humatad/tests/pr_refresh.rs::refreshing_now_finds_new_commits_before_the_next_poll` |
| 13. Update the clone outdates changed drafts | Works (real daemon) | `daemon/humatad/tests/pr_review.rs::updating_the_clone_fetches_the_head_tells_the_agent_and_outdates_changed_lines` |
| 14. Only what changed since my mark | Not built | PR milestone C4, D2 |
| 15. Submit plan per PR | Works (real daemon) | `daemon/humatad/tests/pr_review.rs::the_submit_plan_lists_drafts_per_pr_and_sets_apart_those_outside_the_diff` |
| 16. Submit posts once | Works (real daemon) | `daemon/humatad/tests/pr_review.rs::a_submit_plan_posts_only_once` |
| Branch: PR cannot be fetched | Works (real daemon) | `daemon/humatad/tests/pr_review.rs::a_pull_request_that_cant_be_fetched_says_why_and_what_to_do` |
| Branch: rate limited | Works (real daemon) | `daemon/humatad/tests/pr_review.rs::a_rate_limited_lookup_says_to_wait_until_the_limit_resets` |
| Branch: review session pushes | Works (real daemon) | `daemon/humatad/tests/pr_review.rs::a_review_session_cant_push_until_the_user_lets_it` |
| Branch: one PR's post fails | Works (real daemon) | `daemon/humatad/tests/pr_review.rs::a_failed_post_keeps_that_prs_drafts_and_says_which_pr_failed` |
| Branch: crash just after posting | Works (real daemon) | `daemon/humatad/tests/pr_review.rs::a_review_posted_just_before_a_crash_is_found_on_github_not_posted_again` |
| Branch: machine stopped | Works (real daemon) | `daemon/humatad/tests/pr_stopped.rs::a_stopped_machine_stops_following_its_reviews_and_its_start_follows_them_again` |
| Branch: agent tries to mark commits | Works (real daemon) | `daemon/humatad/tests/pr_commits.rs::only_the_user_marks_commits_the_agent_reads_them` |
| Branch: draft for a PR outside the review | Works (real daemon) | `daemon/humatad/tests/pr_review.rs::a_draft_for_a_pr_outside_the_review_is_refused` |
| Branch: undo a chat view change | Works (real daemon) | `daemon/humatad/tests/review.rs::the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off` (app Undo not built) |
| Branch: note's lines change | Works (real daemon) | `daemon/humatad/tests/review.rs::a_later_diff_marks_notes_whose_lines_changed_outdated_and_moves_the_rest` |
| Branch: comment with no destination | Likely bug | `daemon/humatad/src/review.rs:851` |

## Gaps and likely bugs

- **Not built (PR milestone B1, B2, B3):** the app has no client for `PullRequestReviewService`, `ReviewChatService` changes, or PR comments. `clients/src` has no pull request client.
- **Not built (PR milestone C1, C5, D1):** the "Review a pull request" dialog (frame 67). The palette offers the review entry only for a PR a session already reviews (D197).
- **Not built (PR milestone C2):** the chat follower that moves the app's view, and its Undo.
- **Not built (PR milestone C3, D3):** the Submit review screen (frame 69).
- **Not built (PR milestone C4, D2):** the Refresh button, the head-change notice, the Update the clone button, and the PR commit list in the review session (frame 68).
- **Not built (PR milestone C6):** the "Draft for GitHub" and "Make it a draft" buttons. The daemon accepts both (see steps 7 and 8).
- **Not built (PR milestone E1):** the live flow test `desktop/tests/live_flows/pr_review.rs`. It does not exist yet. The existing `desktop/tests/live_flows/review.rs` covers a local three-repo diff, not a PR.
- **Likely bug:** `daemon/humatad/src/review.rs:849` to `:855` and `:851`. A comment with no destination gets the message "for a pull request". See the branch above.
- **Correction to PR.md:** it says the PR review service is not built and that the identity never yields a review session. Both are out of date. `daemon/humatad/src/serve.rs:936` and `:937` serve `ReviewChatService` and `PullRequestReviewService`, and `daemon/humatad/src/identity.rs:154` gives the review kind. The push refusal in `daemon/access/src/policy.rs:882` to `:883` is live.
- **Correction to REVIEW.md:** it says the MCP PR tools (`review_draft`, `review_draft_edit`, and others) are not built. `daemon/humatad/src/mcp/pull_requests.rs` has them.
- **Correction to the brief:** the brief names `desktop/tests/live_flows/review.rs` as the PR review pointer. That file tests a local diff, not a PR. The PR daemon tests run against a fake `gh`, so the GitHub side is unproven live.
- **Correction to the brief:** the brief's flow starts with "Review this PR" creating the channel. In the app today it does not, because the palette offers that entry only for a PR a session already reviews (D197).
