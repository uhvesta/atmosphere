# FAST: An optional fast mode makes Humata measurably lighter and quicker

- **Status:** blocked
- **Depends on:** RELEASE2 (VERIFY): profile and tune only once behaviour is settled and covered by tests
- **Unblocks:** RELEASE3
- **Layer:** L10 · **Lane:** perf

## Outcome
The owner can turn on a fast mode in Settings. With it on, Humata uses less
memory and CPU time and stutters less. With it off, Humata behaves as before.

The owner's direction: performance matters. Memory, stutters (main-thread
hitches, extra redraws) and CPU time are costly in every change, UI and
non-UI.

## Exit criterion
- Before-and-after benchmark numbers are recorded for every change.
- The whole test suite passes with fast mode on and with it off. The gate
  runs twice.
- Nothing regresses with fast mode off.

## Scope
Profile everything for memory and CPU time:
- `humatad`, the daemon;
- the terminal holder;
- the gRPC and stream traffic between daemon, holder, app and CLIs;
- the desktop app: frame time, idle CPU and memory;
- the CLIs;
- remote machine links.

Rules:
- Measure only with Bazel bench targets. Never run `cargo`.
- Existing benches to start from: `//apps/humata2/chunk:codecs_bench`,
  `//apps/humata2/daemon/humatad:peer_link_latency_bench`,
  `//apps/humata2/daemon/review_diff:big_repo_bench`, and the terminal and
  engine frame and stream benches (`terminal/view`, `terminal/pane`,
  `engine`). Add a bench target for anything not yet covered.
- Fast mode is opt-in, as a setting. It applies what profiling finds.
  A change that is a plain win with no trade-off can apply always; say so
  in its decision entry.

## Read these (pointers, nothing else)
- Code: the `BUILD.bazel` files above for the bench targets; `TESTING.md`.
- Decisions: log each setting and trade-off in DECISIONS.md.
- Settings: the portable config rules (compiled-in defaults plus
  `~/.humata2/config/*.toml`).

## Out of scope
- Behaviour and usability fixes: VERIFY.
- Linux and browser hosts: cut (D485).

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
