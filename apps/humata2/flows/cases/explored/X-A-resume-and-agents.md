# Resume: stopping an agent, Resume agent, the CLI resume and stop, forks across a restart, and how Claude, Codex and Copilot differ

Layer: explored · macOS · harness: desktop/tests/native/session_menu.rs (headless), desktop/tests/live_flows/restart.rs and stop_and_start.rs (real humatad2), cli/tests/sessions.rs and stop.rs (fake daemon), daemon/humatad/tests/fork.rs and fork_remote.rs (daemon)

Phase 1 (map from code and snapshots). Steps marked PENDING RUN are filled in phase 2. The fork dialog is in X-A-fork-dialog.md. The reboot card, Try again, Start fresh and the Idle group are in X-I-restart-and-link-drop.md (steps 1 to 6); the machine-level Stop all sessions and Start sessions are in X-I-stop-and-start.md. Those steps are not repeated here.

## Setup

- Native: `App::new(fixture::design())`. The fixture "ledger" session is Running; the dark stopped snapshot is taken after Stop the agent.
- Live: desktop/tests/live_flows/restart.rs reboots a real `humatad2` (`World::reboot`). `World::start` writes fake agents to agents.json, so the agent's conversation format is set by the test (see Surprises).
- CLI: cli/tests/sessions.rs and stop.rs drive `humata` against FakeDaemon.
- Throwaway daemon: see the last section. Never the owner's `~/.humata2`, `~/.claude`, `~/.codex` or `~/.copilot`.

## Steps

### 1. A stopped session's header: Resume agent
- **Screen:** Session header (right side, before Fork): an accent button `Resume agent` (id `resume-agent`, shown only when the session status is Exited; ui/src/pages.rs ~289). The ⋯ menu's `Stop the agent` is greyed out. In the terminal area the `Resume` button also appears on the card, and the terminal shows the agent's last output. The sidebar row is still listed.
- **Offered:** Resume agent, Fork, Session actions, Expand terminal, Toggle side panel.
- **Action:** None (the state is reached by Stop the agent, step 2).
- **Result:** Read-only state.
- **Evidence:** desktop/tests/__snapshots__/native/session-menu-stopped-dark.view.txt (Button "Resume agent"; Button "Stop the agent" disabled; Button "Fork"; Button "Session actions"). PNG: session-menu-stopped-dark.png.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=the_session_menu_renames_and_stops_the_open_session` (asserts the status is Exited after Stop, then takes this snapshot).

### 2. Session actions › Stop the agent
- **Screen:** The ⋯ menu card (`session-menu-card`, width 230) with: `Rename…`, `Move to another machine…`, `Continue in another channel…`, `Add repo to this session…`, `New desk session for this session`, `Stop the agent`, `Push and open a pull request`, `Archive session…`. On a Desk session Move, Continue, Add repo and New desk session are absent (native `a_desk_sessions_menu_has_no_move`).
- **Offered:** Each item above. Escape or the scrim closes it.
- **Action:** Press `Stop the agent`. No confirm dialog. It calls `StopAgent` (features/session/src/menu.rs), which is ignored unless `can_stop` (the session is not Exited), then `daemon.stop_agent(id)`.
- **Result:** The agent quits; the session's status becomes Exited; the terminal's scrollback and the clones stay ("Stop the agent; the terminal and the clones stay", menu.rs). The header then shows Resume agent (step 1).
- **Evidence:** session-menu-open.view.txt (menu contents, before Stop); session-menu-stopped-dark.view.txt (after).
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=the_session_menu_renames_and_stops_the_open_session` (presses Stop, then asserts the status is Exited). No live test presses Stop the agent. The palette's Stop the agent runs the same action: `bazel test //apps/humata2/features/app:palette_rename_stop_test`.

