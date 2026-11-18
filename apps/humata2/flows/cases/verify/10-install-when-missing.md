# With Install when missing off, a start of a missing CLI is refused with a fix; with it on, one start installs once, a found CLI is never replaced, and nothing else installs an agent

Layer: agents. Platforms: macOS (Settings and the start that installs). The daemon runs on macOS and Linux; Linux is not covered here. Web is not covered.

## Starting state

- This Mac is the host. Channel `Payments` has this Mac and `box-1`, with repo `acme/api`.
- A throwaway `humatad2` runs with its own `HOME`, its own `--root` and `--socket`, and a `PATH` that holds only a throwaway `bin` folder and the system folders. The `bin` folder has no agent CLI in it.
- The throwaway's agents file, given with `--agents`, has one custom kind, `fake`, whose program is `<throwaway>/bin/fake-agent`. That file does not exist yet. `fake` stands in for an agent, so no real agent CLI is involved.
- The throwaway's humata home has no `config/agents.toml`, so every kind is on and "Install when missing" is off (D720).
- Nothing on this Mac has an agent CLI installed by Humata. I check this before the run, because the built-in install commands are compiled in and would run the real installers.
- Settings is closed, and no new-session dialog is open.

## Reproduce headless

- Built-in kinds are never used for the install path. Their install commands (`curl … | bash` and `npm install -g …`) are compiled in and cannot be changed by the file, so a start of `codex` with its CLI missing and install on would run the real installer. Use the custom kind `fake` above, whose `install` command in the file is a fake.
- In process: `HOME=<throwaway home> bazel test //apps/humata2/daemon/humatad:humatad2_unit_test --test_arg=agent_gate --test_env=HOME=<throwaway home>`. The `--test_env` line matters: the installer runs `bash -lc`, which reads the login files of the HOME it is given, and the owner's HOME must not be read. The tests run real `bash` on a fake command: a script that sleeps, appends a line to a count file and writes a fake CLI, or one that prints `boom` and exits 3. No agent CLI runs.
- Settings and its switches: `bazel test //apps/humata2/features/settings:agent_switches_test` (TestStore on the fake daemon) and `bazel test //apps/humata2/desktop:native_test --test_arg=turning_an_agent_off` (headless clicks). Only the Install switch of a built-in kind is toggled here, and no start of that kind is made.
- A running daemon, throwaway: `humatad2 serve` from `//apps/humata2/daemon/humatad:humatad2`, with `HOME=<throwaway home>` (so the humata home is `<throwaway home>/.humata2`), `--root <throwaway>/root`, `--socket <throwaway>/s.sock`, `--agents <throwaway>/agents.json` (the `fake` kind), and `PATH=<throwaway>/bin:/usr/bin:/bin`. Drive it with `//apps/humata2/cli:humata` pointed at that socket. Check how `cli/tests/support` points the CLI at a socket first. The channel `Payments` and its repo can be made the way the rig's `World` makes them (`daemon/humatad/tests/common/mod.rs`).
- Evidence to capture: the CLI output and exit code for each start; the step list the CLI prints for the start; the count file's line count; `ls -l <throwaway>/bin` after each install; and the daemon log lines that say what was installed.
- The rig's fake-agent script is `#!/bin/sh` followed by `sleep 600`, so a start that installs can stay up. Keep the fake agent's script in `<throwaway>`, not in the repo.
- Not headless: the real installers, a real Node.js and npm, the real Settings window (kept off the owner's screen), and a real `box-1`. Nearest equivalents: the `fake` kind with a fake install command, the in-process tests, and the native view test.

## Steps

1. **I do:** open Settings › Agents with no `agents.toml` in the throwaway home.  
   **I see:** each agent row's "Install when missing" switch is off. The Codex row shows the install command `npm install -g @openai/codex` in mono.  
   **State and events:** the daemon reports `install_when_missing` false for each built-in kind, and `can_install` true.  
   **Status:** Code, no test end to end. `agent_switches.rs::tests::a_missing_file_gives_the_defaults` checks the defaults in process. The Settings view shows the fake's defaults in `fake/agent_switches.rs` (`defaults()`), which is Fakes only (`features/settings/tests/agent_switches.rs::the_switches_load_in_the_daemons_order`).

2. **I do:** from this Mac's shell, run `humata session start --agent fake` for channel `Payments`, with `fake` not on the throwaway `PATH`.  
   **I see:** the start is refused with exit code 7. The message says `fake` isn't installed on this machine and names the program path that was not found. The fix says to install it on this machine yourself, using the file's install command, or to turn on "Install when missing" under `fake` in Settings › Agents, then start again. No session appears and no tab starts.  
   **State and events:** `FailedPrecondition`, from `agent_gate::check` at `session_service.rs` line 575, before anything is recorded. Nothing is installed.  
   **Status:** Code, no test end to end. `agent_gate.rs::tests::a_missing_cli_with_install_off_is_missing` checks the refusal in process. The fix is wrong for this kind, though. See the likely bug below: the Settings pane has no row for a custom kind.

3. **I do:** open Settings › Agents and turn on "Install when missing" for Codex. (Built-in kind, so it is safe here: no Codex start is made.)  
   **I see:** the Codex install switch is on at once. Its row still shows the command and "Never replaces one that's there". Nothing else changes.  
   **State and events:** `SetAgentEnabled` for `codex` with `install_when_missing` true. The daemon writes `[agents.codex]` with `install_when_missing = true` and keeps the other lines.  
   **Status:** Fakes only for the app side: `features/settings/tests/agent_switches.rs::install_when_missing_changes_only_its_own_switch`. The daemon's write is Code, no test end to end; `agent_switches.rs::tests::set_appends_a_missing_table_and_keeps_the_rest` covers it in process.

4. **I do:** for the custom kind, edit the throwaway's `config/agents.toml` by hand to add `[agents.fake]` with `install_when_missing = true` and `install = "<fake install command>"`. The fake command makes `<throwaway>/bin/fake-agent` executable and appends a line to `<throwaway>/install-count`. No restart.  
   **I see:** nothing yet. The Settings pane has no row for `fake`, because the app's agent model has only built-in kinds.  
   **State and events:** the daemon reads the file afresh for each check (`agent_gate::switches`), so the new file takes effect at the next start.  
   **Status:** Code, no test. No test changes the file under a running daemon and then starts a session.

5. **I do:** run `humata session start --agent fake` once.  
   **I see:** the start shows an "Installing fake" step first, then "Saving the session", "Cloning" for each repo, "Starting the agent" and "Checking it started". The CLI returns the started session, or the start's own error if the fake agent fails to launch after the install. `<throwaway>/bin/fake-agent` now exists and is executable. The count file has one line.  
   **State and events:** the install runs inside the start (`session_service.rs` lines 575 to 660), and the start waits for it. The install step is Done before "Saving the session" begins. The install runs before `write_record`, so the session record is written only after the install.  
   **Status:** Fakes only. `agent_gate.rs::tests::two_starts_at_once_install_once` covers the single install with a fake command in process. The step names and their order are not checked by a test.

6. **I do:** delete `<throwaway>/bin/fake-agent` and the count file. Then run two `humata session start --agent fake` at the same time, from two shells.  
   **I see:** both starts go on. The count file has exactly one line. The second start waits for the first install, then starts without installing again.  
   **State and events:** one install per kind at a time. The per-kind lock in `agent_gate.rs` (`install_lock`) holds the second start until the first finishes, and then the second start finds the CLI.  
   **Status:** Fakes only. The in-process test `two_starts_at_once_install_once` runs two `ready` calls with `tokio::join!`, not two RPCs. Two real RPCs at once are not tested.

7. **I do:** delete the fake CLI and set the fake install command to fail: it prints `boom` to stderr and exits 3. Run `humata session start --agent fake`.  
   **I see:** the start is refused with exit code 7. The "Installing fake" step shows Failed. The message says installing `fake` with the command failed, then shows the last lines of the output, which include `boom`. The fix says to run the command yourself on this machine (npm needs Node.js first, and the CLI must end up on the daemon's PATH), or turn "Install when missing" off and install it your way. No session record is written.  
   **State and events:** `FailedPrecondition` from `GateError::InstallFailed`, returned by the waiting start through the ready channel (`session_service.rs` lines 644 to 652). The operation is marked failed with the same message.  
   **Status:** Fakes only. `agent_gate.rs::tests::a_failing_install_is_install_failed` checks that the tail holds `boom`. The step's Failed state, the fix text and the absence of a record are not tested.

8. **I do:** with the failing install still set, run two starts at once.  
   **I see:** the owner's outcome says one install. What the code does is run the install once for the first start and again for the second, after the first has failed. Both starts are refused, and the count file has two lines.  
   **State and events:** `install_now` holds the per-kind lock only while it installs. The second start gets the lock after the first fails, finds the CLI still missing, and runs the command again.  
   **Status:** Likely bug. `daemon/humatad/src/agent_gate.rs` lines 164 to 190. To confirm, run step 8 and count the lines in the count file: two, where one is expected.

9. **I do:** create a `fake-agent` that is executable in `<throwaway>/bin`, then run `humata session start --agent fake` with the install switch on and the install command set to append to the count file.  
   **I see:** the start goes on with no install step. The count file is not created.  
   **State and events:** `check` finds the CLI by a `PATH` lookup and returns found, so the install command is never run.  
   **Status:** Fakes only. `agent_gate.rs::tests::a_found_cli_is_ready_and_never_installed` checks it in process with a real executable file.

10. **I do:** put a `fake-agent` in a folder that only the throwaway's login profile adds to `PATH` (a line in its `.bash_profile`), and not in the daemon's `PATH`. Turn the install switch on, and start.
    **I see:** what the owner's rule asks for is that the CLI is found and never installed over. What the code does is treat the CLI as missing and run the install command, which installs over the existing CLI's folder.
    **State and events:** `is_found` looks only at the daemon's `PATH` (`agent_gate.rs` lines 208 to 214). It does not use the login `PATH` that D727 says to use.
    **Status:** Likely bug. `daemon/humatad/src/agent_gate.rs` lines 208 to 214, with D727 in `DECISIONS.md`. To confirm, check the count file after this step: it has one line, where none is expected.

11. **I do:** on a machine where the install switch is on and the start is for an existing session, resume that session when its CLI has been removed.
    **I see:** the resume is not refused by the switch and no install runs. The launch fails, as it would have before this work.
    **State and events:** resume does not call `check` or `ready`. The only calls are in the start (`session_service.rs` line 575) and the review path (`session_service.rs` line 424).
    **Status:** Code, no test. No test covers a resume after the CLI has been removed.

12. **I do:** start a fork, with a different agent, to `fake` on a throwaway where `fake` is not found, with install on.
    **I see:** the fork is not refused with the install fix, and nothing installs. The launch fails, with the launch error the holder reports.
    **State and events:** `check_fork` calls `agent_gate::enabled` only (`session_service/fork.rs` line 372). It never calls `check` or `ready`, so the fork skips the found and install step.
    **Status:** Likely bug. `daemon/humatad/src/session_service/fork.rs` line 372. To confirm, run the fork and check that the count file is missing and the refusal names no install.

13. **I do:** start a desk session with `fake` on the same throwaway.
    **I see:** the same as step 12: no install step, no install fix, and the launch fails later.
    **State and events:** `desk_service/start.rs` calls `agent_gate::enabled` in `agent()` only, and `starter.start_desk` never calls `check` or `ready`.
    **Status:** Likely bug. `daemon/humatad/src/desk_service/start.rs` line 226, with no `check` call in the desk start path.

14. **I do:** from the Humata side, check that nothing else installs an agent CLI: grep the tree for install commands, and read the Migrate path for `box-1`.
    **I see:** the only install commands are in `agent_switches.rs` (`builtin_install_command`) and the only installer is `agent_gate::install`. The other "install" text tells a person to run `brew install gh`, which is the GitHub CLI and not an agent. Humata's own tools are staged on a Migrate: `humatad2`, `humata2_holder`, `humata` and `stack` (`daemon/humatad/tests/upgrade_box.rs`, which stages `.humatad2.next` and the rest).
    **State and events:** the agent CLIs are never part of the bundle (`InstalledFile::ALL`, `daemon/machines/src/install.rs` line 54, has the four Humata files only).
    **Status:** Works (real daemon, fake box) for the Migrate path: `daemon/humatad/tests/upgrade_box.rs::approving_migrate_stages_the_release_over_ssh_and_the_machine_comes_back_at_it`. The setup and link-up installs are Code, no test. The "nothing else installs" part is by grep, not by a test.

## Branches

### B1. The install runs past its 10-minute deadline

**When:** a fake install command sleeps longer than the deadline.
**I do:** start `fake`, with the install command set to `sleep 700`.
**I see:** after 10 minutes the start is refused, with "it did not finish within 10 minutes" and the command.
**Expected:** the start is refused with that message, and the install's child processes do not keep running.
**Status:** Code, no test. The timeout drops the `wait_with_output` future, and `kill_on_drop` kills only the `bash` process. A child process such as `npm` or `curl` started by the command would keep running. To confirm, use an install command that starts a child that sleeps, then check for it after the refusal.

### B2. The install runs, but the CLI is still not found

**When:** the install command finishes with exit 0, but the CLI lands in a folder the daemon's `PATH` does not reach.
**I do:** start `fake`, with a command that writes its CLI to a folder not on the throwaway `PATH`.
**I see:** the start is refused, and the message says the command finished but `fake` is still not found on the daemon's PATH. The fix says to run the command yourself and put the CLI on the daemon's PATH.
**Expected:** the install counts as failed, and the start is refused with that fix.
**Status:** Code, no test. `install_now` builds the message in `agent_gate.rs`. No test covers this branch.

### B3. The daemon restarts during an install

**When:** humatad2 stops while the install runs (step 5 with a slow command).
**I do:** restart the daemon, then start `fake` again.
**I see:** the new daemon checks the CLI again. If it is still missing, it installs again. Nothing is left half-installed by the daemon itself.
**Expected:** the next start decides from the CLI's state on disk, and the operation that was running is shown as failed, not as still running.
**Status:** Code, no test. Whether the interrupted operation is shown as failed after the restart is not checked.

### B4. The file's install command for a built-in kind

**When:** the file gives `[agents.codex]` an `install` command, which differs from the built-in one.
**I do:** start Codex with its CLI missing and install on.
**I see:** the built-in command runs, not the file's. (A real run would be the owner's, so this is checked in process only.)
**Expected:** the file cannot change a built-in install command (D722).
**Status:** Code, no test end to end. `agent_switches.rs::tests::the_file_cannot_override_a_builtin_install_command` checks it in process.

### B5. Install is turned off again

**When:** the owner turns "Install when missing" off for `codex` and starts it with the CLI missing.
**I do:** turn the switch off, then start Codex.
**I see:** refused, with the fix to install it yourself or turn the switch on.
**Expected:** no install runs once the switch is off.
**Status:** Code, no test end to end. `agent_gate.rs::tests::a_missing_cli_with_install_on_asks_for_an_install_and_a_found_one_does_not` checks the install on case in process, and the install-off case is covered by `a_missing_cli_with_install_off_is_missing`.

## Expected outcomes

- [ ] With no file, every kind is on and "Install when missing" is off.
- [ ] A start of an enabled kind whose CLI is missing is refused with exit 7 and a fix that names installing it there or turning the switch on.
- [ ] With the switch on, one start installs once, then starts, and the install shows as a step of that start.
- [ ] Two starts at once run one install, when the install succeeds.
- [ ] A failed install refuses the start, and the refusal shows the command and the last lines of its output.
- [ ] A CLI that is found is never installed over.
- [ ] Humata's own tools are staged on a Migrate and never depend on the switches.
- [ ] Nothing installs a third-party agent CLI except the Install when missing path.
- [ ] A fork or a desk start of a missing CLI is refused with the same fix, or it installs under the same switch.
- [ ] Resume and rebuild never install.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Defaults in Settings | Code, no test end to end | `agent_switches.rs::tests::a_missing_file_gives_the_defaults`; `fake/agent_switches.rs` `defaults()` |
| 2. Start refused with fix, exit 7 | Code, no test end to end | `agent_gate.rs::tests::a_missing_cli_with_install_off_is_missing`; `session_service.rs:575` |
| 3. Install switch for a built-in kind | Fakes only (app); daemon Code, no test end to end | `features/settings/tests/agent_switches.rs::install_when_missing_changes_only_its_own_switch`; `agent_switches.rs::tests::set_appends_a_missing_table_and_keeps_the_rest` |
| 4. File edit read at next start | Code, no test | none |
| 5. One start installs, then starts | Fakes only | `agent_gate.rs::tests::two_starts_at_once_install_once` (install in process); step names untested |
| 6. Two starts, one install | Fakes only | `agent_gate.rs::tests::two_starts_at_once_install_once` (in process, not two RPCs) |
| 7. Failed install refuses with tail | Fakes only | `agent_gate.rs::tests::a_failing_install_is_install_failed` (tail only) |
| 8. Failed install, two starts | Likely bug | `agent_gate.rs:164-190` |
| 9. Found CLI never installed over | Fakes only | `agent_gate.rs::tests::a_found_cli_is_ready_and_never_installed` |
| 10. CLI on login PATH only | Likely bug | `agent_gate.rs:208-214`; D727 |
| 11. Resume never installs | Code, no test | `session_service.rs:575` and `:424` are the only calls |
| 12. Fork of a missing CLI | Likely bug | `session_service/fork.rs:372` |
| 13. Desk start of a missing CLI | Likely bug | `desk_service/start.rs:226` |
| 14. Nothing else installs | Works (real daemon, fake box) for Migrate; Code, no test otherwise | `daemon/humatad/tests/upgrade_box.rs::approving_migrate_stages_the_release_over_ssh_and_the_machine_comes_back_at_it`; grep |
| B1. Install past its deadline | Code, no test | `agent_gate.rs` `install` (timeout) |
| B2. Installed but not on daemon PATH | Code, no test | `agent_gate.rs` `install_now` |
| B3. Daemon restarts during install | Code, no test | none |
| B4. File cannot change a built-in command | Code, no test end to end | `agent_switches.rs::tests::the_file_cannot_override_a_builtin_install_command` |
| B5. Install turned off again | Code, no test end to end | `agent_gate.rs::tests::a_missing_cli_with_install_off_is_missing` |

## Gaps and likely bugs

- Likely bug: a failed install is run again by the second of two waiting starts, so two starts give two installs (`agent_gate.rs` lines 164 to 190, step 8). The owner's rule is one install.
- Likely bug: the refusal's fix for a custom kind names "Install when missing under <kind> in Settings › Agents", but the Settings pane shows only built-in kinds (`clients/live/src/agent_switches.rs` `switch_of` returns nothing for a custom kind). The owner cannot reach that switch, and it can only be set in the file. Confirm by opening Settings with a custom kind in the file.
- Likely bug: "found" uses the daemon's `PATH` (`agent_gate.rs` lines 208 to 214), not the login `PATH` D727 says to use. A CLI that only the login `PATH` reaches is treated as missing, and the install command runs over it. The module's own note says it is the daemon's `PATH` on purpose, but that contradicts D727 and the owner's "never replaces one that is there".
- Not built: the login `PATH` lookup with its 10-second deadline (D727, AGENTSET design "Found"). The design calls for `$SHELL -lc 'command -v <cli>'`.
- Likely bug: fork and desk starts never run `check` or `ready`, so a missing CLI there is not refused with the fix and is not installed (`session_service/fork.rs:372`, `desk_service/start.rs:226`, steps 12 and 13). The design's "on the machine" list names only the local StartSession branch and the MACHINESCRIPTS start.
- Likely bug (low): on the 10-minute deadline, only the `bash` child is killed (`kill_on_drop`), so the child processes of the install keep running (B1).
- Minor: the failure tail joins stdout and then stderr, so the last 20 lines may not be in the order the command printed them (`agent_gate.rs`, `install`).
- Not built: an "Installing" step for review starts. The review path calls `ready`, which installs inline with no step.
- Test gap: no daemon test (`daemon/humatad/tests/`) covers the install, the refusal, or two RPCs at once. All the install evidence is in process.
- Test gap: the Migrate test covers the four Humata files and the bundle, not the setup or link-up install of them.
