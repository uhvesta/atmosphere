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
  `parent`, `depth`, `branch`, `base`, `number`). Since the D800 amendment
  (T3b), the daemon runs it for `review_prepare {from_stack}` and builds the
  layers itself, so the agent no longer sends them by hand. GitHub stacked
  PRs stay the job of `tools/stack`, built from the same metadata.
- Frames: none draws a stack. The layer list needs a frame and a Fable
  golden pick before the UI closes (D76).

## Owner additions (2026-11-18 11:35–11:45, both in scope)
- **Stacks come from the stack tool's metadata, not the agent's tokens.**
  `review_prepare {from_stack: "<repo>"}` makes the daemon run
  `stack status --json` (tools/stack/src/status.rs: topic, parent, depth,
  branch, base, number) and build the layers. Topic names become titles and
  numbers become PR links; the agent may add a purpose per layer. Drift
  re-reads the stack. GitHub stacked PRs stay tools/stack's job, from the
  same metadata. Hand-built layers stay for cross-repo stacks and
  uncommitted work. Small additive tools/stack changes are allowed.
  Fable rules on: what the agent still sends, a machine without `stack`,
  and multi-repo stacks (D800 amendment).
- **The agent orders by importance.** Each file gets an importance: key,
  normal or skim. Skim files fold up at the end ("12 files to skim"); key
  files are marked. Each layer has its own order and groups, and the agent
  may order repos within a layer. Ordering hunks within a file is Fable's
  call: now or a follow-up. With no order sent, today's order stays.
  Importance shows on look-only stacks too. Needs a proto seam (D802).
- Exit adds: a from_stack review of a tools/stack stack opens with the
  right layers and PRs, with no layers sent by the agent; skim files fold
  and key files lead.

## Out of scope
- Detecting stacks other than through `stack status` (the D800 amendment).
  For any other stack the agent sends the layers
  (`skills/shared/stacks.md`), and the user can change only which layer
  shows. A person-made stack (a picker that builds layers) is a
  follow-up.
- Any reviewing: findings, verdicts and drafts are `review-humata-change`
  and the existing note and draft tools.
- Forked stacks drawn as a tree.
- Marking a layer reviewed, and per-layer Viewed. Mark reviewed (RV-28)
  stays per repo.
- A CLI command for `review_prepare`.
- Changes to `tools/stack` beyond the additive ones T3b needs.

