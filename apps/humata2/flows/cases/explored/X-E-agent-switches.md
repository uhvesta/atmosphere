# Agents: Settings › Agents, the agent pickers, and `humata agents` with agents.toml

Layer: explored · macOS · harness: `desktop/tests/native/agent_switches.rs` (native_test), `desktop/tests/native/{overlays,channel_desk,fork_move,pr_review,session,hosts_settings}.rs`, `features/settings/tests/agent_switches.rs`, `features/app/tests/new_session.rs`, `daemon/humatad` unit tests (agent_switches, agent_gate, agent_service), throwaway `humatad2_admin` plus `humata` CLI

Status: PHASE 1 (map from code and snapshots). Nothing was run. Steps marked PENDING RUN need phase 2. Facts marked "code-read" were read from source, not observed.

## Setup

Phase 2 only, one explorer at a time, from the repo root:

- `export PATH=$PWD/.tools/bin:$PATH`
- Build: `bazel build --jobs=4 //apps/humata2/cli:humata //apps/humata2/daemon/humatad:humatad2_admin //apps/humata2/daemon/holder:humata2_holder`
  - `humatad2_admin` is the test daemon (`--admin`, so CLI calls with no credential act as the user). Its visibility is restricted to dependents, but a direct `bazel build` on the target is allowed.
- Scratch: `SCR=<scratchpad>`, `RUN=$SCR/verify/X-E-run`, `H=$RUN/home`, `EV=$SCR/verify/evidence/X-E`.
- Throwaway home: `mkdir -p $H/.humata2/config $RUN/root $RUN/bin $EV`, then `printf '[agents.copilot]\nenabled = false\n' > $H/.humata2/config/agents.toml`.
  - The switches file is `<state parent>/config/agents.toml` (serve.rs: `host_path(state.parent())`). With `--state $H/.humata2/state` that is `$H/.humata2/config/agents.toml`. Never the owner's `~/.humata2`.
- Daemon, minimal PATH so no agent CLI resolves (claude, codex, copilot, opencode are not on `/usr/bin:/bin`):
  `env -i HOME=$H PATH=/usr/bin:/bin:/usr/sbin:/sbin $PWD/bazel-bin/apps/humata2/daemon/humatad/humatad2_admin serve --admin --socket $H/.humata2/humatad.sock --state $H/.humata2/state --root $RUN/root --holder $PWD/bazel-bin/apps/humata2/daemon/holder/humata2_holder --shell /bin/sh --shell-arg=-i > $RUN/daemon.log 2>&1 &`
- CLI: `env HOME=$H HUMATA_SOCKET=$H/.humata2/humatad.sock $PWD/bazel-bin/apps/humata2/cli/humata --json <args>`. Exit code 7 is PRECONDITION (cli/src/exit.rs).
- Stop the daemon when done.

Rules for this area: never install a real agent CLI. Never set `install_when_missing` on claude, codex, copilot or opencode. Never start an enabled built-in kind whose CLI resolves, since that launches a real agent. Start only refusals, or the custom fake kind in step 11.

## Steps

