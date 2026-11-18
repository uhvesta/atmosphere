# Review: prepared views (walkthroughs), review plan groups, and the review CLI

Layer: explored · macOS · harness: `//apps/humata2/desktop:native_test` (filter `views`), `//apps/humata2/desktop:live_flows_test` (filters `views`, `views_remote`), and a throwaway daemon for the CLI steps.

Phase 1 (map from code and snapshots, no run). The `humata` CLI steps need a throwaway daemon, so they are PENDING RUN.

## Setup

- Views: an agent marks a view ready over MCP; the app offers it on the session's page. Native fixture `native/views.rs` (`offered()`); live `live_flows/views.rs` (a running session on This Mac with the ledger clone) and `live_flows/views_remote.rs` (a view from an agent on devbox-1).
- Snapshot: `design/70-view-tab-showing.png` (the view tab showing); `design/50-diff-comment-box.png` (a comment in a view).
- Evidence folder: `<scratchpad>/verify/evidence/X-G/`.
- CLI: throwaway daemon with its own `--root` and a temp `HOME` under the scratchpad (never `~/.humata2`).

## Steps

### 1. An offered view appears on the session's page
- **Screen:** an offer bar on the session page with "Open it", "Later", "Not now" (`offer-open`). A view the agent only drafted is not offered and not in the Journal.
- **Offered:** Open it, Later, Not now.
- **Action:** the agent marks the view ready.
- **Result:** the bar shows; nothing else on the page changes.
- **Evidence:** live `views.rs` `a_view_the_agent_marks_ready_is_offered_and_its_draft_is_not`; remote `views_remote.rs` `a_view_a_box_agent_marks_ready_reaches_the_app_over_the_host`.
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=views::a_view_the_agent`.

### 2. Open it: the view tab shows steps beside the terminal
- **Screen:** a view tab "View" (tab kinds: Review, Board, Pull request, Review plan, View). Header "Showing “<title>”, prepared by Claude Code", a "Steps · N" list, "Step 1 of N", "Next ›", Revert, Keep.
- **Offered:** Next, Revert, Keep, the steps list.
- **Action:** Open it.
- **Result:** the state is Opened; the view tab shows step 1; the terminal tab is still there.
- **Evidence:** native `views.rs` `an_offered_view_opens_with_steps_and_reverts_to_the_terminal`; `design/70-view-tab-showing.png`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=views::an_offered_view`.

### 3. Next opens the next file beside the view; a comment there is kept on Revert
- **Screen:** step 2 opens its file in an editor tab; a comment can be written there.
- **Action:** Next; type a comment on the file; Revert.
- **Result:** Revert puts back what the person had (the terminal tab, `terminal-tab 0`), keeps the comment, and the Journal says Reverted.
- **Evidence:** live `views.rs` `reverting_an_opened_walkthrough_puts_the_page_back_and_keeps_the_comment`; native `views.rs` (step counter `view_step` = 1 after Next).
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=views::reverting_an_opened`.

### 4. Keep leaves the view open and says Kept
- **Screen:** the view tab stays; Journal line "Kept".
- **Action:** Keep.
- **Result:** the ending is recorded on the daemon and in the Journal; the view stays where it is.
- **Evidence:** live `views.rs` `keep_leaves_the_view_open_and_the_journal_says_kept`; remote `keeping_a_box_view_is_recorded_on_the_host`.
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=views::keep_leaves`.

### 5. Not now hides the offer; Later keeps it
- **Action:** Not now.
- **Result:** the offer bar goes; the view state is NotNow; it is not offered again in this session.
- **Evidence:** native `views.rs` `not_now_hides_the_offer_bar`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=views::not_now`.

### 6. The Journal lists a reverted view and opens it again
- **Screen:** Journal panel: an entry "journal-open-0" for the reverted view.
- **Action:** open the Journal entry.
- **Result:** the view opens again (Keep and Revert are back).
- **Evidence:** native `views.rs` `the_journal_lists_a_reverted_view_and_opens_it_again`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=views::the_journal`.

### 7. A restarted app keeps the answers
- **Screen:** after restart, the Journal has the answered view; the unanswered one is still offered; nothing opens by itself.
- **Action:** restart the app.
- **Result:** as above.
- **Evidence:** live `views.rs` `a_restarted_app_has_the_answered_view_in_the_journal_and_the_other_still_offered`.
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=views::a_restarted`.

### 8. A board view shows its bar on the board
- **Screen:** the view bar appears on the Board route; Revert goes back to the session.
- **Action:** Revert.
- **Result:** route goes back to the session; state Reverted.
- **Evidence:** native `views.rs` `a_board_view_shows_its_bar_on_the_board_and_revert_goes_back`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=views::a_board_view`.