## Lane (written by the manager)
- **Design notes** (rules in D800, picked by Fable 5.2):
  - **Engine: a diff with an explicit new side.** `review_diff::Compare` gains
    `head: Option<String>` and `worktree_paths: Vec<String>`. With `head`, the
    new tree starts from `tree_of(head)`, not the checkout. With
    `worktree_paths`, the working-tree files under those pathspecs (tracked
    staged and unstaged changes, plus untracked files that gitignore allows)
    are laid over it in the scratch index. Outside those paths nothing from
    the working tree enters. With either set, `upto`/`untracked` must stay
    at their defaults, else `DiffError::ConflictingCompare`. Proto:
    `DiffSpecCompare.head = 4`, `worktree_paths = 5`. A layer is then an
    ordinary `DiffSpec`, so GetDiff, GetFileDiff, `DiffTrees`, Viewed and
    Mark reviewed work unchanged (they stay per file and per repo; there is
    no per-layer state). Label: `base..head`, or `base..HEAD + <paths>`.
  - **New `daemon/review_diff/src/stack.rs`.**
    - Types: `Stack{layers}`; `Layer{title, purpose, repos}`;
      `LayerRepo{repo, part: Part, pull_request}`; `Part::Range{base, head}`
      or `Part::Worktree{include_paths}`.
    - Resolution: `Resolved{base_commit, head_commit}` is resolved at prepare
      and kept beside the revs as sent.
    - `Stack::check(clones)` refuses with a `StackError` that names the layer
      and the repo and says what to fix:
      - no layers, or more than 50 (send fewer layers, grouping commits);
      - an empty layer, or a repo listed twice in one layer;
      - a title that is empty or longer than 60 characters, or two titles the
        same (trimmed, ignoring case);
      - a purpose that is multi-line or longer than 120 characters;
      - a rev that does not resolve, or a base that is not an ancestor of its
        head (send the layer below's head as base, or fetch);
      - a chain break: in a repo, a range layer's base is not the commit of
        the previous range layer's head there (a fork: send the other branch
        as its own review);
      - a worktree layer below a range layer in the same repo, or HEAD not at
        the top range head at prepare (check out the head, or send the
        uncommitted work as its own review);
      - a worktree layer with no `include_paths`;
      - a repo the session does not have.
    - Per-layer outcomes:
      - A worktree layer with no files right now is accepted and shows "No
        changes", since worktree layers are live.
      - Overlapping `include_paths` are allowed and not checked.
      - `stack` together with `spec`, or neither, is refused in `wire`.
    - `Stack::layer_spec(title) -> DiffSpec`:
      - for a range, `Rev(base)` with `head`;
      - for a worktree, `Rev(<HEAD commit>)` with no head and
        `worktree_paths = include_paths`.
    - `Stack::whole() -> DiffSpec`: the union of the layers. Each repo's diff
      runs from its first layer's base to its top range head. If the repo has
      worktree layers, the diff is taken against HEAD instead, with
      `worktree_paths` set to the union of those layers' paths. Nothing
      appears in Whole stack that no layer shows; an agent that wants
      everything sends `"."`.
    - `Stack::drift(clones) -> Vec<Drift{layer, repo, kind}>` is re-run at
      every diff. A kind is one of:
      - Moved: a rev now resolves elsewhere and ancestry or the chain broke;
      - Missing: the rev is gone, for example after a session move;
      - HeadMoved: HEAD left the top head under a worktree layer.
      A drifted layer's repo returns no diff, only the drift. The rest of the
      stack still works, and a wrong diff is never drawn.
  - **Layer identity: the trimmed title.** Empty or absent means Whole stack,
    everywhere: MCP `layer`, and the proto `string layer` on
    `FocusReviewRequest`, `ReviewNote` and `ReviewViewState`. An unknown
    title is refused and the refusal lists the valid ones.
  - **Notes.**
    - `ReviewNote.layer` (proto 10): a layered note is checked and anchored
      against its layer's diff, and shows on that layer. It also shows in
      Whole stack where its quoted lines are still found there; otherwise it
      shows on its layer only, not as outdated.
    - An unlayered note belongs to Whole stack only.
    - `review_note` gains an optional `layer`; without it, the note is on
      Whole stack, not on the layer showing.
    - D791's open item is settled by T5's test: today `review_note` is gated
      only by the review chat being reported, not by a PR review.
  - **Order and groups** stay per review and apply within whichever layer
    shows; files a layer lacks are skipped. They are no longer the layer
    stand-in.
  - **Review chat.**
    - `ViewState` gains `stack: Option<Stack>` and `layer: Option<String>`.
    - `ChangeKind::Focus` and `Replaced::Focus` carry the layer.
    - `Replaced::Order` keeps the previous stack and layer, so that Undo, Back
      and Revert restore the stack, the layer and the bases as one unit.
    - `review_focus` with `layer`, and optionally no file, moves the view to
      that layer (PR-24: applied at once with Undo, and never while the user
      scrolls). A file not in that layer is refused, naming the layers that
      have it.
    - `review_context` returns the stack, the layer showing and, per layer and
      repo, the resolved commits and any drift, so the agent can send again
      without guessing.
    - A new `review_prepare` keeps the layer showing if a layer with that
      title still exists; otherwise it shows Whole stack.
  - **Persistence.**
    - `ReviewViewState` is saved as the chat bytes in `ReviewRecord` (PR-26),
      so the stack and the layer survive a restart and a move. The stored
      `ReviewUndo.order` bytes carry the previous stack and layer.
    - Views store: `ReviewPlan.stack = 5`, a new message `ReviewStack`, kept
      in the plan as sent.
    - After a move the revs are resolved again in the destination clone; a
      rev that does not resolve is Missing drift, not a failed view.
  - **Cost.** Layer diffs are computed only when a layer is picked, and cached
    by resolved commits. File counts on the rows come from
    `git diff-tree --numstat` per layer (one call per layer repo). Worktree
    layers refresh on the existing file-change push, never by polling.
  - **App.**
    - Model:
      - `RepoBase::Range{base, head}` and
        `RepoBase::Uncommitted{paths: Vec<String>}`, plus a field on
        `RepoDiffSpec` for the overlay paths, so that Whole stack is
        `Range`+paths.
      - `DiffBase::Layer(Vec<RepoDiffSpec>)`: the live client sets the
        `GetDiffRequest.repos` filter to exactly those repos, so repos the
        layer does not touch are not shown at all.
      - `spec_to_wire` picks the compare per `RepoBase`.
    - `PlanView.stack: Option<StackView{layers: Vec<LayerView{title, purpose,
      repos: Vec<LayerRepoView{repo, pull_request, files, drift}>}>}>`.
    - Review state:
      - `ReviewState.layer: HashMap<SessionId, Option<String>>`.
      - Actions `PickLayer(Option<String>)`, `NextLayer` and `PrevLayer`. The
        rows are Whole stack as row 0, then layers 1..N; there is no wrap.
      - The fetch uses the layer's spec and leaves the user's own targets
        alone.
      - The base picker is hidden while a stack shows.
    - The review chat's follower applies a focus with a layer, and `Reverse`
      restores it.
  - **UI** (new `ui/src/panel_stack.rs`, drawn by `panel_changes.rs` above
    `plan_list`):
    - The rows run top-down, 1 being the bottom of the stack: first Whole
      stack with its total files and +/-, then N rows, each with the number,
      the title, the pull requests (`api #412 · web #88`, collapsing to "+N"
      when they don't fit) and the file count.
    - The purpose shows under the title on the selected row only, and the
      selected row is highlighted like a selected file row.
    - At most about 6 rows show; the list scrolls and keeps the selected row
      in view.
    - The header's comparing line reads "Layer 2 of 3 · journal" or "Whole
      stack · 3 layers".
    - A drifted layer's repo shows "Moved since it was sent" with one action,
      Ask the agent to send it again.
    - The offer reads "3 layers over 2 repos". Open it to see Whole stack.
  - **Look versus review** (the owner's key distinction). A stack without
    notes draws no review chrome: no note gutters, no draft affordances, no
    verdicts. This has its own test.
  - **Keys.**
    - `alt-cmd-down` and `alt-cmd-up` move to the next and previous row while
      the Diff panel shows a stack.
    - The palette gets "Next layer", "Previous layer" and "Whole stack".
  - **Goldens.**
    - New `desktop/tests/design/stack.rs` draws two frames:
      - 119-diff-stack-whole: Whole stack picked.
      - 120-diff-stack-layer: layer 2 "journal" picked, its purpose showing,
        and only api shown.
    - The fixture is the milestone's example: 1 "codec" touches api and web,
      with a pull request in each; 2 "journal" touches api only; 3 "Not
      committed yet" is a worktree layer in api.
    - Existing frames 84-86 do not change.
    - Fable approves the two pictures once main has recorded them.
- **Claims** (after main commits the 0.3.2 batch; until then nothing is
  touched):
  - Mine:
    - `daemon/review_diff/src/{spec,checkout,diff,error,plan,chat,wire}.rs`;
    - new `daemon/review_diff/src/stack.rs`;
    - new `daemon/review_diff/tests/stack.rs`;
    - `daemon/review_diff/tests/specs.rs`;
    - `daemon/humatad/src/review.rs` (prepare, focus, context, get_diff and
      saved/undo parts);
    - `daemon/humatad/src/review/views.rs`;
    - `daemon/humatad/src/mcp.rs` (the `review_*` handlers and
      `review::plan`/`repo_spec` only);
    - new `daemon/humatad/tests/review_stack.rs`;
    - `clients/src/{views,review_chat}.rs`, `clients/src/review_chat/fake.rs`
      and `clients/src/daemon/fake/review.rs`;
    - `clients/live/src/{views,diff,review_chat}.rs`;
    - `features/review/src/lib.rs` and new `features/review/src/stack.rs`;
    - new `features/review/tests/stack.rs`;
    - `features/pr_review/src/follower.rs`;
    - `features/app/src/views.rs`;
    - `features/views/src/view_tab.rs` (the offer line);
    - `ui/src/panel_changes.rs` and new `ui/src/panel_stack.rs`;
    - new `desktop/tests/design/stack.rs`;
    - new `desktop/tests/native/stack.rs`;
    - `skills/shared/stacks.md` and `skills/render-humata-review/`;
    - `skills/review-humata-change/guide.md` (note `layer` only);
    - `api/MCP.md` §6.1 rows;
    - DECISIONS D800.
  - SEAMs for main, as scratchpad `stackview/*.py --check` scripts:
    - `api/proto/humata/v1/review.proto`;
    - `api/proto/humata/store/v1/store.proto` (comment only, as the bytes
      already carry it);
    - `daemon/mcp/src/tools.rs` with `daemon/mcp/tests/tools.txt` and
      `schemas.rs`;
    - `clients/src/model.rs`;
    - `features/app/src/lib.rs` and `overlay.rs` (key routing);
    - `ui/src/{lib,keys}.rs` and the palette entry;
    - `desktop/tests/design.rs` (`mod stack;`);
    - `desktop/BUILD.bazel` if the native test needs it;
    - `daemon/review_diff/BUILD.bazel` (none: tests are globbed);
    - `daemon/humatad/tests/common/mod.rs` (additive helpers, if any).
  - Out of my way: AGENTS2 (agent channels) and LIVEHOLDER (daemon/holder,
    humatad upgrade and revive). STACKVIEW touches none of their files.
- **Tasks.** Failing tests come first; I write each test, then a worker makes
  it pass. Haiku 5.5 high unless marked.
  - **T0 (me): failing tests and seam scripts.** No implementation. The
    tests:
    - `daemon/review_diff/tests/stack.rs`: check refusals (one per
      `StackError` kind, asserting the variant, layer and repo, not the
      wording); `layer_spec` and `whole` diffs on a three-layer two-repo
      fixture with the clone checked out at trunk and at the top; a worktree
      layer limited to its paths; an empty worktree layer gives "no changes";
      drift after an amend, and Missing after a rev is deleted.
    - `specs.rs` cases for `head` and `worktree_paths`, and their conflicts.
    - The seam scripts: proto fields, the MCP schema (`stack`, the `layer` on
      `review_focus` and `review_note`) and the `model.rs` variants.
  - **T1: engine new side.** `spec.rs`, `checkout.rs` (`new_tree` from head,
    overlay paths), `diff.rs` (label), `error.rs`, `wire.rs` (compare fields).
    Passes `review_diff:specs_test`.
  - **T2: stack model.** New `stack.rs`, with `check`, `layer_spec`, `whole`,
    `drift` and numstat counts, plus `wire.rs` stack to and from pb. Passes
    `review_diff:stack_test`. Sonnet medium (git ancestry and chain logic).
  - **T3: plan and chat.** `plan.rs` (note layer, anchor per layer),
    `chat.rs` (ViewState stack/layer, Focus layer, Replaced, Undo/Back, keep
    layer on re-prepare), and `wire` for view/context/change. Passes
    `review_diff:review_diff_unit_test` and the new cases in `stack_test`.
  - **T3b: review from the repo's stack metadata (D803).** The agent sends
    `review_prepare {from_stack: {repo, tip?, topics?}}`, and the daemon
    builds the layers from `stack status --json --offline`. This saves the
    tokens of spelling out layers. It runs after T3 is green, in two parts:
    - **T3b-1, `tools/stack`, additive only:**
      - `head` and `base_commit` per topic in `--json`, from
        `local::rebuild`'s `Tip`.
      - `--offline`: no fetch and no gh. The GitHub fields are null and the
        merged set is empty.
      - Commits that `copy_onto` makes keep the source identity and dates,
        so they get the same ids on every run.
      - Tests in `tools/stack` (status run twice gives the same heads;
        offline needs no gh). Haiku 5.5 high.
    - **T3b-2, `review_diff`:** a new `from_stack.rs` with
      `StackSource::read_blocking(checkout, tip, overrides)`. It parses the
      status JSON into `Vec<Layer>`, in chain order, titled by each topic's
      first commit subject, and keeps the topic name on `Layer`.
      - Refusals: a fork without `tip`, an unknown override, a restack in
        progress, no topics, a tool that is missing or too old (the
        Outdated case raises Migrate), and an explicit range in the same
        repo.
      - Drift by topic name, and pinning refs for the heads.
      - Failing tests come first, in `tests/from_stack.rs`, run against a
        JSON fixture and a fake runner so they need no real CLI.
      - Sonnet medium.
    - **Seam S1b** (proto): `ReviewPlan.from_stack` (`ReviewFromStack{repo,
      tip, map<string, ReviewTopicOverride> topics}`) and
      `ReviewLayer.topic`. The MCP schema in `tools.rs` is part of seam S2.
  - **T3c: importance and per-layer order (D802).** It runs after T3b-2.
    - **Seam S1b**, shared with T3b, adds `ReviewImportance`,
      `ReviewPlan.importance`, `ReviewLayer.order`, `.groups` and
      `.importance`, and `FileSummary.tier`.
    - **`review_diff`:**
      - In `plan.rs`, `Tier`, plus `importance` on the plan and on each
        layer.
      - In `stack.rs`, each `Layer` gets its order and groups.
      - A new `importance.rs` with the auto-skim rules (lockfile names,
        `linguist-generated` from `git check-attr`, pure renames) and the
        rule that works out a file's tier.
      - In `ReviewPlan::arrange`, key files first when no order was sent,
        skim files last as a folded section, and groups left empty
        dropped.
      - Whole stack joins the layers' orders, then their importance.
      - `wire` carries all of it.
    - **Tests first,** in `tests/importance.rs`: the auto-skims, the agent
      winning either way, a duplicate refused, the arrangement with and
      without an order, and the Whole stack join.
    - **Reducer and UI:** they draw the key dot, the folded skim row and
      its keys, as part of T7 and T8. Frames 119 and 120 gain the dot and
      the folded row, and a new frame 121 shows the fold opened.
    - **Hunk order** is a follow-up.
    - Sonnet medium.
  - **T4: daemon services.** `review.rs` (checked_plan with stack, get_diff
    for a layer, focus_review with layer, context with drift, saved/undo),
    `review/views.rs` (draft offer, recheck). Failing test
    `daemon/humatad:review_stack_test`, which covers:
    - a three-layer two-repo prepare opens with the layers listed;
    - Whole stack has both repos;
    - layer 2 has only api;
    - a checkout elsewhere still gives the same diff;
    - restart keeps the stack and layer;
    - outside a chat it is an offer and Revert restores it;
    - in a chat it applies, and Undo restores;
    - a re-send keeps the layer;
    - drift is reported per layer.
    Sonnet medium.
  - **T5: MCP.** `mcp.rs` handlers: `stack` in `review_prepare`, and `layer`
    in `review_focus` and `review_note`. Failing tests:
    - `daemon/humatad:mcp_test` case
      `the_review_chats_agent_sends_a_stack_and_steps_the_user_through_it`;
    - a `review_note` outside a PR review (D791);
    - `daemon/mcp:schemas_test` golden after the seam.
  - **T6: clients.** `views.rs` StackView, `review_chat.rs` layer, the fakes,
    and live `views.rs`/`diff.rs`/`review_chat.rs`. Failing: `clients/live`
    unit tests (wire to model and back, `DiffBase::Layer` sets the repo
    filter) and `clients/live:review_chat_test`.
  - **T7: reducer.** New `features/review/src/stack.rs`, `lib.rs` (layer per
    session, PickLayer, Next and Prev, fetch with the layer spec, base picker
    hidden), `follower.rs`, `features/app/src/views.rs` (open and Revert as
    one unit). Failing tests:
    - `features/review:stack_test` (TestStore): pick, next and prev stop at
      the ends, the fetch is for that layer's repos only, and the user's
      targets are untouched;
    - `features/app:views_revert_test` case.
  - **T8: UI.** `panel_stack.rs` and `panel_changes.rs`, the header line,
    drift row, offer line and keys. Failing tests:
    - `desktop:native_test` `stack.rs`, rendered headless: the layer list
      draws, alt-cmd-down picks layer 1, and a stack with no notes has no
      note gutter or draft affordance;
    - `desktop:design_test` frames 119 and 120.
    Sonnet medium.
  - **T9: skills and docs.**
    - `skills/shared/stacks.md` §4 and "Not built" are rewritten to send
      `stack`.
    - `render-humata-review` drops the groups stand-in.
    - `review-humata-change` sends a note's `layer`.
    - `api/MCP.md` rows.
    - The OVERVIEW and GAPS_REVIEW entries.
  - The order is T0, then T1 and T6's model part, then T2, T3, T4/T5, then
    T7, then T8. T9 can run any time after T0.
- **Waiting on:**
  - main's commit of the current batch (the files in api/proto,
    daemon/humatad, clients, features and ui), then main's "go";
  - main to apply the T0 seam scripts (review.proto, tools.rs, model.rs)
    before T4-T6 can build;
  - the key-routing and palette seams before T8;
  - main recording frames 119 and 120, then Fable's pick.
- **Build gates** (main runs them, batched, focused targets only):
  - G1 after T1-T3: `//apps/humata2/daemon/review_diff:specs_test`,
    `:stack_test` and `:review_diff_unit_test`.
  - G2 after T4-T6: `//apps/humata2/daemon/humatad:review_stack_test`,
    `:review_test`, `:review_bases_test`, `:review_per_repo_test` and
    `:mcp_test`, `//apps/humata2/daemon/mcp:schemas_test`, and the
    `//apps/humata2/clients/live` unit and `review_chat_test`.
  - G3 after T7-T8: `//apps/humata2/features/review:stack_test`,
    `:targets_test` and `:plan_rows_test`;
    `//apps/humata2/features/pr_review:all`;
    `//apps/humata2/features/app:views_revert_test`;
    `//apps/humata2/desktop:native_test`; `:design_test` (then record 119
    and 120).
  - Before commit, the fast tier: clippy on the touched crates.
- **Status, 2026-11-18 11:30:**
  - Passed G1 (main ran `review_diff:specs_test`, `:stack_test` and
    `:review_diff_unit_test`):
    - **T0.** Failing tests in `review_diff`: `tests/specs.rs` gains 5
      cases, the new `tests/stack.rs` has 18, and `src/wire.rs` unit tests
      gain 4. Seam S1 (`review.proto`) was applied by main. The proto
      literal sweep adds the new fields to the existing literals in
      humatad, clients, cli and tests.
    - **T1.** `Compare.head` and `worktree_paths`, `Compare::at` and
      `Compare::worktree`, `DiffError::ConflictingCompare`, `Seed::Tree`,
      and the label.
    - **T2.** New `src/stack.rs`: `check_blocking`, `layer_spec`, `whole`,
      `resolved` and `drift_blocking`.
    - **T2b.** `wire` reads and writes the compare's `head` and
      `worktree_paths`, and has `stack_to_pb` and `stack_from_pb`.
  - Unverified: the humatad, cli and clients/live builds (blocked while
    LIVEHOLDER's holder crate does not compile). Main reruns them then.
  - **T3, gate 2 (main):** `stack_chat_test` (10 cases), `notes_test`,
    `specs_test` and `stack_test` passed. humatad2 built and
    `humatad2_unit_test` passed. `review_diff_unit_test` was missing
    `layer: None` in the `line_note` helper at chat.rs:922. Main fixed it,
    and gate 3 reruns it. T3 is green if gate 3 passes.
  - Earlier T3 notes: the failing tests are in
    `review_diff/tests/stack_chat.rs` (10 cases). A worker is adding
    `ReviewPlan.stack`, `Note.layer`, `ViewState.stack` and `.layer`,
    `ReviewChat::focus_layer`, `ChatError::NoSuchLayer`,
    `ChangeKind::Layer` and `Replaced::Layer`, keeping the layer across
    a re-prepare, Undo of stack and layer, and the wire for all of it.
    It also covers humatad's stored undo (`review.rs`) and comment
    anchors (`review/comments.rs`). Gate: `review_diff:stack_chat_test`
    and `:review_diff_unit_test`, then a humatad build.
  - Planned at 11:45 from the owner's input: T3b reads the repo's stack
    metadata (D803, amends D800), and T3c adds importance and per-layer
    order (D802). Both proto changes are in one seam, S1b, at
    `scratchpad/stackview/s1b_from_stack_proto.py`. Main applies it with
    one literal sweep after T3 is green, and then the T3b and T3c workers
    start.
  - Not started: T4 and T5 (daemon and MCP), T6 and T7 (clients and
    reducer), T8 (UI and frames 119-121, which need Fable's approval)
    and T9 (skills). Seams S2 and later (`tools.rs`, `model.rs`, keys and
    palette, `design.rs`) are not written yet. A `ReviewChange` arm for a
    layer move is needed (seam).
- Ready to commit: T0-T2b yes (additive and unused outside `review_diff`).
  T3 only once its gate passes.
- **Status, 2026-11-18 (manager, after the S1b seam):**
  - Seam S1b applied by the manager: `review.proto` (tier = 12,
    `ReviewPlan.from_stack` and `.importance`, layer topic, order, groups and
    importance, `ReviewChange.layer = 12`), the literal sweep in wire.rs,
    mcp.rs and tests, and the Rust types (`FromStack`, `Tier`,
    `Importance`, `Layer` fields). Seam S2 (`daemon/mcp/src/tools.rs`
    schemas) and the app types (`RepoBase::Range`/`Overlay`, `StackView`,
    `ReviewView.stack`/`.layer`, `StackRows`) are in. Decisions in D804.
  - Workers:
    - W1 (Haiku) T3b-1, tools/stack `status --offline` with `head` and
      `base_commit`: written, waiting for `//tools/stack:status_test`.
    - W2 (Sonnet) T3b-2, `review_diff/src/from_stack.rs` and
      `tests/from_stack.rs`: written; the manager capped titles at 60 and
      deduped them, and added serde_json to BUILD.
    - W3 (Sonnet) T3c importance, W4 (Haiku) T9 skills and MCP.md, W5
      (Sonnet) T6 clients, W6 (Sonnet) T7 reducer, W7 (Sonnet) T8
      panel_stack.rs and keys: running.
    - All written by 2026-11-18 (batch G4, unbuilt): W1-W7, T4
      (`humatad/src/review/stack.rs`, review.rs, review/views.rs,
      tests/review_stack.rs) and T5 (`humatad/src/mcp.rs`). Manager fixes:
      FilePlan export, FocusRequest `layer`/`whole_stack` and its literals,
      `ShowLayer` routing in features/app pr_review.rs, NextLayer/PrevLayer
      handlers in ui shell.rs, the humatad test ReviewPlan literals, stale
      stack text in review-humata-change and humata-pull-requests skills.
    - Open: palette entries (features/palette lib.rs + app commands.rs,
      other lanes' files), the Migrate stack-tool Inbox item (needs a
      Migrations handle from serve.rs), FileSummary tier into the app's
      FileDiff and humatad serving tiers (`// T3c: tiers` marker), the
      from_stack refresh on a moved stack, stack kept across a daemon
      restart.
  - 2026-11-18: hook-up built in own files. `ui/src/panel_stack.rs` now has
    `stack_top` (layer list, "web isn't in this layer" lines, failed-ask
    line), `header_line`, `hides_base_picker`, `stack_keys` (DIFF_STACK),
    `skim_row`, `key_dot`, `skim_open`, `tier_of` (Normal until T3c), with
    headless tests. features/review gains `stack::{Tier, fold_skims,
    skim_label, ask_again_text}`, `ReviewAction::AskAgain`/`AskedAgain` (types
    one keyed message into the session) and `ask_again_error`;
    `tests/stack_fold.rs`. The panel_changes.rs lines are a `--check`-able
    seam script (7 anchored edits) for main to apply when DIFFVIEW lets go.
  - Still open after the seam: T3c tiers into the app's FileDiff (so
    `tier_of` returns real tiers), skim fold in the plan-ordered list,
    frames 119-121 with Fable's approval.
  - 2026-11-18 (T3c tiers): daemon side done and tested. GetDiff serves
    each file's final tier (agent's from the chat view, else auto-skim) via
    `served_tiers`; plans and layers listing a file twice in `importance` are
    refused; a joined Whole stack gets the layers' order and importance;
    the chat keeps the plan's tiers through reports and Undo
    (`ViewState::agent_importance`, tests/importance_chat.rs). features/review
    gains `plan_layout`/`plan_layout_with` (skim fold per repo with `after`,
    groups emptied by skims dropped, key first with no order;
    tests/plan_skims.rs). Seams for main, both `--check` clean, to apply
    together: `seam_tiers.py` (FileDiff.tier/FileTier, PlanView/LayerView
    importance, ReviewViewState.importance = 9, `tier_of` body, 11 literal
    sites) and `seam_panel_changes.py` (now also the plan list's skim rows).
    Decision D805. Still open: frames 119-121 with Fable's approval.
## Spec and cases

- The stacks UI rules are DS-60 to DS-68 in [DIFFVIEW-spec.md](DIFFVIEW-spec.md) §2.9. T4, T6, T7, T8 and T3c must meet them (see [REVIEWUI.md](REVIEWUI.md), "What STACKVIEW owns").
- Key change: files keep ⌥⌘↓/↑ (DIFFVIEW). Layers move with ⌥⌘] / ⌥⌘[ (spec §8 item 1). T8's `alt-cmd-down` test and the `stacks` mockup's key hint change with it. Fable confirms before T8 binds keys.
- Cases: [15](../../flows/cases/diff/15-a-stack-across-repos.md) and [16](../../flows/cases/diff/16-a-stack-that-moved.md) in `flows/cases/diff/`.
  - 2026-11-19 (DIFFUI2): Fable judged candidates 119–121: all REJECTED. Shared fixes B1–B7: the review gutter (number, 6 pt, two-cell sign, code), code clipped 14 pt from the right, the picked layer's badge filled accent with a white number or ≡, a "STACK · 3 LAYERS" caption, the file bar without the leading icon and with "Open in editor", the stack footer (Whole stack: "N pull requests open" link + disabled "Send to claude"; a layer: "‹ Previous" / "Next layer ›" + "Looks good"), and file rows with an empty 14 pt tick circle and a right-aligned "key" chip instead of the blue dot (Fable rules the chip supersedes the dot in DS-67). Per frame: 119 repo rows "against main" and a file count that matches the list; 120 the pages.rs path and hunk from stacks.png frame 2; 121 the drift strip with a bordered "Ask the agent to send it again", writer.rs out of layer 1, totals that match their rows. Fix pass running.
  - 2026-11-19 (DIFFUI2): round 2 fixed B1–B7 and the per-frame notes; Fable then asked for two more: no purpose line on a picked Whole stack row, and "≡" in the unpicked Whole stack badge. Both are in, and Fable said no further round was needed. 119–121 are APPROVED and wait to move from CANDIDATES into FRAMES and be recorded.
  - 2026-11-19 (frames in the real panel): the Diff panel's stack UI now copies 119–121. That covers the caption, the badge, the drift strip on the picked layer only, untouched repos, layer and Whole footers, the Push & PR fallback, collision-aware repo names, empty repo heads, and quiet 24 pt stacked rows. The Whole count survives a layer pick. Native test `stack::the_stack_frames_as_the_app_draws_them` with goldens stack-whole, stack-layer and stack-skim-drift. The stack-layer-counts(-full) and looks-good-* goldens were re-recorded. Fable round 1 rejected 3 of them, and round 2 approved all 8. D865 lists the decisions and follow-ups: the two-column gutter and the full-screen uxs-2-2 layout are next.
