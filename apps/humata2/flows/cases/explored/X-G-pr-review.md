# Review: pull request review from a PR link, drafts, submit, heads notice, review chat

Layer: explored · macOS · harness: `//apps/humata2/desktop:native_test` (filters `pr_review`, `pr_session`, `pr_ask`) and `//apps/humata2/desktop:live_flows_test` (filter `pr_review`); features in `//apps/humata2/features/pr_review` (dialog, heads, submit, ask, info, follower)

Phase 1 (map from code and snapshots, no run).

## Setup

- Native tests: fake daemon with PRs `acme/api #482` ("Fix refund rounding for partial captures", by rkhan, `uhvesta/fix-refund-rounding` → main, 7 files +214 −38, checks 11 passed and 1 failed (lint)) and `acme/ledger #91`. Session "pr-reviews" channel. Machines: This Mac (online), devbox-1, gpu-box (offline).
- Live flows: a real app against live clients with a fake GitHub behind the daemon; `live_flows/pr_review.rs` helpers `started`, `dialog_problem`, `person_opens`.
- Snapshots: `design/67-pr-review-dialog.png`, `design/68-pr-review-session.png`, `design/69-pr-submit-review.png`.
- Evidence folder: `<scratchpad>/verify/evidence/X-G/`.

## Steps

