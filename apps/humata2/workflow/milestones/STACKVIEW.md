# STACKVIEW: An agent sends a change as a stack of layers, and I step through it across repos

- **Status:** open (owner, 2026-11-18: "Render tells the agent to prepare the stack for review like it will send the stuff so i can logically navigate a stack of changes across a 1 to many repos")
- **Depends on:** RELEASE3: builds on the released review view. Parallel to AGENTS2: it touches the review code, AGENTS2 the agent kinds, and they share no files.
- **Unblocks:** nothing yet. The skills `render-humata-review` and `review-humata-change` switch from groups to the stack payload when it lands.
- **Layer:** L13 · **Lane:** api + daemon (review_diff, humatad review, mcp) + clients + features/review + UI

## Outcome
I ask an agent to show me a change. It works out the change's layers (stack
topics or chained pull requests, else one per commit, else a logical split of
uncommitted work) and sends them. The review view lists the layers bottom to
top. I pick **Whole stack** or one layer, and see that layer's own diff in
every repo it touches. I step to the next or previous layer with a key, and
the agent can step me with `review_focus`. Nothing in this is a review: the
payload carries titles, ranges and at most a one-line purpose per layer.

## The payload (the design piece)
`review_prepare` gains an optional `stack` beside `spec`. A stack is an
ordered list of layers, bottom first:

```json
{ "title": "Journal compression",
  "stack": { "layers": [
    { "title": "codec",
      "purpose": "Adds the zstd codec",
      "repos": [
        { "repo": "acme/api", "range": { "base": "<rev>", "head": "<rev>" }, "pull_request": "acme/api#412" },
        { "repo": "acme/web", "range": { "base": "<rev>", "head": "<rev>" }, "pull_request": "acme/web#88" } ] },
    { "title": "journal",
      "repos": [ { "repo": "acme/api", "range": { "base": "<rev>", "head": "<rev>" }, "pull_request": "acme/api#413" } ] },
    { "title": "Not committed yet",
      "repos": [ { "repo": "acme/api", "worktree": { "include_paths": ["src/journal/"] } } ] } ] },
  "whitespace": "counts" }
```

Rules to decide in the lane and record as a decision:
- **A layer** has `title` (required, short), `purpose` (optional, one line,
  length-capped; navigation, not a finding) and one or more `repos`.
- **A layer's repo** is exactly one of:
  - `range`: `base` and `head`, each a commit, branch or tag. The diff is
    `base..head` as they are (no merge base), so a layer shows its own change
    whatever is checked out. This is the new part: today's `DiffSpec` always
    ends at the checkout (`DiffSpecCompare.Upto`).
  - `worktree`: work that is not committed, limited to `include_paths`, on
    top of HEAD (or the index). For the top layer and for a logical split.
  - optional `pull_request` (`owner/repo#n`), shown on the layer and opened
    from it.
- **Whole stack** is derived, not sent: per repo, the lowest layer's `base`
  to the highest layer's `head` (plus the worktree layers).
- **Order** is the list's order. A fork in the stack (two layers on one
  parent) is out of scope for the first cut: the agent sends one path, or
  lists branches one after another; say which in the decision.
- `order`, `groups` and `notes` of today's plan stay per review. A note
  gains an optional layer so a finding in a lower layer stays on that
  layer's lines (`review-humata-change` needs it).
- A stack and a plain `spec` in one call is refused: one or the other.
- Validation errors name the layer and the repo, and say what to fix (a
  `head` that isn't a descendant of `base`, a repo the session doesn't
  have, an empty layer, two layers with one title).

## Exit criterion
- An agent's `review_prepare` with a three-layer stack over two repos opens
  a view that lists the three layers; Whole stack shows both repos' full
  diffs; picking layer 2 shows only layer 2's diff in the repos it touches;
  next and previous step through; `review_focus` with a layer moves there.
- A layer's diff is right when the clone is checked out somewhere else
  (the top of the stack, or the trunk).
- A worktree layer shows uncommitted files limited to its paths.
- The stack survives a restart and a session move, as order and notes do
  (PR-26), and `review_context` returns the stack and the layer showing.
- Outside a review chat it arrives as an offer (VW-2) and Revert works
  (VW-9); in a review chat it applies at once with Undo (PR-24).
