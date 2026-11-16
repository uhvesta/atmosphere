# REVIEW: Review what the agent changed, in code and in documents (0.2)

- **Status:** early lane open (lane REVIEW)
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

Early lane (minion REVIEW, Fable, 2026-11-15): daemon and model only. DESK and FL are open, so this lane can't close; `features/review` and everything in `ui/` wait (LOOK1, LOOK2, DESK).

- **Goal of the early part:** humatad serves comments for the session's agent (create, edit, drop, re-attach, send, resolve, reopen, reply, archive) from one model; a batch is typed exactly once across a daemon restart; `comment_reply` and `comment_resolve` work over MCP under their own rules.
- **Design notes:**
  - **One daemon model:** `humata2_review_diff::CommentLedger` (`daemon/review_diff/src/comments.rs`), plain data, no IO. Statuses draft → waiting → sent → addressed, resolved, plus an archived stamp that leaves the status alone (SM-11, SM-17–SM-21). Replies are a thread on the comment (DK-9, SM-19). It serves Markdown, diff, editor and file comments alike: the anchor is repo, path, side, lines, quoted lines, and for a diff its base (SM-13, RV-27).
  - **The message is composed in one place:** `comment_message::compose` (SM-9, frame 43) and `bracketed_paste`, which strips control characters so a comment can't end the paste or drive the terminal.
  - **Re-anchoring and addressed:** `passage::place` finds the quoted lines in the file as it is now: same place, moved (SM-7), or changed (SM-16; whitespace-only differences are the same passage). A sent comment whose lines changed becomes addressed and keeps both passages (SM-18); a draft's becomes detached (SM-7). Resolved and addressed ones never change again on their own (SM-17). Provisional (D138): every change seen in the clone after sending counts as the agent's, until Humata's own editor writes are known to the daemon.
  - **Exactly once (D138):** Send writes the batch into the ledger as Sending, with a key (`d-<n>`, never reused) and the exact text, and saves the review before anything is typed. The text is then typed through `CommentSink` (the session's main agent tab) and the batch settles: typed → comments sent; not typed (no terminal) → comments wait, Humata holds them (SM-10); unknown → the batch stays, Unconfirmed. After a restart, a batch still Sending is in doubt: the daemon asks the sink whether that key arrived (`CommentSink::arrived`) and only types again when the answer is a certain no. Batch 1's sink can't remember across a restart, so it answers unknown and the batch becomes Unconfirmed (the user sends again or takes it back: at most once). Batch 2 makes the holder remember keys (it outlives the daemon), which turns that into exactly once.
  - **Storage (main's pick, 2026-11-15):** tables of their own, `comments` (session, id; delivery-state column; index on session and archived), `comment_deliveries`, `comment_numbers`. The daemon keeps what it last saved per session and writes only the rows that differ, in one transaction (`Store::apply_comments`), so a draft edit never rewrites the review blob. Ids on the wire are `<session>/c-<n>`; notes stay `<session>/note-<n>`.
  - **Send does not wait for a turn end** (D126): explicit Send types at once. `queued` is true only when Humata is holding the comments because nothing could be typed.
- **Claims:** see WORKLOG (REVIEW (early)).
- **Tasks:**
  - Keep the tree building (CONFIG): everything below sits in new files nothing includes yet; the wiring is listed as seams and is applied by main together with the proto and store seams.
  - Batch 1 (model + service, on the terminal as it is today): written, not built
    - [x] T1 comment ledger · worker ledger (Sonnet, medium) · `daemon/review_diff/src/comments.rs`, `tests/pending/comments.rs` (20 tests) · checked by the manager
    - [x] T2 message + passage placement · worker message (Sonnet, medium) · `daemon/review_diff/src/{comment_message,passage}.rs`, `tests/pending/comment_message.rs` (19 tests) · checked; manager made passage matching ignore all whitespace, as `git diff -w` does
    - [x] T3 humatad CommentService for the terminal destination, the sink, restart recovery · worker service (Sonnet, medium) · `daemon/humatad/src/review/{comments,sink}.rs`, `daemon/humatad/tests/pending/comments.rs` (9 tests), wiring of `review.rs` as a patch · checked; manager fix: a batch sent again that isn't typed stays unconfirmed
    - [x] T3b comments in their own tables · worker storage (Sonnet, medium) · `daemon/store/src/comments.rs`, `daemon/store/tests/pending/comments.rs` (4 tests), `review/comments.rs` persistence reworked, wiring patch regenerated · checked
    - [ ] Seams for main, in scratchpad `review-lane/`: `seams-batch1.md` (S1 review.proto, S4 terminal.rs, S5 serve.rs), `seams-store.md` (the migration, store lib.rs lines; replaces S2/S3, no store.proto change), `lib-rs-wiring.patch.md` (W1), move this lane's four pending tests up one folder (W2: `daemon/review_diff/tests/pending/{comments,comment_message}.rs`, `daemon/humatad/tests/pending/comments.rs`, `daemon/store/tests/pending/comments.rs`; other lanes keep files in that humatad folder too), `review-rs-wiring.patch` (W3, `patch -p1` from the repo root)
  - Batch 2 (exactly once for real; needs main's OK on files outside the early scope)
    - [ ] T4 the holder buffers a keyed input, types it once and remembers its key; `TerminalSink::arrived` asks it · `daemon/holder/src/{protocol,server,client}.rs`, `daemon/humatad/src/{holders,terminal}.rs` · main OK'd the files; holder half: worker holder (Sonnet, medium), running, in scratch copies under `review-lane/holder/` (patches, nothing edited in the tree); humatad half after batch 1 is applied
    - [ ] T5 crash test: kill the daemon between typing and saving (`store.review.put.committing`), restart, the agent got the paste once; needs a humatad built with store faults, as `daemon/revive` has
  - Batch 3 (MCP)
    - [ ] T6 capabilities `comment.reply`, `comment.resolve` (Allow by default, own session), tools `comment_reply`, `comment_resolve`, dispatch, the agent paths of ResolveComment and ReplyToComment (not built today), MCP.md and TRACE.md rows
  - After LOOK1 commits / DESK / FL
    - [ ] T7 `features/review` and the client model on the same statuses and message; live client calls; flows §26.3, §26.14, §26.20 against a daemon
    - [ ] T8 `review/`, `diff/`, `markdown/` anchors on the one model where the app embeds them
    - [ ] T9 Resolve N addressed as one call (SM-18), paging in ListComments, the user's own edits left out of "addressed" (D138)
- **Waiting on:** main: apply the batch 1 seams and wiring after the FL+BUG+LOOK1 commit, then gate. Batch 2 and 3 files granted (BUILD lines stay seams; access edits additive).
- **Build gates:** batch 1 asked: `//apps/humata2/daemon/review_diff:all`, `//apps/humata2/daemon/store:all`, `//apps/humata2/daemon/humatad:{humatad2,humatad2_admin,humatad2_unit_test,comments_test,review_test,mcp_test,terminal_test,restart_test}`, `//apps/humata2/cli:all`.
- **Ready to commit:** no
