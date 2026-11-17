# 6. Reviewing someone's pull request with an agent

> "Hey, a teammate asked me to review their PR. Let me paste the link and have an agent walk me through it."

The PR changes refund rounding in `acme/api`, and a linked PR changes `acme/ledger`. I want both clones on the PR heads, an agent that walks me through the diff and drafts comments, and one review per PR posted when I'm done. The agent may also prepare a view for me: a guided set of steps through the change.

How to read this file, and what the tags mean: [README](README.md).

Most of this flow is built and tested in the daemon only. The app has no PR review client, feature or dialog yet. All of that is the PR milestone, which is open with nothing gated.

## Starting state

- `acme/api#482` "Fix refund rounding for partial captures" is open: 7 files, +214 −38, 11 checks passed and 1 failed (lint). It links `acme/ledger#91`.
- `gh` on This Mac is signed in.
- There's no `#pr-reviews` channel yet.

## Steps

### Start the review

1. **Do:** press ⌘K and paste `https://github.com/acme/api/pull/482`.
   **See:** **Review this PR** is the first entry.
   **Status:**
   - Parsing the URL works. But today Review this PR is offered only when a session already reviews that PR (D197). With none, nothing is offered.
   - **Works** for that rule: `desktop/tests/live_flows/find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds`, step 3. Fakes: `features/palette/tests/review_pull_gate.rs::a_pull_request_no_session_reviews_offers_no_review`.
   - Offering it for every pasted PR: **Not built: PR** (C5, which reverses D197).

2. **Do:** press Return.
   **See:** **Review a pull request** "in # pr-reviews" (frame 67), with:
   - the PR card: title, "by …", the branches, "7 files", "+214", "−38" and "checks: 11 passed, 1 failed (lint)";
   - **Also review** with `acme/ledger #91` "linked", and "+ add a PR in another repo";
   - **Machine**, **Agent** and **Context**;
   - the box "Let the agent push to the PR branch" (unticked), and **Cancel** and **Start the review**.

   **Expect:** a PR that can't be fetched says why and what to do. A rate limit says to wait until it resets.
   **Status:**
   - Looking the PR up **Works** in the daemon, with a scripted `gh`: `daemon/humatad/tests/pr_review.rs::a_pull_request_resolves_from_its_url_and_from_owner_repo_number`, `::a_pull_request_that_cant_be_fetched_says_why_and_what_to_do` and `::a_rate_limited_lookup_says_to_wait_until_the_limit_resets`.
   - The dialog: **Not built: PR** (B1 client, C1 feature, D1 frame 67).

3. **Do:** type "Focus on rounding at the boundaries" in **Context** and click **Start the review**.
   **See:** the review session (frame 68): "Claude Code · This Mac", the channel "# pr-reviews", tabs "claude" and "zsh", and **Submit review** in the header.
   **Expect:**
   - `#pr-reviews` is made on the first review and reused after.
   - One session has a clone of each PR's head. A fork PR is cloned from its pull ref.
   - The agent is told each PR and my context.
   - The agent can't push until I let it.
   - The review stays bound to its PRs across a restart.
   - Events: Channel created (the first time); Session created (bound to the PRs).
   **Status:**
   - **Works** in the daemon: `pr_review.rs::starting_a_review_makes_the_pr_reviews_channel_and_one_session_on_every_prs_head`, `::a_second_review_reuses_the_channel_and_clones_a_fork_pr_from_its_pull_ref`, `::the_review_chat_is_told_each_pr_and_the_users_context`, `::a_review_session_cant_push_until_the_user_lets_it` and `::a_review_stays_bound_to_its_prs_across_a_restart`.
   - Starting on another machine: **Not built: PR** (A7).
   - Known daemon gap: a crash during start re-clones from main, not from the PR head.
   - The app side: **Not built: PR** (C1, E1).

### Walk the change

4. **Do:** type "why round() and not floor()?" into the agent's terminal.
   **See:** the agent moves the Diff panel to `refund.rs:140` and leaves a note there, with **Undo** on the change.
   **Expect:**
   - The view changes and notes survive a restart. Undo works the same after a restart.
   - A plan from the agent orders the files and leaves its notes.
   **Status:**
   - **Works** in the daemon, on a plain session: `daemon/humatad/tests/review.rs::the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off`, `::a_plan_in_an_open_review_chat_orders_the_review_and_leaves_its_notes` and `::undo_and_a_repeated_key_act_the_same_after_a_restart`. Highlights are never checked.
   - The app applying the agent's view changes, and the Undo strip: **Not built: PR** (B2, C2, D2).

