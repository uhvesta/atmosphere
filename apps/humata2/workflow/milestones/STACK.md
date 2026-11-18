# STACK: One command turns my commits into a linked stack of PRs

**Outcome:** a Rust CLI (in `tools/stack/`, built and run only through Bazel) makes each commit on a branch into its own GitHub PR. Commits are grouped and linked by `Topic:` and `Relative:` trailers in their messages. The CLI fills each PR description from our PR template, and posts and updates a review-history table on each PR, so reviewers can follow the revisions even though each PR is one commit. It also works with GitHub's own stacked-PR support (`gh stack`) where that is available.

**Owner asks (2026-11-16):** like revup in behaviour, but our own. Include a PR template, agent instructions for writing PR descriptions, and the history table. Add a skill that is a thin pointer to the CLI. The CLI's `--help` and subcommands must be enough for an agent to build a stack.

**Exit:**
- `bazel run //tools/stack -- upload` on a branch of trailer-tagged commits creates or updates one PR per topic, with the right base chain.
- Re-running after an amend updates the PRs in place and adds a row to each history table.
- Tests run on a fake gh and a local bare remote: no network.
- `tools/skills/<name>/SKILL.md` points to the CLI, and its `--help` covers every command.

**Rules:**
- AGENTS.md applies: Bazel only, explicit error enums with sources, actionable messages, minimal visibility, behaviour tests.
- Use the gh CLI for auth. Never store or print tokens.
- Don't name the inspiration in user-facing text.

**Out of scope:** publishing to crates.io, and changing the Lifr revup setup.

## Lane

Coach: STACK manager (Opus 5.5), Haiku 5.5 high workers. Advisor: Opus 5.5 high (Fable out until 2026-11-22).
Scripts: scratchpad `stack/` (`python3 <script> [--check]`; each refuses on a base sha mismatch).

### Picks (Opus advisor, 2026-11-16)
- Binary `stack`; lib crate `stack_cli` (private to the package). Commands: `check`, `upload`, `status`, `guide`
  (`guide --template` prints the effective template). No prompts. Exit codes: 0 ok, 1 git/gh could not run,
  2 usage/trailer/plan/template error, 3 rebuild conflict, 4 GitHub refused.
- `upload [--dry-run] [--json] [--base <trunk>] [--remote origin] [--draft] [--topic <t>]... [--note <text>]
  [--no-native-stack] [--overwrite-body]`; `--topic` pulls in relative ancestors. `check [--json]` is local only
  (no gh, no fetch) and dry-rebuilds. `status [--json]`: per topic branch, PR, state, review decision, base,
  "up to date"/"needs upload"; open `<login>/stack/*` PRs with no local topic as orphaned.
- Trailers (case-insensitive keys, last paragraph): `Topic` (`[a-z0-9._-]+`), `Relative` (one per topic, every
  commit that sets it agrees), `Reviewers`, `Labels` (comma lists, added never removed), `Draft` (on create).
  Untagged commits stay local and are listed.
- Branch `<gh login>/stack/<topic>`. Rebuild with `git merge-tree --write-tree` + `git commit-tree` onto the
  relative's tip (root: merge-base with `<remote>/<trunk>`), only when a commit is not already there; author and
  committer kept so re-runs give the same shas. One `git push --atomic` with `--force-with-lease` per branch.
- Title: first commit's subject. Body: first commit's body in the template's sections (text before the first
  heading is Summary), `## Commits` list when >1 commit, then `<!-- stack:body sha=... -->`. Regenerated every
  upload unless the live body's hash no longer matches (edited on GitHub): then kept with a warning, unless
  `--overwrite-body`.
