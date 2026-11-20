//! D858: a tab opened from New agent tab stays listed as running its agent
//! when the daemon sends the session again, which knows only a shell there.
use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::model::{Agent, SessionId};
use humata2_clients::{Event, Workspace, fixture};
use humata2_session::SessionAction;
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

fn setup() -> (TestStore<App>, humata2_clients::Fakes) {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.settle();
    store.send(A::Navigate(Route::Session(ledger())), |_| {});
    store.settle();
    (store, fakes)
}

/// The tabs of `ledger` listed as running Codex, in `workspace`.
fn codex_tabs(workspace: &Workspace) -> Vec<usize> {
    workspace
        .session(&ledger())
        .map(|session| {
            session
                .terminal_agents
                .iter()
                .filter(|(_, agent)| **agent == Agent::Codex)
                .map(|(tab, _)| *tab)
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn an_agent_tab_keeps_its_agent_when_the_daemon_sends_the_session_again() {
    let (mut store, _fakes) = setup();
    let before = codex_tabs(&store.state().workspace);
    store.send(
        A::Session(SessionAction::OpenAgentTab {
            agent: Agent::Codex,
            prompt: "Look at the changes in the Diff panel.".to_owned(),
        }),
        |_| {},
    );
    store.receive_matching(
        "the agent's tab opened",
        |action| matches!(action, A::Session(SessionAction::AgentTabOpened { .. })),
        |_| {},
    );
    store.settle();
    // The session feature lists the new tab as running Codex at once.
    let opened = codex_tabs(&store.state().session.workspace);
    assert_eq!(opened.len(), before.len() + 1, "the new tab is listed as running Codex");

    // The daemon then sends the session, knowing the tab only as a shell.
    let mut sent = store.state().session.workspace.session(&ledger()).cloned().expect("ledger is listed");
    sent.terminal_agents.clear();
    store.send(A::Event(Event::Session(sent)), |_| {});
    store.settle();

    let new: Vec<usize> = opened.into_iter().filter(|tab| !before.contains(tab)).collect();
    let after = codex_tabs(&store.state().workspace);
    assert!(new.iter().all(|tab| after.contains(tab)), "{new:?} still runs Codex: {after:?}");
}

#[test]
fn open_agent_tab_calls_daemon_directly_and_label_survives_simulated_restart() {
    let (mut store, fakes) = setup();
    let before_shells = store
        .state()
        .workspace
        .session(&ledger())
        .map_or(0, |s| s.shells.len());
    let prompt = "Review the changes in the Diff panel.";
    store.send(
        A::Session(SessionAction::OpenAgentTab {
            agent: Agent::Codex,
            prompt: prompt.to_owned(),
        }),
        |_| {},
    );
    store.receive_matching(
        "the agent's tab opened",
        |action| matches!(action, A::Session(SessionAction::AgentTabOpened { .. })),
        |_| {},
    );
    store.settle();

    let calls = fakes.daemon.calls();
    assert!(
        calls
            .iter()
            .any(|call| call.starts_with("open_agent_tab ledger codex") && call.contains(prompt)),
        "expected open_agent_tab call with agent and prompt, got: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.starts_with("send_to_tab")),
        "daemon handles the opening prompt directly without client-side shell typing: {calls:?}"
    );

    // Simulate an app restart: drop in-memory agent_tabs and reload from daemon.
    let reloaded = fakes.daemon.workspace();
    let (clients, _) = humata2_clients::fake_clients(reloaded.clone(), Arc::new(TestClock::new()));
    let mut restarted = TestStore::new(AppState::launch(&clients), App::new(clients)).non_exhaustive();
    restarted.send(A::Loaded(Ok(reloaded)), |_| {});
    restarted.settle();
    let session = restarted
        .state()
        .workspace
        .session(&ledger())
        .expect("ledger exists after reload");
    let tab = before_shells + 1;
    assert_eq!(session.shells.get(tab - 1).map(String::as_str), Some("codex"));
    assert_eq!(session.terminal_agents.get(&tab), Some(&Agent::Codex));
}

#[test]
fn open_agent_tab_ignores_an_agent_missing_on_the_sessions_machine() {
    use humata2_clients::machines::{MachineDetail, MachineHealth, Reach};
    use humata2_hosts::HostsAction;

    let (mut store, fakes) = setup();
    let machine = store
        .state()
        .workspace
        .session(&ledger())
        .expect("ledger")
        .machine
        .clone();
    store.send(
        A::Hosts(HostsAction::Health(
            machine.clone(),
            Ok(MachineDetail {
                machine,
                os: "Ubuntu 24.04".to_owned(),
                arch: "x86_64".to_owned(),
                address: "devbox-1.lan".to_owned(),
                reach: Reach::Ssh,
                daemon_version: "2.0.3".to_owned(),
                health: MachineHealth {
                    gh: None,
                    agents: vec![(Agent::Claude, "2.4.1".to_owned())],
                    disk_free: 1,
                    disk_total: 2,
                    variables: Vec::new(),
                    skills_problem: None,
                    git_version: String::new(),
                    gh_version: String::new(),
                },
                folders: Vec::new(),
                sessions: Vec::new(),
                rules: Vec::new(),
                stop: None,
            }),
        )),
        |_| {},
    );
    let before = store
        .state()
        .workspace
        .session(&ledger())
        .map_or(0, |s| s.shells.len());
    store.send(
        A::Session(SessionAction::OpenAgentTab {
            agent: Agent::Codex,
            prompt: "Look at the diff.".to_owned(),
        }),
        |_| {},
    );
    store.settle();
    assert_eq!(
        store
            .state()
            .workspace
            .session(&ledger())
            .map_or(0, |s| s.shells.len()),
        before
    );
    assert!(
        !fakes
            .daemon
            .calls()
            .iter()
            .any(|call| call.starts_with("open_agent_tab")),
        "missing agent must not invoke open_agent_tab"
    );
}
