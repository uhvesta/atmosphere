# Brief macro-06: I review a pull request
Output: `apps/humata2/flows/cases/macro/06-review-a-pull-request.md`

Scenario: a teammate asks for a review. I paste the PR URL into the palette, pick "Review this PR", the app makes or reuses the pr-reviews channel with a session on the PR's head, the review chat answers my questions and moves the view to the relevant file, I mark commits reviewed, draft comments, the author force-pushes, I review only what changed, and I submit.

Steps: PR resolves from URL or owner/repo#number; the review channel and session; the review chat (never reviews on its own; answers and drives the view); notes vs drafts; mark reviewed per commit and snapshot; force push carries marks to the same patch; Refresh; submit plan per PR and post once.

Branches: PR can't be fetched (says why); rate limited (wait until reset); review session tries to push (only the user can allow); post fails for one PR; crash just after posting (found on GitHub, not posted again); the machine is stopped (stops following); CommentService.CreateComment "for a pull request" is not_built (review.rs:851) — what does that block in the UI?

Pointers: daemon/humatad/tests/pr_review.rs, pr_commits.rs, pr_refresh.rs, pr_stopped.rs, review*.rs; features/review/tests/*; features/palette/tests/review_pull_gate.rs; desktop/tests/live_flows/review.rs; desktop/tests/native/review.rs; workflow/milestones/PR.md, REVIEW.md; SPEC PR-22–26, D59.
