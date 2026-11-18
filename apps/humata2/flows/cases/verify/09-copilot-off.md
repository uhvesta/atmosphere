# Turning Copilot off in Settings › Agents keeps it out of every new start and leaves the sessions it already runs alone

Layer: agents. Platforms: macOS (Settings, the pickers and the start refusals). The daemon and the CLI also run on Linux; that is not covered here. Web is not covered.

## Starting state

- This Mac is the host. Channel `Payments` has this Mac and `box-1`, with repo `acme/api`.
- Claude Code, Codex, Copilot and OpenCode are all on. The humata home has no `config/agents.toml` yet, so every kind is on and "Install when missing" is off (D720).
- A Copilot session `Fix retries` runs on this Mac. Its agent is up, and I have a writable attach open on its tab with the cursor in it.
- A Claude session `Add retries` runs on this Mac as well.
- The default agent is Claude Code. Settings is closed, and no new-session, fork, desk or review dialog is open.

## Reproduce headless

- Settings and the pickers, with no window on the screen:
  - `bazel test //apps/humata2/features/settings:agent_switches_test` runs the Settings reducer on the fake daemon: the toggle shows at once, the save, the refused save and the picker lists.
  - `bazel test //apps/humata2/features/app:new_session_test` runs `features/app/tests/new_session.rs`, which covers the default agent falling back when it is off.
  - `bazel test //apps/humata2/desktop:native_test --test_arg=turning_an_agent_off` clicks `settings-agent-enabled-copilot` in a headless app and checks the picker list.
  - To look at the card, run `AGENTSET_FRAME_OUT=<scratch folder> bazel test //apps/humata2/desktop:native_test --test_arg=the_agents_card_can_be_drawn_for_review`. It writes `settings-agents.png` into that folder, with Codex's install on and Copilot off. It asserts nothing, so read the image and check it by eye.
