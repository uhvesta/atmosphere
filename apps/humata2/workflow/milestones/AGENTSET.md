# AGENTSET: I choose which agents Humata offers, and Humata installs only its own tools unless I say otherwise

- **Status:** in progress (lane AGENTSET, opened 2026-11-16)
- **Depends on:** AGENTKINDS (codex and copilot as built-in kinds, `agent_service.rs`), MACHINESCRIPTS (machine config shipping beside `repos.toml`; its `start` step goes through the same start path), UPGRADE (Humata's own tools are installed and upgraded on every machine)
- **Unblocks:** CASES rows for remote setup with a chosen agent set
- **Layer:** L4 · **Lane:** daemon + clients + features + UI

## The owner's words (2026-11-16)
"humata should not install an agentic coding CLI when an enabled one is already on the machine (let's add a settings pane to enable and disable agents, say someone only wants claude code or codex or copilot or they want a few). Humata should always install CLIs it manages (humatad and other related things), it just doesn't need to install stuff it doesn't manage like copilot unless explicitly set to."

## Outcome
Settings › Agents has one switch per agent: Claude Code, Codex, Copilot and OpenCode, in that order, then any custom kinds. Only enabled agents appear in agent pickers and in the CLI and MCP listings. Starting a session with a disabled agent is refused, and the fix names the switch. Under each switch, "Install when missing" (off by default) lets Humata install that agent's CLI on a machine that doesn't have it. Humata never installs a third-party agent CLI otherwise, and never replaces one that is already there. Humata's own tools (humatad2, holder, humata, stack) are always installed and upgraded, as before.

## Exit criterion
- With `~/.humata2/config/agents.toml` saying `[agents.copilot] enabled = false`, `ListAgents` (and `humata agents list`) has no copilot, `ListAgents { include_disabled: true }` has it with `enabled = false`, and StartSession / ForkSession `--agent copilot` / desk starts with copilot fail `FailedPrecondition` with a fix that names Settings › Agents › Copilot. With no file, every kind is enabled and nothing is installed.
- A start of an enabled kind on a machine where its CLI is not found fails with a fix that says to install it there or turn on "Install when missing". With "Install when missing" on, the same start installs it once (one install even for two starts at once) and then starts; a failed install refuses the start with the command and the end of its output. A CLI that is found is never installed over.
- Resume, restore after a reboot, move and rebuild of a session whose kind was later disabled still work (sessions that exist keep running).
- The host's `agents.toml` goes to machines beside `repos.toml` (secret filter applied), on setup and on each link-up when it differs.
- App: the Settings › Agents pane toggles each switch and its "Install when missing" through `SetAgentEnabled` (TestStore tests on `FakeDaemon`; a native view test clicks a switch headless). The new session, fork, desk and channel pickers list only enabled agents plus Terminal.
- Nothing in the tree installs an agent CLI outside "Install when missing" (grep: `daemon/machines`, setup, bundle, MACHINESCRIPTS; `testing/agent_clis` stays test-only).
- None of the tests asserts on wording.

## Design (spec)

### Findings: what installs agent CLIs today (2026-11-16)
- Nothing. `daemon/machines` installs `InstalledFile::ALL` = humatad2, humata2_holder, humata, stack (bundle.rs, install.rs, setup.rs). No setup, link or upgrade step names claude, codex, copilot or opencode.
- MACHINESCRIPTS adds no agent install; its `start` step starts sessions through the same start code StartSession uses, so the refusals below apply to it.
- `testing/agent_clis` does not exist in the tree yet. MACHINESCRIPTS `apply_w5.py` (not applied) adds it: pinned Copilot and Codex copied into a Lima VM's `~/.humata2/test-bin` for flow case real/04. It is test-only, never wired into setup, and stays so.
- The `copilot_cli_*` repos in MODULE.bazel belong to Humata 1 (`apps/humata/copilot`), not Humata 2.

### The setting (D720)
`~/.humata2/config/agents.toml`, portable config: compiled-in defaults plus the file, synced to machines.
```toml
[agents.copilot]
enabled = false

[agents.codex]
install_when_missing = true

[agents.my-agent]          # a custom kind from --agents
install = "npm install -g my-agent"
```
- Defaults: every kind enabled, `install_when_missing = false`. A kind not in the file has the defaults. The plain shell (`zsh`) is not an agent: always on, never listed as a switch.
- `install` names the install command for a custom kind. Built-in kinds have compiled-in commands (D722) that the file can't override.
- The file has no secrets. A bad file is an error with what to fix, and the daemon then treats every kind as enabled with no installs (never blocks on a typo), logging the error once per read.

### API (D720, D727)
- `AgentDefinition` gains `bool enabled = 14; bool install_when_missing = 15; bool can_install = 16; string install_command = 17;` (`can_install`: the kind has an install command; `install_command` is shown in the pane).
- `ListAgentsRequest` gains `bool include_disabled = 1;`. Without it only enabled kinds (and zsh) are listed: the CLI, MCP and pickers.
- `AgentService.SetAgentEnabled(SetAgentEnabledRequest { AgentId agent; optional bool enabled; optional bool install_when_missing; })` returns the AgentDefinition. User-only (an agent caller is refused like AddAgent's presence rule; peers refused). Writes the host's file, keeping other lines and comments. Turning on install for a kind with no install command is `invalid_field`.

### Where the checks run (D721)
- **Enabled:** StartSession and review starts (beside `agent_for`; not rebuild placement, which shares it), fork with a different `--agent`, and the desk starts check the kind is enabled in the deciding daemon's file. The host checks before it relays, so a disabled kind is refused at once even if a machine's copy is older.
- **On the machine:** the start path that runs on the machine where the agent will run (the local branch of StartSession, which a remote start reaches on the machine, and MACHINESCRIPTS' `start` step) calls `agent_gate::ready(&agents, kind, machine)`: enabled, then found, then installed when missing and allowed.
- **Not refused:** resume, restore after a reboot, move, rebuild of an existing session, attaching. A session that exists keeps running whatever the switch says.
- "Found" means the kind's CLI resolves on the machine's login PATH (`$SHELL -lc 'command -v <cli>'`, 10 s deadline), or an absolute program path is an executable file. The CLI looked for is the kind's program, except the wrappers: opencode looks for `opencode` (its program is `/usr/bin/env humata opencode`).

### Install when missing (D722, D723)
- Built-in install commands, run with `bash -lc` on the machine:
  - claude: `curl -fsSL https://claude.ai/install.sh | bash`
  - codex: `npm install -g @openai/codex`
  - copilot: `npm install -g @github/copilot`
  - opencode: `curl -fsSL https://opencode.ai/install | bash`
- Run only when the option is on and the CLI is not found. Never to upgrade or replace a CLI that is found. One install per kind at a time (single flight); a second start waits on the first. 10 minute deadline. After it, the CLI must be found or the install counts as failed.
- When: only at a start that needs it on that machine (owner's proxy, D723). The start waits for it and shows it as a step. Nothing installs at daemon start.
- A failure refuses the start: what failed, the command, the last lines of its output, and the fix (run the command on that machine yourself, e.g. after installing Node.js for npm; or turn the option off and install it your way).
- Humata's own tools are unchanged: `InstalledFile::ALL` is installed and upgraded on every setup, link-up and Migrate (UPGRADE). Agent CLIs are never part of the bundle.

### Sync (D724)
- `agents.toml` ships to machines exactly like `repos.toml` (RL-12): `StackConfig::agents_host_path`/`read_agents`, `Builds::agents_config`, `Bundle::with_agents_config`, setup's pending list, and the link-up send in `stack_settings.rs`. Each daemon reads its copy afresh for every check (it's tiny), so a new copy takes effect without a restart.

### Missing agents (D726)
- Said only where it blocks: the refused start. Nothing polls or shows "missing" elsewhere. The machine page listing of the enabled agents a machine has is left for later (not built).

### App (D725, D729; owner's proxy taste 2026-11-16)
- Settings › Agents, above Default agent: one "Agents" card with a single caption under its title and no note on each row. There is one row per agent (Claude Code, Codex, Copilot, OpenCode, then custom): a name and a pill switch like frames 92/93 (`settings-agent-enabled-<id>`).
- Under an enabled agent with an install command there is an indented "Install when missing" row, off by default (`settings-agent-install-<id>`). It shows the exact install command in mono and "Never replaces one that's there", and is hidden while the agent is off.
- There are no installed, missing or version chips, and no Save button. A failed save puts the switch back and shows the error.
- Pickers (new session, fork, desk new and desk channel, session tab "+", PR review) and Default agent list `SettingsState::offered_agents()`: the enabled built-ins in order, plus Terminal where it was offered. Before the list loads, they list all of them. A default agent that is disabled falls back to the first enabled agent (new session, desk, PR review).
- The list loads when the app starts and when Settings › Agents opens.
- Frame for the owner's wave: one light frame, "Settings › Agents with agent switches", showing all three states (on, on with install, off). It is rendered headless to the scratchpad. No golden is recorded until the owner approves it.

## Out of scope
A CLI `humata agents enable|disable` (edit the file, or use the app); the machine page list of present agents; upgrading third-party CLIs; custom kinds in the app pickers (the app's agent model is the built-in enum).

## Lane (written by the manager)
- Manager: AGENTSET (Opus 5.5), opened 2026-11-16. Coordination: AGENTKINDS (acba338a379f7aff7) OK'd our `agent_service.rs` hunks as a SEAM script, no review needed; built-in order claude, codex, copilot, opencode, zsh from its m1 (main's round 27). MACHINESCRIPTS (a949f90c2075e5d12) OK'd our agents.toml shipping hunks in its files, anchored outside its w1/w4 spans (before the repos lines), validated after its w1..w4.
- Claims (new unless SEAM): `daemon/humatad/src/agent_switches.rs` and `daemon/humatad/src/agent_gate.rs` (new modules; no new crate, so no Cargo.toml change), SEAM `daemon/humatad/src/main.rs` mod list; SEAM `daemon/humatad/src/{agent_service.rs, session_service.rs, session_service/fork.rs, desk_service/start.rs, desk_service/channel_desk.rs, serve.rs, lib or main mod list}`, `daemon/humatad/BUILD.bazel`; SEAM `daemon/sessions/src/agents.rs` (settings path only); SEAM `daemon/machines/src/{stack_config,install,bundle,setup}.rs`, `daemon/humatad/src/machine_service/stack_settings.rs`; SEAM `api/proto/humata/v1/settings.proto`; root `Cargo.toml` members (main); `clients/src/daemon/agent_switches.rs` + `clients/src/daemon/fake/agent_switches.rs` + SEAM `clients/src/daemon.rs`, `clients/src/daemon/fake.rs`; `clients/live/src/agent_switches.rs` + SEAM `clients/live/src/{lib,daemon}.rs`; `features/settings/src/agents.rs`, `features/settings/tests/agent_switches.rs`, SEAM `features/settings/src/lib.rs`, `features/settings/BUILD.bazel`; SEAM `features/app/src/{lib,new_session}.rs` (launch load, default fallback); SEAM `ui/src/{settings,overlay_new_session,overlay_fork,overlay_desk_new,overlay_desk_channel,session_tabs,overlay_pr_review,overlay_submit_review}.rs`; `desktop/tests/native/agent_switches.rs` + SEAM `desktop/tests/native.rs`; docs SEAM `api/{GRPC,CLI,MCP}.md`, DECISIONS D720–D729.
- Tasks:
  - [x] T1 `agent_switches.rs` humatad module: read/write agents.toml, defaults, install commands, CLI-to-look-for · worker A (Haiku 5.5 high)
  - [x] T2 daemon: `agent_gate.rs` (ready, found, single-flight install, shown as a start step; no warm per D723), agent_service hunks, start-path calls, proto, agents.rs settings path, config shipping, docs · worker B (Sonnet medium), after T1's API (fixed in its brief)
  - [x] T3 app: client trait + fake + live, settings reducer + TestStore tests, Agents pane, picker filtering, native test · worker C (Sonnet medium)
- Scripts (applied in order p0, a, b, c, d): `<scratchpad>/agentset/` (`apply_*.py`, each with `--check`).
- Worker log: a (Haiku high) agent_switches.rs; b (Sonnet medium) gate, service, start checks, install step, shipping, docs; c (Sonnet medium) clients, reducer, pane, pickers, native test + frame render. All scripts validated as a chain p0, a, b, c, d (3 passes on a fresh live copy, passes 2 and 3 apply 0). Nothing compiled yet (main builds).
- Open for main: golden `29-permission-route` changes (Agents card above Default agent); frame render via AGENTSET_FRAME_OUT; found uses the daemon's PATH like the holder's spawn (a CLI installed off that PATH reports InstallFailed).
- Ready to commit: after main's build and tests pass
