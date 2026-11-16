# SAFE: Nothing is lost, ever

- **Status:** open (lane SAFE, 2026-11-16)
- **Depends on:**
  - MACH: add machine and install must exist to be cut
  - FORK: moves and forks must exist to be cut
  - FILES: transfers must exist to be cut
- **Unblocks:** LINUX
- **Layer:** L4 · **Lane:** daemon

## Outcome
Power-cut the host, a machine or both at any point. On restart everything is back or named in a restart report, and `humata doctor restarts` repairs what it can.

## Exit criterion
An audit: every multi-step operation has fault points (each daemon milestone added its own); the PER-28 power-cut suite passes on macOS for host, machine and both; every restart writes a report; the restart doctor skill and `humata doctor restarts` work; the last good copy restores. VM hard resets run in LINUX.

## Read these (pointers, nothing else)
- SPEC: PER-23–PER-31, TR-7–TR-10, SL-4. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: the restart report line from FRAMES1 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D52, D71, D77, D86, D96 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `daemon/store/src/fault.rs`, `daemon/store/src/last_good.rs`, `daemon/revive/src/report.rs`, `daemon/revive/src/reconcile.rs`, `daemon/humatad/src/restart.rs`, `daemon/humatad/src/last_good.rs`, `daemon/humatad/tests/restart.rs`, `daemon/durable`
- Also: OVERVIEW.md › Known gaps › "Daemon and persistence"
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Writing each operation's fault points (the milestone that adds the operation). Real VM resets (LINUX).

## Lane (written by the manager)

Manager: minion SAFE (Opus 5.5), opened 2026-11-16.

### Audit (2026-11-16)

Targets are under `//apps/humata2/daemon/`. "Cut" = killed at the point and
restarted; "power" = also drops writes not yet fsynced (humata2_durable).

| Operation | Fault points | Test that cuts it at each point | Verdict |
|---|---|---|---|
| Store writes (put, remove, journal append/ack, blobs, atomic files) | `store.<op>.committing/committed`, `store.blob.written`, `store.file.temp_written` | `store:kill_points_test`, `store:crash_test`; power: `store:power_cut_test` (host, machine, both; store level only) | covered |
| Last good copy: save and restore (PER-31) | `store.last_good.copied`, `store.restore.copied`, `store.restore.moved` | `store:kill_points_test`, `store:last_good_test`, `humatad:restart_test` (`a_damaged_state_comes_back_…`) | cut covered; **power cut missing** |
| Agent launch and resume, shell revive | `revive.launch.*`, `revive.resume.*`, `revive.shell.spawned` | `revive:kill_matrix_test` (cut and power) | covered |
| Channel create, local session start | `sessions.channel.*`, `sessions.start.*` | `sessions:kill_matrix_test` | cut covered; power missing |
| Terminal output to the last sync | holder SIGUSR2 power cut | `holder:power_cut_test` | covered |
| Conversation mirror append | (append framing) | `mirror:durability_test` (`a_cut_off_append_is_never_read_back`) | covered |
| Transfers: copy, move, cancel, upload; trash put/restore/empty | `transfer.*`, `trash.*` | `transfers:kill_matrix_test`, `transfers:power_cut_test`, `humatad:transfers_test` | covered |
| Local fork (code, context, handover) | `fork.write_record`, `fork.copy_code:<repo>`, `fork.copy_conversation`, `fork.launch`, `fork.verify`; `sessions.fork.copied/recorded` | `humatad:fork_test` cuts `copy_code:api`, `verify` only; `sessions.fork.*` unreachable (admin daemon links non-faults crates) | **partial** |
| Own copy for a context fork | `own_copy.<event>` per step | `humatad:context_fork_test` cuts `own_copy.copy_code:api` only | **partial** |
| Fork onto another machine | `fork-started/packed/copied/unpacked/copying/recorded`, `fork-context-asked` | `humatad:fork_remote_test` cuts packed, copied, unpacked, recorded, context-asked | **partial** (started, copying) |
| Move a session | `move-started/packed/copied/unpacked/resumed/switched/released` | `humatad:move_session_test` cuts copied, resumed, switched, released | **partial** (started, packed, unpacked) |
| Archive (local) | `archive.recorded` | `humatad:archive_test` | one point; **steps after it not cut** |
| Remote archive, mirror restore, carry beside restored (D222, D224, D230) | none (journaled `Pending`) | `humatad:remote_archive_test` restarts the host cleanly, no cut | **missing** |
| Rebuild a session | `rebuild-clone-<repo>` | `humatad:rebuild_session_test` cuts `rebuild-clone-api` | **partial** |
| Start fresh | `start_fresh.launched` | `humatad:start_fresh_test` | one point; **partial** |
| Delete channel | `delete_channel.agents_stopped` | `humatad:delete_channel_test` | one point; **partial** |
| Forget machine | `forget.archived` | `humatad:forget_machine_test` | one point; **partial** |
| Review send queue (exactly once) | `comments.begun`, `comments.typed` | `humatad:comments_test` | covered |
| Machine add, install, Reconnect, removal | `hosts.json` step records; in-process restarts | `machines:fake_ssh_test` (`an_add_cut_short_by_a_host_restart_at_any_step_is_undone`, `…machine_dies_at_any_step…`, `a_removal_cut_short_finishes…`), `humatad:machines_test` | covered (simulated restart, not a process cut) |
| Installing a file on a machine | temp file + rename in the script | `machines:machines_unit_test` (`a_temporary_file_left_by_a_killed_install_is_removed_by_the_next`) | covered |
| Skills sync apply | link switch | `machines:skills_test` (`a_cut_short_apply_leaves_the_old_set_…`) | covered |
| Remote session start/stop/resume via the host | `remote_session_reports` rows | `humatad:humatad2_unit_test` (`remote_work::tests`, in-process machine restarts) | partial (no process cut) |
| Remove a machine from a channel (plan/apply) | none | `humatad:remove_machine_live_test`, `_plan_test` have no restart | **missing** |
| Takeover | none needed: redone on each connect | n/a | n/a |
| Sync rules | not built (parse only) | n/a | out of scope (its milestone) |
| Machine Inbox journal and replay | AWAY's (D225, D226) | `humatad:away_inbox_test` | AWAY lane |

