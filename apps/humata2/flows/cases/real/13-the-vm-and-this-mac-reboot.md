# After the VM powers off and after this Mac reboots, each session resumes on its exact conversation, and a restart report names each outcome

Real. Platforms: macOS (this Mac and the app). The VM is Debian on arm64. Paths are relative to `apps/humata2`.

This is a shutdown, not a dropped link. The agents die with the machine, and Humata brings each back with the agent's own resume command on the same conversation id. For a link that drops while everything keeps running, see [real/12](12-the-link-drops-and-every-agent-resumes.md). For the owner's own "Stop for the night", see [flows/08](../../08-stop-a-machine-for-the-night.md) and [macro/09](../macro/09-end-of-day-stop-and-start.md). Other context: [flows/07](../../07-after-a-reboot-or-upgrade.md), [macro/10](../macro/10-reboot-or-power-cut-mid-work.md), [micro/18](../micro/18-terminals-after-a-host-reboot.md), [agents/19](../agents/19-an-agent-across-a-restart.md). Agent kinds and the cheapest-model keystrokes: [real/05](05-copilot-on-the-cheapest-model-on-this-mac.md), [real/07](07-codex-on-the-cheapest-model-on-this-mac.md).

This case is for a run by hand or a later test. No test powers off a machine under a real agent CLI through `humatad2`. Each step says which half is proven.

## Starting state

- A throwaway Lima VM, `h2-reboot-1`, is a machine in the app (see the test setup). It is not `h2-terminal-c1`.
- Channel `acme/api` has this Mac and `h2-reboot-1`, with a clone of the repo folder `hello/` on each.
- Six sessions run, each on its cheapest model: `claude-mac`, `codex-mac` and `copilot-mac` on this Mac, and `claude-vm`, `codex-vm` and `copilot-vm` on the VM. Codex and Copilot are custom kinds from an `--agents` JSON file (REALFLOW B1).
- Every task prompt names the repo folder, because Humata does not tell an agent where the repo is. Example: "In the hello/ folder, add the line `Before the reboot.` to README.md".
- Each session finished one turn. `claude-vm` and `copilot-mac` are partway through a second turn.
- I wrote down each session's conversation id, its first turn's text, and the time.

## Test setup

- **A dedicated throwaway VM, never the shared fixture.** `AGENTS.md` and `terminal/platform_test/SSH_DEMO.md` forbid stopping or restarting `h2-terminal-c1`. Create `h2-reboot-1` the way `daemon/machines/tests/lima.rs::a_lima_vm_is_created_set_up_stopped_and_deleted` makes `humata2-machines-e2e`, or the way `terminal/host/src/agent_resume_vm_tests.rs` makes its own isolated Lima guest. Delete it after the run. Never start, stop or delete an unrelated Lima instance.
- The VM needs memory for the CLIs. 0.75 GiB is tight for Copilot (Node-based). Give the throwaway VM 2 GiB or run fewer agents.
- Holders on `h2-terminal-c1` live at `/tmp/h2-terminal-holder`. `/tmp` does not survive a reboot. On the throwaway VM, Humata's own install goes to `~/.humata2/bin` (TS-1), which does. Do not provision with `terminal-ssh-demo.sh` here, or the holder is gone after the reboot and the run proves nothing.
- Each agent CLI has its own home (`COPILOT_HOME`, `CODEX_HOME`, `CLAUDE_CONFIG_DIR`) in the run's temp folder, on this Mac and on the VM. Never the owner's `~/.copilot`, `~/.codex`, `~/.claude`, `~/.humata2` or SSH keys.
- **Needs the owner's login** (Copilot device login, Codex `login`, Claude login). Tokens land only in the temp homes. Alternative with no login: the loopback model server in `terminal/provider/tests/*_resume.py`. It proves resume, not a real model.
- **Needs a free screen** for steps that open the app. A Mac reboot ends the owner's own session, so the owner must be at the Mac and agree. Headless: the `humata` CLI and `desktop/tests/live_flows/restart.rs` style tests.
- The VM's own daemon must come back by itself at boot. The service is a systemd user unit with lingering, or a crontab `@reboot` entry (`daemon/machines/src/service.rs`).
- Never run bazel, cargo or git while writing this case.

## Steps

### Part A: the VM powers off

1. **I do:** with the six sessions at the state above, power the VM off hard from the host: `limactl stop --force h2-reboot-1` (the throwaway only). **Needs a free screen** to watch the app.
   **I see:** within seconds the machine shows offline, and `claude-vm`, `codex-vm` and `copilot-vm` show offline. The three sessions on this Mac keep running.
   **State and events:** the host does not end the VM's sessions. Their records stay, with the agent state kept for a resume.
   **Status:** Works (real daemon), fake box: `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice` (its daemon and holders die, the sessions show offline); `daemon/humatad/tests/stop_box.rs` kills a box's daemon with its `humatad2_stop_cut` helper, which is not itself a test. A real Lima power-off through the app: Code, no test.