### 1. Open the review dialog from a pasted link
- **Screen:** dialog "Review a pull request in # pr-reviews" (or "Review a pull request" in the channel). Top: a search-style field "Pull request link" with a Return hint. Empty state text: "No pull request yet" and "Paste a pull request link in the box above and press Return; Humata reads it with the chosen machine's gh."
- **Offered:** paste or type a link, Return, Cancel. Remove an also-review pull request (`Remove acme #91`), add another (`+ add a PR in another repo`), Machine pop-up, Agent pop-up, Context field, the push checkbox, Start the review.
- **Action:** paste `https://github.com/acme/api/pull/482`.
- **Result:** the card reads "Fetching acme #482 with gh…" and then the facts card: title, author, branch → base, files and line counts, checks.
- **Evidence:** `design/67-pr-review-dialog.png` (frame 67 shows the card, ledger #91 under Also review, This Mac, Claude Code, and context "rounding on partial captures; anything that can double-refund").
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_review::a_pasted_link_shows_its_card`.

### 2. A pull request that can't be read blocks Start
- **Screen:** the card says why (`Fetching` replaced by the error), "Try again" button; Start stays off.
- **Action:** the read fails, then Try again.
- **Result:** Start is blocked until the read succeeds (`a_failed_read_blocks_start_until_try_again_reads_it`).
- **Evidence:** native `pr_review.rs`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_review::a_failed_read`.

### 3. Rate limited read
- **Screen:** the rate-limit message shows on the card and blocks Start.
- **Action:** a read is rate limited.
- **Result:** the card says the limit; Start waits.
- **Evidence:** native `pr_review.rs` `a_rate_limited_read_shows_on_the_card`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_review::a_rate_limited`.

### 4. Also review: add and remove a second PR
- **Screen:** "Also review" row with "+ add a PR in another repo" and linked chips "acme/ledger #91 linked ×". A PR added under Also review shows as linked; the pasted one does not.
- **Action:** add ledger #91; remove it with ×.
- **Result:** Start asks for both PRs (pasted first). Removing takes it out of the review and clears its link.
- **Evidence:** native `pr_review.rs` `also_review_adds_a_pull_request_and_start_asks_for_both`, `a_removed_also_review_pull_request_is_not_started`, `only_an_also_review_pull_request_shows_as_linked`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_review::also_review`.

### 5. Machine and agent pick, push permission off by default
- **Screen:** Machine pop-up ("This Mac · devbox-1 · gpu-box (offline)"), Agent pop-up ("Claude Code"), and the checkbox "Let the agent push to the PR branch" (off).
- **Action:** pick devbox-1 as the machine; leave the box unticked.
- **Result:** the review starts on devbox-1; the agent may not push (`allow_push` false).
- **Evidence:** native `pr_review.rs` `the_picked_machine_is_where_the_review_starts`, `pushing_is_off_until_it_is_ticked`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_review::the_picked_machine` and `pr_review::pushing_is_off`.

### 6. Start the review: waits for the PR, then opens the session
- **Screen:** "Starting…" on the button; after the read the dialog closes and the review session opens in the pr-reviews channel.
- **Action:** Start the review.
- **Result:** the session opens with the PR's title; the clones get the heads; the agent is told the context first.
- **Evidence:** native `pr_review.rs` `start_waits_for_the_pull_request_and_then_opens_the_session`; live `pr_review.rs` `starting_a_review_of_two_pull_requests_clones_both_heads_and_tells_the_agent`.
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=pr_review::starting_a_review`.
- **CLI:** `humata review start <PR>... --machine <M> [--agent A] [--message "..."] [--allow-push]` (PENDING RUN to confirm output).

### 7. Cancel closes the dialog and starts nothing
- **Action:** Cancel.
- **Result:** dialog closes; no session.
- **Evidence:** native `pr_review.rs` `cancel_closes_the_dialog`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_review::cancel_closes`.

### 8. The review session: diff names the PR and each repo
- **Screen:** header "acme/api #482" with "Claude Code · This Mac" and a primary "Submit review" button. Right panel: "PR #482 against main · 2 repos", group `api #482 uhvesta/fix-refund-rounding`, file `refund.rs +62 −21`, lines shown with old and new numbers. The `ledger #91` group below, collapsed.
- **Offered:** Submit review (header), file rows, group expand/collapse, Edit/Drop on drafts, the Update the clone notice, the header controls (fork, expand, panel, more).
- **Action:** none; open the review.
- **Result:** the review page of frame 68.
- **Evidence:** `design/68-pr-review-session.png`; native `pr_session.rs` `the_diff_names_the_pull_request_and_each_repository`; `pr_review_session_frame_68` snapshot name `68-pr-review-session`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_session::the_diff_names`.

### 9. Drafts for GitHub on a line: Edit, Drop, footer count
- **Screen:** under the line a card "Draft for GitHub · by Claude Code" with a Draft tag, body text, Edit and Drop. The footer reads "Drafts: 4 (3 by agent, 1 by you)" and the send control.
- **Offered:** Edit (opens the draft in the comment box), Drop (removes it and its count), Submit review.
- **Action:** Drop one draft; Edit another, then Save/close.
- **Result:** Drop removes it from the line and the footer count. Edit opens the box; closing keeps the draft.
- **Evidence:** `design/68-pr-review-session.png` (footer); native `pr_session.rs` `a_draft_for_github_has_edit_and_drop_and_the_footer_counts_the_drafts`, `drop_removes_the_draft_and_the_footer_follows`, `edit_opens_the_draft_in_the_box_and_closing_keeps_it`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_session::drop_removes`.

### 10. Heads notice: new commits and Update the clone
- **Screen:** a notice under the terminal: "2 new commits on the PR branch" with "Update the clone" (`pages.rs` line 533). The notice goes after the update.
- **Offered:** Update the clone.
- **Action:** click Update the clone.
- **Result:** the clone is updated, the agent is told, the notice is gone.
- **Evidence:** `design/68-pr-review-session.png` (bottom bar); native `pr_session.rs` `update_the_clone_sends_the_update_and_clears_the_notice`; live `pr_review.rs` `new_commits_update_the_clone_and_a_merged_pull_request_offers_archive`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_session::update_the_clone`.

### 11. A merged PR offers Archive, not Update the clone
- **Screen:** the notice changes to an Archive offer.
- **Action:** none; the PR merges.
- **Result:** the Archive offer replaces the update notice.
- **Evidence:** native `pr_session.rs` `a_merged_pull_request_offers_archive`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_session::a_merged`.

### 12. The agent's focus and note reach the view (live)
- **Screen:** the agent's `review_focus` marks the line in the diff ("the line the agent focuses is marked"), and a `review_note` appears in the terminal with Undo. A note becomes a draft after the person is still.
- **Offered:** Undo on the terminal note; Edit and Drop on the draft.
- **Action:** the agent focuses `refund.rs:140` and notes it; one Undo.
- **Result:** the focus is shown and scrolled into view if below the fold; after Undo the view goes back to what the person had.
- **Evidence:** `design/68-pr-review-session.png` (review_focus and review_note lines in the terminal); native `pr_session.rs` `the_line_the_agent_focuses_is_marked`, `a_focus_below_the_fold_is_scrolled_into_view`; live `pr_review.rs` `the_agents_focus_and_note_reach_the_view_when_the_person_is_still_and_undo_puts_it_back`.
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=pr_review::the_agents_focus`.

### 13. Ask in the review chat instead of commenting
- **Screen:** a review box offers "Ask about this" (`panel_changes.rs` line 1654). The box has a type choice: "Draft for GitHub" (`comment_cards.rs` line 341), and "Make it a draft" turns a note into a PR draft.
- **Offered:** Ask (types into the agent's terminal and leaves no comment), Draft for GitHub (saves a draft and types nothing), Make it a draft on a note.
- **Action:** Ask on a line; then Draft for GitHub on another.
- **Result:** Ask types the question into the review chat, a failed question shows under the terminal until dismissed; Draft for GitHub saves a draft only.
- **Evidence:** native `pr_ask.rs` (`ask_types_into_the_review_chat_and_leaves_no_comment`, `draft_for_github_saves_a_pull_request_draft_and_types_nothing`, `make_it_a_draft_turns_a_note_into_a_pull_request_draft`); `design/68-pr-review-session.png` (Draft for GitHub card).
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_ask`.

### 14. Ask on a hunk or a file
- **Screen:** Ask covers the hunk's range or names the file.
- **Action:** Ask on a hunk; Ask on a file.
- **Result:** the question names the hunk's lines or the file (`ask_on_a_hunk_covers_the_hunk`, `ask_on_a_file_names_the_file`). A plain (non-review) session has no Ask on hunks or files.
- **Evidence:** native `pr_ask.rs`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_ask::ask_on`.

### 15. Submit review: per-PR drafts, verdict and summary
- **Screen:** "Submit review" dialog (frame 69). Title "Submit review  2 pull requests". One card per PR: "acme/api #482   3 drafts" with rows (file:line, text, author "Claude Code" or "You", Edit, Drop), a Verdict row (Comment, Approve, Request changes), then a second card "acme/ledger #91 1 draft". Below: "Summary · drafted by Claude Code" (editable text) and the footer "Posts one review per PR with This Mac's gh", Cancel, "Submit 2 reviews".
- **Offered:** Edit, Drop, pick a verdict per PR, edit the summary, Add a summary (if none), Cancel, Submit N reviews.
- **Action:** choose Request changes on #482 and Comment on #91; Submit.
- **Result:** one review per PR with its verdict and drafts; the dialog closes; "Posted to acme #482" then "Posted: <url>".
- **Evidence:** `design/69-pr-submit-review.png`; native `pr_review.rs` `submit_posts_one_review_per_pull_request`, `drafts_are_listed_per_pull_request_with_a_verdict_each`, `the_agents_summary_shows_and_another_can_be_added`; live `pr_review.rs` `a_note_made_a_draft_and_the_persons_own_draft_post_in_one_review_per_pull_request`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_review::submit_posts` and `bazel test //apps/humata2/desktop:live_flows_test --test_filter=pr_review::a_note_made_a_draft`.
- **CLI:** `humata review drafts <SESSION>`; `humata review submit <SESSION> --as comment|approve|request-changes [--summary "..."]`. Without `--plan` it prints the plan and asks to confirm (`--dry-run` prints only). PENDING RUN to confirm the printed plan.

### 16. One PR fails to post: its card shows the error and keeps its drafts
- **Screen:** the failing PR's card shows its error; the other PR posts.
- **Action:** submit with one failing post.
- **Result:** the failed card keeps its drafts; the successful one is posted.
- **Evidence:** native `pr_review.rs` `a_failed_post_shows_its_error_on_its_card_and_keeps_its_drafts`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_review::a_failed_post`.

### 17. A draft whose lines changed is left out: Re-attach or Drop
- **Screen:** "Not posted: its lines changed since it was written. Drop it, or point it at the new lines in the diff." (or "Not posted: its line isn't in this pull request's diff, so GitHub would refuse it. Drop it, or point it at a line in the diff."). Buttons Re-attach and Drop.
- **Action:** Re-attach.
- **Result:** the draft returns to the plan at its current lines.
- **Evidence:** native `pr_review.rs` `a_left_out_draft_re_attaches_into_the_plan`; `ui/src/overlay_submit_review.rs` lines 516–519.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_review::a_left_out_draft`.

### 18. A plan that can't be made says why and submits nothing
- **Screen:** "Working out what would post…" then the reason; Submit is unavailable.
- **Action:** open Submit review when the plan fails.
- **Result:** the reason shows; nothing posts.
- **Evidence:** native `pr_review.rs` `a_plan_that_cannot_be_made_says_why_and_submits_nothing`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_review::a_plan_that_cannot`.

### 19. Cancel closes Submit review without posting
- **Action:** Cancel.
- **Result:** closes; nothing posted; drafts kept.
- **Evidence:** native `pr_review.rs` `cancel_closes_submit_review_without_posting`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=pr_review::cancel_closes_submit`.

### 20. A host-run review posts through the box's gh (live)
- **Screen:** the review session on devbox-1 shows the same screens; posting uses the host's gh.
- **Action:** Submit on a review started on devbox-1.
- **Result:** the review posts through the host's `gh`; line checks read the box's clones over the link.
- **Evidence:** live `pr_review.rs` `a_review_started_on_devbox_1_runs_there_and_posts_through_the_hosts_gh`.
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=pr_review::a_review_started_on_devbox_1`.

## Not reachable headless

- A real `gh` call and a real GitHub review: all PR calls go to a fake GitHub behind the daemon.
- Rate limits from GitHub itself: the native test sets the limit on the card; the real header is not seen.
- Keyboard flow in the review chat box: only the click paths are tested here.

## Surprises

- **The Submit dialog has no "Cancel and keep drafts" wording**, but the drafts are kept on Cancel (test `cancel_closes_submit_review_without_posting`). Fine, but the dialog does not say so.
- **The Start dialog's push checkbox is off by default**, and the review page has no push control of its own (not seen in `pages.rs` or the review header); a person must set it in the dialog or use `humata review allow-push <SESSION> on|off`. Verify on screen in phase 2.
- **"Ask about this" and "Draft for GitHub" look alike** in the box; the difference (types into the chat versus saves a draft) is only in the copy. Verify the copy in phase 2.
