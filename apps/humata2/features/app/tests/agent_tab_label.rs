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
    let mut store = setup();
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
