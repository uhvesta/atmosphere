# dag-walk config: Humata 2

Settings the `dag-walk` skill (`tools/skills/dag-walk/`) reads for this
project.

- **Checkout:** `/Users/avestabarzegar/oss/atmosphere-khodr` (one worktree,
  one bazel server; no parallel worktrees).
- **TDD loop (owner, 2026-11-15: "work faster ... TDD driven fixes"):** a
  smart agent (manager, or a Fable/Opus test writer) first writes a focused
  e2e or boundary test that fails for the right reason and can't flake: no
  sleeps or wall-clock asserts, wait on events or conditions with generous
  deadlines, deterministic fakes and seeds, its own temp dirs and ports or
  sockets, nothing shared between tests. Then a worker implements
  until that one target passes, running only it. Fable or Opus reviews the
  diff and the findings before it is handed to main.
- **Focused tests by any agent:** managers and workers may run
  `bazel build`/`bazel test` on their own focused targets in this checkout
  (same server; commands queue on its lock). Never `//...` or a whole app
  tree, never `clean`, `shutdown`, `--output_base` or a new worktree, never
  git. Main still runs the batched gates and owns git.
- **Managers apply their own seams (2026-11-15):** a manager may run its
  own `--check`-ed seam script on shared seam files, then build its focused
  targets at once and fix or revert anything it broke. It logs every applied
  script in WORKLOG › Event log. Never Codex's files (C1_HANDOFF.md lists
  them), never MODULE.bazel or the lock files (main), never git.
- **Don't break the shared tree:** after a live edit to a crate other lanes build, build its dependents (at least `//apps/humata2/daemon/humatad:humatad2` and `//apps/humata2/desktop:humata2`) before moving on; revert at once if they break.
- **Build and test (batched gates):** the main agent, only through Bazel:
  `export PATH=$PWD/.tools/bin:$PATH`, then `bazel build <targets>` and
  `bazel test <targets>`, in the background. Design goldens:
  `bazel run //apps/humata2/desktop:design_test_record`. Never cargo, never
  `bazel clean` or `shutdown`. Repin with `CARGO_BAZEL_REPIN=1 bazel mod tidy`.
- **Gate target set (2026-11-17):** run `//apps/humata2/...` with `--test_tag_filters=-manual`, and never list `manual` targets explicitly.
  - **Terminal live targets:** these are `manual` and/or `external`. Run them in a separate batch with `--strategy=TestRunner=local` and these `--test_env` values:
    - `TERMINAL_LAB_SSH_CONFIG=~/.lima/h2-terminal-c1/ssh.config`, `TERMINAL_LAB_SSH_HOST=lima-h2-terminal-c1`, `TERMINAL_LAB_SSH_HOLDER=/tmp/h2-terminal-holder`
    - `TERMINAL_CODEX_BIN`, `TERMINAL_CLAUDE_BIN`, `TERMINAL_COPILOT_BIN`
    - `TERMINAL_LAB_NODE`, `TERMINAL_LAB_NVIM`, `TERMINAL_LAB_REPO`, `TERMINAL_LAB_ZSH_HOME`
    - `TERMINAL_VM_LINUX_HOLDER` (an aarch64 ELF holder)
    - `TERMINAL_VM_IMAGE`, the cached Lima image under ~/Library/Caches/lima/download, because HOME under Bazel isn't the owner's.
  - **Linux holder:** `--config=linux_arm64` writes it to the same bazel-out path as the Mac holder. Copy it out right after building and check it with `file`.
  - **Not run:** demo:milestone_test, linux_browser_test and demo:fixture_runtime_test, which are cut scope (D485).
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
- **Lane limit:** 10 managers (raised 2026-11-15, owner: not fast enough); was 7 (2026-11-15, owner wants more parallelism):
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
- **Models:** managers Opus 5.5 or Fable 5.2 (owner OK'd heavy Fable use); workers Haiku 5.5, high effort, for well-scoped tractable tasks (named files, a failing target to make pass, a clear brief), which is the default; Sonnet 5.5 medium only for work too open-ended to scope that tightly (owner, 2026-11-15); reviewers
  and judges Fable.
- **Docs:** SPEC.md (rules by ID), OVERVIEW.md (living work list),
  DECISIONS.md (D-entries), GAPS_REVIEW.md (one entry per commit), goldens in
  `desktop/tests/__snapshots__/design/`.
