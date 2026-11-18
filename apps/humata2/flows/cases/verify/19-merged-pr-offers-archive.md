# A merged pull request says so in its review session and offers Archive, while the other pull request in the review stays open

Layer: macro, verify layer (flow 06, step 10). Platforms: macOS (the app and this Mac's daemon, with a fake `gh`). Linux is not covered by the app's test.

## Starting state

- A review session is open on This Mac in `#pr-reviews`, for two pull requests: `acme/api#482` (the main one) and `acme/ledger#91` (linked, "Also review").
- Both PRs were open at the last check. The session's clones are on the PR heads I reviewed.
- The review has drafts I already submitted, so nothing is unposted. (The archive dialog that lists unsubmitted drafts is a separate case and is not covered here.)
- The app is focused on the review session's page. The Diff panel is closed.
- `gh` on This Mac is signed in (a fake `gh` in the tests).

## Reproduce headless

- Live app with a real daemon and fake `gh`: `//apps/humata2/desktop:live_flows_test`, file `desktop/tests/live_flows/pr_review.rs`, test `new_commits_update_the_clone_and_a_merged_pull_request_offers_archive`. It moves a head, updates the clone, then sets `#482` to MERGED, calls the check, and asserts that the app offers Archive for `#482` only and that Archive starts the session's archive. Capture the test log.
- Daemon state: `//apps/humata2/daemon/humatad:pr_review_test`, file `daemon/humatad/tests/pr_review.rs`, test `a_merged_pr_shows_as_merged`. It sets the PR to MERGED and checks the daemon reports `Merged` for `#91` and `Open` for `#482`.
- The offer bar's text and the Archive button, without a window: `//apps/humata2/desktop:native_test` (the file `desktop/tests/native/pr_session.rs`, test `a_merged_pull_request_offers_archive`). It checks the `pr-ended` bar and `pr-archive` button exist and `pr-new-commits` does not.
- Feature reducer: `features/pr_review/tests/heads.rs::a_merged_pull_request_offers_archive_and_keeps_it_after_the_clone_updates`.
- Headless CLI route: a throwaway `humatad2` with its own `--root` and `HOME`, the PRs set to MERGED in the fake `gh`, then `humata sessions show <id> --json` for the review session. Capture the PR state and the exit code.
- Cannot be done headless: the 1-minute poll on a real clock. The live test calls the check itself. The timed poll is shown only for moved heads (`daemon/humatad/tests/pr_review.rs::the_daemon_notices_a_moved_head_on_its_own_and_says_so_on_the_feed`, run with `HUMATA2_PR_POLL_MS=200`), so a merge seen on the timer is the same code path but not tested by itself. Real GitHub is not used.

## Steps

1. **I do:** the PR `acme/api#482` merges on GitHub. I wait for the daemon's next check (or the review's refresh).  
   **I see:** the review session shows a bar: "acme/api#482 was merged. Archive this review when you are done with it.", with **Archive**. The other PR does not get that bar.  
   **State and events:** the daemon records the PR as Merged (its change state). The session's head change for `#482` is Merged; `#91` stays Open. The app's review state lists `#482` as archivable.  
   **Status:** Works (real daemon) for the state and the offer: `desktop/tests/live_flows/pr_review.rs::new_commits_update_the_clone_and_a_merged_pull_request_offers_archive` (it asserts `archivable` holds `#482` and not `#91`). Daemon state: `daemon/humatad/tests/pr_review.rs::a_merged_pr_shows_as_merged`. The bar's words and button are **Fakes only**: `desktop/tests/native/pr_session.rs::a_merged_pull_request_offers_archive`. The words come from `ui/src/pages.rs:546-550`.

2. **I do:** look at the other PR in the review.  
   **I see:** `acme/ledger#91` shows no merged or Archive offer. Its "Also review" chip, if open, still reads open.  
   **State and events:** no change to `#91`. The session is not archived because of `#482` alone.  
   **Status:** Works (real daemon). The same live test asserts `#91` is not archivable (`archivable(&id)` excludes `ledger_ref()`). The chip's "merged" text (`ui/src/overlay_pr_review.rs:630`) is **Code, no test** for a merged chip in the review dialog.

3. **I do:** click **Archive** on the bar.  
   **I see:** the session's own archive starts. If its clones hold unpushed work, the archive dialog comes up first. If not, the session is archived.  
   **State and events:** the app sends the review's Archive action; it delegates to the session's archive (`features/pr_review/src/heads.rs:226-229`). The session shows archived, or its archive confirm is open for that session.  
   **Status:** Works (real daemon): the same live test waits until the session's archive starts. The archive dialog's list of unsubmitted drafts is out of scope here (the archive writer owns PR drafts on archive, D513 and PR-17). The archive itself is covered by `daemon/humatad/tests/archive.rs`.

4. **I do:** open the `#pr-reviews` channel after the archive.  
   **I see:** the channel is still there, ready for the next review. The archived session is in Archived.  
   **State and events:** the channel is kept. The next review reuses it.  
   **Status:** Works (real daemon) for reuse: `daemon/humatad/tests/pr_review.rs::a_second_review_reuses_the_channel_and_clones_a_fork_pr_from_its_pull_ref`. The channel staying after an archive is **Code, no test**; the archive test does not check the channel.

