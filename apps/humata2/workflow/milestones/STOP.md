# STOP: Stop everything on a machine, and bring it back later

- **Status:** open (2026-11-16, asked for by the owner)
- **Depends on:**
  - MACH: a machine page and the machine link must exist
  - SAFE*: a stop reuses the shutdown and bring-back paths SAFE hardens, its restart report and `humata doctor restarts`; STOP can't close until they hold
- **Unblocks:** V1
- **Layer:** L4 · **Lane:** daemon + UI

## Outcome
Agents are running wild on a machine and I want them stopped, now, as a last resort. I press Stop on that machine (or run `humata machine stop <machine>`, or `humata stop` on the machine itself). Humata does what a clean shutdown does: no agent gets another keystroke, every agent and shell is ended gracefully (and killed after a short grace period if it won't end), and everything a restart needs is saved: scrollback, conversations, todos, rewind checkpoints, open tabs. The sessions stay stopped, across reboots too, until I start them again. Starting again works exactly like coming back from a shutdown: the same bring-back, the same restart report, with each session back or named and why.

## The owner's words (2026-11-16)
"a way to 'kill' stuff we have running on a machine as in terminate gracefully and save progress so we can resume it. Say we have agents that are running wild and I just want to stop them in a last ditch way on a machine. It would be equivalent to simulating a shutdown and restarting would be like recovering from a shutdown."

## Exit criterion
- On the host and on a box: Stop ends every session's processes (graceful, then forced after the grace period) and leaves nothing running that Humata started. Nothing an ordinary restart would bring back is lost. The machine reads Stopped in the sidebar and on its page.
- No agent can type, call `humata`, or be answered while stopped. Its open Inbox requests end as "stopped by you". An agent can never stop or start a machine unless its rule allows it (agent access switch, off by default).
- A reboot while stopped keeps it stopped. Start (the button, `humata machine start`, `humata start`) runs the normal bring-back and writes a restart report naming the stop.
- Stop reaches a box whose agents flood the link, and works from the box's own terminal when the host can't reach it.
- A Stop cut at any point (fault points, power cut) still leaves everything stopped or running and nothing lost (SAFE's rules).
- Fakes: the Stop button, its confirm, the Stopped state and Start are drawn and match frames that Fable signs off.

## Read these (pointers, nothing else)
- SPEC: PER-23–PER-31 (restarts), the machine page (MP-*), agent access (AC-*). Grep `\*\*<ID>\*\*` in `apps/humata2/SPEC.md`.
- Decisions: D52, D71, D77, D86, D96, D240–D243.
- Code (under `apps/humata2/`): `daemon/revive/src/{reconcile,report}.rs`, `daemon/humatad/src/restart.rs`, `daemon/holder/`, `daemon/sessions/`, `daemon/humatad/src/machine_service*`, `daemon/access/`, `cli/src/commands/`, `features/hosts/`, `ui/src/` machine page.
- SAFE's Lane (what it is changing in restart.rs and tests/common right now).

## Out of scope
Stopping a single session (Archive and closing a tab exist). Linux VM runs (LINUX).

## Lane (written by the manager)

Manager: minion STOP (Opus 5.5), opened 2026-11-16.

### Design (picks by a Fable 5.2 high subagent, 2026-11-16; main logs them as D260–D265 Provisional, text in scratchpad `stop/decisions-D260-D265.md`)
- **D260 words, UI, CLI, frames.** "Stop all sessions…" / "Sessions stopped" / "Start sessions" (provider VM power keeps "stopped"/Start). Stop: machine page ⋯ menu and sidebar machine context menu, red, with a confirm (the in-app presence). Start: page banner, Hosts row action, context menu. CLI `humata host stop|start <machine>` (`machine` an alias of `host`), `humata stop|start` on the machine itself. Frames 72–76 in new `desktop/tests/design/stop.rs`, appended rows only.
- **D261 a stop.** One machine's own sessions. Record `<data>/stop/this.json` (humata2_durable), not a store row (a PER-31 restore can't undo it); host copy `<data>/stop/<machine>.json` + `delivered`. Stop = write record, reply, re-exec humatad2; a boot with the record skips every start-time recovery and sweeps: SIGTERM each holder (its own HUP, 2 s, KILL; scrollback synced), withdraw Inbox, wait ≤5 s, SIGKILL holder + program group, verify, write `ended`. Agents stay `Running` so Start revives them. Daemon serves the user's reads/archive/delete/Settings/Start, refuses every agent and script caller (`Refusal::MachineStopped` in `guard::judge`) and launches, resumes, attach, typing. Fault points `stop.record.written`, `stop.exec`, `stop.holders.signalled`, `stop.inbox.withdrawn`, `stop.holders.gone`, `stop.record.settled`.
- **D262 reaching a box.** Peer RPC `StopSessions{stop_id, by}` (3 s) races a one-off SSH `script` (`Purpose::StopSessions`) running `humatad2 stop --id … --by …`; on the box `humatad2 stop` asks the socket, else writes the record, SIGKILLs the daemon (service manager restarts it into the sweep) and SIGTERMs holders. Undelivered: host copy `delivered: false`, frame 76, sent at next link-up.
- **D263 who.** New `Capability::MachineSessionsStop` (NeverByDefault) for `MachineActionKind::{StopSessions, StartSessions}`; a user's stop is started only by the user; presence for both; `humata stop` on the machine itself needs no login when the caller has no agent/script credential and is outside every Humata session.
- **D264 Inbox and stopped.** `WITHDRAW_REASON_MACHINE_STOPPED = 8`; card text from the record's `by`. `SESSION_STATUS_MACHINE_STOPPED = 12`. While stopped: reads, archive, delete, rename, remove/forget allowed; new session, fork, move, resume, rebuild, typing, attach, Open a shell refused "Start sessions on <machine> first".
- **D265 Start.** Rename `this.json` → `this.starting.json`, re-exec, normal `bring_back`, report gains `after_stop`, remove the file. `LastStart.after_stop = 7`, `LastStart.sessions_stopped = 8`. A boot while stopped writes no report and sets `LastStart{started_at, sessions_stopped}`. Fault points `start.record.renamed`, `start.exec`, `start.report.written`, `start.record.removed`.
- **D269 sweep order (Opus 5.5 high subagent, 2026-11-16; supersedes D261's sweep steps).** The sweep SIGHUPs each holder's program groups itself, waits 2 s and SIGKILLs survivors (holder counted Killed); only then SIGTERMs holders, waits 2 s, SIGKILLs any left. A pid counts as a holder under either build's name or the configured one. Start waits ≤5 s on the state lock (a blocking flock on its own thread, not a retry loop); stop/start relaunch unlocks before exec. New fault point `stop.programs.hungup` (before `stop.holders.signalled`).
- **D270 streams end when the daemon leaves (manager, 2026-11-16).** A tower layer next to the services (`src/stop/streams.rs`) ends every response stream with nothing ready with UNAVAILABLE once the daemon leaves; ready replies still go. An open Attach (and Hook, Inbox wait, Uplink, Watch*) had held every stop for the full 3 s SHUTDOWN_GRACE, which now only backs a call still being answered.
- **Deviation from Fable (manager):** no holder pid-file change (`daemon/holder` is Codex C1's): the sweep snapshots each holder's child process groups (`ps -A -o pid=,ppid=,pgid=`) before signalling it.

### Code layout
- New `daemon/humatad/src/stop.rs` + `src/stop/{record,sweep,exec,service}.rs`: the record, the process sweep, the re-exec, the DaemonService handlers. `stop::Boot` read in `serve::run` after the state lock and before the store opens.
- Seams (main or the owning lane): `api/proto/humata/v1/{daemon,common,inbox,peer,hosts}.proto`; `serve.rs` (boot hook, skip recoveries while stopped, handlers); `main.rs` (`mod stop;`, `Command::Stop`); `restart.rs` (SAFE: a stopped boot sets LastStart without reconcile; `after_stop` on the report); `daemon/revive/src/report.rs` (SAFE/AWAY: optional `after_stop` in `RestartJson`); `guard.rs` (AWAY: MachineStopped gate); `inbox.rs` (AWAY: new reason); `daemon/access/src/{verdict,capability,call,policy}.rs` (Refusal::MachineStopped, the new capability); CLI registration.

### Tasks (failing test first; names are outcome sentences)
- [ ] T0 "Stop ends every process a host's sessions started, keeps everything, and Start brings them back with a report naming the stop" · Fable test writer · new `daemon/humatad/tests/stop_sessions.rs` (+ RIG_TESTS seam) · e2e on the admin daemon: stop/start, reboot while stopped, a holder that ignores HUP, agent refused, Inbox withdrawn, cuts at every stop./start. point.
- [ ] T1 "A stop record survives a power cut and reads back whole" · Haiku · `src/stop.rs`, `src/stop/record.rs` (unit tests in module)
- [ ] T2 "The sweep ends every holder and the program groups under it, killing what won't end" · Haiku · `src/stop/sweep.rs` (unit tests with real child processes)
- [ ] T3 "Stop and Start re-exec the daemon into the sweep or the bring-back" · Sonnet · `src/stop/{exec,service}.rs` + seams (proto, serve, main, restart, report, guard, inbox, access) · after T0–T2
- [ ] T4 "Stop reaches a box that floods or drops its link, and `humata stop` works on the box" · after T3 (peer.proto, machines `Purpose::StopSessions`, `humatad2 stop`, host copy, Hello field)
- [ ] T5 "An agent can never stop or start a machine unless its rule allows it" · after T3 (access capability, CLI `humata stop|start`, `host stop|start`)
- [ ] T6 "The app shows Stop, its confirm, Sessions stopped and Start, on fakes and live" · after T3 (clients model + fake, features/hosts, ui machine page/sidebar/hosts) and frames 72–76 with Fable's sign-off
- [ ] T7 "A stop or start cut by power at any point leaves everything stopped or running, nothing lost" · after SAFE T5 (power_cut rig)

### Claims
`daemon/humatad/src/stop.rs`, `daemon/humatad/src/stop/` (new), `daemon/humatad/tests/stop_sessions.rs` (new), later `daemon/humatad/tests/stop_box.rs` (new), `desktop/tests/design/stop.rs` (new), this Lane. Everything else as seams named above. Scratch: scratchpad `stop/` mirroring repo paths, bases in `stop/BASES.sha256`.

### Workers
- T0 Fable test writer · `tests/stop_sessions.rs` · done in scratch (17 tests; seam: "stop_sessions" in RIG_TESTS)
- T3 Sonnet · `src/stop/{exec,service,gate,boot}.rs` + seams · done in scratch, reviewed; batch 1 sent to main (42 anchored hunks / 17 files + 8 new)
- T1 Haiku · `src/stop.rs`, `src/stop/record.rs` · done in scratch, reviewed (needs BUILD seam: `//apps/humata2/daemon/durable` in FAULT_SWAPPED)
- T2 Haiku · `src/stop/sweep.rs` · done in scratch, reviewed; superseded by the batch 2 fix-up (`programs` = `holder_names(holder)`)
- Fix-up (manager) · `src/stop/sweep.rs`, `serve.rs` (SAFE seam), `tests/stop_sessions.rs` · batch 2: lint (`Sweep::new` removed), holder names, D269 order, blocking state-lock wait. Gate 1: 6 pass / 11 fail in stop_sessions_test, all three causes load-dependent. Green at gate_b3 (17/17, lint clean).

- Fix-up (manager) · `src/stop/streams.rs` (new), `serve.rs` (SAFE seam), deps `http-body` · batch 3: D270.
- T5 Opus test writer · access capability, local RPC check, `humata stop|start` · scratch `stop/t5/` · running
- T4 Opus test writer · `tests/stop_box.rs` (new), peer RPC + SSH script + host copy · scratch `stop/t4/` · running

- Batch 3 green (stop_sessions 11.7 s, was 28.9 s). Batch 4 = T4 + T5 + UPGRADE's asks (D281: `exec_at(path, point)` with `installed_path()` stripping Linux's " (deleted)"; the stop withdraw skips MIGRATE, kind 8). T4 and T5 are implemented; batch 4 is merged and sent (`stop/batches/batch4.py`, --check clean, uncompiled).
- Waiting on: main to gate batch 4 and log picks (a)–(f) (relay path, hedged SSH, start waits 20 s, last.json, host copies, the agent capability); T6 frames need an Opus high sign-off; T7 waits on SAFE T5.
- Ready to commit: no
