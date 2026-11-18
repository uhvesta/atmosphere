# FAST: An optional fast mode makes Humata measurably lighter and quicker

- **Status:** done (in 0.2.5, d716424c, 2026-11-18)
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
- Design notes: profiled with the existing benches (before: `/private/tmp/.../scratchpad/fast/before/`). The daemon and holder are well tuned already (adaptive LZ4/Zstd, batched frames) and are other lanes' files today; the clear, low-risk waste was idle redraws in the desktop app: a focused terminal with a blinking cursor draws a whole frame every 600 ms, even with Humata in the background. Two changes: fast mode turns the blink off (D787, a trade-off, behind the setting); a window in the background never blinks (D788, a plain win, always on). The fast-mode setting is an app preference with a compiled-in default (off) and an `HUMATA2_FAST_MODE` override for test runs; moving it into `~/.humata2/config/settings.toml` is left open in D787.
- Claims: daemon/holder/src/scrollback.rs, daemon/holder/src/saved.rs (`rewrite`), daemon/holder/benches/cost.rs, daemon/holder/BUILD.bazel (cost_bench), daemon/humatad/benches/peer_link_latency.rs, features/settings/src/lib.rs, features/settings/tests/fast_mode.rs, ui/src/settings.rs (General section only), ui/src/terminals.rs (blink wiring), terminal/view/src/view.rs (`render`), terminal/view/tests/view.rs, terminal/view/benches/frame.rs.
- Tasks:
  - [x] T1 Fast mode setting (Preferences, Row, toggle, env default) · manager · features/settings, ui/src/settings.rs · done
  - [x] T2 Fast mode turns terminal cursor blink off · manager · ui/src/terminals.rs · done
  - [x] T3 No blink in a background window (always) · manager · terminal/view · done
  - [x] T4 Idle-minute bench case · manager · terminal/view/benches/frame.rs · done
  - [x] T5 Holder memory within its scrollback (D789) · manager · daemon/holder/src/scrollback.rs, saved.rs · done
  - [x] T6 Holder cost bench + idle-daemon line in peer link bench · manager · daemon/holder/benches/cost.rs, daemon/humatad/benches/peer_link_latency.rs · done
- Before/after (`bazel run -c opt //apps/humata2/terminal/view:frame_bench`, 200x60, one idle minute of the test clock, focused terminal, program asked for a blinking cursor):

  | Case | Before | After |
  |---|---|---|
  | Window active, fast mode off | 100 frames, 186.7 ms drawing / min | 100 frames, 186.4 ms / min (unchanged by design) |
  | Window active, fast mode on (D787) | 100 frames, 186.7 ms / min | 0 frames, 0 ms |
  | Window in the background (D788) | 100 frames, 186.7 ms / min (old code ignored activation) | 0 frames, 0 ms |
  | Idle frame, no row changed | median 1.80 ms | median 1.80 ms |
  | One row changed | median 1.89 ms | median 1.88 ms |
  | Every row changed | median 2.39 ms | median 2.46 ms (noise) |
  | pane stream, 200x60 idle (`terminal/pane:stream_bench`) | p50 0.478 ms | p50 0.468 ms |
  | pane stream, agent 300 B/frame | p50 2.066 ms | p50 2.071 ms |
  | Holder after 24 MiB output (3x 8 MiB scrollback), resident (`daemon/holder:cost_bench`, D789) | 42.1 MiB (fastbuild) | 19.2 MiB fastbuild, 18.3 MiB -c opt |
  | Holder, nothing printed, resident / idle CPU 10 s | 7.7 MiB / 0 ms | 7.7 MiB / 0 ms |
  | Scrollback buffer held at an 8 MiB limit (`holder_unit_test`) | 16,777,216 B | <= 8,388,608 B |
  | humatad2 idle on a quiet peer link (`peer_link_latency_bench`) | 11.9 MiB, 0 ms CPU over 10 s (-c opt) | (unchanged code) |
  | peer link WhoAmI round trip | p50 45 µs, p90 54, p99 80, max 147 (-c opt) | (unchanged code) |

- After numbers: 05:30, main's run, `scratchpad/fast/after/`. Per-frame cost is unchanged; the win is frames not drawn.
- Build gates: 05:19 pass run 186 (features/settings fast_mode, hidden_terminals, option_as_alt; palette search; app every_action; ui_unit; terminal/view view). 05:26 pass run 188 (terminal/view view, terminal_view, both engines; terminal/pane inband, links, pane, panes, stream, tabs, unfitted, both engines; ui_unit). In run 188, desktop design_test (118-machine-running-menu has no golden) and native_test (59-machine-page, session-menu-stopped-dark) failed on the D784 lane's hosts/session-menu changes, not on FAST's. Fast-mode-on run (`--test_env=HUMATA2_FAST_MODE=1`) is with main.
- 05:42 run 193 red as expected (scrollback tests); run 194 pass holder:all, cost_bench.
- 05:45 run 195 pass humatad sessions, adopt_holder, holder_swap_identity; revive:all; sessions:all. 05:48 main's -c opt cost_bench and peer_link_latency_bench passed.
- Ready to commit: yes, FAST's files only (see Claims)
