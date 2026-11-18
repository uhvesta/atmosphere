//! Settings › Agents › Agents (AGENTSET, D725): the switches load from the
//! daemon, a toggle shows at once and is saved, a refusal puts it back, and
//! pickers list only the agents that are on.
use humata2_clients::model::Agent;
use humata2_clients::{AgentSwitch, Clients, FakeDaemon, Operation, fixture};
use humata2_settings::{AgentsAction, Settings, SettingsAction as A, SettingsState};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<Settings>, FakeDaemon) {
    let (clients, fakes) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    // Pin the switches so the tests don't hang on the fake's defaults.
    fakes.daemon.set_agent_switches(all_on());
    let store = TestStore::new(SettingsState::default(), Settings::new(clients));
    (store, fakes.daemon)
}

fn switch(agent: Agent, enabled: bool, install: bool) -> AgentSwitch {
    AgentSwitch {
        kind: agent.command().to_owned(),
        name: agent.name().to_owned(),
        agent: Some(agent),
        enabled,
        install_when_missing: install,
        can_install: true,
        install_command: None,
        asks_in_inbox: true,
    }
}

fn all_on() -> Vec<AgentSwitch> {
    [Agent::Claude, Agent::Codex, Agent::Copilot, Agent::OpenCode]
        .map(|agent| switch(agent, true, false))
        .to_vec()
}

fn with_one_off(agent: Agent) -> Vec<AgentSwitch> {
    all_on()
        .into_iter()
        .map(|s| AgentSwitch {
            enabled: s.agent != Some(agent),
            ..s
        })
        .collect()
}

/// A store that has loaded the daemon's default switches.
fn loaded() -> (TestStore<Settings>, FakeDaemon) {
    let (mut store, daemon) = setup();
    store.send(A::Agents(AgentsAction::Load), |_| {});
    store.receive(A::Agents(AgentsAction::Loaded(Ok(all_on()))), |s| {
        s.agents.switches = Some(all_on());
    });
    (store, daemon)
}

#[test]
fn the_switches_load_in_the_daemons_order() {
    let (mut store, _) = setup();
    assert_eq!(store.state().agents.switches, None);
    store.send(A::Agents(AgentsAction::Load), |_| {});
    store.receive(A::Agents(AgentsAction::Loaded(Ok(all_on()))), |s| {
        s.agents.switches = Some(all_on());
    });
    store.finish();
}

#[test]
fn turning_an_agent_off_shows_at_once_and_is_saved() {
    let (mut store, _) = loaded();
    let before = switch(Agent::Copilot, true, false);
    store.send(A::Agents(AgentsAction::Toggle(kind(Agent::Copilot))), |s| {
        s.agents.switches = Some(with_one_off(Agent::Copilot));
    });
    store.receive(
        A::Agents(AgentsAction::Saved {
            kind: kind(Agent::Copilot),
            before,
            result: Ok(switch(Agent::Copilot, false, false)),
        }),
        |_| {},
    );
    // The daemon holds it: loading again shows it off.
    store.send(A::Agents(AgentsAction::Load), |_| {});
    store.receive(
        A::Agents(AgentsAction::Loaded(Ok(with_one_off(Agent::Copilot)))),
        |_| {},
    );
    store.finish();
}

#[test]
fn turning_it_back_on_is_saved_too() {
    let (mut store, _) = loaded();
    store.send(A::Agents(AgentsAction::Toggle(kind(Agent::Codex))), |s| {
        s.agents.switches = Some(with_one_off(Agent::Codex));
    });
    store.receive(
        A::Agents(AgentsAction::Saved {
            kind: kind(Agent::Codex),
            before: switch(Agent::Codex, true, false),
            result: Ok(switch(Agent::Codex, false, false)),
        }),
        |_| {},
    );
    store.send(A::Agents(AgentsAction::Toggle(kind(Agent::Codex))), |s| {
        s.agents.switches = Some(all_on());
    });
    store.receive(
        A::Agents(AgentsAction::Saved {
            kind: kind(Agent::Codex),
            before: switch(Agent::Codex, false, false),
            result: Ok(switch(Agent::Codex, true, false)),
        }),
        |_| {},
    );
    store.finish();
}

#[test]
fn install_when_missing_changes_only_its_own_switch() {
    let (mut store, _) = loaded();
    let installing = |s: &mut SettingsState| {
        s.agents.switches = Some(
            all_on()
                .into_iter()
                .map(|x| AgentSwitch {
                    install_when_missing: x.agent == Some(Agent::Codex),
                    ..x
                })
                .collect(),
        );
    };
    store.send(
        A::Agents(AgentsAction::ToggleInstall(kind(Agent::Codex))),
        installing,
    );
    store.receive(
        A::Agents(AgentsAction::Saved {
            kind: kind(Agent::Codex),
            before: switch(Agent::Codex, true, false),
            result: Ok(switch(Agent::Codex, true, true)),
        }),
        |_| {},
    );
    store.finish();
}

#[test]
fn a_refused_change_goes_back_and_shows_why() {
    let (store, daemon) = loaded();
    let mut store = store.non_exhaustive();
    daemon.fail_next(Operation::SetAgentSwitch, "link unavailable");
    store.send(A::Agents(AgentsAction::Toggle(kind(Agent::Copilot))), |s| {
        s.agents.switches = Some(with_one_off(Agent::Copilot));
    });
    store.receive_matching(
        "failure",
        |a| matches!(a, A::Agents(AgentsAction::Saved { result: Err(_), .. })),
        |_| {},
    );
    assert_eq!(store.state().agents.switches, Some(all_on()));
    assert!(store.state().error.is_some());
    store.finish();
}

#[test]
fn a_toggle_before_the_switches_load_does_nothing() {
    let (mut store, _) = setup();
    store.send(A::Agents(AgentsAction::Toggle(kind(Agent::Claude))), |_| {});
    store.finish();
}

#[test]
fn pickers_offer_every_built_in_before_the_list_loads() {
    let state = SettingsState::default();
    assert_eq!(
        state.offered_agents(),
        vec![Agent::Claude, Agent::Codex, Agent::Copilot, Agent::OpenCode]
    );
}

#[test]
fn pickers_leave_out_the_agents_that_are_off() {
    let mut state = SettingsState::default();
    state.agents.switches = Some(with_one_off(Agent::Codex));
    assert_eq!(
        state.offered_agents(),
        vec![Agent::Claude, Agent::Copilot, Agent::OpenCode]
    );
}

fn kind(agent: Agent) -> String {
    agent.command().to_owned()
}
