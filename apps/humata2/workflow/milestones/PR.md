# PR: Review a pull request with an agent beside me

- **Status:** blocked
- **Depends on:**
  - REVIEW: a PR review is the review surface plus drafts
  - ASK: the review chat's tools are scoped by access
  - FRAMES2: the PR dialog, review session and Submit review have no frame yet
- **Unblocks:** V1
- **Layer:** L4 · **Lane:** UI + daemon

## Outcome
Paste a PR link. Ask questions. The agent focuses lines and leaves notes for you, never a verdict. Drafts post only on Submit. New commits update the review.

## Exit criterion
Flow §26.17 passes against a real daemon and a test GitHub remote.

## Read these (pointers, nothing else)
- SPEC: PR-1–PR-26, RV-31–RV-34, KP-4, FC-18; flow §26.17. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: the PR frames from FRAMES2 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D55, D59, D66, D88, D92, D98 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `daemon/review_diff`, `daemon/humatad/src/review.rs`, `daemon/humatad/tests/review.rs`, `review`, `features/review`, `skills/order-humata-review`, `skills/render-humata-review`
- API: `api/proto/humata/v1/review.proto` (PullRequestReviewService, ReviewChatService)
- Also: OVERVIEW.md › Known gaps › "review_diff wiring"
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Reviewing a session's own changes (REVIEW).

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