2. **I do:** start the VM again: `limactl start h2-reboot-1`. Wait for it to boot.
   **I see:** the machine returns online by itself. I do not press Reconnect.
   **State and events:** the VM's `humatad2` starts at boot and the host redials. The old holders and agents are gone, because the VM's memory was lost.
   **Status:** Works (real shell), terminal component only: `terminal/host/src/agent_resume_vm_tests.rs::whole_guest_power_loss_preserves_disk_identity_and_requires_confirmed_new_boot` (own isolated Lima guest, forced off, new boot id, synced conversation and installed files intact, no automatic resume during the outage). It runs a strict disk-backed fixture, not a real vendor CLI, and it is not through `humatad2`. The service starting at boot: Works (real daemon), unit level: `daemon/machines/src/service.rs::setsid_starts_the_daemon_and_adds_one_boot_entry` and `::a_user_service_needs_lingering`. Real boot of the app-installed service: Code, no test.

3. **I do:** look at the three VM sessions after the link is back.
   **I see:** each is running again in its conversation. `claude-vm` shows the first turn's text. No session starts a new conversation.
   **State and events:** the VM's daemon revives each agent tab with its resume command and the saved conversation id, in its own folder. The hook state from before is cleared (D169).
   **Status:** Works (real daemon), fake agent: `daemon/humatad/tests/sessions.rs::a_lost_holder_is_resumed_in_its_conversation_after_a_restart`, `daemon/humatad/tests/turns.rs::a_resume_forgets_what_the_hooks_said`, and for the box's own restart `daemon/humatad/tests/start_resume_failed_remote.rs::a_resume_the_boxs_restart_failed_is_taken_over_and_runs_one_agent` (one agent ever runs). Power cut at each fault point on a box: `daemon/humatad/tests/power_cut_machine.rs` (generated tests, fake box). Real Claude, Codex and Copilot on the VM: Code, no test.

4. **I do:** for `copilot-vm`, open the tab and run `/resume`-style checks: ask "What was the first thing I asked?"
   **I see:** the agent answers from the first turn, so the conversation is the same one, not a fresh one.
   **State and events:** the conversation id is unchanged. The Copilot resume looks under the CLI's home.
   **Status:** Likely bug (seen live, B5): resume looks for the conversation under `--root` instead of the CLI home, so a resumed Copilot or Codex session can't find it. Seen on this Mac in the REALFLOW run, with the real CLI. The real CLI on the VM: Code, no test. Fake-agent equivalent: Works (real daemon), `desktop/tests/live_flows.rs::after_a_reboot_the_session_is_running_again_in_its_conversation`.

5. **I do:** for `codex-vm`, check the id the resume used.
   **I see:** the resume names the exact id Codex wrote to its dated rollout file.
   **State and events:** Humata must know Codex's real session id. It only knows its own.
   **Status:** Likely bug (seen live, B9): Codex resume fails because Humata never learns Codex's real session id (the id is in a dated rollout path). The Codex adapter resumes by exact full id (`daemon/conversation/src/codex.rs`). Fixture proof of the adapter: `daemon/humatad/tests/continue_in.rs` (Fakes only for the CLI).

6. **I do:** stop `copilot-vm` by hand, then start it again.
   **I see:** the session resumes, and the agent stays up.
   **State and events:** a start after a stop is a resume, not an interruption.
   **Status:** Likely bug (seen live, B7): resume after stop is treated as an interruption and kills the correctly resumed agent (seen on Copilot, this Mac). Works (real daemon), fake agent: `daemon/humatad/tests/stop_sessions.rs::a_stop_ends_every_agent_and_shell_the_sessions_started_and_writes_no_restart_report` for the stop, and `macro/09` for the start. Confirm by stopping and starting with the real CLI.

7. **I do:** open the machine's restart report.
   **I see:** one line for each session: brought back, or not brought back with the reason. The report names `claude-vm`, `codex-vm` and `copilot-vm` by title.
   **State and events:** the VM's start writes a report. The host shows it. `humata doctor restarts` can repair a session that did not come back.
   **Status:** Works (real daemon) for the report's content: `daemon/humatad/tests/restart.rs::the_restart_report_has_what_the_daemon_logged_before_it_stopped` and `::a_shell_that_cant_come_back_is_one_line_and_the_others_still_do`. Drawing the report in the app: Not built, no milestone (macro/10 step 9; OVERVIEW says the start's report is not pushed to the app). The toast and the card's Try again button: Works (real daemon): `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt`.

### Part B: this Mac reboots

