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
- **Rolling batches (owner, 2026-11-18: "speed up our iterations ... in parallel and in batches"):**
  - Every agent builds and tests its own focused targets before it reports (allowed above). A batch that doesn't compile never reaches main.
  - Main runs a rolling gate instead of waiting for every lane. Whenever one or more lanes report, it first runs one compile-only `bazel build //apps/humata2/...` (minutes), then one `bazel test` over just those lanes' targets and their direct dependents.
  - Commit and push each lane's green paths straight after its gate. Don't wait for other lanes. Cut a point release once the tracked tree is clean.
  - The full `//apps/humata2/...` test set and the slow tier run once, before a release.
  - Golden re-records are batched: re-record every changed frame in one run, then send them to Fable in one judging round.
- **Test tiers (owner, 2026-11-18: "cheap presubmit checks ... then the full post submit checks ... scoped tests until right before release"):**
  - **Presubmit** (`apps/humata2/tools/tests/presubmit.sh [targets…]`): the default for every agent and every train. It first compiles what it will test, then runs the given targets, or with no arguments the tests affected by the changed files. It skips everything in `tools/tests/slow.txt`.
  - **Scoped slow:** `--with-slow <target>`, or (one case, ~10× faster) `bazel test <target> --test_arg=<case_name> --test_arg=--exact` (rules_rust ignores `--test_filter`). Only for the slow target a fix is about, filtered to its cases where possible.
  - **Postsubmit** (`presubmit.sh --full`): every non-manual test plus the slow tier. Main runs it once, right before a release, then fixes everything it finds.
  - **Train:** when two or more lanes report, main runs one presubmit over the union of their targets. Failures are split per lane by Claims, and each green lane is committed at once.
  - **A slow test failure becomes a small test** (owner, 2026-11-18: "write a small short test targeting how it failed and iterate on that"):
    1. Read the slow failure once and name the exact failure mode (the step, the cut point, the state left behind).
    2. Write a short, fast test, about 10 s and outside slow.txt, that reproduces that one failure mode and fails for the same reason.
    3. Iterate on the fix with only that small test (and its presubmit neighbours) until it passes.
    4. Keep the small test. It becomes presubmit coverage.
    5. Rerun the slow target only after all presubmit work is green, batched with the other slow reruns in the full run before release.
  - `slow.txt` holds the targets over 60 s. TESTSPEED takes a target off once it is fast. A new test that takes more than 60 s goes on the list.
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
