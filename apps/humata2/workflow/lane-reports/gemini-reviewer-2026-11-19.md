warning: --mode auto is not available for this account; continuing in the default mode.
## Files changed, new, and deleted

### New
- [agent_tabs.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_terminal/src/agent_tabs.rs): daemon-side D858 items 1–2 implementation (`AgentTabError` + `to_status`, `ShellTabEntry`, `set_tab_agent`/`tab_agent`/`clear_tab_agent`, `shell_tabs`/`shell_tabs_by_session`/`tab_index`, `AgentTabLaunch`, `resolve_launch`, `build_holder_spec`, `save_agent_tab`, `deliver_prompt`) and unit tests.

### Changed
- [terminal.proto](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/api/proto/humata/v1/terminal.proto): added `TerminalService.OpenAgentTab(OpenAgentTabRequest) returns (OpenAgentTabResponse)`, `Tab.agent = 10`, `OpenAgentTabRequest{idempotency_key, session, agent, prompt}`, and `OpenAgentTabResponse{tab, index}`.
- [lib.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_terminal/src/lib.rs): exported `agent_tabs` and re-exported `AgentTabError`, `AgentTabLaunch`, `ShellTabEntry`.
- [remote_terminal.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_terminal/src/remote_terminal.rs): added `RemoteTerminal::open_agent_tab`, updated test `Machine` `TerminalService` impl, and added `open_agent_tab_forwards_request_and_returns_tab_and_index`.
- [running_holders.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_terminal/src/running_holders.rs): surfaced persisted `tab_agent` on running/stopped session shell tabs, cleared `tab_agent` on `end()`, and added `ending_an_agent_shell_tab_clears_its_persisted_agent_kind`.
- [daemon.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/clients/src/daemon.rs): added `Operation::OpenAgentTab` and `Terminals::open_agent_tab` (additive only).
- [fake.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/clients/src/daemon/fake.rs): implemented `FakeDaemon::open_agent_tab` and added `open_agent_tab_tests` (additive only).
- [terminals.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/clients/live_daemon/src/terminals.rs): implemented `LiveDaemon::open_agent_tab` over `TerminalService.OpenAgentTab`, added `resolve_agent_tab_index` helper and unit tests.
- [agent_tab.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/ui/changes/src/panel_changes/agent_tab.rs): guarded click against missing agents, read the open session from `state.workspace` (falling back to `state.session.workspace`), and dispatched `SessionAction::OpenAgentTab` (routed to `Terminals::open_agent_tab`).
- [lib.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/features/app/src/lib.rs): routed `AppAction::Session(SessionAction::OpenAgentTab { agent, prompt })` through `Terminals::open_agent_tab` with missing-agent guard, and synced the returned `Session` + `terminal_agents` into `state.workspace` and `state.session.agent_tabs` in `AgentTabOpened`.
- [agent_tab_label.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/features/app/tests/agent_tab_label.rs): added tests for direct `open_agent_tab` call + simulated app-restart persistence and missing-agent guard.
- [REVIEWER.md](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/workflow/milestones/REVIEWER.md): logged the D858 items 1–2 batch and seam lines.

### Deleted
- None.

---

## Seam lines for the main agent