8. **I do:** with the VM running and the six sessions up, reboot this Mac. **Needs a free screen** and the owner's agreement.
   **I see:** after login, the app opens. Humata's daemon on this Mac starts at login. The three Mac sessions are running again in their conversations. The three VM sessions never stopped, and show live again after the daemon redials the VM.
   **State and events:** the Mac's agents and holders died. The daemon revives each tab with the agent's resume command and the same conversation id. The VM's agents kept running through the Mac's reboot, because they do not depend on this Mac.
   **Status:** Works (real daemon, simulated reboot): `desktop/tests/live_flows.rs::after_a_reboot_the_session_is_running_again_in_its_conversation` and `daemon/humatad/tests/sessions.rs::a_lost_holder_is_resumed_in_its_conversation_after_a_restart` (the holders and the daemon are killed, not a reboot). Whole-computer power loss on this Mac: Code, no test. Starting at login: Code, no test with real launchd (`desktop/install/src/launchd.rs`; the installer tests use a fake: `desktop/install/tests/install.rs::unloaded_agent_is_loaded_again`). The VM side after this Mac's reboot: Works (real shell), component only, `terminal/session/tests/ssh.rs::ssh_terminals_keep_their_process_after_transport_and_app_restart`.

9. **I do:** open `claude-mac` and `copilot-mac`, the two that were partway through a second turn.
   **I see:** each runs in its conversation, with the first turn there. The interrupted turn is not run again, and nothing is typed on its own.
   **State and events:** the resume starts the agent with its own resume command. It does not re-send the unfinished prompt.
   **Status:** Works (real daemon) for the resume: `daemon/humatad/tests/sessions.rs::a_lost_holder_is_resumed_in_its_conversation_after_a_restart`. "The interrupted turn is not repeated": Code, no test (agents/19, branch "mid-turn"). Unfinished turns are provider-defined (`terminal/lab/HANDOFF.md`).

10. **I do:** run `humata whoami` in a resumed tab, and read each session's conversation with `humata session read`.
    **I see:** whoami works with a new token. The old token is refused. The read shows the history.
    **State and events:** a tab brought back by a start gets a fresh token (`daemon/humatad/src/restart.rs`).
    **Status:** Code, no test for the new token (agents/19 step 9). Likely bug (seen live, B3/B4): `humata session read` is empty for Copilot.

11. **I do:** open the restart report on this Mac.
    **I see:** each Mac session named with its outcome, brought back or not.
    **Status:** Works (real daemon) for the content and toast, as step 7. Drawing it: Not built, no milestone.

## Comparison with Stop for the night

| | Stop for the night (flows/08) | Power-off or reboot (this case) |
| --- | --- | --- |
| Who ends the agents | Humata, on request, gracefully first, then a hard kill if a shell ignores hang-up | The power loss. Humata does not end them |
| Is resume automatic | No. The sessions show Stopped and stay stopped, even across a reboot, until I press Start sessions | Yes. The machine's daemon starts and revives each session's agent |
| What the report says | A stop id and reason. No restart report is written at the stop. Starting writes a report naming the stop | A restart report names each session as brought back or not, with the reason |
| Pending Inbox requests | Withdrawn with reason MachineStopped | A pending permission card is not saved. The hook ends with its agent. A saved request is kept; a deferred call is withdrawn |
| Evidence | `stop_sessions.rs`, `stop_box.rs` | `sessions.rs`, `restart.rs`, `power_cut_*.rs` |

**Status:** Works (real daemon), fake agent, for each cell. See `daemon/humatad/tests/stop_sessions.rs::a_daemon_killed_and_started_again_while_stopped_stays_stopped_and_starts_nothing` for "stays stopped". The pending-request row is from `agents/19` and `flows/08`.

## Branches

### A session's conversation file was not synced before the power loss
When: the VM lost power before the agent's conversation file reached disk.
I see: that session shows Exited with a "couldn't resume" line and a Try again, and Start fresh. Nothing new starts on its own.
**Status:** Works (real daemon): `daemon/humatad/tests/sessions.rs::a_reboot_that_lost_the_conversation_says_the_resume_failed`. The VM component test says synced conversations survive and unsynced data is not recovered (`terminal/lab/HANDOFF.md`).

### The VM boots but its daemon does not start
When: the service is off, or the machine has no lingering.
I see: the machine stays offline, with its sessions offline, and the machine page says what to run. Nothing starts a second agent.
**Status:** Code, no test through a real boot. `daemon/machines/tests/setup.rs::without_lingering_an_admin_command_or_setsid_is_offered` covers the offer (fake ssh).

### Both power off at once
When: the Mac and the VM lose power together.
I see: after both start, nothing is lost: each item is as it was, or finished, or the report names it.
**Status:** Works (real daemon), simulated cut: `daemon/humatad/tests/power_cut_both.rs`. Real: Code, no test.

