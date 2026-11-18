# A moved session's arrival rule types into it on the new machine, and its leaving rule runs first on the old one without holding the move

Layer: macro (a real daemon in a temp folder with fake boxes and a fake agent, plus the CLI against a throwaway `humatad2`). Platforms: macOS. The two boxes stand in for Linux arm64 machines.

## Starting state

- A throwaway `HOME` holds the daemon's state, config and scripts, as in the other cases. Nothing of the owner's `~/.claude`, `~/.copilot`, `~/.codex` or git config is read.
- Two fakes, `box-1` (old) and `box-2` (new); This Mac is the host and has no triggers. `~/.humata2/config/machine-scripts.toml` holds:
  - `arrive`: `on = "session-arrived"`, steps `send = "/restart"` with the defaults (wait for idle, Enter pressed).
  - `leave`: `on = "session-leaving"`, step `run = "save-state.sh"`. The script writes its `HUMATA_SESSION`, `HUMATA_FROM` and `HUMATA_TO` to `left.txt` in the session folder, then sleeps for `SLEEP` seconds if the variable is set.
- `~/.humata2/scripts/` holds `save-state.sh` and `arrive-fail.sh` (exits 3, for a branch).
- Channel `setup-lab` exists, with session `Warm up` on `box-1` (a fake agent, idle after one turn).

## Reproduce headless

- The move, against two fake boxes and a fake agent, in a real daemon: `bazel test //apps/humata2/daemon/humatad:move_session_test`. Capture the output.
- The hooks themselves have no test through a move. Their parts run on fakes: `bazel test //apps/humata2/daemon/humatad:humatad2_unit_test --test_arg=machine_scripts`, and the refusal of `start` on a session trigger: `bazel test //apps/humata2/daemon/machines:machine_scripts_test`.
- The CLI against a throwaway `humatad2` (`export HOME=<throwaway>`, `humatad2 serve`): `humata machine script check; echo $?`. An `arrive` rule holding `start = {...}` must exit 2. `humata session move` needs two machines, so only the fake-box rig can run it. Not headless: the real VM, the real Copilot login and resume, and the move's view in the app.

## Steps

1. **I do:** Write the two rules above. Then run `humata machine script check`.  
   **I see:** Valid, exit 0. If I add `start = { agent = "copilot", channel = "setup-lab" }` to `arrive`, check exits 2 and names the step, because a session trigger already has its session.  
   **State and events:** Check reads the file and the script names only. A session trigger refuses `start`, so a second agent can never start from a move.  
   **Status:** Fakes only. `machines/tests/machine_scripts.rs::each_kind_of_problem_has_its_variant` covers `StartOnSessionTrigger`.

2. **I do:** Move `Warm up` from `box-1` to `box-2`. In the app that is ⌥⌘M and a machine pick. From the CLI, `humata session move <session> --to box-2`.  
   **I see:** The move runs its steps: check, pause, pack, copy, verify, resume, switch, release. It ends Succeeded. The session shows on `box-2`, with its id, branch and conversation.  
   **State and events:** The move runs in `session_service/move_session.rs` (`Mover::run`, `go`). The session id, branch and conversation are kept.  
   **Status:** Works (real daemon). `daemon/humatad/tests/move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes`. The scripts are not part of that test.

3. **I do:** Watch the move between its verify step and its resume step, on `box-1`.  
   **I see:** The `leave` rule runs on `box-1` after the copy is verified and before the old agent stops. `save-state.sh` writes `left.txt` in the session folder.  
   **State and events:** `Mover::hand_over` (`move_session.rs` ~796) calls `script_leaving`, which fires `session-leaving` with `HUMATA_SESSION`, `HUMATA_CHANNEL`, `HUMATA_SESSION_DIR` and `HUMATA_TO`, before `stop_old`.  
   **Status:** Code, no test for the hook point (`session_service/move_session/scripts.rs` lines 38 to 52). The leaving run itself is Fakes only: `machine_scripts/live.rs::tests::leaving_rules_have_run_when_the_move_may_go_on`.

4. **I do:** Measure how long the move waits for `leave` (make `save-state.sh` sleep 50 seconds with `SLEEP`).  
   **I see:** With a 50 second script, the move waits about 50 seconds, stops the old agent and goes on. With a script that sleeps 120 seconds, the move waits 60 seconds and goes on. The script is killed at 60 seconds, and the `leave` run shows Timed out.  
   **State and events:** `leaving` caps the wait at 60 seconds (`LEAVING_WAIT`, `move_session/scripts.rs` line 13). The step itself is also capped at 60 seconds for a leaving rule (`step_timeout`), so the kill comes from the step limit.  
   **Status:** Likely bug. D681 and the exit criterion say a leaving rule never holds a move. The code awaits it inside the resume step, so a slow rule holds the move for up to a minute. D684's 60 second cap is the only bound, so the spec conflicts with itself. Confirm by timing a move with a slow script.