- Daemon refusals and listing, in process: `bazel test //apps/humata2/daemon/humatad:humatad2_unit_test --test_arg=agent_` runs the `agent_switches`, `agent_gate` and `agent_service` unit tests. They use a temp folder and never start an agent.
- A running daemon, throwaway: run `humatad2 serve` from `//apps/humata2/daemon/humatad:humatad2` with its own `HOME` (so the humata home is `<throwaway>/home/.humata2`), its own `--root` and `--socket`, and a `PATH` that holds only a throwaway bin folder and the system folders. Write `<throwaway>/home/.humata2/config/agents.toml` with `[agents.copilot]` and `enabled = false` before the start. Drive it with `//apps/humata2/cli:humata`. Check how `cli/tests/support` points the CLI at a socket before you run it, because no test in `cli/tests` covers `humata agents`.
- An existing Copilot session, throwaway: use the rig in `daemon/humatad/tests/common/mod.rs` (its `World` writes an agents file with a script agent, as `start_fresh.rs` does), and name that script agent `copilot`. The script stands in for the agent, so no real Copilot CLI runs and no login is used. If the rig cannot name the agent `copilot`, this step needs a new test in `daemon/humatad/tests/`.
- Not headless: the real Settings window (kept off the owner's screen), a real Copilot login, the real Copilot CLI, and a real `box-1`. The nearest headless equivalents are the native view test, the fake daemon, and the rig's script agent.

## Steps

1. **I do:** open Settings and go to the Agents section.  
   **I see:** an "Agents" card with four rows in this order: Claude Code, Codex, Copilot, OpenCode. Each row has an on switch. Under each row, an indented "Install when missing" row shows the install command in mono, the words "Never replaces one that's there", and an off switch.  
   **State and events:** the app reads the switches with `ListAgents` and `include_disabled` set. Nothing is written.  
   **Status:** Fakes only. `desktop/tests/native/agent_switches.rs::turning_an_agent_off_in_settings_takes_it_out_of_the_pickers` finds `settings-agent-enabled-copilot`. The install rows and their text are not asserted by a test. Their code is in `ui/src/settings.rs` (`agent_switches_block`).

2. **I do:** click the Copilot switch.  
   **I see:** the switch shows off at once. The "Install when missing" row under Copilot goes away. No error line appears.  
   **State and events:** the app sends `SetAgentEnabled` for `copilot` with `enabled` false. The reducer keeps the new value until the daemon answers.  
   **Status:** Fakes only. `features/settings/tests/agent_switches.rs::turning_an_agent_off_shows_at_once_and_is_saved`, against the fake daemon.

3. **I do:** wait for the save to finish, then close Settings and open it again.  
   **I see:** Copilot is still off. The other three are on.  
   **State and events:** the daemon has written `[agents.copilot]` with `enabled = false` into `config/agents.toml` in the humata home. The other lines and comments in the file are unchanged. The daemon's answer is the saved definition with `enabled` false.  
   **Status:** Code, no test. The write is `agent_switches::set`, called from `agent_service.rs` (`set_agent_enabled`). The unit tests run in a temp folder, not a running daemon: `agent_switches.rs::tests::set_keeps_comments_and_other_kinds` and `agent_service.rs::tests::set_agent_enabled_writes_the_switch_and_answers_the_definition`.

4. **I do:** quit humatad2 and start it again. Then open Settings › Agents.  
   **I see:** Copilot is still off.  
   **State and events:** the daemon reads the file afresh for each check (`agent_gate::switches`), so the switch survives a restart without a copy in memory.  
   **Status:** Code, no test. No test restarts a daemon with a switch set.

5. **I do:** open the New session dialog and open its agent picker.  
   **I see:** Claude Code, Codex and OpenCode, then Terminal. Copilot is not in the list.  
   **State and events:** the picker reads `SettingsState::offered_agents()`, which leaves Copilot out. No event is sent.  
   **Status:** Fakes only. The native test checks what `offered_agents()` returns, not the rows the picker draws. The rendered picker (`ui/src/overlay_new_session.rs`, line 47) has no test.

6. **I do:** open Fork on `Add retries`, open the desk New dialog and open the PR review dialog. In each, open the agent list.  
   **I see:** none of the three lists has Copilot.  
   **State and events:** each list reads `offered_agents()` (`ui/src/overlay_fork.rs`, line 184; `ui/src/overlay_desk_new.rs`, line 47; `ui/src/overlay_pr_review.rs`, line 123).  
   **Status:** Code, no test. No test renders these three lists.

7. **I do:** run `humata session start --agent copilot` for channel `Payments` on this Mac.  
   **I see:** the start is refused. The message says Copilot is turned off in Settings › Agents, so Humata won't start it. The fix says to turn Copilot on in Settings › Agents › Copilot, then start the session again. The CLI exits 7 (precondition). No session appears and no tab starts.  
   **State and events:** `FailedPrecondition`, from `agent_gate::enabled` at `session_service.rs` line 540. It runs before the start is recorded or relayed. No operation is written.  
   **Status:** Code, no test. `agent_gate.rs::tests::a_disabled_kind_is_refused` checks the gate in process, but no test sends a start to a daemon with a kind off. The exit code 7 is not checked for this error. It maps the kind in `cli/src/exit.rs`, and the only check is `cli/tests/exit_codes.rs`, which uses a fake daemon.

8. **I do:** run `humata session start --agent copilot --machine box-1`.  
   **I see:** the same refusal, from this Mac. `box-1` gets nothing.  
   **State and events:** the host checks the switch before it relays the start (`session_service.rs` line 540 runs before `begin_remote`). The machine never sees the call.  
   **Status:** Code, no test.

9. **I do:** fork `Fix retries`, the Copilot session, with the agent left empty, so the fork runs Copilot again.  
   **I see:** what the owner's outcome asks for is a refusal that names Settings › Agents › Copilot. What the code does is start the fork.  
   **State and events:** `check_fork` calls `agent_gate::enabled` only when the fork's agent differs from the parent's (`session_service/fork.rs` lines 370 to 373). The same agent is not checked. A new Copilot session starts.  
   **Status:** Likely bug. `daemon/humatad/src/session_service/fork.rs` lines 370 to 373. To confirm, turn Copilot off, fork `Fix retries` with no agent, and see whether a new Copilot session starts.

10. **I do:** fork `Add retries` with agent Copilot.
    **I see:** refused, with the fix that names Settings › Agents › Copilot.
    **State and events:** `fork.rs` line 372 refuses before the handover is made.
    **Status:** Code, no test.

11. **I do:** start a desk session with Copilot, from the desk New dialog or from the CLI's desk start. Then start a channel desk with Copilot.
    **I see:** each start is refused with the same kind of fix.
    **State and events:** `desk_service/start.rs` line 226 and `desk_service/channel_desk.rs` line 534 call `agent_gate::enabled`.
    **Status:** Code, no test.

12. **I do:** start a pull request review with Copilot.
    **I see:** refused, with the same fix.
    **State and events:** `session_service.rs` line 417 (`begin_review`) and `session_service/remote_review.rs` line 116 call `agent_gate::enabled` before anything is recorded.
    **Status:** Code, no test.

13. **I do:** with `Fix retries` still running, type a line into its tab, and open a read-only attach and a writable attach to the tab.
    **I see:** the line reaches the agent. Both attaches open.
    **State and events:** nothing checks the switch on type or attach. The gate is called only from start paths.
    **Status:** Code, no test.

14. **I do:** stop `Fix retries`, then resume it.
    **I see:** the session resumes on its own conversation with Copilot, and its tab comes back.
    **State and events:** the resume path does not call the gate. `grep -rn 'agent_gate::'` finds calls only in the start, review, fork and desk paths listed above.
    **Status:** Code, no test. No test resumes a session whose kind is off.

15. **I do:** move `Fix retries` to `box-1`, where Copilot is on.
    **I see:** the move completes and the session runs on `box-1`.
    **State and events:** a move does not check the switch.
    **Status:** Code, no test.

16. **I do:** turn Copilot back on in Settings.
    **I see:** the switch is on, its "Install when missing" row comes back (still off), Copilot is listed in every picker again, and a new start with Copilot is accepted by the switch check.
    **State and events:** the file now has `enabled = true` under `[agents.copilot]`, and nothing else in the file changes. The next start still needs the CLI to be found on the machine (case E-2).
    **Status:** Fakes only for the app side: `features/settings/tests/agent_switches.rs::turning_it_back_on_is_saved_too`. The daemon's write and the accepted start are Code, no test.

## Branches

### B1. The save fails and the switch goes back

**When:** the daemon refuses the change, for example because its file can't be written.
**I do:** click the Copilot switch.
**I see:** the switch shows off, then goes back to on. The error line "settings-agents-error" shows the daemon's message. The picker still lists Copilot.
**Expected:** the switch and the pickers show what the daemon holds, and the error says why.
**Status:** Fakes only. `features/settings/tests/agent_switches.rs::a_refused_change_goes_back_and_shows_why`, where the fake daemon is told to fail (`fail_next`). The real refusal from `agent_service.rs` (`switches_failed`, for an unwritable file) has no test.

### B2. The file has a typo

**When:** `config/agents.toml` is not valid TOML, for example `[agents.copilot` with no closing bracket.
**I do:** start a Copilot session, then click a switch.
**I see:** the start is not refused by a switch, because a bad file counts as every kind on, with no installs. The daemon logs one warning for the problem. A click on a switch fails with a message that names the file, says to fix the line it points to or delete the file, and says Humata leaves the file as it is. The file is unchanged.
**Expected:** a typo never blocks starts, and a change never rewrites a bad file.
**Status:** Code, no test end to end. `agent_switches.rs::tests::a_bad_file_is_an_error_and_set_leaves_it_alone` covers the file in process. The "counts as on" behavior has no test.

### B3. An agent tries to change a switch

**When:** the agent in `Add retries` calls `SetAgentEnabled` with its own token.
**I do:** the agent runs the call.
**I see:** refused as user-only.
**Expected:** only the person can change a switch.
**Status:** Code, no test. `agent_service.rs` calls `desk_service::user_only`, which refuses agent and script callers. The unit test covers only a peer.

### B4. No file at all

**When:** the humata home has no `config/agents.toml`.
**I do:** open Settings, then start a session with any kind.
**I see:** every kind is on, every "Install when missing" is off, and the start is not refused by a switch.
**Expected:** the defaults from D720.
**Status:** Code, no test end to end. `agent_switches.rs::tests::a_missing_file_gives_the_defaults` checks the defaults in process.

### B5. The host's file reaches box-1

**When:** `box-1` links up and its copy of the file differs from the host's.
**I do:** link `box-1` to the host.
**I see:** `box-1` takes the host's copy, and its starts use it.
**Expected:** the switches on `box-1` match the host's (D724).
**Status:** Code, no test. `daemon/machines/src/bundle.rs` has `a_bundle_gives_the_hosts_agent_switches_apart_from_the_others`, which covers the bundle only, not the link-up send in `stack_settings.rs`.

## Expected outcomes

- [ ] With Copilot off in Settings › Agents, the Settings switch shows off at once and stays off after a restart.
- [ ] The new-session, fork, desk and PR review pickers do not list Copilot, and a default agent that is off falls back to the first agent that is on.
- [ ] `humata agents ls` (the CLI command) does not list Copilot. `ListAgents` with `include_disabled` still does.
- [ ] A start, a fork with a different agent, a desk start and a PR review with Copilot are refused before anything runs, and the fix names Settings › Agents › Copilot.
- [ ] A fork that keeps Copilot as its agent is refused too.
- [ ] An existing Copilot session keeps its tab, takes typing, takes an attach, resumes, and moves.
- [ ] Turning Copilot back on restores it in every picker and accepts a new start.
- [ ] A refused save puts the switch back and shows the error.
- [ ] A bad `agents.toml` never blocks a start, and a switch change never rewrites it.
- [ ] An agent cannot change a switch.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Settings card and its rows | Fakes only | `desktop/tests/native/agent_switches.rs::turning_an_agent_off_in_settings_takes_it_out_of_the_pickers` (rows and install text not asserted) |
| 2. Switch goes off at once | Fakes only | `features/settings/tests/agent_switches.rs::turning_an_agent_off_shows_at_once_and_is_saved` |
| 3. Daemon writes the file | Code, no test | unit only: `agent_switches.rs::tests::set_keeps_comments_and_other_kinds`; `agent_service.rs::tests::set_agent_enabled_writes_the_switch_and_answers_the_definition` |
| 4. Still off after restart | Code, no test | none |
| 5. New session picker | Fakes only | `desktop/tests/native/agent_switches.rs` checks `offered_agents()`; rendered picker untested |
| 6. Fork, desk, PR review pickers | Code, no test | `ui/src/overlay_fork.rs:184`, `overlay_desk_new.rs:47`, `overlay_pr_review.rs:123` |
| 7. Start refused with fix | Code, no test | unit only: `agent_gate.rs::tests::a_disabled_kind_is_refused` |
| 8. Start on box-1 refused on host | Code, no test | `session_service.rs:540` |
| 9. Fork keeping Copilot | Likely bug | `session_service/fork.rs:370-373` |
| 10. Fork to Copilot refused | Code, no test | `session_service/fork.rs:372` |
| 11. Desk starts refused | Code, no test | `desk_service/start.rs:226`; `desk_service/channel_desk.rs:534` |
| 12. PR review refused | Code, no test | `session_service.rs:417`; `remote_review.rs:116` |
| 13. Type and attach still work | Code, no test | none |
| 14. Resume still works | Code, no test | none |
| 15. Move still works | Code, no test | none |
| 16. Turn back on | Fakes only (app) | `features/settings/tests/agent_switches.rs::turning_it_back_on_is_saved_too` |
| B1. Refused save goes back | Fakes only | `features/settings/tests/agent_switches.rs::a_refused_change_goes_back_and_shows_why` |
| B2. Typo in the file | Code, no test | `agent_switches.rs::tests::a_bad_file_is_an_error_and_set_leaves_it_alone` (file only) |
| B3. Agent changes a switch | Code, no test | `user_only` in `desk_service.rs`; unit covers a peer only |
| B4. No file | Code, no test | `agent_switches.rs::tests::a_missing_file_gives_the_defaults` (in process) |
| B5. Host's file reaches box-1 | Code, no test | `daemon/machines/src/bundle.rs` test (bundle only) |

## Gaps and likely bugs

- Likely bug: a fork that keeps a disabled kind is not refused (`session_service/fork.rs:370-373`, step 9). The owner's outcome says forking with Copilot is refused.
- Not built: an MCP listing of agents. `daemon/mcp/src/tools.rs` has no agents tool, so the "MCP listings" part of the outcome has nothing to filter.
- Not built: the machine page's list of agents a machine has (D726 leaves it for later).
- Likely bug (doc and CLI mismatch): the command is `humata agents ls` (`cli/src/args.rs`, `AgentsCommand::Ls`, no alias). AGENTSET and this case say `humata agents list`, which should fail as a usage error. Confirm by running it.
- Test gap: no daemon test (`daemon/humatad/tests/`) covers `SetAgentEnabled`, a refused start, a resume after the kind is turned off, or a restart reading the file. The daemon side is tested only in process.
- Test gap: the rendered fork, desk, PR review and new-session pickers are not checked. Only `offered_agents()` is.
- Test gap: the native test's second half re-reads in-memory state only. It does not check the daemon's file.