- Template: `.github/pull_request_template.md` (also GitHub's web default); plain `##` headings with HTML-comment
  hints; `<!-- required -->` marks Summary, Why, Testing. A built-in copy is used when a repo has none.
- One history comment per PR, marker `<!-- stack:history v1 -->`, edited in place: stack tree (this PR bold) and
  `| Rev | Head | Base | Diff | Pushed (UTC) | Summary |`, absolute links, a row only when the head changes; Diff
  is `full` (rev 1), `since N` (same base), `rebase` (patch-id unchanged) or `rebase + changes`; `--note` is
  appended to Summary. State is parsed back from the comment; no local state.
- Native stacks: if `gh stack --help` exits 0 and not `--no-native-stack`, after everything else run
  `gh stack link --base <trunk> <pr-url>...` for each first-child root-to-leaf path of length >= 2; failure is a
  one-line note, never an upload failure. Membership is additive: say so when the chain changes.
- Upload order: retarget PRs whose base changes to trunk, push, create parent first, set final bases, bodies,
  history comments, native link. Never delete remote branches. Closed PR: refuse ("reopen #N or rename the
  Topic"). Merged topic: skipped, dependents based on trunk, note to rebase locally.
- Tests: behaviour tests drive the built `stack` binary against a local bare remote and a fake `gh` (a Rust
  binary symlinked as `gh` on PATH, state in a JSON file). No network.

### Layout
`tools/stack/`: `BUILD.bazel`, `Cargo.toml`, `src/{main,lib,cli,error,trailers,plan,git,gh,body,history,clock,
upload,status,check,guide}.rs`, `src/guide.md`, `src/default_template.md`, `tests/{fake_gh,upload,check,status,
help}.rs`, `tests/support/mod.rs`. Also `.github/pull_request_template.md`, `tools/skills/stack/SKILL.md`, root
`Cargo.toml` member line (seam; repin).

### Tasks
| ID | Task | Who | Files | Status |
|---|---|---|---|---|
| S0 | Scaffold: crate, BUILD, stubs with every signature, error enums | coach | all `tools/stack/` src stubs, Cargo, BUILD, root Cargo.toml | green (b17) |
| S1 | Fake gh, test support, failing behaviour tests | coach | `tools/stack/tests/` | green (b17) |
| W1 | trailers + plan (pure, unit tests) | Haiku | `src/trailers.rs`, `src/plan.rs` | green (b17) |
| W2 | git runner | Haiku | `src/git.rs`, BUILD (tempfile for unit tests) | green (b17) |
| W3 | gh runner | Haiku | `src/gh.rs` | green (b17) |
| W4 | body/template + clock | Haiku | `src/body.rs`, `src/clock.rs` | green (b17) |
| W5 | history comment | Haiku | `src/history.rs` | green (b17) |
| W6 | local read/rebuild + check (W6a); upload flow (W6b) | Haiku | `src/{local,check}.rs`; `src/upload.rs` | green (b17) |
| W7 | status | Haiku | `src/status.rs` (guide.rs done in S0) | green (b17) |
| W9 | stack graph model (pure, unit tests) | Haiku | `graph/src/lib.rs` | green (b17) |
| W8 | docs: guide.md, repo PR template, SKILL.md | Haiku | `src/guide.md`, `.github/pull_request_template.md`, `tools/skills/stack/SKILL.md` | green (b17) |

### Shared stack model (agreed with main for the PR review lane, 2026-11-16)
- The PR base chain is the truth about the stack. Trailers only decide what upload pushes. Readers (`status`, the history tree, PR review) build the stack from PR bases.
- Linear chains are linked natively through GitHub stacked PRs where GitHub allows it (GA 2026-10-06; `gh stack`, gh >= 2.90). Each root's chain is the root, then its lowest-numbered child, and so on. Other branches of a tree chain by base branch only. When native linking is unavailable or refused, the upload still succeeds and prints a one-line note.
- Head branch names are stable across amends: `<login>/stack/<topic>`. Renaming a Topic opens a new PR.
- `//tools/stack/graph` (crate `stack_graph`, no I/O, no deps) holds the model: `Pr{number, head, base, title}`, `Stack::new`, `nodes`, `node`, `tree_of`, `native_chains`. Its visibility is `//tools/stack:__pkg__` for now; widen it to `//apps/humata2/daemon/...` when PR review needs it. Delta script: `s2_graph_crate.py` (apply it after S0; it needs a second repin). Implementation: W9.

### Workers
- Every worker is Haiku 5.5 high. Each writes one apply script with `scratchpad/stack/mkapply.py`, against `scratchpad/stack/cur`, the tree as it is once every sent script is applied. The script refuses when a file's sha256 doesn't match the base. The coach reviews each diff before sending it.
- Send order: S0, S2, W1, W3, W2, W9, W4, W5, W6a, W7, W8, W6b, then F1 (sha2 import in body.rs). All applied; main fixed one history test (`trim_end`).
- Known v1 gaps (by design, for later):
  - a merged topic with new local commits is skipped with a note, not refused;
  - there is no virtual diff base for `since N` across a rebase;
  - PRs from forks are out of scope;
  - stale `<login>/stack/*` branches are never deleted (status lists their open PRs).

### Ready to commit (2026-11-16)
Green in main's batch 17: `//tools/stack:stack_cli_test` (71 unit), `check_test` (6), `help_test` (5),
`status_test` (2), `upload_test` (16), `//tools/stack/graph:graph_test` (9). The exit criterion holds:
- `upload_opens_one_pr_per_topic_chained_by_base` and `upload_of_one_topic_takes_the_topics_it_builds_on`:
  one PR per topic, bases chained root to leaf.
- `uploading_after_an_amend_updates_the_same_prs_and_adds_a_row`, and
  `uploading_again_without_changes_adds_no_row_and_pushes_nothing`: updates in place, one history row per new head.
- Every test runs the built `stack` against a local bare remote and `fake_gh` (state in a JSON file) on PATH: no network.
- `tools/skills/stack/SKILL.md` points to `bazel run //tools/stack -- guide`; `help_test` checks every command has `--help`.
How to check: `bazel test //tools/stack/...`; then, in a scratch clone, `bazel run //tools/stack -- guide` and
`bazel run //tools/stack -- check` (local only, never calls GitHub).
Follow-up for the PR lane: D300 describes the shared model as `Stack{repo, trunk, source, github_number, entries}`
with `parent_of`/`children_of`; the crate as built has `Pr{number, head, base, title}`, `Node{pr, depth, parent}`,
`Stack::new`, `nodes`, `node`, `tree_of`, `native_chains`. PR adds what it needs (`children_of`, repo/trunk) when it
first depends on the crate and widens its visibility then; nothing uses it outside `tools/stack` yet.
Decision: D410 (Provisional).

### v2: portable config, chain, restack, pins (2026-11-16, stack_v2.py)
- Prefix `<prefix>/stack/<topic>` from `--prefix`, stack.toml or the gh login; config in `~/.humata2/config/stack.toml`
  (unknown and secret keys refused); compiled-in template; `stack config show`.
- Every body carries the stack table; `stack status` draws the tree (`--json` has parent, depth, restack).
- `stack restack edit|continue|skip|abort` with `--step`/`--exec`; state in the git dir.
- `stack commands --json`, `stack guide`, thin `tools/skills/stack/SKILL.md`, `Next:` on every error; `help_test`
  and a unit test derive the command list from the CLI enum.
- Revision pins under `refs/stack/<prefix>/<topic>/r<N>[-base]`, `stack diff <topic> [a b]` (range-diff).
- Tests: `config_test`, `prefix_test`, `restack_test`, `diff_test`, plus additions to `upload_test`, `status_test`,
  `help_test`. Written uncompiled; waiting on main's `bazel test //tools/stack/...`.
- v2b: `upload <topic>` builds every table from all open stack PRs (gh), not just that run's.
- v2c: `//tools/stack:stack` visible to humatad and the app; `stack_linux` static builds and `static_linux_test`.
- D411 (stack_d411.py): every linked machine gets `stack` and the filtered host stack.toml; test
  `//apps/humata2/daemon/humatad:remote_stack_test`.

### v3: the shared model for PR review (2026-11-16, scratchpad stack2/)
- Status 2026-11-16: S0–W9, F1, v2, v2_docs, v2b, v2c, F2, close and D411 are live (STACK gate green in batch 18);
  only `stack_d411_docs.py`'s D411 rewrite was missing (`stack2_01_d411_doc.py`).
- `stack2_02_repo_stack.py`: `RepoStack` with `parent_of`/`children_of`, repo, trunk, source and native number, and
  `Pr.from_fork` (D412). Graph tests include A10's `a_native_stack_is_ordered_trunk_up`,
  `a_base_chain_finds_parent_and_children`, `two_children_of_one_pr_are_both_listed`,
  `a_cross_fork_or_cyclic_base_is_no_stack`. `upload.rs` sets `from_fork: false` on its four graph PRs.
- PR review's refresh (A8), reviewed commits (A9) and stack navigation (A10 daemon/UI) belong to the PR lane; STACK's
  part of A10, the graph crate, is done. The PR lane widens `//tools/stack/graph` visibility with its first dep.
- The CLI's own native-stack link (`gh stack link`) shipped in v1; nothing else is owed.
