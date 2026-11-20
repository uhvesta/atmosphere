//! D857: an agent the session's machine lacks can't be picked from New agent
//! tab, and one that is there still can. Until the machine's health is read,
//! every agent counts as installed.
use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::machines::{MachineDetail, MachineHealth, Reach};
use humata2_clients::model::{Agent, SessionId};
use humata2_clients::fixture;
use humata2_hosts::HostsAction;
use humata2_review::ReviewAction;
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

fn setup() -> TestStore<App> {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.settle();
    store.send(A::Navigate(Route::Session(ledger())), |_| {});
    store.settle();
    store
}

fn claude_only(machine: humata2_clients::model::MachineId) -> MachineDetail {
    MachineDetail {
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
    }
}

#[test]
fn picking_a_missing_agent_does_nothing_and_an_installed_one_is_remembered() {
    let mut store = setup();
    assert!(store.state().missing_agents().is_empty(), "unknown health lists every agent as installed");
    let machine = store
        .state()
        .workspace
        .session(&ledger())
        .map(|session| session.machine.clone())
        .expect("ledger is listed");
    store.send(A::Hosts(HostsAction::Health(machine.clone(), Ok(claude_only(machine)))), |_| {});
    let missing = store.state().missing_agents();
    assert!(missing.contains(&Agent::Codex));
    assert!(!missing.contains(&Agent::Claude));

    store.send(A::Review(ReviewAction::AgentTabPicked(Agent::Codex)), |_| {});
    assert_eq!(store.state().review.last_agent_tab, None);
    store.send(A::Review(ReviewAction::AgentTabPicked(Agent::Claude)), |_| {});
    assert_eq!(store.state().review.last_agent_tab, Some(Agent::Claude));
}