| Exit item | State | Proof |
|---|---|---|
| Daemon test build can fire every point and power-cut | **missing**: `humatad2_admin` links plain store/sessions/revive/transfers/holder; humatad's five copied `stop_point`s are SIGKILL only | none |
| PER-28 power-cut suite, macOS: host alone, machine alone, both | **missing** at daemon level (store, transfers, holder, revive, durable have crate-level ones) | none |
| PER-29 every restart writes a report | written on every reconcile, host and box (`revive:kill_matrix_test`, `humatad:restart_test`); **operations finished/undone/stuck at start are not named in it** | partial |
| PER-30 restart doctor skill and `humata doctor restarts` | **missing** (no CLI command, no skill) | none |
| PER-31 last good copy restores | `store:last_good_test`, `store:kill_points_test`, `humatad:restart_test` | covered (power cut of save/restore missing, see row above) |

- Design notes (Fable 5.2 high picks, 2026-11-16; to log as D240–D243 via main):
  - **D240 fault build.** A testonly `humatad2_faults` (`admin` + `faults`, DAEMON_DEPS with store/holder/revive/sessions/transfers swapped for `*_faults`, plus `durable_faults`); `humatad2_admin` unchanged. New `daemon/humatad/src/fault.rs`: `point()` delegates to `humata2_store::fault::point`, `hold_point` moves there, the five `stop_point` copies go; under `faults` SIGUSR2 power-cuts the daemon (as the holder does). `HUMATA2_FAULT_TRACE=<file>` appends each point name reached; `HUMATA2_FAULT_ARM=<file>`: hits don't count until the file exists (in `store/src/fault.rs`, so sessions/revive/transfers points trace too). `tests/common` starts the faults build (`HUMATAD2_FAULTS`, `humata2_holder_faults`) whenever a start's env has STOP_AT, HOLD_AT, POWER_CUT_ROOTS or FAULT_TRACE; boxes too. Risk: ~48 humatad `std::fs` writes bypass humata2_durable, so a cut doesn't undo them (T6).
  - **D241 interrupted operations in the restart report.** `restart::Operations` recorder in humatad: each start-time recovery records `{id, kind, step, state}` with state `pending{waits_for}`, `finished`, `undone{reason}`, `stuck{reason}`; `pending` when its journal is read, the outcome when it settles; latest line per id wins. Appended and fsynced to `restarts/<time>/operations.jsonl` (torn last line ignored) and logged; buffered until the report folder is known. No revive/restart.json/proto change.
  - **D242 doctor.** `humata doctor restarts [report]` (newest by default) reads `restart.json` + `operations.jsonl` with its own serde types. Safe repairs through one new RPC `DaemonService.RepairRestart{session|tab|store}` (re-runs bring-back for one entry under the Starter lock; store: integrity check, healthy refused, damaged → daemon replies and exits so the next start restores). Lossy repairs listed with ids like `start-fresh:<session>`, run only with `--allow <id>`; `--explain` read-only, works daemon-down. Writes `diagnosis.md` in the report folder (`--diagnosis -` takes the skill's text). Skill `skills/restart-doctor/SKILL.md`. `DoctorError` in the CLI.
  - **D243 PER-28 suite.** `tests/power_cut_{host,machine,both}.rs` over `tests/common/power_cut.rs`, in RIG_TESTS (large), sharded. Host + one box on the faults build, POWER_CUT_ROOTS on each side's state and holders. Setup puts every PER-25 item on both sides; each operation (start, local fork, remote fork, move, remote archive+restore, comment send, transfer) runs once traced, then once per first armed hit of each traced name. Daemons die before holders; waits on exits, no sleeps. Check: an Inventory through the public API: untouched items equal, touched items before-or-finished, anything else named in `restart.json` or `operations.jsonl`; comments typed once; terminal output a gap-free prefix.
- Claims: `daemon/humatad/src/fault.rs` (new), `daemon/humatad/src/session_service/{fork,own_copy,fork_remote,fork_remote/*,move_session,move_recover,archive,archive_remote,archive_remote/*,rebuild,start_fresh}.rs`, `daemon/humatad/src/channel_service/delete.rs`, `daemon/humatad/src/machine_service/forget.rs`, `daemon/humatad/src/review/comments.rs` (stop_point only), `daemon/humatad/src/restart.rs`, new `daemon/humatad/src/restart/operations.rs`, new `daemon/humatad/src/doctor.rs`, `daemon/store/src/fault.rs`, tests `daemon/humatad/tests/{fork,context_fork,fork_remote,move_session,archive,remote_archive,rebuild_session,start_fresh,delete_channel,forget_machine,restart,faults,doctor,power_cut_*}.rs`, new `daemon/humatad/tests/common/power_cut.rs`, `daemon/store/tests/power_cut.rs`, `daemon/sessions/tests/kill_matrix.rs`, new `cli/src/commands/doctor.rs`, new `cli/tests/doctor.rs`, new `skills/restart-doctor/`, this Lane. All edits in scratch `scratchpad/safe/` mirroring repo paths. Seams to main: `daemon/humatad/BUILD.bazel`, `daemon/humatad/src/main.rs` (`mod fault; mod doctor;`, SIGUSR2), `daemon/humatad/tests/common/mod.rs` (binary pick), `daemon/{store,durable,holder,revive,sessions,transfers}/BUILD.bazel` (visibility of `*_faults`), `api/proto/humata/v1/daemon.proto` (RepairRestart), `daemon/humatad/src/serve.rs`, CLI registration (`cli/src/args.rs`, `commands/mod.rs`).
- Tasks (each a failing test first; names are outcome sentences):
  - [x] T8 "A power cut while the last good copy is saved or restored leaves a store that opens whole" · Haiku · `store/tests/power_cut.rs` · written, checked; gate `//apps/humata2/daemon/store:power_cut_test`
  - [ ] T1 "A daemon test can cut, trace and power-cut at every point, its own and its crates'" · Sonnet · `humatad/src/fault.rs`, the five stop_point files, `store/src/fault.rs`, `humatad/tests/faults.rs`, BUILD/main/common seams · running
  - [ ] T2a "A local fork and an own copy cut at any step are undone or finished at restart" · Haiku · `humatad/tests/{fork,context_fork}.rs` · running
  - [ ] T2b "A move and a fork onto a machine cut at any phase are undone or finished at restart" · Haiku · `humatad/tests/{move_session,fork_remote}.rs` · running
  - [ ] T2c "A remote archive and a mirror restore cut at any step finish once the box links" · Sonnet · `archive_remote*`, `tests/remote_archive.rs` · after T1
  - [ ] T2d "An archive, rebuild, start fresh, channel delete and forget cut at any step finish or are undone" · Sonnet · their src + tests · after T1
  - [ ] T3 "Each operation found mid-way at a start is named in the restart report with how it ended" · Sonnet · `restart.rs`, new `restart/operations.rs`, move/fork recoveries, `tests/restart.rs` · running
  - [ ] T3b same, for archives, deletes, forgets and comment sends · after T2c/T2d
  - [ ] T4 "`humata doctor restarts` and the restart doctor skill repair what can't lose anything and write a diagnosis" · Sonnet · `humatad/src/doctor.rs`, `cli/src/commands/doctor.rs`, `cli/tests/doctor.rs`, `skills/restart-doctor/`, proto/serve/CLI seams · running
  - [ ] T5 "Power-cutting the host, a machine or both at any traced point loses nothing PER-25 lists, or names it" · Sonnet · `tests/power_cut_*.rs`, `tests/common/power_cut.rs` · after T1, T3
  - [ ] T6 "Every humatad write PER-25 relies on goes through humata2_durable" · audit (Haiku) then fixes · after T5's first run
  - [ ] T7 "A session start or channel create cut by power at any point leaves a running session or none" · Haiku · `sessions/tests/kill_matrix.rs` · running
  - [ ] T9 "Removing a machine from a channel cut at any step finishes or is undone" · later (needs points in `channel_service/remove_machine*`; ask main for the claim)
- Workers: T8 Haiku done · T1 Sonnet running · T2a Haiku running · T2b Haiku running · T3 Sonnet running · T4 Sonnet running · T7 Haiku running (2026-11-16).
- Waiting on: main's gate for T8; main to log D240–D243 Provisional and add the SAFE claims row.
- Build gates:
- Ready to commit: no
