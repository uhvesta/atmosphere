# dag-walk config: Humata 2

Settings the `dag-walk` skill (`tools/skills/dag-walk/`) reads for this
project.

- **Checkout:** `/Users/avestabarzegar/oss/atmosphere-khodr` (one worktree,
  one bazel server; no parallel worktrees).
- **Build and test:** only the main agent, only through Bazel:
  `export PATH=$PWD/.tools/bin:$PATH`, then `bazel build <targets>` and
  `bazel test <targets>`, in the background. Design goldens:
  `bazel run //apps/humata2/desktop:design_test_record`. Never cargo, never
  `bazel clean` or `shutdown`. Repin with `CARGO_BAZEL_REPIN=1 bazel mod tidy`.
- **Commit:** explicit paths only, never `git commit -a`, never bare
  `git stash`. Shared docs with others' hunks: stage via `git hash-object -w`
  + `git update-index --cacheinfo`. Message "Humata 2 (minion <lane>): …",
  no AI attribution trailers. Same commit updates OVERVIEW.md ticks and adds
  a GAPS_REVIEW.md entry "(this commit)"; then a hash-fill commit.
- **Push:** `GH_TOKEN=$(gh auth token --user uhvesta) git -c credential.helper= -c 'credential.helper=!gh auth git-credential' push -q origin HEAD:main`;
  verify with `git ls-remote origin main`.
- **Never touch:** `apps/humata2/prototype/`, `apps/humata2/ui-prototype/`,
  `.claude/`, the design goldens without the owner's OK, other agents'
  uncommitted work.
- **Seam files (main agent only):** `features/app/src/lib.rs` (AppAction,
  AppState), `clients/src/lib.rs` and the model module, `ui/src/lib.rs`,
  `ui/src/shell.rs`, `desktop/BUILD.bazel`, the daemon's `serve.rs`, shared
  protos, `MODULE.bazel` and the lock files. Managers ask for exact lines.
- **Architecture:** TCA split: reducers and state in `features/`, views in
  `ui/`, clients behind traits with fakes in `clients/` (real client in
  `clients/live` with a fake-vs-real parity suite). Code rules: AGENTS.md
  (thiserror enums with `#[source]`, actionable messages, minimal visibility,
  behavioural tests), no force unwraps, never name inspirations.
- **Approved designs:** goldens in `apps/humata2/desktop/tests/__snapshots__/design/`;
  only with the owner's OK.
- **Screen:** headless only; no windows on the owner's screen.
- **Lane limit:** 7 managers (2026-11-15, owner wants more parallelism):
  dormant lanes don't count; at most 4 edit `daemon/humatad`; drop to 6 if a
  batched gate fails twice with unclear attribution. Early lanes may open on
  a blocked milestone for the part its open deps don't block; they close only
  when the hard deps are done.
- **Keep the tree building:** with many lanes in one checkout, a half-written
  edit to a crate others depend on (clients, features/*, ui, daemon/humatad)
  breaks every lane's gate. Workers write new code in new files that nothing
  includes yet, or keep each edit compiling; wiring lines come last, as seams.
- **Main-only seam files:** `daemon/store/src/{schema,lib}.rs` (main assigns
  migration numbers), `not_built.rs`, `daemon/mcp/src/tools.rs`, CLI command
  registration, `clients/src/model.rs`; `daemon/humatad/tests/common/mod.rs`
  is additive only. Managers write seam lines for main to apply.
- **Models:** managers Opus 5.5 or Fable 5.2 (owner OK'd heavy Fable use); workers Sonnet 5.5, medium effort; reviewers
  and judges Fable.
- **Docs:** SPEC.md (rules by ID), OVERVIEW.md (living work list),
  DECISIONS.md (D-entries), GAPS_REVIEW.md (one entry per commit), goldens in
  `desktop/tests/__snapshots__/design/`.