### 1. Settings › Agents, all agents on (native fixture)
- **Screen:** Settings opens on Agents (`Section::Agents`). The card has the title "Agents" and the caption "Agents shown in pickers and started on your machines". Rows: Claude Code, Codex, Copilot, OpenCode, each with a toggle `settings-agent-enabled-<id>`. Under each enabled row: "Ask in the Inbox" with toggle `settings-agent-asks-<id>` (D782; subtitle "Off: it asks in its own terminal" only while off), then "Install when missing", the install command in mono, "Never replaces one that's there", and a toggle `settings-agent-install-<id>`. Below the card: "Default agent" with buttons Claude Code, Codex, Copilot, OpenCode, Terminal (the chosen one is filled in accent). D199's single "Ask for permission in the Inbox" card (`settings-asks-in-inbox`) is gone.
- **Offered:** the four enabled toggles, four asks toggles, four install toggles, five default buttons.
- **Action:** `Section(Agents)` (or `Reveal` of a row). Reducer returns `read_asks`: asks, codex trust, then `Agents(Loaded)`.
- **Result:** until the daemon answers, the card shows only "Loading agents". The fixture's switches have install commands, so the install rows show in the native test.
- **Evidence:** existing `desktop/tests/__snapshots__/native/29-permission-route.view.txt` is STALE. It shows the default buttons with no Agents card (it predates AGENTSET). New frame: PENDING RUN step 3.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=agent_switches` (test `turning_an_agent_off_in_settings_takes_it_out_of_the_pickers` asserts `settings-agent-enabled-copilot` exists).

### 2. Turn Copilot off
- **Screen:** Copilot toggle off. Copilot disappears from `offered_agents()`.
- **Offered:** toggle (enabled), install toggle (hidden while off).
- **Action:** click `settings-agent-enabled-copilot` → `Toggle(Copilot)`. The switch flips at once, then `set_agent_switch` runs.
- **Result:** `Saved(Ok)` replaces the slot with the daemon's answer. Pickers no longer list Copilot. Claude stays.
- **Evidence:** none yet (fake daemon). Reducer test only.
- **Replay check:** `bazel test //apps/humata2/features/settings:agent_switches_test --test_filter=turning_an_agent_off`

### 3. Frame for the owner's wave (card with all three states)
- **Screen:** Claude on, Codex on with install on, Copilot off. Rendered headless only.
- **Offered:** same as step 1.
- **Action:** `AGENTSET_FRAME_OUT` set, test clicks `settings-agent-install-codex` and `settings-agent-enabled-copilot`, then draws and captures.
- **Result:** writes `settings-agents.png` into the folder. No golden touched. Nothing is recorded as a golden.
- **Evidence:** PENDING RUN → `$EV/frame/settings-agents.png`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=agent_switches --test_env=AGENTSET_FRAME_OUT=$EV/frame --nocache_test_results`

### 3b. Ask in the Inbox per agent, and custom kinds (D782)
- **Screen:** every enabled agent has its own "Ask in the Inbox" row, on by default. Turning Copilot's off leaves the others on. Turning Copilot itself off hides its asks row. A custom kind in `agents.toml` (e.g. `[agents.my-agent]`) is listed on the card under its own name, with its enable and asks rows; pickers still offer only the built-in agents.
- **Action:** click `settings-agent-asks-copilot` → `ToggleAsks("copilot")`; the daemon writes `asks_in_inbox = false` under `[agents.copilot]` and pushes the file to linked machines.
- **Result:** Copilot's permission hook gets an empty reply (it asks in its terminal), its question hook passes through, and a screen ask raises no card but keeps the session NEEDS_INPUT until the dialog is gone. RequestAccess and AskUser from Copilot still come to the Inbox.
- **Evidence:** candidate frame `117-settings-agents-ask-in-inbox.png` (scratch, for Fable).
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=agent_switches`, `//apps/humata2/features/settings:agent_asks_test`, `//apps/humata2/daemon/humatad:hooks_test`, `:screen_questions_test`, `:remote_stack_test` (`an_agent_switch_changed_on_the_host_reaches_a_linked_box`).

### 4. A refused save goes back and says why
- **Screen:** the switch returns to its old state. Error line `settings-agents-error` shows the message under the card.
- **Action:** `Toggle` (or `ToggleAsks`) with the fake's `fail_next(SetAgentSwitch, "link unavailable")`.
- **Result:** `Saved(Err)` restores `before`, sets `settings.error`.
- **Evidence:** none. Reducer test.
- **Replay check:** `bazel test //apps/humata2/features/settings:agent_switches_test --test_filter=a_refused_change`

