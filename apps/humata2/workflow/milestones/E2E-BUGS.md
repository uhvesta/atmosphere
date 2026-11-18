# E2E-BUGS: Every step the cases mark as a likely bug works

- **Status:** in progress (lane E2E-BUGS)
- **Depends on:** BUGS, which fixed most of the code these rows name; TERM, for the terminal rows it fixed
- **Unblocks:** none

## Outcome
Every step the cases mark as a likely bug works: each "Likely bug" row in
`flows/cases/{micro,macro,agents}/*.md` is fixed with a test, or retagged
honestly with the reason it is not a bug.

## Exit criterion
`flows/cases/README.md` counts 0 Likely bug rows. Each retagged row names the
test that proves it: a real-daemon or live-client test wherever the step
crosses the daemon, else a reducer, model or headless test (Fakes only).

## Read these (pointers, nothing else)
- Cases: the summary rows tagged "Likely bug" in `flows/cases/*/*.md`; the
  "Likely bugs" list in `flows/cases/README.md`
- Milestones: `workflow/milestones/BUGS.md` (what it fixed), `TERM.md`
- Decisions: D520 to D529 (this lane)

## Out of scope
Rows squarely another lane's are reported to main, not fixed: DOCV (doc
versions, Desk folders), NOPATH (paths and roots), REPOS (the repo library),
TERM (terminal), SAFE (power cuts, journals), STOP and UPGRADE, VIEWS and PR,
WIRE-A (live test waits; Fakes only rows). The goldens and `prototype/` are
not touched.

## Lane (written by the manager)
- Design notes: most rows were already fixed in code by BUGS and TERM; this
  lane confirms each, adds the live proof where the step crosses the daemon,
  fixes what is still broken, and retags. Apply scripts edit by anchor only,
  so a row another lane changed is skipped with a warning, not overwritten.
- Claims: `flows/cases/**` (status lines of Likely bug rows only),
  `flows/cases/README.md` Counts (SEAM), `daemon/humatad/src/session_service/start_fresh.rs`
  (digest), new tests in `daemon/humatad/tests/fork.rs`,
  `desktop/tests/live_flows/lifecycle.rs`, `desktop/tests/live_flows/find.rs`,
  `clients/live/tests/github_repos.rs` (SEAM with REPOS), new `daemon/humatad/tests/cli_waiting.rs`,
  new tests in `daemon/humatad/tests/cli_files.rs`, new `daemon/humatad/tests/cli_actor.rs`
- Tasks:
  - [x] e2e_1 sessions, channels, Inbox, moves (10 rows)
  - [x] e2e_2 palette, review comments, upgrade refusals, panels (9 rows)
  - [x] e2e_3 CLI waits, queue, tokens, boards, away login (14 rows)
  - [x] e2e_4 SPEC SE-21: OpenCode's turn end is hook-backed (D524)
  - [x] e2e_5 agents-14 on a real daemon: cli_waiting.rs (6 tests), cli_files.rs (2 tests), 15 rows
  - [x] e2e_6 tests: cli_actor.rs (3 tests); retags in e2e_6r after its gate
- Waiting on: main's bazel runs of each batch's targets
- Build gates:
- Ready to commit: no
