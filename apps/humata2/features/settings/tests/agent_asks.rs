//! Settings › Agents › Ask in the Inbox (D782): each agent has its own
//! switch for where its permission and question asks go. Turning one off
//! changes only that agent in the daemon; a change the daemon refuses goes
//! back and says why.
use humata2_clients::model::Agent;
use humata2_clients::{AgentSwitch, Clients, FakeDaemon, Operation, fixture};
use humata2_settings::{AgentsAction, Section, Settings, SettingsAction as A, SettingsState};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<Settings>, FakeDaemon) {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    // Pin the switches so the tests don't hang on the fake's defaults.
    fakes.daemon.set_agent_switches(all_on());
    let store = TestStore::new(SettingsState::default(), Settings::new(clients));
    (store, fakes.daemon)
}

fn loaded(store: &mut TestStore<Settings>) {
    store.send(A::Section(Section::Agents), |s| s.section = Section::Agents);
    store.receive(A::CodexTrustShown(Ok(false)), |_| {});
    store.receive(A::Agents(AgentsAction::Loaded(Ok(all_on()))), |s| {
        s.agents.switches = Some(all_on());
    });
}

fn asks(store: &TestStore<Settings>, agent: Agent) -> Option<bool> {
    store
        .state()
        .agents
        .switches
        .as_ref()?
        .iter()
        .find(|switch| switch.agent == Some(agent))
        .map(|switch| switch.asks_in_inbox)
}

#[test]
fn every_agent_asks_in_the_inbox_by_default() {
    let (mut store, _) = setup();
    loaded(&mut store);
    for agent in [
        Agent::Claude,
        Agent::Codex,
        Agent::Copilot,
        Agent::OpenCode,
        Agent::Agy,
    ] {
        assert_eq!(asks(&store, agent), Some(true));
    }
    store.finish();
}

#[test]
fn turning_one_agent_off_changes_only_that_agent_in_the_daemon() {
    let (store, _) = setup();
    let mut store = store.non_exhaustive();
    loaded(&mut store);
    store.send(A::Agents(AgentsAction::ToggleAsks(kind(Agent::Copilot))), |_| {});
    // The switch shows off at once.
    assert_eq!(asks(&store, Agent::Copilot), Some(false));
    store.receive_matching(
        "saved",
        |a| matches!(a, A::Agents(AgentsAction::Saved { result: Ok(_), .. })),
        |_| {},
    );
    // The daemon kept it, and the other agents still ask in the Inbox.
    store.send(A::Section(Section::Notifications), |_| {});
    store.send(A::Section(Section::Agents), |_| {});
    store.receive_matching(
        "reloaded",
        |a| matches!(a, A::Agents(AgentsAction::Loaded(Ok(_)))),
        |_| {},
    );
    assert_eq!(asks(&store, Agent::Copilot), Some(false));
    assert_eq!(asks(&store, Agent::Claude), Some(true));
    // Turning it back on asks in the Inbox again.
    store.send(A::Agents(AgentsAction::ToggleAsks(kind(Agent::Copilot))), |_| {});
    store.receive_matching(
        "saved again",
        |a| matches!(a, A::Agents(AgentsAction::Saved { result: Ok(_), .. })),
        |_| {},
    );
    assert_eq!(asks(&store, Agent::Copilot), Some(true));
    store.finish();
}

#[test]
fn a_refused_change_goes_back_and_says_why() {
    let (store, daemon) = setup();
    let mut store = store.non_exhaustive();
    loaded(&mut store);
    daemon.fail_next(Operation::SetAgentSwitch, "link unavailable");
    store.send(A::Agents(AgentsAction::ToggleAsks(kind(Agent::Claude))), |_| {});
    store.receive_matching(
        "failure",
        |a| matches!(a, A::Agents(AgentsAction::Saved { result: Err(_), .. })),
        |_| {},
    );
    assert_eq!(asks(&store, Agent::Claude), Some(true));
    assert!(store.state().error.is_some());
    store.finish();
}

#[test]
fn a_custom_kind_has_its_own_switch_and_pickers_still_offer_only_built_in_agents() {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let custom = AgentSwitch {
        kind: "my-agent".to_owned(),
        name: "My agent".to_owned(),
        agent: None,
        ..all_on()[0].clone()
    };
    let mut switches = all_on();
    switches.push(custom);
    fakes.daemon.set_agent_switches(switches);
    let mut store =
        TestStore::new(SettingsState::default(), Settings::new(clients)).non_exhaustive();
    store.send(A::Section(Section::Agents), |_| {});
    store.receive_matching(
        "loaded",
        |a| matches!(a, A::Agents(AgentsAction::Loaded(Ok(_)))),
        |_| {},
    );
    store.send(A::Agents(AgentsAction::ToggleAsks("my-agent".to_owned())), |_| {});
    store.receive_matching(
        "saved",
        |a| matches!(a, A::Agents(AgentsAction::Saved { result: Ok(_), .. })),
        |_| {},
    );
    let custom_asks = |store: &TestStore<Settings>| {
        store
            .state()
            .agents
            .switches
            .as_ref()
            .and_then(|all| all.iter().find(|s| s.kind == "my-agent"))
            .map(|s| s.asks_in_inbox)
    };
    assert_eq!(custom_asks(&store), Some(false));
    assert_eq!(asks(&store, Agent::Claude), Some(true));
    assert_eq!(
        store.state().agents.offered(),
        vec![
            Agent::Claude,
            Agent::Codex,
            Agent::Copilot,
            Agent::OpenCode,
            Agent::Agy,
        ]
    );
    store.finish();
}

fn all_on() -> Vec<AgentSwitch> {
    [
        Agent::Claude,
        Agent::Codex,
        Agent::Copilot,
        Agent::OpenCode,
        Agent::Agy,
    ]
    .map(|agent| AgentSwitch {
            kind: agent.command().to_owned(),
            name: agent.name().to_owned(),
            agent: Some(agent),
            enabled: true,
            install_when_missing: false,
            can_install: true,
            install_command: None,
            asks_in_inbox: true,
        })
        .to_vec()
}

fn kind(agent: Agent) -> String {
    agent.command().to_owned()
}