### S1. `apps/humata2/api/proto/humata/v1/sessions.proto` (MASSDELETE's)
1. **Anchor:** immediately above `message Session {` ([sessions.proto](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/api/proto/humata/v1/sessions.proto#L171)):
```protobuf
// D858: an extra terminal tab of a session (shell or added agent), kept
// on the daemon so its label and agent survive app restarts and reloads.
message ShellTab {
  TabId tab = 1;
  string label = 2;
  // Unset for a plain shell tab; set for an agent tab opened through
  // TerminalService.OpenAgentTab.
  AgentId agent = 3;
}

```
2. **Anchor:** inside `message Session`, immediately after `google.protobuf.Timestamp status_since = 35;` ([sessions.proto](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/api/proto/humata/v1/sessions.proto#L233)):
```protobuf
  // D858: extra terminal tabs after the primary agent (oldest first), with
  // each tab's daemon-kept label and agent kind.
  repeated ShellTab shell_tabs = 36;
```

### S2. `apps/humata2/daemon/humatad/src/terminal.rs`
1. **Anchor:** inside `TerminalsConfig`, immediately after `pub(crate) started: Arc<crate::restart::Started>,` ([terminal.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/terminal.rs#L188)):
```rust
    /// D858: configured agents and home folder for `OpenAgentTab`.
    pub(crate) agents: humata2_sessions::Agents,
    pub(crate) home: PathBuf,
```
2. **Anchor:** inside `Core::describe`, immediately after `let program = { ... };` ([terminal.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/terminal.rs#L921)):
```rust
        let tab_agent = {
            let (store, id) = (self.config.store.clone(), tab.id.clone());
            blocking(move || humata2_svc_terminal::agent_tabs::tab_agent(&store, &id))
                .await
                .ok()
                .and_then(Result::ok)
                .flatten()
        };
```
   And inside the `pb::Tab { ... }` literal at line 938, right before `}`:
```rust
            agent: tab_agent.map(|value| pb::AgentId { value }),
```
3. **Anchor:** inside `Core::open_shell` -> after `Core::open_shell` closing `}` ([terminal.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/terminal.rs#L1031)):
```rust
    /// D858: starts `agent` directly under a holder of its own in `session`'s
    /// folder, saves its tab with persisted `label` and `agent` kind, and
    /// schedules `prompt` delivery once the agent is ready.
    async fn open_agent(
        &self,
        session: &SessionRecord,
        agent: &str,
        prompt: String,
    ) -> Result<pb::OpenAgentTabResponse, Status> {
        let config = &self.config;
        let tab_uuid = uuid::Uuid::new_v4();
        let id = format!("tb_{}", tab_uuid.simple());
        let socket = holder_socket(&config.holders, tab_uuid);
        let log = config.logs.join(format!("{id}.log"));
        let identities = config.identities.clone();
        let (mint_id, mint_record) = (id.clone(), session.clone());
        let token = blocking(move || identities.mint(&mint_id, &mint_record))
            .await?
            .map_err(|error| {
                status::store(error, &config.machine, "save the tab's session token")
            })?;
        let env = identity::tab_env(token, &config.socket);
        let launch = humata2_svc_terminal::AgentTabLaunch {
            holder: config.holder.clone(),
            holder_socket: socket.clone(),
            log: log.clone(),
            home: config.home.clone(),
            cwd: session.folder.clone(),
            env,
            cols: INITIAL_COLS,
            rows: INITIAL_ROWS,
            scrollback_bytes: config.scrollback_bytes,
        };
        let (canonical_agent, spec) =
            humata2_svc_terminal::agent_tabs::build_holder_spec(&config.agents, agent, &launch)
                .map_err(|error| humata2_svc_terminal::agent_tabs::to_status(&error))?;
        let (builds, tab_id_copy) = (config.builds.clone(), id.clone());
        blocking(move || builds.record_shell(&tab_id_copy)).await?;
        let spawned = match blocking(move || humata2_holder::spawn(&spec)).await? {
            Ok(spawned) => spawned,
            Err(error) => {
                self.revoke(&id).await;
                return Err(holder_failed("start the agent tab", &error, &log));
            }
        };
        let tab = TabRecord {
            id,
            session: Some(session.id.clone()),
            kind: TabKind::Shell,
            title: canonical_agent.clone(),
            holder_socket: socket.clone(),
            holder_pid: spawned.pid,
            created_unix_ms: unix_ms(),
            holder_generation: 0,
        };
        let store = config.store.clone();
        let saved = tab.clone();
        let agent_name = canonical_agent.clone();
        let index = match blocking(move || {
            humata2_svc_terminal::agent_tabs::save_agent_tab(&store, &saved, &agent_name)
        })
        .await?
        {
            Ok(index) => index,
            Err(error) => {
                if let Err(end_err) = holder_end::shut_or_end(
                    &tab.holder_socket,
                    tab.holder_pid,
                    &format!("new agent tab {}", tab.id),
                )
                .await
                {
                    crate::log::say!("humatad2: {end_err}");
                }
                self.revoke(&tab.id).await;
                return Err(humata2_svc_terminal::agent_tabs::to_status(&error));
            }
        };
        let described = self.describe(&tab).await;
        self.config
            .tell
            .session(&session.id, pb::Change::Updated, None)
            .await;
        humata2_svc_terminal::agent_tabs::deliver_prompt(socket, prompt);
        Ok(pb::OpenAgentTabResponse {
            tab: Some(described),
            index,
        })
    }
```
4. **Anchor:** immediately after `Core::shell_in_session` closing `}` ([terminal.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/terminal.rs#L1385)):
```rust
    async fn agent_in_session(
        &self,
        caller: &Caller,
        request: pb::OpenAgentTabRequest,
        approved: bool,
    ) -> Result<Gated<pb::OpenAgentTabResponse>, Status> {
        let key = idempotency("OpenAgentTab", request.idempotency_key.clone());
        let session = id_of(request.session.clone().map(|session| session.value))
            .ok_or_else(|| status::missing("session"))?;
        let session = self.session(&session).await?;
        desk_service::refuse_if_closed(&self.config.store, &self.config.machine, &session.id)
            .await?;
        if let Some(asked) = self.gate_input(caller, self.session_target(&session), approved)? {
            return Ok(Gated::Ask(asked));
        }
        if let Some((machine, remote)) = self.elsewhere(&session) {
            return Ok(Gated::Done(
                remote.open_agent_tab(&machine, request).await?.into_inner(),
            ));
        }
        if let Some(Outcome::AgentTab(reply)) = self.recall(key.as_deref()) {
            return Ok(Gated::Done(reply));
        }
        let agent = request
            .agent
            .map(|agent| agent.value)
            .unwrap_or_default();
        let reply = self.open_agent(&session, &agent, request.prompt).await?;
        self.remember(key, Outcome::AgentTab(reply.clone()));
        Ok(Gated::Done(reply))
    }
```
5. **Anchor:** inside `Core::close_one`, replace `blocking(move || store.remove_tab(&id))` ([terminal.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/terminal.rs#L1486)):
```rust
        blocking(move || {
            store.remove_tab(&id)?;
            let _ = humata2_svc_terminal::agent_tabs::clear_tab_agent(&store, &id);
            Ok::<_, StoreError>(())
        })
```
6. **Anchor:** inside `impl pb::terminal_service_server::TerminalService for Terminals`, immediately after `open_shell_tab` ([terminal.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/terminal.rs#L1807)):
```rust
    async fn open_agent_tab(
        &self,
        request: Request<pb::OpenAgentTabRequest>,
    ) -> Result<Response<pb::OpenAgentTabResponse>, Status> {
        let caller = identity::caller(&request)?;
        crate::stop::gate::require_running(&caller)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.agent_in_session(&caller, request.clone(), false).await;
        let (core, who) = (self.core.clone(), caller.clone());
        answer_gated(
            &self.config.inbox,
            &caller,
            key,
            "humata.v1.OpenAgentTabResponse",
            first,
            move || async move { core.agent_in_session(&who, request, true).await },
        )
        .await
    }
```
7. **Anchor:** inside `enum Outcome`, immediately after `Tab(pb::Tab),` ([terminal.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/terminal.rs#L2060)):
```rust
    AgentTab(pb::OpenAgentTabResponse),
```

### S3. `apps/humata2/daemon/humatad/src/serve.rs`
- **Anchor:** inside `Terminals::new(TerminalsConfig { ... })`, immediately after `started: last_start.clone(),` ([serve.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/serve.rs#L788)):
```rust
        agents: starter.agents.clone(),
        home: starter.home.clone(),
```

### S4. `apps/humata2/daemon/humatad/src/session_service.rs`
1. **Anchor:** inside `struct Loaded`, immediately after `moved: Option<&'static str>,` ([session_service.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/session_service.rs#L321)):
```rust
    /// D858: extra terminal tabs (shell or added agent), oldest first.
    shell_tabs: Vec<pb::ShellTab>,
```
   And inside `Loaded::read` (line 339, right before `})`):
```rust
            shell_tabs: humata2_svc_terminal::agent_tabs::shell_tabs(store, key)
                .unwrap_or_default()
                .into_iter()
                .map(|entry| pb::ShellTab {
                    tab: Some(pb::TabId { value: entry.tab }),
                    label: entry.label,
                    agent: entry.agent.map(|value| pb::AgentId { value }),
                })
                .collect(),
```
2. **Anchor:** in `Tell::session_when` (line 283) and `SessionSvc::show` (line 1422), immediately after `shown.pull_request = loaded.review.as_ref().map(pr_binding);`:
```rust
        shown.shell_tabs.clone_from(&loaded.shell_tabs);
```
3. **Anchor:** in `list_sessions`, inside the blocking store closure around line 2513 (add `let shell_tabs = humata2_svc_terminal::agent_tabs::shell_tabs_by_session(&store).unwrap_or_default();` and return it in the tuple), then right after `shown.pull_request = reviews.get(&session.id).cloned();` ([session_service.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/session_service.rs#L2561)):
```rust
                shown.shell_tabs = shell_tabs
                    .get(&session.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|entry| pb::ShellTab {
                        tab: Some(pb::TabId { value: entry.tab }),
                        label: entry.label,
                        agent: entry.agent.map(|value| pb::AgentId { value }),
                    })
                    .collect();
```

### S5. `apps/humata2/clients/live_core/src/map.rs`
1. **Anchor:** in `map::session`, replace lines 362–367 (`shells: ... terminal_agents: BTreeMap::new(),` in [map.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/clients/live_core/src/map.rs#L362-L367)) by computing `(shells, terminal_agents)` right before `Session {`:
```rust
    let (shells, terminal_agents) = if !session.shell_tabs.is_empty() {
        let mut shells = Vec::with_capacity(session.shell_tabs.len());
        let mut terminal_agents = BTreeMap::new();
        for (pos, entry) in session.shell_tabs.iter().enumerate() {
            shells.push(entry.label.clone());
            if let Some(agent_id) = entry.agent.as_ref().filter(|id| !id.value.is_empty()) {
                let kind = agents.agent(&agent_id.value);
                if kind != Agent::Shell {
                    terminal_agents.insert(pos + 1, kind);
                }
            }
        }
        (shells, terminal_agents)
    } else {
        let mut shells = Vec::new();
        let mut terminal_agents = BTreeMap::new();
        for (pos, tab) in tabs
            .iter()
            .filter(|tab| tab.kind == pb::tab::Kind::Shell as i32)
            .enumerate()
        {
            shells.push(tab.title.clone());
            if let Some(agent_id) = tab.agent.as_ref().filter(|id| !id.value.is_empty()) {
                let kind = agents.agent(&agent_id.value);
                if kind != Agent::Shell {
                    terminal_agents.insert(pos + 1, kind);
                }
            }
        }
        (shells, terminal_agents)
    };
```
   And inside `Session { ... }`:
```rust
        shells,
        terminal_agents,
```
2. Add a unit test in `clients/live_core/src/map.rs` verifying `pb::Session.shell_tabs` populates both `Session.shells` and `Session.terminal_agents` (and falls back to `tabs` + `tab.agent` when `shell_tabs` is empty).

### S6. Existing desktop tests for pre-D858-item-2 shell-typing behavior
1. **`apps/humata2/desktop/tests/native/diff_full_screen.rs`** ([diff_full_screen.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/desktop/tests/native/diff_full_screen.rs#L226-L245)):
   - Replace `an_agent_pick_waits_with_its_prompt_and_terminal_has_none` assertion on `awaits_opening_prompt` with checking `app.fakes.daemon.calls()` contains `open_agent_tab` with `copilot` and the opening prompt (and no `send_to_tab`), while clicking `new-agent-tab-zsh` calls `open ... zsh` and never `open_agent_tab`.
2. **`apps/humata2/desktop/tests/live_flows/reviewer.rs`** ([reviewer.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/desktop/tests/live_flows/reviewer.rs#L125-L155)):
   - Because `OpenAgentTab` now launches the test agent directly instead of a bash shell, replace `typing::wait_for(&mut app, "claude")` with `typing::wait_for(&mut app, "got: Look at the diff")` and update `prove_live` to type a line to the test agent (`"ping reviewer"` -> `typing::wait_for(app, "got: ping reviewer")`) rather than waiting for a `$` shell prompt and running `echo WIRE$((40+2))`. Also assert after reboot (`world.reboot()`) that `terminal_agents` still records `Agent::Claude` on the reviewer tab!

### S7. `apps/humata2/DECISIONS.md` (next free D number, Provisional)
- **Anchor:** append at end of [DECISIONS.md](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/DECISIONS.md):
```markdown
## D877: Extra terminal tabs persist label and agent on the daemon, and OpenAgentTab launches agents directly (REVIEWER manager, 2026-11-19; D858 items 1–2). Provisional
- `Session.shell_tabs` (`repeated ShellTab { tab, label, agent }`) and `Tab.agent` carry every extra terminal tab's persisted label (`TabRecord.title`) and agent kind (`tab_agent:<tab_id>` in the daemon store), so agent-tab labels, dots, and counts survive daemon and app restarts without client-side memory.
- `TerminalService.OpenAgentTab { idempotency_key, session, agent, prompt } -> { tab, index }` resolves a non-shell agent via `Agents`, spawns it directly under its own holder with a fresh conversation id in the session folder, persists its label and agent kind, notifies session subscribers, and delivers `prompt` in the background once the agent reads keys. Plain shells are refused with `InvalidArgument` (`OpenShellTab` stays for shells).
- Extra agent tabs stay `TabKind::Shell` in `tabs` so they sit after tab 0, remain closable via `CloseTab` / `End for good`, and never hijack session-wide `typing_tab`.
- **Reverse:** remove `OpenAgentTab` and `Tab.agent` from `terminal.proto`, remove `ShellTab` / `shell_tabs` from `sessions.proto`, drop `svc_terminal/src/agent_tabs.rs`, and restore `AppAction::Session(SessionAction::OpenAgentTab)` pass-through to client-side shell typing.
```

---

## Focused Bazel targets to build and test

After applying seams S1–S6:
```
//apps/humata2/api/proto:terminal_rust
//apps/humata2/api/proto:sessions_rust
//apps/humata2/daemon/svc_terminal:svc_terminal_unit_test
//apps/humata2/clients:clients_unit_test
//apps/humata2/clients/live_core:live_core_unit_test
//apps/humata2/clients/live_daemon:live_daemon_unit_test
//apps/humata2/features/app:agent_tab_label_test
//apps/humata2/features/app:missing_agent_tab_test
//apps/humata2/features/session:agent_count_test
//apps/humata2/features/session:opening_prompt_test
//apps/humata2/ui/changes:changes
//apps/humata2/daemon/humatad:humatad2_test_away_board
//apps/humata2/desktop:native_test
//apps/humata2/desktop:live_flows_test
```

---

## Goldens and frames for Fable
- **None.** No visual layout, spacing, copy, or styling changed in `ui/changes/src/panel_changes/agent_tab.rs`; existing frames 3-3 / 3-4 stay exact.

---

## Still open
- Applying seams S1–S7 and running the focused Bazel build/test gate (plus lint without `--fast`).
- Adding a real-daemon `humatad` integration test in `daemon/humatad/tests/terminal.rs` covering `TerminalService.OpenAgentTab` (idempotency retry, `Session.shell_tabs` persistence across daemon restart, `CloseTab` clearing `tab_agent`, and remote relay) once main wires S1–S4.
- Real-agent Copilot review run (still open from REVIEWER exit criteria).
agy exit 0 for reviewer
