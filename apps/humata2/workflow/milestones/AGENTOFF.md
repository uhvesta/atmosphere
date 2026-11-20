# AGENTOFF: an agent turned off is never picked, offered as a default or failed on

- **Status:** open (owner, 2026-11-20)
- **Ships:** 0.10.1
- **Depends on:** AGENTSET (done), the agent switches and the agent gate it added.
- **Unblocks:** none
- **Owner's words:** "there are some bugs in the app when i disable claude and codex and it defaults to them and fails because they dont exist. Can we document and address?"

## Outcome
With an agent turned off in Settings › Agents, nothing in the app picks it,
offers it as a default or fails on it. Every default falls to an agent that is
on, and with none on, the app says so and points to Settings.

## Exit criterion
- With Claude and Codex off and Copilot on, every default lands on Copilot:
  - New session, New desk session (Desk and channel Desk tab), Fork, Review a pull request;
  - CLI `session start` and MCP `session_start` with no agent, and a Desk start with no agent.
- With every agent off:
  - New session opens on Terminal with a note.
  - Desk, Fork and pull request review open nothing. A toast says why and offers Open Settings.
  - The daemon refuses a start that names no agent, naming Settings › Agents.
- A stored default that is off is kept and falls back at use time. Turning it back on restores it.
- The Settings › Agents default row says the saved default is off (the hunk is routed to the Settings lane).
- Tests: `features/app:agent_off_test`, `daemon/runtime:runtime_unit_test` (pick), and `daemon/humatad:humatad2_unit_test` (agent_gate pick and the refusals).

## Read these
- Decisions: D720–D725 (AGENTSET), D770 (a channel's start agent), D915 (this milestone's picks).
- SPEC: ST-15 (default agent chain).
- Code: `daemon/runtime/src/agent_switches.rs`, `daemon/humatad/src/agent_gate.rs`, `features/app/src/new_session.rs`, `features/settings/src/agents.rs` (`offered()`).

## Bug list

