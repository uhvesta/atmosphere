# REVIEW: Review what the agent changed, in code and in documents (0.2)

- **Status:** blocked
- **Depends on:**
  - DESK: markdown comments and diff comments are one model; the document tab opens from the Desk too
  - FRAMES1: comment states and the comment box in a diff have no frame yet
  - FL (to close, not to open): sending comments needs a real terminal and turn ends; diffs need real clones
- **Unblocks:** VIEWS, PR
- **Layer:** L3 · **Lane:** UI, then daemon seam

## Outcome
Open Diff on a 3-repo session, comment, send; the comments arrive as one paste when the turn ends. Mark reviewed, and later see only what changed since. Read a markdown file as Preview, Source or Side by side and comment on its lines. Resolve and archive comments; the agent replies in the thread.

## Exit criterion
On fakes first: frames 39 and 41–43 match; one comment model serves the diff, the editor and markdown. Then real: flows §26.3, §26.14 and §26.20 pass against a daemon; a comment is delivered exactly once across a daemon restart; `comment_reply` and `comment_resolve` work over MCP with their own access rule.

## Read these (pointers, nothing else)
- SPEC: RV-1–RV-30, RV-35, ST-7–ST-12, SM-3, SM-4, SM-6–SM-21, DK-7–DK-9, SE-17, SE-21–SE-23, MV-12, PER-27; flows §26.3, §26.14, §26.20. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 06, 11, 39, 41, 42, 43, and the comment frames from FRAMES1 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D104, D101, D100, D53, D55, D66, D67, D78, D85, D86, D13 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `review`, `diff`, `markdown`, `daemon/review_diff`, `daemon/humatad/src/review.rs`, `daemon/humatad/src/paste.rs`, `daemon/humatad/tests/review.rs`, `features/review`, `features/files`, `ui/src/panels.rs`, `ui/src/code_wrap.rs`
- API: `api/proto/humata/v1/review.proto` (DiffService, CommentService); `api/MCP.md`
- Also: OVERVIEW.md › Known gaps › "Markdown", "Review pane", "review_diff wiring", "Diff view"; N1 items on markdown review and resolve/archive
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
PR reviews and the review chat (PR). Agent-prepared reviews, RV-31–RV-34 (VIEWS, PR). Spec-mode leftovers in the API (TIDY).

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