### 3. Stop the agent on a stopped session is greyed out
- **Screen:** `session-menu-stop` disabled in the menu (session-menu-stopped-dark.view.txt: `Button "Stop the agent" disabled`).
- **Offered:** The other menu items only.
- **Action:** Open the menu.
- **Result:** Cannot press Stop; the reducer ignores it.
- **Evidence:** session-menu-stopped-dark.view.txt.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=the_session_menu_renames_and_stops_the_open_session`.

### 4. Resume agent (header button) starts the same conversation
- **Screen:** The header button from step 1. No dialog.
- **Offered:** Resume agent.
- **Action:** Press `Resume agent`. Sends `SessionAction::Resume`, which calls `daemon.resume(id)` (features/session/src/lib.rs 1449-1452). The daemon refuses a Desk session (`refuse_if_closed`) and a session on a stopped machine (`require_running`, humatad session_service.rs resume_session). If the agent is still being brought back after a restart, the call waits for the restart (session_service.rs `resume`).
- **Result:** The agent starts with the same conversation file (the restart test checks `agent up --resume` and that the same conversation file still holds the line). The header button goes away; the Stop item is enabled again.
- **Evidence:** PENDING RUN. No snapshot shows the button pressed.
- **Replay check:** No test presses the header button. The same daemon call is made by the Try again card: `bazel test //apps/humata2/desktop:live_flows_test --test_filter=after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt` (asserts the same conversation is resumed). Gap: the header button path itself.

### 5. humata session resume <session> (CLI)
- **Screen:** Terminal. Prints the daemon's Operation (or JSON with `--json`).
- **Offered:** `humata session resume <session>`, and `--fresh` (start over with a summary, not resume).
- **Action:** `humata session resume ledger`; `humata session resume ledger --fresh`.
- **Result:** Without `--fresh` the daemon resumes the same conversation (`Mode::Resume`). With `--fresh` it starts a new conversation seeded with a summary (`Mode::StartFreshWithSummary`, humatad session_service.rs 1010 routes to start_fresh). Exit code: 0 on success (fake-daemon test).
- **Evidence:** PENDING RUN (fake-daemon test output).
- **Replay check:** `bazel test //apps/humata2/cli:sessions_test --test_filter=resume_fresh_starts_over_with_a_summary`. Real daemon: PENDING RUN, see the last section.

### 6. humata session stop <session> and the whole-machine stop/start (CLI)
- **Screen:** Terminal.
- **Offered:** `humata session stop <session>` (one session; the agent stops, the session stays); `humata stop [--reason <text>]` (every session on this machine, kept stopped until `humata start`); `humata start` (start this machine's sessions again after a stop).
- **Action:** Run each with a fake daemon, then read the request.
- **Result:** Each sends one request; `humata stop` takes a reason and an idempotency key and returns a stop id (`stop_in_json_names_the_stop_the_daemon_made`). `humata start` asks the daemon to start the sessions. A denied agent's stop exits with the denied code (`an_agents_stop_denied_by_its_rule_exits_denied_with_the_fix`).
- **Evidence:** PENDING RUN (fake-daemon test output).
- **Replay check:** `bazel test //apps/humata2/cli:stop_test`. Single-session stop is covered by `stop_takes_no_machine_and_session_stop_is_unchanged`.

### 7. After a restart, a session that was running comes back, or shows the card
- **Screen:** See X-I-restart-and-link-drop.md steps 1 to 6: the toast, Home's groups (Couldn't resume after the restart · N; Idle after the restart · N), the Couldn't resume card with its reason and Try again / Start fresh with a summary, and the restart report link.
- **Offered:** Try again, Start fresh with a summary (or "Start fresh (nothing left to summarize)"), Restart report link (goes to the Hosts page, features/app/src/restart.rs `OpenReport` → `Route::Hosts`).
- **Action / Result / Evidence / Replay:** see X-I-restart-and-link-drop.md. Not repeated.

