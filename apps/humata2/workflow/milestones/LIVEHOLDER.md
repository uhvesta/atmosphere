# LIVEHOLDER: My terminals keep running: a version bump, a rename or a move never ends them

- **Status:** open (owner, 2026-11-18: "when doing a version bump is there a way to gracefully transfer terminal sessions over so they don't end and restart? Add that to the roadmap")
- **Depends on:** RELEASE3 (the released holder and the "Migrate the holders" item, D373).
- **Owner rule, 2026-11-18:** "Instead of killing terminals. As much as possible I want terminals to keep running basically." Humata ends a running terminal only when the owner ends it (End, End for good, Stop) or when it can't be kept, and then it says why.
- **Unblocks:** a quieter upgrade path. Migrate the holders stops being a restart.
- **Layer:** L13 · **Lane:** daemon/holder + daemon/humatad upgrade + revive

## Today
- **A daemon upgrade or crash leaves holders untouched** (PER-5).
- **A new holder build restarts agent sessions.** "Migrate the holders" (D373) stops each idle agent session and resumes its conversation on the new holder. The program restarts, and anything not saved in the conversation is lost: running tool calls, shell state, environment.
- **Shells stay on the old holder**, without the newer frames (D753), until they are reopened.

## Outcome
After an upgrade, Migrate the holders moves each running terminal, agent or shell, onto the new holder build in place:
- the program keeps its pid and never sees a hangup;
- the screen and scrollback carry over;
- every viewer reconnects by itself.

Nothing is restarted and nothing is resumed. Only a holder from before this milestone, which can't hand itself over, still needs today's restart, one last time.

## Design to settle in the lane (record as a decision)
- **Re-exec in place (preferred).**
  - The holder writes its state to a handoff file or memfd:
    - the terminal emulator state, scrollback and size;
    - the save log position;
    - its socket and the PTY master as kept file descriptors;
    - the child pid.
  - It then `execve`s the new `humata2_holder` with `--take-over <state>`, keeping those descriptors open across the exec.
  - The pid stays the same, so the agent or shell is still its child: `waitpid` and exit status keep working, and nothing reparents.
- **Rejected alternative: a new holder process taking the descriptors over `SCM_RIGHTS`.** The child would be reparented to launchd, losing `waitpid` and the exit code. Note why in the decision.
- **The handoff format is versioned.** The new build must read every handoff version it may receive. If it can't, it refuses before the exec and the old holder keeps running: the force-upgrade rule applies, with no mixed-version running.
- **Failure safety.** If the new binary fails to start, an exec failure returns to the old code, which keeps serving. A crash after the exec is the case to test: the state must still be reattachable or the session reported, never silently lost.
- **Viewers and the daemon** see a short disconnect and reattach by the session's socket, the same path as a daemon restart.
- **Remote machines** follow the same path. The host's Migrate sends `Upgrade{thing = HOLDERS}` (D378), and the box re-execs its own holders.

## The audit: every path that ends a holder today
Each path below either keeps the terminal running or records why it can't, as a decision. The default is to keep it running.

- **On the same machine, the process survives a rename.** A same-filesystem `rename(2)` keeps every open file and the working directory valid. A folder rename or move on one machine should rename under the running program and update the record, without restarting it.
- **Paths to check:**
  - `channel_service/rename_folder.rs`: a channel folder rename. Keep the terminal running.
  - `desk_service/move_to_channel.rs`: a Desk session moved into a channel on the same machine. Keep it running.
  - `session_service/move_session.rs`, same machine: a relocation (D757 moves carry paths). Keep it running, and stop only when the move crosses machines.
  - `session_service/archive.rs`: Archive. Decide whether Archive should leave a live terminal running (hidden, still accounted for) instead of ending it.
  - `desk_service/close.rs`: closing a Desk tab. Closing a view must not end the program unless End was chosen.
  - `channel_service/delete.rs`: deleting a channel with live terminals. Ask, and offer keeping them running elsewhere (on the Desk).
  - `carry_service.rs`, `away/uplink.rs`, `fault.rs`, `terminal.rs`, `running_holders.rs`, `stop/`: list what ends a holder in each, keep it running where possible, and make sure no automatic path ends one (D784: sessions are accountable and never auto-killed).
- **Across machines, the program can't move.** A cross-machine move still stops and resumes, but LIVEHOLDER should keep the old terminal running until the new one is confirmed up.
- **Write a test per path:** the program's pid is unchanged after the action.

## Exit
- Tests: a shell and a stand-in agent run through a holder re-exec onto a newer build:
  - same pid, no SIGHUP;
  - screen and scrollback identical;
  - typed input after the re-exec reaches the program;
  - the exit code is still reported when the program exits.
- Tests: a failed exec, a refused handoff version, and a crash right after the exec. Each leaves the session running or clearly reported.
- "Migrate the holders" wording changes from "restart" to "move", and lists only holders too old to move.
- Slow tier: a live SSH session on h2-terminal-c1 survives a holder bump there.

## Pointers
D373 (Migrate the holders), D378 (box holders), D753 (old holder frames), PER-5/PER-6, `daemon/holder/`, `daemon/humatad/src/upgrade/holders.rs`, revive.