### The VM powers off while this Mac is asleep
When: the Mac is closed, so it cannot see the cut.
I see: on wake, the machine shows offline until the VM is up, then online.
**Status:** Code, no test.

### An attach is refused during the resume
When: I open a tab while its session is resuming.
**Status:** Likely bug (seen live, B10): attach refused while resuming.

### The shared fixture
When: someone points this case at `h2-terminal-c1`.
**Status:** Not run, by rule. It would delete the holder in `/tmp` and break other runs.

## Expected outcomes

- [ ] After a hard power-off and a start, the VM's machine returns online without a button press.
- [ ] Each VM and Mac session runs again on its exact conversation id, with its first turn there. None starts a fresh conversation.
- [ ] No second agent runs for a session.
- [ ] The interrupted turn is not repeated, and no text is typed into an agent on its own.
- [ ] A restart report names each session and its outcome.
- [ ] A session whose conversation was lost says it could not resume and offers Try again and Start fresh.
- [ ] After "Stop for the night", sessions stay stopped. After a power loss they come back. The two paths are not mixed.
- [ ] The shared `h2-terminal-c1` VM and the owner's agent homes were never touched.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. VM powers off | Works (real daemon), fake box; real Lima Code, no test | `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice` |
| 2. VM boots, link returns | Works (real shell), component only; service at boot Works (real daemon), unit level | `terminal/host/src/agent_resume_vm_tests.rs::whole_guest_power_loss_preserves_disk_identity_and_requires_confirmed_new_boot` |
| 3. Sessions resume | Works (real daemon), fake agent | `daemon/humatad/tests/sessions.rs::a_lost_holder_is_resumed_in_its_conversation_after_a_restart` |
| 4. Same conversation | Likely bug (seen live, B5); fake agent Works (real daemon) | `desktop/tests/live_flows.rs::after_a_reboot_the_session_is_running_again_in_its_conversation` |
| 5. Codex exact id | Likely bug (seen live, B9) | REALFLOW run |
| 6. Stop then start | Likely bug (seen live, B7) | REALFLOW run |
| 7. Restart report | Works (real daemon) for content; drawing Not built | `daemon/humatad/tests/restart.rs::the_restart_report_has_what_the_daemon_logged_before_it_stopped` |
| 8. This Mac reboots | Works (real daemon), simulated reboot; real Code, no test | `desktop/tests/live_flows.rs::after_a_reboot_the_session_is_running_again_in_its_conversation` |
| 9. Mid-turn not repeated | Works (real daemon) for resume; repeat Code, no test | `daemon/humatad/tests/sessions.rs::a_lost_holder_is_resumed_in_its_conversation_after_a_restart` |
| 10. New token, read history | Code, no test; Likely bug (seen live, B3/B4) | `daemon/humatad/src/restart.rs` |
| 11. Mac restart report | Works (real daemon) for content; drawing Not built | same as step 7 |
| Comparison with Stop for the night | Works (real daemon), fake agent | `daemon/humatad/tests/stop_sessions.rs::a_daemon_killed_and_started_again_while_stopped_stays_stopped_and_starts_nothing` |
| Branch: conversation not synced | Works (real daemon) | `daemon/humatad/tests/sessions.rs::a_reboot_that_lost_the_conversation_says_the_resume_failed` |
| Branch: daemon doesn't start | Code, no test | `daemon/machines/src/service.rs` |
| Branch: both power off | Works (real daemon), simulated cut | `daemon/humatad/tests/power_cut_both.rs` |
| Branch: VM off while Mac asleep | Code, no test | none |
| Branch: attach during resume | Likely bug (seen live, B10) | REALFLOW run |
| Branch: shared fixture | Not run, by rule | `AGENTS.md` |

## Gaps and likely bugs

Not built:
- Drawing the restart report in the app (no milestone; macro/10 step 9). The start's report is not pushed to the app.
- A real Lima power-off through the app with real agent CLIs. The only whole-machine proof is `terminal/host/src/agent_resume_vm_tests.rs`, with a fixture agent and no `humatad2`. No milestone owns it.
- A throwaway-VM helper for this case. No Bazel target creates and deletes `h2-reboot-1`. A target is needed, because Bazel is the only way to build and test.
- Real-launchd proof that this Mac's daemon starts at login (the installer tests use a fake launchd).
- A model pin: Humata records no model, so a resumed session's model is the CLI's own setting (unproven).

Likely bugs:
- B5: resume looks for the conversation under `--root` instead of the CLI home, so a resumed Copilot or Codex can't find it.
- B7: resume after stop is treated as an interruption and kills the correctly resumed agent.
- B9: Codex's real session id is never learned, so its resume fails.
- B10: attach is refused while a session resumes.
- B3/B4: `humata session read` is empty for Copilot, so history can't be checked that way.