5. **I do:** Let the leaving script change a file in the session folder while the move waits.  
   **I see:** After the script, the move packs the session again. If the folder's digest changed, the move copies it again before the new agent starts.  
   **State and events:** `hand_over` (`move_session.rs` around lines 800 to 810) packs again and copies again on a digest change. The leaving script's writes are therefore carried over.  
   **Status:** Code, no test. The re-pack on a digest change exists; no test drives it with a script's write.

6. **I do:** Let the old agent stop, and wait for the new agent on `box-2` to resume.  
   **I see:** The agent stops on `box-1`, then starts on `box-2` in its conversation. The session shows Idle on `box-2`.  
   **State and events:** `stop_old` runs after the leaving rule. `resume_on(box-2)` starts the agent. Nothing starts a second agent.  
   **Status:** Works (real daemon). The same move test as step 2.

7. **I do:** Wait for the move to end.  
   **I see:** `arrive` runs on `box-2`. It waits until the moved session is Idle and types `/restart` with Enter. The session shows Working, then Idle.  
   **State and events:** `Mover::run` calls `script_arrived` once the move's cleanup is done, after `journal_done` (`move_session.rs` around lines 505 and 506). The `send` step waits on the session's turn state, then types through `type_for_script`, which sends the text and Enter as two keyed requests. The key is per run and per step.  
   **Status:** Code, no test for the hook point. The engine's order (wait, then send, into the session the trigger names) is Fakes only: `machine_scripts/engine.rs::tests::send_types_into_the_moved_session_on_a_session_trigger` and `the_wait_for_idle_comes_before_a_send_that_asks_for_it`.

8. **I do:** Read the session's terminal on `box-2`.  
   **I see:** `/restart` is in the input and Enter has run, so Copilot restarts. The script's send presses Enter itself, as `humata session type` now does by default (B12 fixed, D754).  
   **State and events:** `type_for_script` sends the text, then after a 100 ms pause a separate return, so an agent that reads a paste whole does not take the return as part of it.  
   **Status:** Code, no test. `terminal.rs` `type_for_script` (around line 1279).

9. **I do:** Run `humata machine script log box-2 --limit 5` and `humata machine script log box-1 --limit 5`.  
   **I see:** `arrive` is a row on `box-2` with the session id, outcome Passed and its steps. `leave` is a row on `box-1` with the session id. Neither adds an Inbox item.  
   **State and events:** Each run record keeps its session id. A passing run adds nothing to the Inbox.  
   **Status:** Fakes only. `machine_service/script_rpc.rs::tests::a_run_is_in_the_log_with_its_steps` and `machine_scripts/live.rs::tests::a_failed_run_posts_one_request...`, which checks that a pass withdraws nothing it did not post.

10. **I do:** Check the session on `box-2` and the folder on `box-1`.
    **I see:** One agent, on `box-2`. The folder on `box-1` is released only after the move's release step.
    **State and events:** No script starts a second agent (step 1). The move's release is unchanged.
    **Status:** Works (real daemon) for the end state: `daemon/humatad/tests/move_session.rs::moving_from_this_mac_frees_its_folder_only_after_it_runs_on_box_1`. The test name is about a move from This Mac, so it is a neighbour of this case rather than a test of its scripts.

11. **I do:** Move `Warm up` from `box-2` back to This Mac.
    **I see:** A `leave` run is listed on `box-2`, because `box-2` is a remote machine and is the old one here. No `arrive` run happens on This Mac, because This Mac has no triggers.
    **State and events:** `script_arrived` returns when `to` is the host (`move_session/scripts.rs` line 57). `script_leaving` returns when `from` is the host (line 39). Only the machine side fires.
    **Status:** Code, no test.

## Branches

### The arrival script fails

- **When:** `arrive` is `run = "arrive-fail.sh"`, which exits 3.
- **I do:** Move the session to `box-2` and wait for the run.
- **I see:** The move shows Succeeded and the session runs on `box-2` with its work and conversation. The run shows Failed at step 1 with exit 3 and the output line. One Inbox request, "Script arrive failed on box-2", shows the step, the exit, the tail and the trigger, with Approve and Deny.
- **Expected outcome:** The move is never rolled back, retried or duplicated by a script. A pass withdraws the request.
- **Status:** Likely bug. The failure path is Fakes only (`machine_scripts/live.rs` tests). The card offers Approve and Deny, not Retry and Dismiss (D686, `inbox/host_asks.rs` line 124).

### The arrival script waits too long for idle