5. **Do:** the agent offers a prepared view, "PR review: refunds". Click **Open it** on the offer bar.
   **See:**
   - The offer bar has **Open it**, **Later** and **Not now**.
   - Opening it shows a view tab (frame 70): its steps ("Step 1 of 4", "‹ Prev", "Next ›"), and a bar "Showing “PR review: refunds”, prepared by Claude Code" with **Revert** and **Keep**.
   **Expect:**
   - **Later** brings the offer back on my next visit. **Not now** never does.
   - Events: PreparedView created (draft, the agent's), then updated (offered), then updated (opened).
   **Status:**
   - The daemon side **Works**: `review.rs::a_draft_view_is_the_agents_until_offered_then_the_users_and_a_restart_keeps_it` and `::the_feed_tells_the_agent_of_its_drafts_and_the_user_of_offers_only`.
   - The offer bar is **Fakes only**: `desktop/tests/native/session.rs::prepared_views_wait_for_open_and_later_returns_on_the_next_visit` and `features/views/tests/offers.rs::returning_shows_a_deferred_offer_but_never_a_dismissed_one`. The live views client exists but nothing uses it, and offers are matched by title. Left to the VIEWS lane.
   - The view tab, its steps and the Showing bar: **Not built: VIEWS** (D2, E2).

6. **Do:** step through with **Next ›**, add a comment on step 3, then click **Revert**.
   **Expect:**
   - The panel and terminal tab are exactly as they were before I opened the view.
   - My comment stays queued.
   - The Journal shows the view as Reverted, and I can open it again.
   **Status: Not built: VIEWS** (D3, E2). The Revert action today only clears an "accepted" flag.

### Draft and submit

7. **Do:** turn the agent's note on `refund.rs:140` into a draft. Edit it, and write one of my own on `refund_test.rs:58`.
   **See:** cards "Draft for GitHub · by Claude Code" with **Edit** and **Drop**, and "Drafts: 2 (1 by agent, 1 by you)".
   **Expect:**
   - Nothing reaches GitHub yet.
   - A draft on a PR outside this review is refused.
   - Events: Comment created (Draft) for each, by the agent or by me.
   **Status:**
   - **Works** in the daemon: `pr_review.rs::a_note_of_the_review_chat_becomes_a_draft_in_place`, `::the_agent_drafts_edits_and_drops_comments_without_reaching_github`, `::the_user_drafts_for_github_beside_the_agents_drafts` and `::a_draft_for_a_pr_outside_the_review_is_refused`.
   - Known daemon gap: dropping a draft made from a note brings the note back.
   - The app: **Not built: PR** (B3, C6). Today's drafts in the app are terminal comments only.

8. **Do:** the author pushes 2 more commits. Click **Update the clone** on the notice.
   **See:** "2 new commits on the PR branch" with **Update the clone**. Changed lines with my drafts show as outdated.
   **Expect:**
   - The new head is fetched, and the agent is told.
   - A force-push counts as one change.
   - The daemon notices on its own; no refresh is needed.
   - Event: PullRequestHeadsChanged, then Session updated.
   **Status:**
   - **Works** in the daemon: `pr_review.rs::new_commits_on_a_prs_head_are_counted`, `::a_force_push_is_reported_as_one`, `::updating_the_clone_fetches_the_head_tells_the_agent_and_outdates_changed_lines` and `::the_daemon_notices_a_moved_head_on_its_own_and_says_so_on_the_feed`; `daemon/humatad/tests/pr_refresh.rs::refreshing_now_finds_new_commits_before_the_next_poll`.
   - Known daemon gap: an opened review keeps polling every minute.
   - The app: **Not built: PR** (C4, C4b).

9. **Do:** click **Submit review**. Under each PR, pick a **Verdict** (**Comment**, **Approve** or **Request changes**), edit the summary Claude drafted, and click **Submit 2 reviews**.
   **See:** "Submit review", "2 pull requests" (frame 69). A section per PR with its drafts. "Posts one review per PR with This Mac's gh".
   **Expect:**
   - One review per PR is posted at the head I reviewed, with its drafts.
   - Drafts outside the diff are set apart before posting.
   - The submit plan posts only once. A failed PR keeps its drafts and is named.
   - A review posted just before a crash is found on GitHub, not posted twice.
   - Events: Operation Running, then Succeeded; Comment updated (Posted).
   **Status:**
   - **Works** in the daemon: `pr_review.rs::the_submit_plan_lists_drafts_per_pr_and_sets_apart_those_outside_the_diff`, `::submitting_posts_one_review_per_pr_at_the_reviewed_head_with_its_drafts`, `::a_submit_plan_posts_only_once`, `::a_failed_post_keeps_that_prs_drafts_and_says_which_pr_failed` and `::a_review_posted_just_before_a_crash_is_found_on_github_not_posted_again`.
   - Posting from another machine: **Not built: PR** (A7).
   - The dialog: **Not built: PR** (C3, D3).

10. **Do:** the PR merges. Click **Archive** on the notice.
    **See:** the session says the PR merged and offers **Archive**. The archive dialog lists any drafts I never submitted.
    **Status:**
    - **Not built: PR** (A6, C4). The daemon only reports the PR as merged (`pr_review.rs::a_merged_pr_shows_as_merged`).
    - There's no frame for the merged notice yet.

## Expected outcome

Two reviews are posted, one on `acme/api#482` and one on `acme/ledger#91`, each at the head I reviewed. No draft was lost or posted twice. The review session is archived after the merge. `#pr-reviews` stays for the next one.

## Gaps this flow shows

- None of this flow is in the app. The daemon is far along; the app needs the PR client, feature and dialogs (PR B1–B3, C1–C6, D1–D3). The whole flow live is PR E1, `desktop/tests/live_flows/pr_review.rs`, which doesn't exist yet.
- Prepared views stop at the offer bar. The view tab, steps, Revert and Keep are VIEWS.
- Review on another machine is unproven (PR A7).
- Known daemon gaps: a crash during start re-clones from main, a dropped draft brings its note back, and an opened review polls forever.
