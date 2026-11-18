//! SE-2/3: launch choices stay in the selected channel; a pending launch is stable.
use humata2_app::{App, AppAction as A, AppState, NewSessionAction as N, Overlay, Route};
use humata2_clients::{
    AgentSwitch, Clients, fixture,
    model::{Agent, ChannelId, MachineId},
};
use std::sync::Arc;
use tca::{TestClock, TestStore};

#[test]
fn changing_channel_selects_an_online_host_in_that_channel() {
    let mut workspace = fixture::design();
    workspace
        .channels
        .iter_mut()
        .find(|c| c.id.as_str() == "infra")
        .unwrap()
        .machines = vec![MachineId::new("devbox-1")];
    workspace.sessions.retain(|s| s.channel.as_str() != "infra");
    let (clients, _) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(workspace);
    state.route = Route::Channel(ChannelId::new("infra"));
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::StartNewSession, |_| {});
    let Some(Overlay::NewSession(sheet)) = store.state().overlay() else {
        panic!("launch dialog")
    };
    assert_eq!(sheet.machine, Some(MachineId::new("devbox-1")));
    store.send(
        A::NewSession(N::Channel(ChannelId::new("payments-rewrite"))),
        |_| {},
    );
    store.send(
        A::NewSession(N::Machine(MachineId::new("this-mac"))),
        |_| {},
    );
    store.send(A::NewSession(N::Channel(ChannelId::new("infra"))), |_| {});
    let Some(Overlay::NewSession(sheet)) = store.state().overlay() else {
        panic!("launch dialog")
    };
    assert_eq!(sheet.machine, Some(MachineId::new("devbox-1")));
    store.finish();
}

#[test]
fn a_pending_launch_ignores_repeated_start_and_field_changes() {
    let (clients, fakes) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(fixture::design());
    state.route = Route::Channel(ChannelId::new("payments-rewrite"));
    // Model the in-flight state directly: duplicate actions must create no effect.
    let app = App::new(clients);
    let mut setup = TestStore::new(state, app).non_exhaustive();
    setup.send(A::StartNewSession, |_| {});
    let mut state = setup.state().clone();
    setup.finish();
    let Some(Overlay::NewSession(sheet)) = state.overlays.last_mut() else {
        panic!("launch dialog")
    };
    sheet.creating = true;
    let count = fakes.daemon.workspace().sessions.len();
    let (clients, fakes) = Clients::fake(fakes.daemon.workspace(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(state, App::new(clients));
    store.send(A::NewSession(N::Create), |_| {});
    store.send(A::NewSession(N::Title("Changed".into())), |_| {});
    store.send(A::NewSession(N::Channel(ChannelId::new("infra"))), |_| {});
    store.finish();
    assert_eq!(fakes.daemon.workspace().sessions.len(), count);
}

#[test]
fn a_default_agent_that_is_turned_off_falls_back_to_the_first_that_is_on() {
    let (clients, _) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.settings.preferences.default_agent = Agent::Claude;
    state.settings.agents.switches = Some(
        [Agent::Claude, Agent::Codex, Agent::Copilot, Agent::OpenCode]
            .map(|agent| AgentSwitch {
                agent,
                enabled: agent != Agent::Claude && agent != Agent::Codex,
                install_when_missing: false,
                can_install: true,
                install_command: None,
            })
            .to_vec(),
    );
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::StartNewSession, |_| {});
    let Some(Overlay::NewSession(sheet)) = store.state().overlay() else {
        panic!("launch dialog")
    };
    assert_eq!(sheet.agent, Agent::Copilot);
}

#[test]
fn a_default_agent_that_is_on_stays_the_choice() {
    let (clients, _) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.settings.preferences.default_agent = Agent::Codex;
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::StartNewSession, |_| {});
    let Some(Overlay::NewSession(sheet)) = store.state().overlay() else {
        panic!("launch dialog")
    };
    assert_eq!(sheet.agent, Agent::Codex);
}