- **When:** The moved agent does not go idle, for example because it is stuck on a prompt.
- **I do:** Move the session and wait two minutes.
- **I see:** The run shows Failed with "still busy after 120 seconds" and nothing is typed. The session stays as the move left it.
- **Expected outcome:** A late idle does not make `/restart` type after the wait has failed. A retry is the only way it runs again, and the keys of that retry are new (see the gap below).
- **Status:** Code, no test. `machine_scripts/live.rs` `wait_idle` wraps the loop in a timeout (around line 225). The engine's failure of the step is Fakes only.

### The leaving script fails

- **When:** `leave` is `run = "fail.sh"` or exits 1.
- **I do:** Move the session from `box-1` to `box-2`.
- **I see:** The move still ends Succeeded. The `leave` run shows Failed. An Inbox request "Script leave failed on box-1" appears.
- **Expected outcome:** The old agent stops at the normal time, and the folder is released at the normal time. A failed leaving rule never fails the move (D681).
- **Status:** Code, no test. `leaving` returns only whether it ended in time (`live.rs` line 73). The failure is reported through the engine's `finish`, which has Fakes only coverage.

### The leaving script hangs past its cap

- **When:** `leave` sleeps for 120 seconds.
- **I do:** Move the session from `box-1` to `box-2` and time it.
- **I see:** The move goes on after about 60 seconds. The daemon's log says the leaving scripts were still running after a minute and the move went on without them. The `leave` run shows Timed out, and an Inbox request appears for it.
- **Expected outcome:** The move ends, and the leaving run's timeout is recorded.
- **Status:** Fakes only. `machine_scripts/live.rs::tests::leaving_rules_that_hang_never_hold_a_move_past_the_cap`. The timing in the move itself is the Likely bug in step 4.

### The move is undone after the leaving script ran

- **When:** The leaving script has run, and then the move fails before the old agent stops. A cancel is refused once the resume step has begun, so this needs a fault in the stop step.
- **I do:** In the fake-box test rig, make the stop of the old agent fail after the leaving rule ran.
- **I see:** The move ends Failed and the old agent runs again on `box-1`. `leave` already ran, so `save-state.sh` has already written its file.
- **Expected outcome:** Nothing is undone by a script, and `arrive` does not run for a move that did not happen.
- **Status:** Code, no test. The arrival hook runs only after a successful move (`move_session.rs` around line 505). There is no compensating script. Whether one is wanted is an open question.

### The move is cut by a restart after the agent moved

- **When:** `humatad2` on This Mac restarts after the agent resumed on `box-2` and before the move finished.
- **I do:** Restart the daemon. Let the recovery finish the move.
- **I see:** The move ends Succeeded after recovery. `arrive` runs once on `box-2`. `leave` does not run again, because recovery does not repeat the leaving hook.
- **Expected outcome:** One arrival per move, and no second leaving run.
- **Status:** Code, no test. Recovery calls `script_arrived` when the journal says `resumed` or `switched` (`move_session.rs` around lines 1183 to 1185). The recovery of the move itself is Works (real daemon): `daemon/humatad/tests/move_session.rs::a_move_cut_short_after_the_agent_moved_is_finished_at_restart`.

### The link to box-2 drops during the arrival script

- **When:** I cut the link to `box-2` while `arrive` is waiting or typing.
- **I do:** Cut the link, then let it come back.
- **I see:** The run shows Failed with "out of reach" or the send's error. The session is on `box-2` and its terminal reattaches as in the link-drop test. `connected` fires once when the link returns, and `arrive` does not run again by itself.
- **Expected outcome:** No typed text is replayed on reconnect. Retry runs `arrive` once, from the start.
- **Status:** Code, no test. The reattach is Works (real daemon, stand-in ssh): `desktop/tests/live_flows/link_drop.rs::a_dropped_link_refuses_typing_and_the_same_shell_takes_it_again_once_back`.

### A fork lands on a machine

- **When:** I fork `Warm up` onto `box-2` instead of moving it.
- **I do:** Fork onto `box-2`.
- **I see:** No `arrive` run and no `leave` run.
- **Expected outcome:** Undecided. The spec names moves only (D681).
- **Status:** Not built. No fork path calls the script hooks; only `move_session` does. This is an open question in the spec, not a milestone.

### A once rule on a session trigger

- **When:** `arrive` has `once = true`, and the session moves to `box-2`, then a second session moves to `box-2`.
- **I do:** Move both sessions.
- **I see:** Only the first session's arrival runs. The second moved session gets no `/restart`.
- **Expected outcome:** Unclear. D684 says `once` is per machine, so this may be as written, but it is probably not what a person wants for a session trigger.
- **Status:** Likely bug (design). `once` is keyed by machine and rule only (`machine_scripts/log.rs` `once_passed`, `mark_once`). Confirm with two moves.

### The folder name a script sees