### 9. Review plan groups and outdated notes in the Diff panel
- **Screen:** "plan-group-0" titled groups in the diff panel, each with the agent's note; a note whose lines changed is tagged outdated ("plan-note-outdated-0").
- **Offered:** Revert (removes the groups).
- **Action:** a review plan view is open; then Revert.
- **Result:** groups drawn until Revert; the outdated tag shows only on the rewritten note.
- **Evidence:** native `views.rs` `a_review_plans_groups_are_drawn_in_the_diff_panel_until_revert`, `a_review_plans_outdated_line_note_is_tagged_in_the_diff_panel`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=views::a_review_plans`.

### 10. CLI: list views for a session
- **Screen:** none (terminal).
- **Action:** `humata views --session <SESSION>`.
- **Result:** JSON list of views (`view.list`); `--session` scopes it; forks are not included.
- **Replay check:** PENDING RUN, after the throwaway daemon is up: `HUMATA_SOCKET=<sock> <bazel-bin>/apps/humata2/cli/humata views --session <SESSION>` (expect exit 0 and a JSON list, possibly empty).

### 11. CLI: open, revert, keep, not-now, later a view
- **Action:** `humata view open|later|not-now|revert|keep <VIEW_ID>`.
- **Result:** `view.set_state` for the given state; same states as the app's buttons.
- **Replay check:** PENDING RUN: `humata view open <VIEW_ID>` then `humata views`; expect the state to read Opened. Also `humata view bogus <ID>` should be refused by clap (exit 2) with the valid actions listed.

### 12. CLI: diff against a base, and the refused bases
- **Action:** `humata diff <SESSION> [--repo owner/name] --against main|uncommitted|last-reviewed|fork|pr|<session>`. `--against last-turn` (or `turn`) is refused.
- **Result:** the diff JSON for that base; the refused value gives "this daemon can't compare against a turn yet" with the valid list.
- **Replay check:** PENDING RUN: `humata diff <SESSION> --against last-turn; echo $?` (expect a non-zero exit and the refusal text).

### 13. CLI: mark reviewed refuses paths
- **Action:** `humata diff mark-reviewed <SESSION>`; then the same with a path.
- **Result:** the first marks the whole diff reviewed (`diff.mark_reviewed`); the second is refused ("no paths: this daemon marks the whole diff reviewed").
- **Replay check:** PENDING RUN: `humata diff mark-reviewed <SESSION> path/x.rs; echo $?` (expect refusal, non-zero exit).

### 14. CLI: comments
- **Action:** `humata comment add <SESSION> <owner/repo>:<path>:<first>-<last> "<text>"`, `humata comment ls <SESSION>`, `humata comment send <SESSION> <ID>...`, `humata comment rm <ID>`.
- **Result:** create is always a terminal-destination comment; send returns the sent ids; rm drops by id.
- **Replay check:** PENDING RUN: `humata comment add <SESSION> ledger:src/x.rs:1-1 "note"` then `humata comment ls <SESSION>` (expect the comment with status draft).

### 15. CLI: review start, drafts, submit, allow-push
- **Action:** `humata review start <owner/repo#n>... --machine <M> [--agent A] [--message ...] [--allow-push]`; `humata review drafts <SESSION>`; `humata review submit <SESSION> --as comment|approve|request-changes [--summary ...]`; `humata review allow-push <SESSION> on|off`.
- **Result:** start opens a review (`review.start`); drafts lists draft comments; submit prints the plan and asks to confirm unless `--plan <token>` or `--dry-run`; allow-push sets push.
- **Replay check:** PENDING RUN: `humata review submit <SESSION> --as comment --dry-run; echo $?` (expect the plan printed and exit 0, nothing posted). Needs a PR fixture on the throwaway daemon; if none, record as not reachable.

### 16. CLI: push with a PR
- **Action:** `humata session push <SESSION> --repo <owner/repo> --pr [--message ...]`.
- **Result:** pushes that repo and opens a PR; output names the PR.
- **Replay check:** PENDING RUN: `humata session push <SESSION> --repo ledger --pr; echo $?` (needs a repo with a remote; likely refused without one: record the exact message).

## Not reachable headless

- The view tab's visible layout and the step bar text: native snapshots cover the view bar; the tab's rows come from live tests.
- A real remote box's view: `views_remote.rs` uses a box agent in the test fixture, not a real box.
- Submit review on a real GitHub PR: the fake GitHub only.

## Surprises

- **`comment add` has no destination flag.** The CLI always sends the terminal destination, so a CLI draft cannot be a GitHub draft for a PR review. A person can only make GitHub drafts from the review UI (or the agent's MCP). Expected in CLI.md? Check in phase 2.
- **`--against last-turn` is in CLI.md but refused**; the message says the daemon can't do it yet, which is honest but means the flag's documented value is dead.
- **`review submit` prompts** (interactive confirm) when run without `--plan`; in a script this needs `--plan` or `--dry-run`. The help text does not say so (check `args.rs` doc comments: the `Submit` variant has no doc line).
- **`views` excludes forks** (`include_forks: false`), so a fork's views are only seen with its own session id.
- **The view tab's tab name is "View" while the Journal calls the same thing a walkthrough**; the words differ across screens (Steps, Walkthrough, View).
- **`view` action names differ from the button labels**: CLI `not-now`/`later`/`keep`/`revert`/`open` vs. buttons "Open it", "Later", "Not now", "Revert", "Keep" (check the exact clap spelling in phase 2).