- Daemon tests beside `daemon/humatad/tests/review.rs`,
  `review_bases.rs` and `review_per_repo.rs`; engine tests in
  `daemon/review_diff`; MCP schema in `daemon/mcp/tests/schemas.rs`;
  reducer tests in `features/review/tests`; a headless render of the layer
  list in `desktop/tests`.
- `skills/shared/stacks.md` section 4 and its "Not built" are rewritten to
  send `stack`, and `render-humata-review` drops the groups stand-in.

## Read these (pointers, nothing else)
- SPEC: RV-31, RV-32, RV-33, RV-34, RV-36 (this milestone's rule), PR-22
  to PR-26, VW-2, VW-9, found with
  `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md`.
- Decisions: D55 and D57 (the diff spec is data), D59 and D791 (who
  reviews), D80 (the review chat applies a plan at once), D299
  (`review_commits`), D790 (the skills).
- Skills: `skills/shared/stacks.md` (how an agent picks layers; today's
  stand-in with groups), `skills/shared/diff-spec.md`,
  `skills/render-humata-review/guide.md`,
  `skills/review-humata-change/guide.md`.
- API: `api/proto/humata/v1/review.proto`:
  - `DiffSpec`, `RepoDiffSpec`, `DiffSpecBase`, `DiffSpecCompare` (the spec
    today, one base per repo, always up to the checkout);
  - `ReviewPlan`, `ReviewGroup`, `ReviewNote`, `PrepareViewRequest`;
  - `ReviewViewState`, `ReviewContext`, `FocusReviewRequest`,
    `ReviewChange`;
  - `ListPullRequestCommits` (a pull request's commits, D299).
  `api/MCP.md` §6.1 rows `review_prepare`, `review_focus`,
  `review_context`, `review_note`.
- Code, daemon:
  - `daemon/review_diff/src/spec.rs` (`DiffSpec`, `RepoSpec`, `Base`,
    `Compare`, `BaseContext`): where a range with a head goes;
  - `daemon/review_diff/src/diff.rs` and `git.rs`: computing a diff from a
    spec with the git CLI;
  - `daemon/review_diff/src/plan.rs` (`ReviewPlan`, `Group`, `Note`,
    `Arrangement`) and `chat.rs` (`ViewState`, `Place`, `ReviewChat`,
    `Undoable`): where the stack and the layer showing are kept and undone;
  - `daemon/review_diff/src/wire.rs`: proto to model;
  - `daemon/humatad/src/review.rs` and `review/views.rs`, `review/bases.rs`,
    `review/pull_requests.rs`, `review/comments.rs`: the services;
  - `daemon/mcp/src/tools.rs`: `diff_spec()` (the schema, line ~514),
    `Op::ReviewPrepare`, `Op::ReviewFocus`, `Op::ReviewContext`;
    `daemon/mcp/tests/tools.txt` and `schemas.rs`.
- Code, app:
  - `clients/src/model.rs`, `clients/src/views.rs`,
    `clients/src/review_chat.rs`, `clients/src/daemon/fake/review.rs`;
  - `features/review/src/lib.rs` and `order.rs` (order and groups),
    `features/review/tests/targets.rs`; `features/app/src/views.rs`;
    `features/views/src/view_tab.rs`;
  - `ui/src/panel_changes.rs` (the Diff panel's files list and base
    picker), `review/src/view` (the review pane).
- Tools: `tools/stack/src/status.rs` (`stack status --json`: `topics` with
  `parent`, `depth`, `branch`, `base`, `number`), for what an agent has in
  hand when it builds the payload. Humata does not run `stack` itself.
- Frames: none draws a stack. The layer list needs a frame and a Fable
  golden pick before the UI closes (D76).

## Out of scope
- Detecting stacks in the daemon or the app. The agent works the layers
  out (`skills/shared/stacks.md`) and the user can edit nothing but which
  layer shows. A person-made stack (a picker that builds layers) is a
  follow-up.
- Any reviewing: findings, verdicts and drafts are `review-humata-change`
  and the existing note and draft tools.
- Forked stacks drawn as a tree.
- Marking a layer reviewed, and per-layer Viewed. Mark reviewed (RV-28)
  stays per repo.
- A CLI command for `review_prepare`.
- Changes to `tools/stack`.

## Lane (written by the manager)
- Design notes:
- Claims:
- Tasks:
- Waiting on:
- Build gates:
- Ready to commit: no