## Out of scope
Moving terminals between machines (that's a move, D757). Keeping a session alive across a reboot: the program can't survive a reboot, and the restart reports cover it.

## Lane

- **Manager:** Opus 5.5 (opened 2026-11-18 10:50, plan only until main's "go").
- **State:** batch 1 (holder) and T5 written (see the Workers log, 'Where it stops at 12:00'); T1–T4 with one worker, now done. Batch 2 (T5–T14) not started. `holders.rs` and revive resume are free now (AGENTKINDS is done). T8's `sessions/src/agents.rs` change is a seam to ask AGENTS2 for.
- **Goal:** a holder upgrade re-execs in place (same pid, child keeps running, screen and scrollback carry over, viewers reattach); every path that ends a holder today keeps the terminal running or records why it can't.
- **Exit:** the Exit list above, proved by the failing tests listed below.

### Design: re-exec in place

**Trigger frame.** New client frame `MOVE_TO = 8 {holder: path}`; replies `MOVING = 110` (sent before the exec) or `REFUSED` (why, nothing changed). Added without a protocol version bump, like keyed input: a pre-LIVEHOLDER holder closes the connection on the unknown tag, and the client turns that into `HolderError::TooOldToMove` (after confirming the holder still answers, as `older_holder` does). Client API: `humata2_holder::move_to(socket, holder) -> Result<Moved, HolderError>`:
- sends `MoveTo`, reads `Moving`;
- then reads on: EOF means the exec happened (the connection was CLOEXEC); `Refused` means the exec failed and the old image still serves;
- then attaches `from = u64::MAX` within a bound: answering = moved; refused connection = `HolderError::LostInMove` (crash after exec).

**What the old image does on `MoveTo`** (`server.rs`, new `handoff.rs` + `take_over.rs`):
1. Checks the path: absolute, a regular executable file. Asks it `<new> --handoff-versions` (child process, 10 s bound) and picks the highest version both read. None in common → `Refused` ("this holder writes handoff versions X; <new> reads Y; it is from an older or newer build than this holder; install one Humata release and click Migrate again"), nothing changed.
2. **Quiesces, in this order, holding each guard until the exec:** the print gate (the reader thread takes it between `poll` and `push`, so no read chunk is in flight), the reap gate (see below), the typing writer mutex (no half-written input), the keys mutex, then the state lock.
3. Syncs the save log (`sync_saved`), so the file is good through `kept.end()`.
4. Writes `<socket>.handoff` (0600, create-new after removing a stale one; through `humata2_durable` so it is whole or absent).
5. Runs `<new> --check-take-over <handoff>` (10 s bound): the new build parses the whole file and checks the version, then exits 0 having touched nothing. Non-zero exit, a signal or a timeout → guards dropped, handoff removed, `Refused` with the check's stderr tail. This catches a missing, unsigned, wrong-arch or broken binary before anything is lost.
6. Clears `FD_CLOEXEC` on the listener and the PTY master only. Every other fd is already CLOEXEC (std, tokio and mio set it), including client connections, which is how viewers learn of the move.
7. Sends `Moving`, then `execve(new, ["humata2_holder", "--socket", sock, "--take-over", handoff], environ)`. `--socket` stays in argv so `find_holder`'s `ps` fallback still finds it. The environment carries over unchanged (the agent's env tokens live there).
8. **Exec failed:** sets CLOEXEC back, removes the handoff, drops the guards, replies `Refused` ("couldn't start <new>: errno; the terminal is still on its old holder"), and keeps serving.

**The reap gate.** Today a thread blocks in `child.wait()`. An exec while it reaps could lose the exit code. New scheme, the same in the first and every later image:
- The waiter calls `waitid(P_PID, pid, WEXITED | WNOWAIT)`: it waits without reaping.
- It then takes the reap gate, reaps with `waitpid(pid, WNOHANG)`, and stores the code in state.
- The handoff holds the gate. So either the code is already in state (it goes in the handoff), or the child is unreaped and still ours: a zombie, if it exited during the exec, which the new image's waiter reaps. Same pid and same parent, so `waitpid` and the exit code keep working.
- Needs `rustix` `process` waitid with `NOWAIT`. If the pinned features lack it, that's a seam for main (`MODULE.bazel`).

**The new image (`main.rs --take-over <path>`):**
- Prints nothing on stdout: the spawner's pipe is gone.
- Reads and validates the handoff and adopts the fds: `UnixListener::from_std` (nonblocking), and `OwnedFd` for the master.
- Rebuilds `Shared`:
  - from the handoff: the scrollback runs, end and limit; cols and rows; the exit code; the same `instance`; the keys; `started` = now − elapsed; `group` = child pid;
  - the save log reopened for append by path, with its `len`, `reserved` and `ahead`.
- Starts the reader thread on a dup of the master, plus the waiter, unless the exit code is already known.
- Removes the handoff once accepting, then runs the normal accept loop. Nothing is printed into the terminal, so the program never sees it moved.
- `pid` file: unchanged (same pid).

**Handoff format, v1** (new `holder/src/handoff.rs`, same hand-rolled big-endian style as `protocol.rs`/`saved.rs`, no new crates):
- `b"H2HANDOFF"`, u32 version, u64 body length, body, u64 checksum (the save log's checksum fn).
- Body fields, in order:
  - socket path;
  - listener fd and master fd (i32);
  - child pid (u32);
  - exit code (`Option<i32>`);
  - cols and rows (u16);
  - scrollback limit and end (u64), then a u32 count of runs, each `(start u64, len u64, bytes)`;
  - save log: `Option{path, len, reserved, ahead, failed}`;
  - instance (u64);
  - program elapsed time in ms (u64);
  - keys: u32 count of strings;
  - program path, args and cwd (for reports only).
- Length-prefixed bytes throughout. Decoding fails closed: an unknown version, a bad checksum, a truncated body or an fd that isn't open are each an `HandoffError` variant with the file named.

**Versioning rule:**
- Every build lists the versions it reads (`--handoff-versions`) and reads every one it lists.
- A new version is added only beside the old reader, never in place of it.
- A writer writes the highest common version.
- So any LIVEHOLDER-or-later holder moves onto any later build. A rollback onto an older build moves only if that build reads the writer's version; otherwise it is refused before the exec, and the old holder keeps running. That is the force-upgrade rule: never a mixed state.

**Failure safety, summed up:**

| When | What happens |
|---|---|
| The precheck fails | `Refused`; the old image serves on, untouched |
| The exec fails | `Refused`; the old image serves on, untouched |
| A crash after the exec | The process ends, and the program gets the PTY hangup. Left on disk: the synced scrollback and `<socket>.handoff`. |

- **After a crash:** `move_to` returns `LostInMove`. Revive and reconcile treat "`.handoff` and no holder" as `Lost::DuringMove`: the restart report says "its terminal was lost while moving onto the new holder build; what it printed is kept". The agent is resumed by its conversation as today, a shell is reopened with its saved output, and the marker is removed. Never silent.
- **Alternatives:** whether to add a keeper process is ruling B below.

**Viewers and the daemon.**
- The attach streams and humatad's `holders::Link` see EOF without `Exited`.
- The Link already closes (`offsets.closed`) and reopens on next use, rebuilding the screen from the same kept bytes, so the screen is identical.
- The app's live link reattaches as after a daemon restart.
- Gap found: `client.rs` keyed-input, `KeyQuery` and `AwaitKeys` treat "connection closed, holder still answers" as an older holder (`older_holder`, `client.rs:739`). A move in flight would read as `Outdated`.
  - Fix: on EOF they reconnect and ask once more. Only a second close on the same frame from the same `instance` means an older holder.
  - Typing through `holders::Link` that hears the link close without `Exited` retries once on a fresh link.

**Daemon: Migrate the holders becomes a move** (`upgrade/holders.rs`):
- `restart()` becomes `move_or_restart()`.
- It first calls `move_to(socket, current holder)` with no idle wait, for agents and shells alike. On `Moved` it rewrites the `holder_builds` record (`record_agent` / `record_shell`).
- On `TooOldToMove` it falls back to today's idle-gated restart (agents) or leaves the shell (shells), and the item lists it as "too old to move".
- On `Refused` the item shows the reason and Migrate again. On `LostInMove` the session goes to revive, which reports it.
- Item wording changes from "restart" to "move". A box does the same under `Upgrade{thing = HOLDERS}` (D378), and its report lists only the too-old ones as waiting.

### Audit: every path that ends a holder today

Socket paths are `<state>/holders/<tab>.sock` (`humatad/src/terminal.rs:133`, `sessions/src/start.rs:732`), independent of session folders, so no folder rename breaks a socket. Changes marked (F-x) follow the Fable ruling of that letter below.

| Path | Ends today (file:line) | Same machine? | Change |
|---|---|---|---|
| Channel rename | `channel_service/rename_folder.rs:328-336` `stop_for_rename` → `holder_end::shut_or_end` :419; then `rename(2)` :343, `finish_move`, `resume_all` :368. Agents only; shells already survive. | yes (each box runs its own, `follow_rename.rs:59`) | (F-D) keep agents running; the rename doesn't stop them; record/conversation-key rewrite deferred to the agent's next stop/resume |
| Desk → channel | `desk_service/move_to_channel.rs:794-839` `stop_for_move`: shells `shut_or_end` :833, agent `close.rs:240/269`; copy, then old to Bin | yes | (F-E) same filesystem: `rename(2)` instead of copy, keep running |
| MoveSession | `session_service/move_session.rs:813-846` `stop_old`/`stop_on` (local :830; remote `remote_work.rs:1052` → `session_service.rs:1064`); undo :1043, :1258; recovery `finish` :1197 | cross-machine only (same machine refused :336) | (F-J) old kept until the new one is confirmed up |
| Carry release | `carry_service.rs:180-195`, `shut_or_end` on the old box's shells :188 | cross-machine | after the new side is confirmed (F-J); recorded as can't-keep: the program can't cross machines |
| Archive | `session_service/archive.rs:964-1004` `stop_agent`, `shut_or_end` :981; restore-undo :1377 | yes | (F-F) D784: ask, keep running is the default |
| Desk close | `desk_service/close.rs:61-62` `stop_agent` (:269), `stop_shells` (:299) | yes | (F-G) Hide keeps the holder; only End ends it |
| Channel delete | `channel_service/delete.rs:742-797` `stop_all` (:753, :777, :780) | yes | (F-H) |
| Channel Desk watch | `desk_service/channel_desk_close.rs:798,806` `close_locked` (:512-513); :810 → `remote_view.rs:112` | yes, automatic | (F-I) hide and list under "Not in a channel" instead of ending |
| New tab save failed | `terminal.rs:1024` ends the just-spawned shell holder | yes, automatic | stays: nothing ran in it yet; recorded in the decision |
| Tab close | `terminal.rs:1488` (`close_one` :1442) | yes, user | stays user-only (D784 ask) |
| Running holders | `running_holders.rs:543-571` Stop (`end_by_pid` :566), :604-640 End for good | yes, user | stays: user Stop/End |
| Machine Stop | `stop/sweep.rs:102-160`, `stop/boot.rs:82`, `stop/cli.rs:309` | yes, user | stays: user Stop of the machine |
| Migrate the holders | `upgrade/holders.rs:689-740` → `revive/src/reconcile.rs:392-395` → `revive/src/agent.rs:841-860` `stop_holder` | yes | **move** (re-exec); restart only for holders too old to move |
| Revive stale | `revive/src/reconcile.rs:365-394` (stale Launching/Resuming), `revive/src/agent.rs:167-372` (a just-spawned holder whose hand-off failed) | yes, automatic | (F-I) |
| `away/uplink.rs`, `fault.rs` | end nothing (uplink only reports; `fault.rs` kills the daemon itself in admin test builds) | n/a | none |

### Tasks (worker-sized; Haiku 5.5 high unless noted)

Batch 1 is holder-only and can start at "go"; no other lane claims `daemon/holder/` except the `client.rs` WIP that main commits first.

| # | Task | Files | Proves it |
|---|---|---|---|
| T0 (manager) | Write the failing tests below | `holder/tests/take_over.rs` (new), `holder/tests/take_over_faults.rs` (new), `holder/BUILD.bazel` (a `take_over_faults_test` beside `power_cut_test`) | they fail for the right reason |
| T1 | Handoff format: encode, decode, versions, `HandoffError`; unit tests (round trip, each corruption refused) | `holder/src/handoff.rs` (new), `holder/src/error.rs` (variants) | `holder_unit_test` |
| T2 | Reap gate and print gate: waiter via `waitid NOWAIT`; reader takes the gate around read+push | `holder/src/server.rs` | existing `terminal_test`, `end_test` stay green |
| T3 (Sonnet) | `MoveTo` in the old image (check, quiesce, sync, write, precheck, clear CLOEXEC, exec, failure path); `--take-over`, `--check-take-over`, `--handoff-versions` in `main.rs`; adopt in `take_over.rs` | `holder/src/{server,protocol,main}.rs`, `holder/src/take_over.rs` (new) | `take_over_test`, `take_over_faults_test` |
| T4 | Client `move_to`, `Moved`, `TooOldToMove`, `LostInMove`; keyed/KeyQuery/AwaitKeys retry once on EOF before `Outdated` | `holder/src/{client,lib,error}.rs` | `take_over_test` (keyed input across a move), `return_after_text_test`, `keyed_input_test` stay green |
| T5 | Revive reads `.handoff` + no holder as lost during a move and reports it; removes the marker | `revive/src/reconcile.rs`, `revive/src/report.rs`, new `revive/tests/lost_in_move.rs` | `revive:lost_in_move_test` |
| T6 (Sonnet) | Migrate = move: `move_or_restart`, shells included, no idle wait for movers, wording, box path | `humatad/src/upgrade/holders.rs`, `upgrade/items.rs` (wording), `humatad/tests/upgrade_holders_move.rs` (new) | `humatad:upgrade_holders_move_test`; `upgrade_holders_test`, `upgrade_remote_holders_test` updated |
| T7 | `holders::Link` typing retries once on a link closed without `Exited` | `humatad/src/holders.rs` | in `upgrade_holders_move_test` (typed during a move arrives once) |
| T8 (Sonnet) | Rename keeps running (D801 D): pending re-key flag on agent records, re-key on every resume path, alias symlink lifecycle, atomic swap; test `channel_rename_live_test` (pid unchanged, alias resolves an old path, gone after the last old-path holder, re-key on resume incl. after a power cut) | `channel_service/rename_folder.rs`, `follow_rename.rs`, `sessions/src/` resume re-key (after main commits the sessions WIP), new `humatad/tests/channel_rename_live.rs` | pid unchanged |
| T9 | Desk→channel same filesystem is a rename (D801 E) | `desk_service/move_to_channel.rs`, new `humatad/tests/desk_move_live.rs` | pid unchanged; undo keeps it |
| T10 | Archive ask (D801 F): `ArchiveSession{end}`, keep running by default, listed as archived | `session_service/archive.rs`, proto field (seam: main), new `humatad/tests/archive_live.rs` | pid unchanged after Archive |
| T11 | Hide never ends (D801 G): test only, plus the fix if it fails | new `humatad/tests/desk_hide_live.rs` | pid unchanged after Hide |
| T12 | Delete moves live sessions to the Desk by default (D801 H) | `channel_service/delete.rs`, proto field (seam: main), new `humatad/tests/channel_delete_live.rs` | pids unchanged on the Desk |
| T13 | The channel-desk watch and carry release detach, not end (D801 I); revive never treats a live `.handoff` as stale | `desk_service/channel_desk_close.rs`, `carry_service.rs`, `revive/src/reconcile.rs`, new `humatad/tests/detach_live.rs` | pid unchanged, listed under Not in a channel |
| T14 (Sonnet) | `Freeze`/`Thaw` frames and the freeze-first cross-machine move (D801 J) | `holder/src/{server,protocol,client}.rs`, `session_service/move_session.rs`, `remote_work.rs`, new `humatad/tests/move_session_freeze.rs` | a failed move thaws the old agent with its pid unchanged |

### Claims

- `daemon/holder/` (src, tests, BUILD.bazel), after main commits the `client.rs` WIP.
- `daemon/revive/src/{reconcile,report}.rs`, new `daemon/revive/tests/lost_in_move.rs`.
- `daemon/humatad/src/upgrade/{holders,items}.rs`, `daemon/humatad/src/holders.rs`, new `daemon/humatad/tests/upgrade_holders_move.rs`, `daemon/humatad/tests/upgrade_holders.rs`, `upgrade_remote_holders.rs`.
- Batch 2 (after the ruling): the files of the audited paths: `channel_service/rename_folder.rs`, `desk_service/{move_to_channel,close,channel_desk_close}.rs`, `session_service/{archive,move_session}.rs`, `channel_service/delete.rs`, `carry_service.rs`, plus a new test file for each.
- Docs: SPEC PER-6 (wording to main), this Lane, the DECISIONS entry (appended).
- Seams for main: `MODULE.bazel` if rustix lacks waitid `NOWAIT`.

### First failing tests (T0)

All of them use their own `/tmp/h2…` temp dirs and end only their own holders. "The newer build" is the test's own copy of `humata2_holder` in another temp folder, so `ps -o comm=` tells the two images apart.

**`holder/tests/take_over.rs`:**
- `a_shell_moved_onto_a_newer_build_keeps_its_pid_and_never_sees_a_hangup`:
  - the program is `sh` with `trap 'echo hup >> hup.log' HUP`, and it writes its pid;
  - after `move_to`, the holder pid and the program pid are unchanged, the program's ppid is still the holder, the holder's exe is the copy, and `hup.log` is absent.
- `the_screen_and_scrollback_are_identical_after_a_move`: an attach from 0 before and after gives the same `Welcome` (earliest, end, cols, rows) and the same bytes.
- `typing_after_a_move_reaches_the_program`: `echo moved-$((6*7))` shows `moved-42`.
- `the_exit_code_is_still_reported_after_a_move`: `exit 7` after the move gives `Exited{7}`, to a live client and to a later attach.
- `a_stand_in_agent_survives_a_move`: `fake_agent_tui` (raw mode) keeps its pid, and `AwaitKeys` answers ready after the move.
- `a_keyed_input_across_a_move_is_typed_once`: the same key sent before and after, `KeyQuery` sees `arrived` with the same `instance`, and the text shows once.
- `a_move_onto_a_build_that_reads_no_common_version_is_refused_and_changes_nothing`: the target is a stub script answering `--handoff-versions` with `99`; the result is `Refused`, the same exe and pid, and typing still works.
- `a_target_that_fails_its_check_is_refused`: the stub exits 1 on `--check-take-over`.
- `a_failed_exec_returns_to_the_old_holder`: the stub passes both checks, then removes itself, so the `execve` fails with `ENOENT`; the result is `Refused`, and the holder still serves the same bytes and takes input.
- `a_holder_too_old_to_move_says_so`: the stand-in old holder from `return_after_text.rs` (it closes on unknown frames) gives `TooOldToMove`.

**`holder/tests/take_over_faults.rs`** (faults build):
- `a_crash_right_after_the_exec_is_reported_not_lost`: a `<sock>.crash-after-take-over` file makes the new image abort after reading the handoff. The result is `LostInMove`, `.handoff` and the synced scrollback remain, and `read_saved` has every byte printed before the move.

**Batch 2** (after T5 and T6):
- `revive:lost_in_move_test`;
- `humatad:upgrade_holders_move_test`: Migrate moves a running agent (stand-in) and a shell with their pids unchanged, the item reads "move", and a holder too old to move still restarts;
- one test per audit change, asserting the program pid is unchanged after the action.

### Decisions

D801 (Fable 5.2, 2026-11-18) rules A–K. Plan changes from it:
- **Order of the move.** Negotiate versions first, then quiesce. The check is bounded at about 2 s. Reap by pid only. The new image resets its signal state and does one `WNOHANG` reap. CLOEXEC is cleared only after the check child has exited.
- **Busy refusal.** `MoveTo` is refused while keyed input or `AwaitKeys` is pending.
- **`Moving` to every client.** Each client gets `Moving` before the exec and resends un-acked keyed input under the same keys.
- **The handoff carries everything Welcome serves.** That includes modes, a partial escape or UTF-8 tail, and `holder_generation`. Welcome reports the build.
- **Handoff lifetime.** `.handoff` is fsynced before the check. With a live pid it means in progress, so revive leaves it alone.
- **The check.** `--check-take-over` runs the real take-over code minus adopting the fds.
- **Migrate.** It moves a canary first and stops at the first failure.
- **Audit outcomes.**
  - Rename and Desk→channel keep running, with a deferred re-key and an alias symlink.
  - Archive builds D784's ask.
  - Delete defaults to moving live sessions to the Desk.
  - The channel-desk watch and carry release detach instead of ending.
  - Cross-machine moves freeze, then end only after the new side is confirmed.
- **Specs.** PER-6 is reworded and PER-32 is added (wording in D801; main applies it to SPEC.md).

### Workers

- 2026-11-18 11:14: "go" for batch 1 (main, after 0.3.2 df0f1684). T0 done by the manager: `holder/tests/take_over.rs` (14 cases), `holder/tests/take_over_faults.rs` and the `take_over_faults_test` BUILD target. Targets sent to main.
- 2026-11-18 11:15: one Sonnet worker on T1–T4 together (`holder/src/*`, new `handoff.rs`, `take_over.rs`). Deadline 11:45. Reason: the four tasks share `server.rs` and `client.rs`.
- 2026-11-18 11:22: the worker handed back T1–T4, written but unbuilt: `handoff.rs` (v1, FNV checksum, unit tests), `take_over.rs` (MoveTo path, adopt), the waiter on `waitid NOWAIT` under a reap lock, the `--handoff-versions`, `--check-take-over` and `--take-over` flags, the faults abort, and client `move_to` with the retry before `Outdated`. Main is building and running the holder tests. Its earlier compile break (11:18) came from exports added before the code behind them; fixed.
- 2026-11-18 11:27, gate 2. Green: `holder_unit_test`, `take_over_faults_test` and the 7 existing holder tests. `take_over_test`: 10 of 14 passed. Four failed (keyed input across a move, typing after a move, moved twice, identical screen): each was refused because the new build "didn't answer within 2 seconds".
  - Diagnosis: one 2 s limit covered both runs of the new build. The first run, `--handoff-versions`, is the first exec of a freshly copied binary, which macOS scans before it runs, so under parallel load it timed out.
  - Fix (manager, in `take_over.rs`): the versions query gets 20 s, since nothing is held yet; the check, run while output is frozen, keeps 2 s (D801 A1). Waiting on the rerun.
- 2026-11-18 11:29, T5 (manager), in `revive/src/report.rs` only:
  - a restart-report entry gets `lost` ("its terminal was lost while moving onto the new holder build; what it printed is kept") when `<socket>.handoff` exists and no holder pid is alive (checked with `ps`; nothing is signalled);
  - the marker is removed once the entry is reported;
  - 3 unit tests, run in `revive_unit_test`. Unbuilt at the time of writing.

**Where it stops at 12:00 (owner stop):**
- **T0–T4:** written. Green except `take_over_test`, pending the timeout fix's rerun.
- **T5:** done in `report.rs`.
  - Not done: the planned `revive/tests/lost_in_move.rs` end-to-end test, i.e. a reconcile over a store with a `.handoff` beside a dead holder.
  - Not done: revive's stale Launching/Resuming branch (`reconcile.rs:347-394`) doesn't yet skip a holder with a live `.handoff`. It is only reachable if Migrate moves a Launching or Resuming agent, which T6 must never do.
  - Not done: `report.rs`'s `handoff_path` copies `holder/src/take_over.rs`'s naming. Export one from the holder crate, then use it here.
- **T6–T14:** not started.
  - Next, T6: `upgrade/holders.rs` `move_or_restart`, moving only Running agents and shells.
  - T8 needs a seam from AGENTS2 for `sessions/src/agents.rs`.

**Batch 2, resumed 2026-11-18 (manager: minion LIVEHOLDER, Opus 5.5):**
- 14:05 T5 leftover done (manager): `humata2_holder::handoff_path` exported (`holder/src/{take_over,server,lib}.rs`); `revive/src/report.rs` uses it, its own copy removed.
- 14:10 T6 design (manager): a holder record (`holder_builds/<name>.sha`) gets a second line `moves` when the holder file it was spawned from answers `--handoff-versions` (probed once per holder-file stamp, 20 s bound, any failure = can't move). No `moves` line (records from before LIVEHOLDER, or none) = too old to move: agents keep today's idle-gated restart, shells stay. Migrate moves every Running agent and every movable shell one at a time, canary first, stopping at the first Refused/LostInMove (D801); a runtime TooOldToMove drops the `moves` line and falls back. After a move the agent's watch is started again (the old image's connection EOFs). Movable shells now hold the item open. New notes `HOLDER_NOTE_MOVES`/`MOVING`: seam for main (proto + clients/live + clients model + ui label).
- 14:12 T6 worker (Sonnet medium): `upgrade/holders.rs`, `upgrade/items.rs`. T6 tests (manager): new `humatad/tests/upgrade_holders_move.rs` (3 cases: a working agent and a shell move with holder and program pids unchanged and no hangup; a too-old holder still restarts while a movable one keeps its pid; a movable shell alone holds the item open until moved); `tests/upgrade_holders.rs` now strips `moves` before its upgrade so it keeps testing the too-old path. `upgrade_remote_holders.rs` unchanged (it stands for old holders by deleting records, still too old).
- 14:15 T5 e2e worker (Haiku high): new `revive/tests/lost_in_move.rs` + `lost_in_move_test` target.
- T7 deferred: plain typing has no key, so a blind retry after a link closed mid-move could type twice; the retry belongs in `terminal.rs`'s callers (not claimed). Typing during a move fails today with "the shell exited before the text reached it" and types nothing; recorded for the next batch.
- 14:40 T6 worker done (unbuilt): records carry `moves`; Migrate moves agents then shells one at a time, stopping on Refused/LostInMove (item Failed), TooOldToMove drops `moves`; restart only for too-old idle agents; `outstanding`/`waiting_sessions` in items.rs. Manager review: no holder ended, no unwraps outside tests, errors per variant.
- 14:45 T5 e2e test written (unbuilt): `revive/tests/lost_in_move.rs`, `lost_in_move_test`. T5 left: reconcile's stale Launching/Resuming branch skipping a live `.handoff` (unreachable while only Running agents move; with T13).
- Not started this batch: T8–T14. T7 deferred (above).

**Batch 3 (T8–T14), 2026-11-18, after main's gate on batch 2 (all daemon targets green; seam applied):**
- Claims added: `daemon/sessions/src/agents.rs` (T8; AGENTS2 idle), `humatad/src/channel_service/{rename_folder,follow_rename,delete}.rs`, `desk_service/{move_to_channel,channel_desk_close}.rs`, `session_service/{archive,move_session}.rs`, `carry_service.rs`, `remote_work.rs` (T14 parts only), `revive/src/reconcile.rs`, holder `server/protocol/client.rs` (T14), new tests `humatad/tests/{channel_rename_live,desk_move_live,archive_live,desk_hide_live,channel_delete_live,detach_live,move_session_freeze}.rs`. Proto fields are seams for main.
- Wave 1 (parallel, disjoint files): T8 (Sonnet), T10 (Sonnet), T11 (Haiku), T13 (Sonnet), T14 (Sonnet). Wave 2 after T8: T9, then T12 (E in reverse).
- T11 done (unbuilt): `humatad/tests/desk_hide_live.rs`. Hide sends no RPC (features/session close.rs `choose_close` -> view-only `CloseTab`), so no fix needed. CloseDeskSession is `user_only` today, stricter than D801 G's "agent needs the destroy switch"; kept (safer, nothing ends), noted for a later pass if agents should End with the switch.
- T13 done (unbuilt). Files changed:
  - `desk_service/channel_desk_close.rs`: `detach_live`. Live terminals lose their session and are listed as Not in a channel; the agent record is saved Stopped so a restore can't start a second copy. Remote sessions still go through `stop_for_watch`, which is open for a later pass.
  - `carry_service.rs`: the release no longer ends the old machine's shells.
  - `revive/src/reconcile.rs`: stale Launching/Resuming with a live `.handoff` gives `Revived::Left`, with unit tests. This closes the last T5 item.
  - `tests/detach_live.rs` is new; `tests/move_session.rs` now expects the old shell kept, and the test ends it itself.
  - Open: the "kept, with reason" comments in the stopped-boot sweep and Remove machine are outside the claims. A detached session's folder still goes to the trash with a shell in it (reversible pick).
- T10 done (unbuilt): `session_service/archive.rs` reads `end` (unset keeps running; true ends the agent and the shells; an agent needs MachineDestroy), keeps the folder while a holder answers, and a restart finishes with end=false. `tests/archive.rs` now passes `end: Some(true)`, and its cut-at-recorded cases expect the terminal kept. New `tests/archive_live.rs`. Seam `scratchpad/seam_T10.md`: proto `end`, `RunningHolder.archived`, archive_remote forwarding, every `ArchiveSessionRequest` literal, confirm labels.
- T14 done (unbuilt).
  - Holder: `Freeze`/`Thaw`/`FrozenQuery` frames. While frozen, typing is refused, attach and the screen still work, MoveTo is refused, and shutdown sends SIGCONT so the program can end. Clients get `freeze`/`thaw`/`is_frozen`; an old holder answers `TooOldToFreeze`.
  - `move_session.rs`: the move freezes first and writes the journal `frozen` before the SIGSTOP. Failure or cancel thaws, recovery thaws phases before "resumed", and a holder too old to freeze is refused naming Migrate. Shells are never frozen. `remote_work.rs` gets `freeze_session_on`.
  - Tests: new `holder/tests/freeze.rs` and `humatad/tests/move_session_freeze.rs`.
  - Seam `scratchpad/seam_T14.md`: FreezeSession rpc + handler, `Journaled.frozen`, RIG_TESTS entry.
  - Open: the "Paused for move" label in the running list (UI/listing owner).
- T8 done (unbuilt).
  - `rename_folder.rs`: a same-filesystem rename moves the folder and records at once and stops nothing. The old name becomes a symlink, swapped in atomically (EXCHANGE, else rename then symlink). Cross-filesystem renames keep stop/resume.
  - New `channel_service/rename_alias.rs`: the alias record lives in `.aliases/<channel>.json`; a 1 s watcher removes a link once its last old-path holder ends; `recover_renames` runs at start-up (hooked from `delete.rs`).
  - New `revive/src/rekey.rs`: the pending re-key is a `<socket>.rekey` note; `agent::resume` re-keys before resuming and keeps the note when it fails.
  - `sessions/src/agents.rs`: `CliVersions::follow`.
  - Tests: new `tests/channel_rename_live.rs`; `channel_folder_follows.rs` and `channel_rename_roam.rs` updated to the keep-running behaviour.
  - Seam `scratchpad/seam_T8.md`: revive lib re-exports + `CliVersions::follow`, `known_folders` pub(super).
  - Unchecked: placement treating the link name as taken, and trash/delete/carry walks not following the link.
- Wave 2: T9 (Sonnet) and T12 (Sonnet), in parallel on separate files.
- T9 done (unbuilt).
  - `desk_service/move_to_channel.rs`: a scratch Desk session moving within one filesystem is a rename. It writes re-key notes, journals `Renaming`, renames with an alias link, then switches; there is no stop/resume and no busy confirm. A failure before the switch renames back, and recovery undoes or finishes. Picked folders and cross-filesystem moves keep the copy path, whose plan says the terminals restart.
  - Tests: new `tests/desk_move_live.rs`; `desk_move.rs`, `desk_into_channel.rs` and `desk_into_channel_roam.rs` updated (copy-path cases now use picked folders).
  - Manager applied the visibility half of `seam_T9.md` in rename_folder.rs and rename_alias.rs. Main still flips `mod rename_folder` to `pub(crate)` in channel_service.rs.
  - Open: a later channel rename leaves a Desk session's link dangling (`repoint` only walks the channel's own records). Codex/Copilot `follows` are empty on rename re-keys (T8 and T9). Undo doesn't put a repointed clone back.
- T12 done (unbuilt).
  - `channel_service/delete.rs`: by default a delete moves each live session to the Desk. It renames the session folder under `<channels>/desk`, writes re-key notes, files the session on the Desk and keeps its holders running. Sessions that can't move are named in the plan; their terminals are detached and run on, and the folder goes to the Bin. `end_live` ends everything as before (an agent needs MachineDestroy, though user_deletes still refuses agents). The `.deleting` note records the choice and the moves for recovery.
  - Tests: new `tests/channel_delete_live.rs`; `delete_channel.rs` passes end_live.
  - Seam `scratchpad/seam_T12.md`: proto fields, literals, `DeskSvc::tell_session_updated`, confirm labels.
  - Open: no old-path link (the channel goes to the Bin); `.humata` notes and clone paths aren't rewritten; `end_grants` isn't called; there's no store transaction.
- 2026-11-18 batch 3 written, all unbuilt. All seams combined in `scratchpad/liveholder_batch3_seams.md` (parts A–E). T7 still deferred.
- 2026-11-18 gate fixes after batch 3, each with a fast repro kept as presubmit coverage:
  - A box power cut during Start (`store.file.temp_written`, `store.session_detail.put.committing`). The new fast target `power_cut_start_box_test` (not in slow.txt) passes both points in 24 s on the current tree, and I made no code change for it. The leftover start journal and `.partial` clone didn't reproduce once main's relay allowlist fix (SessionService/FreezeSession) was in.
  - read_marks (3–4 cases, only under load). The agent watch was spawned after the start reported Succeeded, and its holder attach (from the end) could land after the agent's first answer, so nothing marked the session unread. Fix: `watch::agent_attached` returns a receiver that fires once the attach is done, and the start path waits on it before `progress.succeed`. read_marks_test now passes in 4 s, against a 19.8 s failure before.
  - An archive cut after `archive.recorded` with `end: true` never ended its terminals, so the folder was never freed (restart_operations, remote_archive). `recover_archives` always passed `end=false`. Fix: the end choice is kept durably as `state/archive_ends/<session>` before the Archiving row is saved, at both save sites. Recovery reads it, and both finish paths remove it; an archive that keeps its terminals removes any stale one. Repro: `archive_live_test::an_archive_that_ends_its_terminals_cut_once_recorded_ends_them_at_restart` (13 s target). I restored the archive_test expectations for cuts at archive.recorded with end=true: the terminals end and the folder is freed.
  - Removed the unused `store` parameter from `plan_pending` in rename_folder.rs (it was a compiler warning).
- 2026-11-18 19:10 open gaps after the gate, in main's order (unbuilt at the time of writing; build results below):
  1. Remote holders no longer end when a channel-desk target goes. `channel_desk_close.rs` detaches the session for every Target::Gone with Markdown, and `detach_live` leaves a remote session's agent running. `remote_view.rs` `stop_for_watch` was removed. `channel_desk.rs` `set_channel_desk_target` resumes only local stopped sessions. The two desk_roam cases now expect box-2's agent to keep running.
  2. A Desk session's link into a channel follows a later channel rename. `rename_alias::repoint_under` repoints every alias record whose target sat under the renamed folder, and `settle_aliases` calls it. Unit test: `a_desk_link_into_a_channel_follows_the_channel_rename`.
  3. T12: notes, paths and grants. `move_folder::rewrite_session_folder` rewrites the moved session's `.humata` notes and its clones' git files to name the Desk folder, and `put_on_desk` calls it after the rename (new error variant `DeskPaths`). The delete ends the standing grants of every deleted or moved session after withdrawing their requests (new `Grants` variant), as a move into a channel does (DK-18). Test: `channel_delete_live_test::a_session_moved_to_the_desk_has_its_notes_name_its_new_folder`.
  4. Codex/Copilot follows on the T9 and T12 re-keys. `rename_folder::folder_moves` was pulled out of `follow_or_defer`. The new `session_follows` fills the note's `follows` for a single session's move, used by `delete.rs` `write_rekey` and `move_to_channel.rs` `prepare_rename` (both now take the Starter). Unit test: `a_moved_session_s_codex_conversation_follows_it_by_the_steps_its_note_keeps`.
  5. Reason comments (D801 I, PER-32) at the stopped-boot sweep (`stop/boot.rs` `Settle::run`) and at Remove machine's archive (`archive_remote.rs` `dispose_for_removal`, `end: Some(true)`).
  6. Paused for move. The daemon keeps the holders it froze, by socket and pid (`move_session::paused_for_move`, set and cleared in `freeze_agent_here`). It is held by `#[expect(dead_code)]` until the seam lands. Seam `scratchpad/seam_PAUSED.py` (dry-run checked) adds proto `RunningHolder.paused_for_move = 15`, fills it in `running_holders.rs`, adds the clients field and live mapping, sets literals to false, and makes `ui/src/machine_running.rs` show "Paused for move" in place of the uptime.
  7. Agent callers behind the destroy switch (MachineDestroy, Never by default; no tier or UI change). CloseDeskSession: `DeskSvc::user_or_destroy_switch` replaces `user_only`. Channel plan and delete: an agent goes through `destroy_switch` instead of the user-only ChannelDelete check. Test: `channel_delete_live_test::an_agent_with_the_destroy_switch_on_deletes_the_channel_and_the_terminals_run_on`. `delete_channel_test::an_agent_cannot_plan_or_delete_a_channel` still holds with the switch off.
- 2026-11-18 20:05 gaps 1–7 built and green on focused runs.
  - humatad2_unit_test: `rename_alias::tests::a_desk_link_into_a_channel_follows_the_channel_rename` and all 3 `follow_tests` pass.
  - channel_delete_live_test: 6/6 pass in 5.7 s. desk_move_live_test passes. desk_roam_test: the two box-2 cases pass. delete_channel_test: `an_agent_cannot_plan_or_delete_a_channel` passes.
  - Main's failing case is fixed. The test was missing its session row. It also exposed a real bug: `folder_moves` mapped the moved folder itself to `new.join("")` (a trailing slash), so Codex's cwd became `fix//api/src`. It now maps to the new folder as is. This matters for T12 and the Desk-move re-keys, where the session folder is the moved root.
  - Not run: the full delete_channel_test (slow tier), and move_session_freeze_test (rig) for the freeze bookkeeping.

**Exit run, resumed 2026-11-19 (manager: LIVEHOLDER, Opus 5.5), after the crate split:**
- Paths now: holder `daemon/holder/`, Migrate `daemon/svc_upgrade/src/{holders,items}.rs`, tests still under `daemon/humatad/tests/` on `humatad_test_common`. Left: run `move_session_freeze_test` (rig) and the full `delete_channel_test` (slow tier), fix what they find.
- First runs of both rig tests failed to build, not to pass. Since the split, the `svc_upgrade` and `svc_stop_restart` `_faults` twins swapped only `runtime`, so the faults daemons linked two copies of `humata2_store`/`humata2_sessions`. Fixed in both BUILD files: `durable`/`holder`/`revive`/`sessions`/`store` are now per variant. The rest of the faults graph (`svc_away`/`svc_files`/`svc_inbox` twins, humatad FAULTS_DEPS) is BUILDFIX's. Reruns wait on main's green.
- New `holder/tests/take_over_ssh.rs` + `take_over_ssh_test` (manual, external): on h2-terminal-c1, a holder running a shell moves onto a second copy of the Linux holder. Same holder and program pid, the parent is unchanged, no HUP, the screen carries over, typing works, and the exit code reports 7. It reaches the remote socket through an `ssh -L` unix-socket forward. Runs in the terminal live batch with `TERMINAL_LAB_SSH_CONFIG`, `TERMINAL_VM_LINUX_HOLDER`. Built, not yet run.
- Exit check: Migrate wording is "Move N terminals … in place", with restart only for holders too old to move (`svc_upgrade/src/items.rs`). Left for main: the SPEC PER-6 rewording and the new PER-32 (text in D801 K).