### 5. Pickers: New session, Desk, Fork, PR review, session "+" menu
- **Screen (all on):** New session tabs Claude Code, Codex, Copilot, OpenCode, Terminal (`31-new-session.view.txt`); Desk new the same (`40-new-desk-session.view.txt`); session tab "+" menu `new-terminal-menu` (native `session.rs`).
- **Screen (Copilot off, code-read for the list):** every picker drops Copilot. Terminal stays where it was offered. Before the list loads, all four plus Terminal show.
- **Offered:** the agent buttons or tabs in each dialog.
- **Action:** open each dialog from its entry point.
- **Result:** pickers use `offered_agents()` (features/app/src/new_session.rs and the ui overlays). PR review uses `review_agents()`, which has no Terminal.
- **Evidence:** existing snapshots `native/31-new-session.view.txt`, `native/40-new-desk-session.view.txt`. Golden drift check: PENDING RUN step 9.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=overlays` and `--test_filter=session` and `--test_filter=pr_review` (all-on defaults, no copilot change).

### 6. New session with a default agent that is off
- **Screen:** the New session dialog opens on the first enabled agent, not on the disabled default.
- **Action:** preference default = Claude, Claude and Codex off, `StartNewSession`.
- **Result:** `default_agent(state)` returns Copilot. Settings' "Default agent" shows no highlighted button, because it uses the raw preference (code-read, ui/src/settings.rs:521).
- **Evidence:** none.
- **Replay check:** `bazel test //apps/humata2/features/app:new_session_test --test_filter=a_default_agent_that_is_turned_off`