- **When:** A leaving or arrival script reads `HUMATA_SESSION_DIR`.
- **I do:** Print `HUMATA_SESSION_DIR` from a script.
- **I see:** A path made of the channel folder and `sessions/<name>`. It is not a path on the machine, so the script cannot cd into it.
- **Expected outcome:** A usable path on the machine, or a clear name for what it is.
- **Status:** Likely bug (confirm). `move_session/scripts.rs` lines 18 to 23 join `channel.folder` and the session folder, which is `sessions/<name>` relative to the channel folder (`move_session.rs` around line 369).

## Expected outcomes

- [ ] A session trigger refuses `start`, and `session-leaving` runs on the old machine after the copy is verified and before the old agent stops.
- [ ] A leaving run waits at most 60 seconds, and a slow or failing one never fails the move.
- [ ] `session-arrived` runs once the move has ended and the agent has resumed, and `/restart` is typed once, after idle, with Enter.
- [ ] A failing arrival or leaving run leaves the session where the move put it, with one agent and its work intact.
- [ ] A recovered move runs its arrival once, and no leaving run repeats. This Mac, on either side, runs no script.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Check refuses `start` on a session trigger | Fakes only | `machines/tests/machine_scripts.rs::each_kind_of_problem_has_its_variant` |
| 2. Move to box-2 | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes` |
| 3. `leave` runs before the old agent stops | Code, no test (hook); run Fakes only | `move_session/scripts.rs` 38 to 52; `move_session.rs` ~796 |
| 4. Move waits up to 60 s for `leave` | Likely bug (conflicts with "never holds") | `move_session.rs` ~796; `move_session/scripts.rs` line 13 |
| 5. Leaving script's writes are carried over | Code, no test | `move_session.rs` ~800 to 810 |
| 6. Old agent stops, new agent resumes | Works (real daemon) | same move test as step 2 |
| 7. `arrive` types `/restart` after idle | Code, no test (hook); engine order Fakes only | `move_session.rs` ~505; `engine.rs` send tests |
| 8. Enter is pressed by the script's send | Code, no test | `terminal.rs` `type_for_script` ~1279 |
| 9. Log shows both runs with the session | Fakes only | `script_rpc.rs::a_run_is_in_the_log_with_its_steps` |
| 10. One agent, folder freed after | Works (real daemon) end state | `move_session.rs::moving_from_this_mac_frees_its_folder_only_after_it_runs_on_box_1` |
| 11. No script on This Mac | Code, no test | `move_session/scripts.rs` lines 39 and 57 |
| Branch: arrival fails | Likely bug (labels) | `live.rs` tests (Fakes only); `host_asks.rs` line 124 |
| Branch: arrival waits too long for idle | Code, no test | `live.rs` `wait_idle` |
| Branch: leaving fails | Code, no test | `live.rs` `leaving` line 73 |
| Branch: leaving hangs past cap | Fakes only | `live.rs::leaving_rules_that_hang_never_hold_a_move_past_the_cap` |
| Branch: move undone after leaving ran | Code, no test | `move_session.rs` ~505 |
| Branch: cut by restart after the agent moved | Code, no test (hook); move recovery Works (real daemon) | `move_session.rs` ~1183; `move_session.rs::a_move_cut_short_after_the_agent_moved_is_finished_at_restart` |
| Branch: link drops during arrival | Code, no test; reattach Works (real daemon, stand-in ssh) | `link_drop.rs::a_dropped_link_refuses_typing...` |
| Branch: fork lands on a machine | Not built | no fork hook; no milestone |
| Branch: once on a session trigger | Likely bug (design) | `machine_scripts/log.rs` `once_passed` |
| Branch: HUMATA_SESSION_DIR | Likely bug (confirm) | `move_session/scripts.rs` 18 to 23 |
## Gaps and likely bugs

- **Likely bug: a leaving rule holds the move for up to a minute.** The spec says a leaving rule never holds or fails a move (D681). `hand_over` awaits `script_leaving` before `stop_old` (`move_session.rs` ~796), and `leaving` waits up to 60 seconds (`move_session/scripts.rs` line 13). Confirm by timing a move with a 50 second leaving script.
- **Likely bug: a retry repeats the arrival's send.** Keys are per run and per step, and a retry is a new run (`engine.rs` ~308 and ~378), so `/restart` can be typed again.
- **Likely bug: a session trigger's `once` is per machine.** `log.rs` keys `once` by machine and rule, so only the first moved session on a machine gets the rule. Confirm with two moves.
- **Likely bug: `HUMATA_SESSION_DIR` is not a path on the machine.** It is the channel folder joined to `sessions/<name>` (`move_session/scripts.rs` 18 to 23).
- **Likely bug: an idle wait trusts Idle** (B6): a stopped session can show Idle and the wait returns on it.
- **Likely bug (spec): the Inbox card has Approve and Deny, not Retry and Dismiss** (`inbox/host_asks.rs` line 124).
- **Code, no test:** the script hooks in the move and the live `send` and `wait_idle` steps against a real session.