### 8. A fork made just before a restart
- **Screen:** The child session is listed after the restart only if its agent was started. No card or message is named in the daemon tests.
- **Offered:** None in the app for this case.
- **Action:** Fork, then the daemon restarts before the fork's agent starts (daemon test `a_fork_cut_before_its_agent_starts_is_undone_at_restart`); versus after it started (`a_fork_cut_after_its_agent_started_is_kept_at_restart`).
- **Result (per test names; read the tests in phase 2):** The first is undone at restart. The second is kept. A handover cut after launch is typed once after a restart (`a_handover_cut_after_the_launch_is_typed_once_after_a_restart`).
- **Evidence:** PENDING RUN (test output).
- **Replay check:** `bazel test //apps/humata2/daemon/humatad:fork_test --test_filter=a_fork_cut_before_its_agent_starts_is_undone_at_restart`; then `--test_filter=a_fork_cut_after_its_agent_started_is_kept_at_restart`.

### 9. Resume is refused while the machine is stopped
- **Screen:** Expected: the machine page has `Start sessions` (X-I-stop-and-start.md step 8), and the header Resume agent is refused.
- **Offered:** Resume agent (the app sends it); the message is the daemon's.
- **Action:** On a machine with Stop all sessions in effect, press Resume agent.
- **Result (inferred from code):** `resume_session` calls `stop::gate::require_running` first, so the call fails before any launch (humatad session_service.rs). The exact words of the refusal are not read.
- **Evidence:** PENDING RUN.
- **Replay check:** PENDING RUN. No test found that presses Resume agent on a stopped machine. Candidate: `bazel test //apps/humata2/daemon/humatad:stop_sessions_test`, then read its resume case.

### 10. Claude, Codex and Copilot: what each one lets Humata do with a conversation
- **Screen:** The Fork from picker (X-A-fork-dialog.md step 4) and the Preview's summary note (step 7) are where a person sees the difference. The rest is invisible.
- **Offered:** As in those steps.
- **Action:** Pick an agent whose conversation Humata reads, then an agent whose conversation it does not.
- **Result (from code):**
  - Claude Code (`claude-jsonl`): turns are listed from the JSONL file; Fork from before a turn copies bytes up to that turn; a fork to another agent is handed the conversation file and a summary (daemon fork.rs `hands_over`).
  - Codex (`codex-rollout`): read from its rollout files under CODEX_HOME. Codex has no flag to choose its conversation id, so Humata finds the id by the folder the agent runs in (`FoundInFolder`, daemon/sessions/src/agents.rs). A fork of a Codex parent handed to another agent gets its summary read from the rollout (test `a_fork_of_a_codex_parent_is_handed_a_summary_read_from_its_rollout`).
  - Copilot (`copilot-session-state`): read from its session-state folder under COPILOT_HOME; resumed with `--session-id`; its version is probed with `--no-auto-update` so the probe doesn't update the CLI (agents.rs `version_args`).
  - An agent with no readable format (`none`, e.g. Shell or a custom agent): the turn list is empty, so the picker shows only Now; asking for a turn (`before_turn >= 2`) is refused with "fork from now, with before_turn 0, or use an agent whose conversation Humata reads (Claude Code, Codex or Copilot)" (daemon session_service/fork.rs ~490-502).
  - Code only ignores the conversation for every agent (`hands_over` returns false; a fresh agent).
- **Evidence:** PENDING RUN for the picker on a Codex parent (not in any snapshot).
- **Replay check:** `bazel test //apps/humata2/daemon/humatad:fork_test --test_filter=a_fork_of_a_codex_parent_is_handed_a_summary_read_from_its_rollout`; `--test_filter=a_fork_with_another_agent_is_handed_the_conversation_and_a_summary`; `--test_filter=a_fork_with_an_agent_humata_does_not_know_is_refused`; `--test_filter=code_only_copies_the_work_into_a_fresh_conversation`.