### 7. Channel desk tab "New desk session" with a default agent that is off (code-read, not run)
- **Screen:** channel Desk tab with the button `channel-desk-new`.
- **Action:** click it, default Claude, Claude off.
- **Result (code-read):** ui/src/channel_desk.rs:288 reads `state.settings.preferences.default_agent` directly, not `default_agent(state)`. The start carries Claude, and the daemon refuses it (`channel_agent` → `enabled`). The user gets a refusal naming Claude while the New session dialog would have offered Copilot.
- **Evidence:** none. No test covers a disabled default on this button.
- **Replay check:** none yet. A headless test would need a new test (not explorer's job). Coach to decide.

### 8. Fork overlay (code-read, not run)
- **Screen:** Fork dialog, Agent row `fork-agent`. The label shows the parent's agent, even when the parent's kind is off, because the default is `fork.agent.or(parent.agent)`.
- **Offered:** the agent menu lists `offered_agents()` plus Terminal. The parent's kind is not in the menu if it is off.
- **Result (code-read):** a fork whose agent equals the parent's is not checked (session_service/fork.rs:371 gates only `agent != parent.record.agent`). A copy of a disabled kind still starts.
- **Evidence:** existing `native/fork-dialog.view.txt` (pre-AGENTSET, shows no agent field text). Replay: `bazel test //apps/humata2/desktop:native_test --test_filter=fork_move` (test `the_fork_dialog_matches_frame_62...` asserts `fork-agent` exists).

### 9. Golden drift check (stale snapshot)
- **Screen:** Settings › Agents after the card change.
- **Action:** run the native test that snapshots `29-permission-route`.
- **Result (expected, to be confirmed):** the test fails on the snapshot comparison because the golden has no card. Record what the run shows. Do NOT record a new golden. Main owns golden changes (milestone note: "golden 29-permission-route changes").
- **Evidence:** `bazel-testlogs/apps/humata2/desktop/native_test/test.outputs/` for any `.new.png`. Copy the `.new.png` into `$EV/golden-29/`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=hosts_settings`

### 10. `humata agents ls` with copilot off in agents.toml (CLI)
- **Screen / output:** JSON list of agent definitions.
- **Offered:** `agents ls`, `agents add`, `agents edit` only. There is no flag to show disabled kinds, and no CLI command to enable or disable.
- **Action:** `humata --json agents ls`.
- **Result (expected):** exit 0; ids claude, codex, opencode, zsh; no copilot. Copilot is NOT shown (ListAgentsRequest default is include_disabled=false, cli/src/commands/hosts.rs:664).
- **Evidence:** PENDING RUN → `$EV/agents-ls.json`.
- **Replay check:** `env HOME=$H HUMATA_SOCKET=$H/.humata2/humatad.sock <cli> --json agents ls` → exit 0, no `"copilot"` id.

### 11. `humata agents edit` and `agents add` are refused (CLI)
- **Action:** `humata --json agents edit copilot --launch /bin/true`, then `humata --json agents add probe --launch /bin/true`.
- **Result (expected):** `edit` finds copilot (it lists include_disabled), then UpdateAgent refuses: FailedPrecondition, exit 7, fix says the agents come from the agents file and need a restart. `add` refused the same way (AddAgent).
- **Evidence:** PENDING RUN → `$EV/agents-edit.json`, `$EV/agents-add.json`.
- **Replay check:** the two commands above, expected exit 7 each.

### 12. Start a disabled kind from the CLI
- **Setup:** `humata --json channel create agentset-probe`, then `--channel agentset-probe` on each start.
- **Action:** `humata --json --channel agentset-probe session start --agent copilot`.
- **Result (expected):** refused before any session is created; exit 7; message says copilot is turned off; fix says "turn Copilot on in Settings › Agents › Copilot". Gate runs `enabled` before `operations` (session_service.rs:540).
- **Evidence:** PENDING RUN → `$EV/start-copilot-off.json`.
- **Replay check:** the command above, expected exit 7 and the fix text naming Settings › Agents › Copilot.

### 13. Start an enabled kind whose CLI is missing (install off)
- **Action:** `humata --json --channel agentset-probe session start --agent codex`.
- **Result (expected):** codex is not on the daemon's minimal PATH; install is off. Refused with GateError::Missing: "isn't installed on this machine: `codex` was not found"; fix offers to install it yourself (`npm install -g @openai/codex`) or turn on "Install when missing" under Codex. Exit 7. No install runs.
- **Evidence:** PENDING RUN → `$EV/start-codex-missing.json`.
- **Replay check:** the command above, expected exit 7; `$H` has no `.npm` or install side effect.

### 14. Custom fake kind: missing, install once for two starts, failed install, install outside the daemon PATH
- **Setup:** stop the step-10–13 daemon. Restart with `--agents $RUN/agents.json` (JSON list, `name=fake`, `program=$RUN/bin/fake-cli`, `start_args=[]`, `resume_args=[]`, `conversation_file=""`). This replaces the built-in list, which is fine for this run. Create `$RUN/fake-cli-src` containing `#!/bin/sh` then `sleep 600`, and `$RUN/install-fake.sh` that appends `installed` to `$RUN/install.log`, sleeps 2 s, copies `fake-cli-src` to `$RUN/bin/fake-cli`, and chmods it. Write `$H/.humata2/config/agents.toml`: `[agents.fake]`, `install_when_missing = true`, `install = "sh $RUN/install-fake.sh"`.
- **Action:** two `humata --json --channel agentset-probe session start --agent fake` in parallel, then `wait`.
- **Result (expected):** one install line in `install.log`; both starts exit 0 after the install step; the session runs the fake sleep, not an agent. Stop those sessions afterwards.
- **Second part:** change `install` to `echo boom >&2; exit 3` with no restart. Start `--agent fake`: refused, exit 7, message includes the command and "boom", fix says to run it yourself or turn Install when missing off. Confirms the file is read per check.
- **Third part (PATH mismatch, code-read risk):** `program` set to bare `fake-cli`; install writes into `$RUN/login-bin` and `$H/.bash_profile` adds that folder to PATH (login shell only). Expected: "the command finished, but `fake-cli` is still not found on the daemon's PATH".
- **Evidence:** PENDING RUN → `$EV/fake-install/` (install.log, start outputs, daemon.log).
- **Replay check:** `grep -c installed $RUN/install.log` = 1 after two parallel starts.

### 15. Fork to a disabled kind from the CLI
- **Setup:** a running fake session from step 14 (its parent).
- **Action:** `humata --json session fork <parent-session-id> --kind code-and-context --agent copilot`.
- **Result (expected):** `agent != parent` → `enabled` refuses copilot: exit 7, Settings › Agents › Copilot named. Fork kinds accepted: `code-and-context`, `code`, `context`.
- **Evidence:** PENDING RUN → `$EV/fork-copilot-off.json`.
- **Replay check:** the command above, expected exit 7.

### 16. Resume or restore of a session whose kind was turned off (not run)
- **Screen / result (code-read):** resume, restore, move, rebuild never call the gate (agent_gate.rs header). Not run: needs a daemon restart with the fake kind disabled, then checking the existing session. Optional PENDING RUN if time allows; otherwise leave.

## Not reachable headless

- Live toggling from the Settings card against a real daemon: the native test uses FakeDaemon. A live toggle needs the GUI app, which is off-limits (no windows).
- Load failure of the switches read: the fake's `agent_switches` reply has no Operation hook (clients/src/daemon/fake/agent_switches.rs:24), so `fail_next` cannot script it.
- Desk-start install and fork install paths: no CLI reaches desk start (`DeskCommand` has no start verb), and fork goes through `starter.fork` without the install check (see surprises).
- Channel desk tab click with a disabled default: needs the native app; code-read only.

## Surprises

1. **Load failure hides its error.** In ui/src/settings.rs, `agent_switches_block` returns "Loading agents" before the error line is appended. If the first load fails, the card shows "Loading agents" forever and the error is never shown (code-read).
2. **Error line sticks.** `agents::reduce` never clears `settings.error`. A failed save stays on the card after a later successful toggle. The same field is shown on the Agents card for errors from other sections (code-read).
3. **Channel desk tab ignores the fallback.** ui/src/channel_desk.rs:288 passes the raw default agent. A disabled default gives a refusal from the daemon on "New desk session" (code-read).
4. **Default agent highlight vs. new session.** Settings highlights the raw preference (none, if it is off). New session uses the fallback. The two disagree (code-read, ui/src/settings.rs:521).
5. **Fork of a disabled kind runs.** The gate checks only when the fork's agent differs from the parent's. The fork overlay defaults to the parent's disabled kind, and the picker does not list it (code-read).
6. **Desk starts and forks skip the CLI check and install.** Desk start calls only `enabled()` and then `starter.start_desk`. Fork calls only `enabled()` then `starter.fork`. Neither reaches `agent_gate::check` or `install_now`. The spec's exit criterion says any start installs when allowed (code-read).
7. **PR review install is invisible.** `begin_review` awaits `agent_gate::ready()` before creating the operation, so an install blocks the RPC with no "Installing" step. The spec says the start shows it as a step (code-read).
8. **PATH for "found" differs from the spec.** The spec says `$SHELL -lc 'command -v'`. The code uses the daemon's own PATH (agent_gate.rs `is_found`). The install runs under `bash -lc`. An installer that puts its binary in a folder only the login PATH sees (e.g. `~/.local/bin`) will report "still not found on the daemon's PATH" even though it installed (code-read; step 14 third part tests it).
9. **No way to see disabled kinds from the CLI.** `humata agents ls` has no flag, and there is no enable or disable verb. Editing copilot's switch works only through the app or the file.
10. **No integration test for the StartSession refusal.** Only unit tests in agent_gate.rs cover `enabled()`. No daemon test (daemon/humatad/tests) starts a disabled kind over gRPC.
11. **Golden drift.** `native/29-permission-route.view.txt` predates the card. The Settings golden will change when the card is recorded (main's job).
12. **MCP has no agent listing.** The spec mentions "MCP listings", but no MCP tool lists agents (daemon/humatad/src/mcp*), so nothing to filter there.
13. **Name collision.** `review.proto` has `SetAgentSwitches` (PR review chat switches) and AGENTSET uses `agent_switches` for kind switches. Two unrelated "agent switches" in the tree.
14. **Typos in agents.toml are silent.** `[agents.codx]` is accepted as an unknown kind with no effect (only invalid names are errors, and the `deny_unknown_fields` applies to keys inside a kind). Code-read.
15. **Write-failure fix text is wrong.** `switches_failed` (agent_service.rs) says "fix or remove agents.toml" for every failure, including a write error where the file is fine.