| # | Where | What fails | Fix |
|---|---|---|---|
| B1 | `daemon/humatad/src/session_service.rs:1513` `agent_for` | A CLI or MCP `session start` with no agent, a PR review start and an app start with no agent all take the chain channel → Settings → "made with", which is `claude`, whatever is on. The gate at :628 / :505 then refuses with "Claude is turned off". When the switch is on but the CLI is missing, the start fails on the missing CLI instead. | Walk the chain and take the first kind in it that is on, then the first on in Settings › Agents order (`agent_gate::pick`). With none on, refuse with `GateError::NoneOn`. A start that names an off agent is still refused. |
| B2 | `daemon/humatad/src/settings_service/sections.rs:619` `default_agent_for` | Collapses the channel default and the Settings default into one value. If the channel's pick is off, the Settings pick is never tried (Fable, D915 4). | New `default_agent_chain` returns `[channel, settings]`. `default_agent_for` keeps its meaning. |
| B3 | `daemon/humatad/src/serve.rs:180` `--default-agent` default `claude` → `svc_desk/src/desk_service/start.rs:226` `agent()` and `channel_desk.rs:528` `channel_agent()` | A Desk start that names no agent runs `claude` even when Claude is off, and is refused. | The Desk port gets `pick_agent` (`wire_desk.rs`, via `agent_gate::pick`). Both use it when no agent is named. |
| B4 | `daemon/humatad/src/channel_service.rs:741` `effective_agent` | A channel's start agent, shown to the app, can be a kind that is off. The app then pre-picks it. | Run the chain through `agent_gate::pick`. Fall back to the old value when every agent is off. |
| B5 | `daemon/humatad/src/session_service/fork.rs:365` | A fork that names no agent runs the parent's, with no gate when the agent is the same. So a fork of a Claude session with Claude off launches Claude and fails on the missing CLI. | Refuse with `GateError::ForkAgentOff`: "name an agent that is on for the fork, or turn X on". Don't switch agents silently, because a different agent changes what the fork carries (D915 3). |
| B6 | `daemon/humatad/src/session_service/continue_in.rs:104` | Continue in another channel restarts the source's agent with no gate. | Refuse with `GateError::ContinueAgentOff` (D915 6). |
| B7 | `features/app/src/new_session.rs:90` `default_agent` | With every agent off it returns the disabled preferred agent (`unwrap_or(preferred)`). It also skips Settings' pick when the channel's is off. | Use one shared `humata2_clients::pick_agent(offered, preferred chain)`, which returns `Result<Agent, NoAgentOn>`. Expose it as `AppState::default_agent`. |
| B8 | `features/app/src/new_session.rs:119` new session sheet | All off: the sheet opens on a disabled agent and Create fails. | Open on Terminal with a `note` (Fable wording); the overlay shows it as a note. |
| B9 | `ui/desk/src/channel_desk.rs:288` | The channel Desk tab sends `preferences.default_agent` raw, with no fallback. | Use `state.default_agent(Some(channel))`. `features/app/src/desk.rs` `desk_delegate` also corrects any Desk form opened with an off agent, and on all off closes it and toasts. |
| B10 | `features/app/src/desk.rs:36,58` Desk new forms | All off: the form opens on a disabled agent. | Open nothing; toast with `ToastAction::OpenAgentSettings`. |
| B11 | `features/app/src/pr_review.rs:85` | All off: the dialog opens on a disabled agent. | Open nothing; toast with Open Settings. |
| B12 | `features/session/src/lib.rs:1742` fork draft `agent: None` → parent's agent; `ui/overlays/src/overlay_fork.rs:189` `.unwrap_or(Agent::Claude)` | The Fork dialog defaults to the parent's agent even when it is off, and to Claude with no parent. | In `features/app/src/desk.rs` `fork_unless_desk`, preset `fork.agent` to the first agent that is on, with `more = true`. All off: close the draft and toast. The overlay falls back to the first offered agent (else Terminal) and shows "X is off, so this fork starts with Y." |
| B13 | `features/pr_review/src/dialog.rs:111` `DialogState::default` `agent: Agent::Claude` | Only a placeholder, since `Open` sets the agent. No user-visible bug once B11 is fixed. | None. |
| B14 | `features/settings/src/lib.rs:163` `default_agent: Agent::Claude` | The saved default may be off. Every use now falls back (B7), so it is never started. | Keep it, since it is the stored pick (D915 2). Settings row: see B15. |
| B15 | `ui/settings/src/settings.rs:866` Default agent row | Lists only agents that are on, so an off saved default shows nothing selected and no reason. | Under the buttons, a dim line naming the off default and the fallback (Fable wording). **Held file: hunk routed to main.** |
| B16 | `features/settings/src/agents.rs:21` `offered()` | Until the daemon answers, every agent counts as on, so a sheet opened in the first instant can pre-pick Claude. | Not fixed: the switches load at launch, and the daemon falls back too (B1). Logged as known. |
| B17 | Model defaults | Checked: the daemon applies only `request.model`, and the app sends none by default. A Settings default model is never sent with a fallen-back agent. | None. |
| B18 | CLI `cli/src/commands/sessions.rs:233`, MCP `daemon/svc_mcp/src/mcp/session_start.rs:68` | They send no agent, so they hit B1. | Fixed by B1. |

## Out of scope
- The Settings › Agents switches themselves (AGENTSET).
- Install-when-missing.

## Lane (written by the manager)

Manager: minion AGENTOFF (Opus 5.5), 2026-11-20.

- **Design notes:**
  - **One rule, two homes.** The rule: the first kind in the preference chain that is on, else the first on in Settings › Agents order. The plain shell counts only when named. None on means a refusal that names Settings › Agents.
    - Daemon: `AgentSwitches::pick(preferred, order)` in `daemon/runtime`, wrapped by `agent_gate::pick(agents, preferred)`, which returns `GateError::NoneOn`.
    - App: `humata2_clients::pick_agent(offered, preferred) -> Result<Agent, NoAgentOn>` in `clients/src/daemon/agent_switches.rs`, wrapped by `AppState::default_agent(channel)`.
  - **Chain** (Fable, D915 4):
    - daemon: channel's own default → Settings' → the channel's "made with" → first on;
    - app: the channel's start agent (from the daemon) → Settings' → first on.
  - **Stored defaults stay and fall back at use time.**
  - **The app layer** (`features/app`) does all fallbacks, because `features/settings` and `ui/settings` are held by other lanes.