5. **I do (other case):** a PR that was closed without merging.  
   **I see:** the same bar, but the words say "was closed", and the same Archive button.  
   **State and events:** the PR's change state is Closed.  
   **Status:** Code, no test. `ui/src/pages.rs:546` chooses "closed" when the state is not Merged, and the daemon maps CLOSED in `daemon/humatad/src/review/pull_requests/heads.rs:123` and `resolve.rs:167`. No test in `daemon/humatad/tests/pr_*.rs` or the desktop tests sets a PR to CLOSED.

## Branches

### The PR merges but the daemon has not checked it yet

- **When:** the merge happens between two checks.
- **I do:** wait for the next check. Nothing else.
- **I see:** no bar until the check runs. Then the bar appears once.
- **Expected outcome:** the offer shows on the next check; the clone is not changed by a merge.
- **Status:** Works (real daemon) for a moved head on a timer: `daemon/humatad/tests/pr_review.rs::the_daemon_notices_a_moved_head_on_its_own_and_says_so_on_the_feed` (run with a 200 ms poll). For a merge on the timer: **Code, no test**. The merge is read by the same check (`daemon/humatad/src/review/pull_requests/heads.rs:122`), so the path is shared but not asserted.

### Archive is pressed while the clone has unpushed work

- **When:** a clone in the review has commits that are not pushed.
- **I do:** press **Archive**.
- **I see:** the archive dialog lists the unpushed work, and the session is not archived until I confirm.
- **Expected outcome:** nothing is archived or deleted before the confirm.
- **Status:** Works (real daemon) for the archive plan that names unpushed work: `daemon/humatad/tests/archive.rs::the_plan_names_unpushed_work_and_running_forks`. The rule for PR drafts left unsubmitted on archive (D513, PR-17) is owned by the archive writer and is not tested here.

### The review chat tries to mark the merged PR's commits reviewed

- **When:** after the merge, the agent is asked to mark commits reviewed.
- **I do:** type into the chat "mark the commits reviewed" (or an equivalent agent command).
- **I see:** the user can mark commits; the agent can read them but not mark them.
- **Expected outcome:** only the user marks commits; the merged PR's marks are not changed by the agent.
- **Status:** Works (real daemon): `daemon/humatad/tests/pr_commits.rs::only_the_user_marks_commits_the_agent_reads_them` (the rule, on an open PR). Not run on a merged PR: Code, no test.

## Expected outcomes

- [ ] A PR that merges shows a bar that says it was merged, with **Archive**.
- [ ] Only the merged PR offers Archive; the other PR in the review does not.
- [ ] **Archive** starts the session's archive, and the review's channel stays.
- [ ] The daemon reports the merged PR as Merged and the other as Open.
- [ ] A PR closed without merging shows the same bar with "closed" (not run).
- [ ] A merge seen on the timer is offered without a manual refresh (not run; only the head change is).

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. PR merges; bar with Archive | Works (real daemon); bar words Fakes only | `desktop/tests/live_flows/pr_review.rs::new_commits_update_the_clone_and_a_merged_pull_request_offers_archive`; `desktop/tests/native/pr_session.rs::a_merged_pull_request_offers_archive` |
| 2. The other PR offers nothing | Works (real daemon); "merged" chip in the dialog Code, no test | `desktop/tests/live_flows/pr_review.rs::new_commits_update_the_clone_and_a_merged_pull_request_offers_archive`; `ui/src/overlay_pr_review.rs:630` |
| 3. Archive starts the session's archive | Works (real daemon) | `desktop/tests/live_flows/pr_review.rs::new_commits_update_the_clone_and_a_merged_pull_request_offers_archive` |
| 4. The channel stays for the next review | Works (real daemon) for reuse; Code, no test for the channel after archive | `daemon/humatad/tests/pr_review.rs::a_second_review_reuses_the_channel_and_clones_a_fork_pr_from_its_pull_ref` |
| 5. Closed PR shows "closed" with Archive | Code, no test | `ui/src/pages.rs:546` |
| Branch: merge seen on the timer | Works (real daemon) for a moved head; Code, no test for a merge | `daemon/humatad/tests/pr_review.rs::the_daemon_notices_a_moved_head_on_its_own_and_says_so_on_the_feed` |
| Branch: Archive with unpushed work | Works (real daemon) for the plan; drafts on archive belong to the archive writer (D513) | `daemon/humatad/tests/archive.rs::the_plan_names_unpushed_work_and_running_forks` |
| Branch: agent marks a merged PR's commits | Works (real daemon) for an open PR; Code, no test for a merged one | `daemon/humatad/tests/pr_commits.rs::only_the_user_marks_commits_the_agent_reads_them` |

## Gaps and likely bugs

- Stale doc: `flows/06-review-a-pr.md` step 10 says the merged notice is Not built (PR A6, C4) and that there is no frame. The bar and Archive exist and are tested live. The flow's gap list should drop it.
- Stale doc: `workflow/milestones/PR.md` still leaves A6 and C4 unchecked; the merged offer is part of them and works.
- Code, no test: a closed PR's bar and its "closed" words. Confirm by setting a fixture PR to CLOSED and checking `pr-ended` and the words.
- Code, no test: the timed poll for a merge. The head change is tested on a 200 ms poll; a merge is not. Confirm with the same fixture by setting MERGED and advancing the poll.
- Known daemon gap (already in flow 06): an opened review keeps polling every minute. Not a bug in the merge path.
- Likely bugs: none found in the merged path. `features/pr_review/src/heads.rs:226` starts the archive only when `archivable` is non-empty, which matches the offer.
