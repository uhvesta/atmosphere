# A script runs once per reconnect, and a session that moves gets its scripts without risk to the session

Layer: real. Platforms: macOS host, Linux arm64 VM. Spec: `workflow/milestones/MACHINESCRIPTS.md` (D680-D689, Provisional). Every MACHINESCRIPTS step is Not built (MACHINESCRIPTS). Steps that are Humata's existing reconnect and move flows carry tags from `real/03` and `agents/09-moving-a-session.md`.

## Starting state

- `real/01` is done: the VM `h2-terminal-c1` is a machine, online, with Humata's tools. `real/10` rules may already exist.
- `~/.humata2/config/machine-scripts.toml` holds these rules (in file order):
  - `warm` : `on = "connected"`, one `run = "touch-connected.sh"` step (appends a line `up $HUMATA_TRIGGER` to `/var/tmp/h2-real-clis/connected.txt`).
  - `arrive` : `on = "session-arrived"`, `send = "/restart"` (types into the moved session's own agent).
  - `leave` : `on = "session-leaving"`, `run = "save-state.sh"` (copies a marker file into `/var/tmp/h2-real-clis/left/`).
- A channel `setup-lab` has this Mac and the VM. A Copilot session `Warm up` runs on this Mac in it (see `real/05`), idle, with one turn done.
- I am on the session's page.

## Test setup

- As `real/01` and `real/04`: throwaway `HOME`, Lima SSH config, `--agents` file with `copilot`, isolated `COPILOT_HOME`. Needs the owner's login for Copilot.
- Cut the link by killing the Humata-owned tunnel process on this Mac. Never restart or stop the shared VM, and never use `limactl` network stop (see `real/03`).
- Needs a free screen for the machine page. Headless equivalent: `humata machine script log <machine> --json`.
- Cleanup: remove `connected.txt`, `left/` and the rules and scripts from the throwaway home.

## Steps

1. **I do:** Cut the link to the VM, wait until the machine shows Away, then let the daemon redial. I touch nothing.
   **I see:** The machine returns online. The same holder keeps its shell, as in `real/03`.
   **State and events:** The link goes down and up once.
   **Status:** Works (real shell) for the component only: `terminal/host/src/roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay`. In the app with a stand-in `ssh`: Works (real daemon) `desktop/tests/live_flows/link_drop.rs::a_dropped_link_refuses_typing_and_the_same_shell_takes_it_again_once_back`.

2. **I do:** On the VM, read `/var/tmp/h2-real-clis/connected.txt`.
   **I see:** One new line `up connected`. Not two, and not one for every heartbeat while the link was down.
   **State and events:** The `connected` trigger fires once per link-up, never twice for the same one (D681). It runs on the VM with `HUMATA_TRIGGER=connected` and `HUMATA_MACHINE`.
   **Status:** Not built (MACHINESCRIPTS T2).

3. **I do:** Open the machine page, Scripts.
   **I see:** `warm` shows Passed at the time of the redial. `arrive` and `leave` show "never run".
   **State and events:** One run in the host's log for this machine, trigger `connected` (D686).
   **Status:** Not built (MACHINESCRIPTS T2, T5).

4. **I do:** Flap the link twice quickly, so a second link-up comes while `warm` is still running (make `touch-connected.sh` sleep 5).
   **I see:** `warm` finishes, then runs once more. It never runs in parallel with itself.
   **State and events:** A link-up while a run for the same machine is going queues one more run after it (D681). The log shows two runs, in order, not overlapping.
   **Status:** Not built (MACHINESCRIPTS T2).

5. **I do:** Move `Warm up` to the VM: press ⌥⌘M, pick `lima-h2-terminal-c1` and Confirm.
   **I see:** The session moves. The operation shows its steps (see steps 6 and 7 for where scripts fall).
   **State and events:** The move runs check, pause, pack, copy, verify, resume, switch, release. The session id, branch and conversation stay.
   **Status:** Works (real daemon) with fake machines and a fake agent: `daemon/humatad/tests/move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes`. A real Copilot conversation moving to the real VM: Likely bug (seen live, B5, B7 in the REALFLOW run: resume looks under `--root` and a resume after stop is treated as an interruption). See `real/06`.

6. **I do:** Watch the move between its verify step and its release.
   **I see:** The `leave` rule runs on this Mac (the old machine) after the copy is verified and before the old agent stops. This Mac is the host, and it has no triggers (D681), so for a session leaving this Mac `leave` does not run. See branch "Leaving the host" and open question 1.
   **State and events:** For a session leaving the VM, `session-leaving` runs on the VM with `HUMATA_SESSION`, `HUMATA_CHANNEL`, `HUMATA_SESSION_DIR` and `HUMATA_TO`. Each step is capped at 60 seconds and it never holds or fails the move (D684, exit criterion).
   **Status:** Not built (MACHINESCRIPTS T2). The move's steps themselves: Works (real daemon), `move_session.rs` as in step 5.

7. **I do:** Wait for the move to end.
   **I see:** The session runs on the VM. Then `arrive` runs: `/restart` is typed once into this session's own Copilot, after it is idle and Enter is pressed.
   **State and events:** `session-arrived` fires after the move ends on the new machine, after the agent has resumed, not before (D681). The case expects this order because `send` needs an agent that is up and idle (SE-21), and before the resume there is no agent to type into. The scripts run on the new machine, with `HUMATA_FROM` set. A move finished by recovery fires it too (D681).
   **Status:** Not built (MACHINESCRIPTS T2). Agent idle detection for Copilot: Works (real CLI, manual REALFLOW run). B12 is Fixed (B12, D754): `humata session type` presses Enter by default.

8. **I do:** Open the move operation, and the session's page.
   **I see:** The move shows Succeeded with its eight steps. The `arrive` run is listed with the session, outcome and output tail, and the machine page's Scripts section shows it on the VM.
   **State and events:** The run log row records the session. A passing script adds nothing to the Inbox.
   **Status:** Not built (MACHINESCRIPTS T2, T5). Whether the run shows on the move operation itself, or only on the machine page, is open question 2.

9. **I do:** Check the old folder on this Mac and the VM's processes.
   **I see:** One agent, on the VM. This Mac's folder is freed only after the session runs on the VM.
   **State and events:** The arrival script never started a second agent. `start` is refused on session triggers when the file is checked, because the session is already there (D682).
   **Status:** Works (real daemon) for the end state: `move_session.rs::moving_from_this_mac_frees_its_folder_only_after_it_runs_on_box_1` (name claims more than it asserts, see `agents/09` step 10). The no-second-agent check for scripts is Not built (MACHINESCRIPTS T2).

## Branches

### The arrival script fails

- **When:** `arrive` is `run = "fail.sh"` (exit 3), or `send` cannot find the agent idle.
- **I do:** Read the Inbox and the session page.
- **I see:** The move is Succeeded. The session runs on the VM with its work and conversation. The run shows Failed, and one Inbox item "Script arrive failed on lima-h2-terminal-c1" has the step, its exit, the output tail and the trigger, with Retry and Dismiss (D686, D105).
- **Expected outcome:** The session is never lost, rolled back or duplicated by a script. The move is not re-run on Retry; only the rule is.
- **Status:** Not built (MACHINESCRIPTS T2).

### The arrival script times out

- **When:** the agent never goes idle within the 2 minute wait, or the script sleeps past its `timeout`.
- **I do:** Wait.
- **I see:** The run shows Timed out. Nothing is typed. The session stays as it is and usable.
- **Expected outcome:** A late idle does not make `/restart` type after the timeout, since a retry is the only way it runs again (the send is keyed, D682).
- **Status:** Not built (MACHINESCRIPTS T2).

### The leaving script fails or hangs

- **When:** `leave` is `save-state.sh` and it exits 1, or sleeps past 60 seconds, for a session leaving the VM.
- **I do:** Move the session from the VM back to this Mac.
- **I see:** The move still ends Succeeded. The run shows Failed or Timed out, and an Inbox item appears with Retry.
- **Expected outcome:** The old agent stops and the folder is released at the normal time. A script cannot hold the move (D684).
- **Status:** Not built (MACHINESCRIPTS T2). Moving back from the VM: see `real/06`.

### The machine goes offline during the arrival script

- **When:** I cut the link after the move ends and before `arrive` finishes.
- **I do:** Wait for the link to return.
- **I see:** The run shows Failed with the cause "out of reach". The session is on the VM and the shell tab reattaches as in `real/03`. When the link returns, `connected` runs (once), but `arrive` does not run again by itself.
- **Expected outcome:** No typed text is replayed. Retry from the Inbox runs `arrive` once.
- **Status:** Not built (MACHINESCRIPTS T2). Reattach: Works (real daemon, stand-in `ssh`), `link_drop.rs`.

### The agent CLI is not installed on the new machine

- **When:** I move a session whose agent is not on the VM.
- **I do:** Start the move.
- **I see:** The move's own check step refuses it before any script runs, as it does today for a missing agent. No arrival or leaving script runs for a move that did not happen.
- **Expected outcome:** Scripts fire only for a move that took place. If the agent CLI vanishes after the check, `arrive` fails on its `send` with the kind named.
- **Status:** Code, no test for the check refusing a missing agent kind on the target (search `daemon/humatad/src/session_service/move_session.rs`). The script part is Not built (MACHINESCRIPTS T2).

### A move cut by a restart

- **When:** `humatad2` on this Mac restarts after the copy and before the switch.
- **I do:** Restart the daemon and let it recover the move.
- **I see:** The recovered move finishes. `leave` ran once before the restart or once after, never both. `arrive` runs once, when the move ends.
- **Expected outcome:** One run per (move, rule). A recovered move fires `session-arrived` (D681), and the run log, not memory, is how a second run is avoided.
- **Status:** Not built (MACHINESCRIPTS T2). Recovery of a cut move: Works (real daemon), fake machines, the cut-at-restart cases in `daemon/humatad/tests/move_session.rs`.

### Leaving the host

- **When:** the session leaves This Mac, the host.
- **I do:** Move `Warm up` from this Mac to the VM.
- **I see:** No `leave` run is listed for This Mac. `arrive` still runs on the VM.
- **Expected outcome:** Matches D681 (This Mac has no triggers). Whether the owner can script what happens as a session leaves the host is open question 1.
- **Status:** Not built (MACHINESCRIPTS; undecided).

### A fork lands on a machine

- **When:** I fork `Warm up` onto the VM instead of moving it.
- **I do:** Fork onto the VM.
- **I see:** Unknown. The spec names only moves (D681), so a fork landing is not a trigger.
- **Expected outcome:** Decided by open question 3. The owner's words said a session arriving on or leaving a machine.
- **Status:** Not built (MACHINESCRIPTS; undecided).

## Expected outcomes

- [ ] A reconnect runs `connected` rules once per link-up. A flap runs one at a time and never in parallel, and a heartbeat runs nothing.
- [ ] `session-leaving` runs on the old machine after the copy is verified, before the old agent stops, and never holds or fails the move.
- [ ] `session-arrived` runs on the new machine after the move ends and the agent resumed, and types into the moved session's own agent after SE-21 idle.
- [ ] A failing, timing out or offline script leaves the session exactly where the move put it: one agent, its work and conversation intact.
- [ ] Each failure shows on the machine page and as one Inbox item with Retry, and the passing runs show only on the machine page.
- [ ] A move recovered after a restart runs each script rule once.
- [ ] No secret or Humata folder is written by Humata's side of any run.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Link drops and returns | Works (real shell), component; stand-in `ssh` in the app | `roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay`; `link_drop.rs::a_dropped_link_refuses_typing_and_the_same_shell_takes_it_again_once_back` |
| 2. `connected` runs once | Not built (MACHINESCRIPTS T2) | none |
| 3. Scripts section on machine page | Not built (MACHINESCRIPTS T2, T5) | none |
| 4. Flap queues one run | Not built (MACHINESCRIPTS T2) | none |
| 5. Move to the VM | Works (real daemon), fakes; real Copilot Likely bug (B5, B7) | `move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes` |
| 6. `session-leaving` on the old machine | Not built (MACHINESCRIPTS T2) | none |
| 7. `session-arrived` types `/restart` | Not built (MACHINESCRIPTS T2) | B12 fixed (D754) |
| 8. Result on the move and the page | Not built (MACHINESCRIPTS T2, T5) | none |
| 9. One agent, folder freed after | Works (real daemon) end state; scripts Not built | `move_session.rs::moving_from_this_mac_frees_its_folder_only_after_it_runs_on_box_1` |
| Branch: arrival fails | Not built (MACHINESCRIPTS T2) | none |
| Branch: arrival times out | Not built (MACHINESCRIPTS T2) | none |
| Branch: leaving fails or hangs | Not built (MACHINESCRIPTS T2) | none |
| Branch: offline during arrival | Not built (MACHINESCRIPTS T2) | `link_drop.rs` for reattach only |
| Branch: agent not installed | Code, no test; scripts Not built | `move_session.rs` (move's own check) |
| Branch: move cut by restart | Not built (MACHINESCRIPTS T2) | `move_session.rs` cut-at-restart for the move |
| Branch: leaving the host | Not built (undecided) | none |
| Branch: a fork lands | Not built (undecided) | none |

## Gaps and likely bugs

- **Not built:** every script trigger (MACHINESCRIPTS T1-T5). `daemon/machines/src/sync_rules.rs` only parses TS-3 rules ("Nothing here runs a command") and nothing in `daemon/humatad/src` runs them.
- **Likely bug (seen live, B5, B7, B10):** a real Copilot session's resume after a stop or a move can fail or be killed, so `arrive` may have no live agent to type into until those are fixed (see `real/06`).
- **Fixed (B12, D754): `humata session type` presses Enter by default.** The `send` step presses Enter by default too.
- **Likely bug (seen live, B6):** a stopped session shows Idle, so `wait = "idle"` could type into a stopped session if idle alone is trusted. It must also check the agent is running.
- **Open questions the spec must answer:**
  1. Can the owner script a session leaving or arriving on This Mac (the host), given D681 gives it no triggers?
  2. Do script runs show on the move operation's steps, or only on the machine page and Inbox?
  3. Does a fork landing on a machine fire `session-arrived`, and does a fork's source fire `session-leaving`?
  4. Should `session-arrived` be allowed before the agent resumes (for example to prepare files), as a separate trigger?
  5. What keys the "once per move" dedup after a recovered move, and for how long is it kept?
  6. Does a `send` to a stopped or Idle-while-stopped session (B6) fail the step?