- **Claims** (new to the WORKLOG table, via main):
  - `workflow/milestones/AGENTOFF.md`
  - `daemon/runtime/src/agent_switches.rs` (pick + tests)
  - `daemon/humatad/src/agent_gate.rs`
  - `daemon/humatad/src/session_service.rs` (the `agent_for` hunk only)
  - `daemon/humatad/src/session_service/fork.rs` (the agent block at :365 only)
  - `daemon/humatad/src/session_service/continue_in.rs`
  - `daemon/humatad/src/settings_service/sections.rs` (`default_agent_chain` hunk)
  - `daemon/humatad/src/channel_service.rs` and `channel_service/{add,rename_folder,remove_machine_apply}.rs` (`effective_agent`, `readable_channels`, `tell_channel` and its calls)
  - `daemon/humatad/src/session_service/{forget,remote_review}.rs` (`tell_channel` calls only)
  - `features/app/src/lib.rs` (one hunk at the end of `reduce`)
  - `clients/src/api.rs` (re-export hunk)
  - `daemon/svc_desk/src/ports.rs` (`pick_agent` port method + its test fake)
  - `daemon/svc_desk/src/desk_service/{start.rs,channel_desk.rs}` (agent fns)
  - `daemon/humatad/src/wire_desk.rs`
  - `clients/src/daemon/agent_switches.rs`
  - `clients/src/daemon.rs` (one `pub use` hunk)
  - `daemon/humatad/src/settings_service.rs` (one `pub(crate) use` line)
  - `features/app/src/{new_session.rs,desk.rs,pr_review.rs,toast.rs}`
  - `features/app/tests/agent_off.rs` (new)
  - `ui/desk/src/channel_desk.rs`
  - `ui/overlays/src/{overlay_fork.rs,overlay_new_session.rs,overlay_toasts.rs}`
  - `DECISIONS.md` (the D915 hunk)
- **Tasks:**
  - [ ] T0 failing-first tests · manager · `features/app/tests/agent_off.rs`, runtime `pick` tests · done
  - [ ] T1 daemon pick + refusals · worker daemon (Sonnet, medium) · daemon files above · written, reviewed, awaiting build. `effective_agent` takes `&Agents` and is used by GetChannel, ListChannels and every channel event (`tell_channel` callers in channel_service/**, session_service/{forget,remote_review}.rs, settings_service.rs). Also touched `settings_service.rs` (re-export line and two `tell_channel` calls).
  - [ ] T2 app pick, toasts, fork preset, overlays · worker app (Sonnet, medium) · clients/features/app/ui files above · written, reviewed, awaiting build
  - [ ] T3 Settings row note (B15) · held: hunk in the report to main
- **Waiting on:** main's build gate; main routes the B15 hunk to the Settings lane.
- **Follow-ups:** channels aren't re-told when a switch flips, so a channel's shown start agent can lag until its next event. The app's own fallback still never starts an off agent. The fix is in `daemon/humatad/src/serve.rs`, which BROWSER holds. Main applies this exact hunk when it is free:

  ```diff
  @@ (~line 585, beside the other per-service clones)
       let inbox_agents = agents.clone();
       let ready_agents = agents.clone();
  +    // AGENTOFF (D915): a flipped switch re-tells every channel's start agent.
  +    let channel_agents = agents.clone();
  @@ (~line 1244, inside `Some((work, host_links, host_reports)) => {`)
               // D782: a changed agents.toml goes to every linked machine now,
               // not only at its next link-up.
               let pushes = work.clone();
  -            agents_changed.set(move || machine_service::agents_changed(&pushes));
  +            let (retell_feed, retell_store) = (feed.clone(), store.clone());
  +            agents_changed.set(move || {
  +                machine_service::agents_changed(&pushes);
  +                // AGENTOFF (D915): a switch that flips can change every
  +                // channel's start agent, so the app hears each again.
  +                if let Ok(runtime) = tokio::runtime::Handle::try_current() {
  +                    let (feed, store, agents) = (retell_feed.clone(), retell_store.clone(), channel_agents.clone());
  +                    runtime.spawn_blocking(move || {
  +                        crate::channel_service::tell_every_channel(&feed, &store, &agents, None);
  +                    });
  +                }
  +            });
  ```

  `OnChange` is set once, so the re-tell rides the one closure that is set. On a profile with no machine service, nothing calls `set`, so nothing is re-told there. If that matters, add an `else` arm that sets a re-tell-only closure. `tell_every_channel` is already in `channel_service.rs` and used by Settings' save.
  - [ ] T4 Continue-in (D915 5, app side): `App::continue_unless_agent_off` after every action closes the dialog and toasts with Open Settings · manager · written
- **Build gates:** none yet.
- **Ready to commit:** no