### 11. An agent Humata does not know, or a machine it does not know
- **Screen:** The Agent pop-up lists only enabled agents plus Shell, so an unknown agent can't be picked in the app. A CLI `--agent` or `--machine` can name one.
- **Offered:** CLI only.
- **Action:** `humata session fork ledger --kind context-only --agent unknown-agent`.
- **Result:** Refused by the daemon (test `a_fork_with_an_agent_humata_does_not_know_is_refused`). Unknown machine: `a_fork_onto_a_machine_this_host_does_not_know_is_refused_as_an_unknown_machine`.
- **Evidence:** PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/daemon/humatad:fork_test --test_filter=a_fork_onto_a_machine_this_host_does_not_know`.

## Not reachable headless
- The header Resume agent press (step 4): no test presses it; only the Try again card reaches the same daemon call.
- A real Claude, Codex or Copilot conversation: the harnesses use fake agents and seed JSONL by hand. Codex and Copilot conversations are reached only in daemon tests.
- Resume refused on a stopped machine: the words of the refusal are not read in any test found (step 9).
- A real restart of the owner's machine: the live tests reboot the throwaway daemon, not the Mac.

## Surprises
- **The daemon's `resume_session` doc comment is stale.** It says "Starting fresh with a summary isn't built" (daemon/humatad/src/session_service.rs ~2394), but `resume()` routes `StartFreshWithSummary` to `start_fresh` (line ~1010), and the CLI `--fresh` test passes. A reader who trusts the comment will think `--fresh` does nothing.
- **Stop the agent has no confirm, while Stop all sessions does.** One click ends the agent with no question, and the item sits in the same card as Archive. The page says the terminal stays; a person may still expect the session to end. Fact from features/session/src/menu.rs (no confirm step).
- **The live tests set Claude's conversation format by hand.** `restart.rs` and `fork_kinds.rs` write `conversation_format = "claude-jsonl"` into agents.json before the daemon starts, with the comment "The agent declares Claude's conversation format, so the daemon can list turns". So with the fake agent's default format (`none`), a fork from an earlier turn is not offered at all. Phase 2 should confirm the real agent definitions ship `claude-jsonl` (not checked here).
- **A fork can be undone by a restart without a card.** Per the daemon test name, a fork cut before its agent started is removed at restart, and the app shows no message about it (step 8). The app never tells the person the fork they made is gone. Confirm in phase 2.
- **The restart report link goes to the Hosts page**, not to the machine's page or to a report view (features/app/src/restart.rs `OpenReport` → `Route::Hosts`).
- **Resume refused while a machine is stopped has no app-side check.** The app sends the call and shows the daemon's answer; the header button stays enabled on a stopped machine's session (inferred from pages.rs, which only checks Exited status). Confirm in phase 2.

## Throwaway daemon (for PENDING RUN CLI transcripts)

The CLI tests and live tests already run the real binaries in phase 2 through Bazel. A hand-started daemon needs a fake agent (as World::start writes), so prefer the Bazel targets. If a transcript against a hand-started daemon is wanted:

1. `export PATH=$PWD/.tools/bin:$PATH; bazel build --jobs=4 //apps/humata2/cli:humata //apps/humata2/daemon/humatad:humatad2 //apps/humata2/daemon/humatad:holder_for_clients`
2. `S=<scratchpad>/X-A-daemon; mkdir -p $S/home $S/root $S/state`
3. `HOME=$S/home bazel-bin/apps/humata2/daemon/humatad/humatad2 serve --socket $S/h.sock --state $S/state --root $S/root --holder bazel-bin/apps/humata2/daemon/humatad/holder_for_clients --shell /bin/sh --shell-arg=-i` (verify the bin paths and the serve subcommand in the first build; ServeArgs is in daemon/humatad/src/serve.rs).
4. `HOME=$S/home HUMATA_SOCKET=$S/h.sock bazel-bin/apps/humata2/cli/humata --json session resume <id>`; stop the daemon when done.

Phase 2 must not use the owner's `~/.humata2` or agent homes.
