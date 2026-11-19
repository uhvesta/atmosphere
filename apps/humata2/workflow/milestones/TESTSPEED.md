# TESTSPEED

**Outcome:** a full gate of `//apps/humata2/...` runs in well under half its
current time (30–60 min). No test is weakened and none flakes.

**Why:** the owner (2026-11-18): "we should optimize slow tests". Slow
targets serialize every gate on the one bazel server.

**Exit:**
- Every target that took more than 120 s in the 2026-11-18 gates is at most
  half its old time, or its milestone file says why it can't be.
- The whole set passes three times in a row with no flake.
- No assertion is removed or loosened, and no tolerance is widened.

**Slowest targets (2026-11-18 gates, seconds):** move_session 900 (TIMEOUT),
power_cut_host 887, power_cut_both 834, rebuild_session 489,
desktop live_flows 403, power_cut_machine 379, forget_machine 307,
remove_machine_cut 305, opencode client 300, fork_remote 293, pr_review 290,
remote_archive 265, desktop design 215, pr_commits 211, delete_channel 206,
start_fresh 200, live_flows_ssh 197, desk_roam 182, holder terminal 174,
restart_operations 174, pr_diff_stress 169, move_session_freeze 168, and
everything else above 120 s. The full list is in the main agent's
scratchpad slowest.txt.

**Levers, in order:**
1. **Measure first.** Per test case, from each target's test.xml under
   bazel-testlogs. Find where the wall time goes: daemon start-up, rig
   build, polling intervals, fixed deadlines that always run out, or serial
   cases.
2. **Wait on events, not timers.** Swap any poll-with-sleep for the feed or
   a condition wait. Shrink poll intervals in the test rigs (common/mod.rs,
   remote_rig.rs, power_cut.rs). Generous deadlines stay generous; only the
   time actually spent goes down.
3. **Shard.** Split big targets with `shard_count` (the machine has 10
   cores), or split them into several test binaries in the BUILD file. Power
   cuts run once per cut point, so they shard well.
4. **Share expensive setup** where cases don't depend on each other's state,
   for example built repos and fixtures made once per binary. Cases must
   still be isolated: their own dirs, sockets and ports.
5. **Build-time knobs** in the test-only daemon (feature `faults`/`admin`):
   for example shorter internal retry and backoff delays set by test env.
   Release behaviour must not change.

**Out of scope:**
- Changing what a test checks.
- Deleting tests.
- Marking tests manual to skip them.
- Production timing changes.
- MODULE.bazel and lock files.
- git.

**Files:** `daemon/humatad/tests/**`, `daemon/humatad/BUILD.bazel`, the test
rigs, `desktop/tests/live_flows*.rs` and its BUILD entries. The
test-env-only knobs in daemon src belong to the faults/admin features, and
the lane claims them per file in WORKLOG. Other lanes (LIVEHOLDER,
STACKVIEW, DIFFVIEW) are editing some of these tests right now, so
coordinate through Claims and keep each edit small.

## Lane
(manager writes here)

### Lane (minion TESTSPEED, Opus 5.5) — 2026-11-18

**Goal / exit:** as above. Times in slow.txt were under a loaded gate; each
target is measured alone first (owner rule, CONFIG › Test tiers).

**Measurements (alone unless noted):**
- power_cut_focus (9 single cuts at once, quiet-ish machine): 30 s total; per
  case rig 7.7 s, setup 12.8 s, cut 1.3 s, restart 0.6 s, settle 5.6 s. The
  same cases inside a loaded gate: 75–120 s (rig 25–50 s). So the gate is
  CPU-bound: per-rig CPU is the lever, then fixed waits.
- power_cut_{host,both} under gate (16 rigs/target): per case rig 10–15 s,
  setup 20–31 s, cut 5–7 s, settle 8–9 s; traces 55–90 s each; a shard is its
  slowest operation (trace + its cases), 590–730 s.
- move_session alone: 22 tests in 213 s (17:38); 900 s TIMEOUT only under the gate.

**Claims:** daemon/humatad/tests/common/{power_cut,remote_rig}.rs (shared rigs,
small edits), daemon/humatad/BUILD.bazel (test sharding only),
tools/tests/slow.txt (removals only), daemon/humatad/src/peer_client.rs
(admin-only `HUMATA2_PEER_RETRY_MS`).

- Daemon stack samples (host + box, faults build, mid-suite): almost all
  threads parked (cvwait/kevent), some `__wait4` on children (git). The time
  goes to waits and subprocesses, not daemon CPU.
- power_cut_machine under gate: its restart phase averages 5.4 s, which is
  peer_client's `RETRY_FIRST = 5s`: after a box restart the host waits out its
  backoff before it redials.
- A single case runs alone with `--test_arg=<name> --test_arg=--exact`
  (`--test_filter` does not filter rules_rust tests): ~24 s for one
  remote_archive case. That is the cheap repro for a slow-target failure.

**Log:** (before → after per target, filled in as each lands)
- Peer redial: `retry_waits()` in src/peer_client.rs reads
  `HUMATA2_PEER_RETRY_MS` under `#[cfg(feature = "admin")]` only (admin and
  faults builds, never the release); remote_rig passes 100 ms to every host
  start and restart. RETRY_FIRST/RETRY_MOST and their unit test are
  unchanged. power_cut_machine alone, knob off → on: shards 155/203 s →
  162/210 s; per case restart 5.3 → 0.5 s but settle 3.1 → 7.0 s, so no net
  gain on its own. A second ~5 s wait was hidden behind the redial (below).
- Fake agents read keys themselves (`stty -icanon`, echo kept) in the shared
  rigs' scripts: common/mod.rs `agent_script`, remote_rig.rs `agent_script`
  and `CLI_AGENT`. Why: the holder types a note or prompt into a program
  that is still editing lines only after `KEYS_LIMIT` (5 s from its start,
  holder/src/client.rs). The canonical-mode `read` loop never turned
  editing off, so every revive (restart's "you were interrupted") and every
  typed first message waited out 5 s. Real agent CLIs read raw keys, so the
  fake is now closer to them. One machine cut (new focus case
  `a_move_cut_on_the_box_while_it_keeps_the_session_finishes_or_undoes`,
  20 s alone): settle 5.5 → 1.5 s; the box's restart report comes ~1 s
  (revive SETTLE) after relaunch.
- `presubmit.sh --case <name> <target>`: one case of one slow target
  (passes `--test_arg=<name> --test_arg=--exact`, keeps slow targets, and
  fails when no case had that name).
- Pre-existing failure, not this lane (it fails with the knob off too):
  move_session `a_codex_session_with_no_conversation_yet_moves_from_box_1_to_this_mac_and_starts`,
  where the start is refused with "Codex isn't installed on this machine". Reported to main.
