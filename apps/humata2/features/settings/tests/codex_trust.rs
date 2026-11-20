//! Settings › Agents › Codex's trust step (D202): opening the section asks
//! the daemon whether Codex's hooks still need trusting in its `/hooks`.
//! The step shows only while the daemon says so, and has no dismiss.
use humata2_clients::{Clients, FakeDaemon, fixture};
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

#[test]
fn trusted_or_unused_hooks_show_no_step() {
    let (mut store, _) = setup();
    store.send(A::Section(Section::Agents), |s| s.section = Section::Agents);
    store.receive(A::CodexTrustShown(Ok(false)), |_| {});
    store.receive(A::Agents(AgentsAction::Loaded(Ok(all_on()))), |s| {
        s.agents.switches = Some(all_on());
    });
    store.finish();
}

#[test]
fn hooks_not_yet_heard_show_the_step_until_the_daemon_hears_one() {
    let (mut store, daemon) = setup();
    daemon.set_codex_needs_trust(true);
    store.send(A::Section(Section::Agents), |s| s.section = Section::Agents);
    store.receive(A::CodexTrustShown(Ok(true)), |s| s.codex_needs_trust = true);
    store.receive(A::Agents(AgentsAction::Loaded(Ok(all_on()))), |s| {
        s.agents.switches = Some(all_on());
    });
    // The user trusted them in Codex and a hook came in: reopening clears it.
    daemon.set_codex_needs_trust(false);
    store.send(A::Section(Section::Agents), |_| {});
    store.receive(A::CodexTrustShown(Ok(false)), |s| s.codex_needs_trust = false);
    store.receive(A::Agents(AgentsAction::Loaded(Ok(all_on()))), |s| {
        s.agents.switches = Some(all_on());
    });
    store.finish();
}

fn all_on() -> Vec<humata2_clients::AgentSwitch> {
    use humata2_clients::model::Agent;
    [
        Agent::Claude,
        Agent::Codex,
        Agent::Copilot,
        Agent::OpenCode,
        Agent::Agy,
    ]
        .map(|agent| humata2_clients::AgentSwitch {
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
