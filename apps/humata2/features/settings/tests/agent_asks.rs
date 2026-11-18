//! Settings › Agents › Ask for permission in the Inbox (D199): opening the
//! section reads where the daemon sends asks; the switch changes it there,
//! and a change the daemon refuses goes back and says why.
use humata2_clients::{Clients, FakeDaemon, Operation, fixture};
use humata2_settings::{AgentsAction, Row, Section, Settings, SettingsAction as A, SettingsState};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<Settings>, FakeDaemon) {
    let (clients, fakes) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    // Pin the switches so the tests don't hang on the fake's defaults.
    fakes.daemon.set_agent_switches(all_on());
    let store = TestStore::new(SettingsState::default(), Settings::new(clients));
    (store, fakes.daemon)
}

#[test]
fn the_switch_is_one_row_in_the_agents_section() {
    let rows: Vec<Row> = Row::ALL
        .into_iter()
        .filter(|row| *row == Row::AsksInInbox)
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(Row::AsksInInbox.section(), Section::Agents);
}

#[test]
fn opening_agents_shows_asks_going_to_the_inbox_by_default() {
    let (mut store, _) = setup();
    store.send(A::Section(Section::Agents), |s| s.section = Section::Agents);
    store.receive(A::AsksShown(Ok(true)), |s| s.asks_in_inbox = Some(true));
    store.receive(A::CodexTrustShown(Ok(false)), |_| {});
    store.receive(A::Agents(AgentsAction::Loaded(Ok(all_on()))), |s| {
        s.agents.switches = Some(all_on());
    });
    store.finish();
}

#[test]
fn revealing_the_row_reads_the_daemon_too() {
    let (mut store, _) = setup();
    store.send(A::Reveal(Row::AsksInInbox), |s| {
        s.section = Section::Agents;
        s.revealed = Some(Row::AsksInInbox);
    });
    store.receive(A::AsksShown(Ok(true)), |s| s.asks_in_inbox = Some(true));
    store.receive(A::CodexTrustShown(Ok(false)), |_| {});
    store.receive(A::Agents(AgentsAction::Loaded(Ok(all_on()))), |s| {
        s.agents.switches = Some(all_on());
    });
    store.finish();
}

#[test]
fn turning_it_off_and_on_changes_it_in_the_daemon() {
    let (mut store, _) = setup();
    store.send(A::ToggleAsksInInbox, |s| {
        s.asks_in_inbox = Some(false);
        s.asks_saving = true;
    });
    store.receive(
        A::AsksSaved {
            wanted: false,
            result: Ok(false),
        },
        |s| s.asks_saving = false,
    );
    // The daemon keeps it: reading it again shows it off.
    store.send(A::Section(Section::Agents), |s| s.section = Section::Agents);
    store.receive(A::AsksShown(Ok(false)), |_| {});
    store.receive(A::CodexTrustShown(Ok(false)), |_| {});
    store.receive(A::Agents(AgentsAction::Loaded(Ok(all_on()))), |s| {
        s.agents.switches = Some(all_on());
    });
    store.send(A::ToggleAsksInInbox, |s| {
        s.asks_in_inbox = Some(true);
        s.asks_saving = true;
    });
    store.receive(
        A::AsksSaved {
            wanted: true,
            result: Ok(true),
        },
        |s| s.asks_saving = false,
    );
    store.finish();
}

#[test]
fn a_second_toggle_while_saving_is_ignored() {
    let (clients, _) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    let state = SettingsState {
        asks_in_inbox: Some(false),
        asks_saving: true,
        ..SettingsState::default()
    };
    let mut store = TestStore::new(state, Settings::new(clients));
    store.send(A::ToggleAsksInInbox, |_| {});
    store.finish();
}

#[test]
fn a_refused_change_goes_back_and_says_why() {
    let (store, daemon) = setup();
    let mut store = store.non_exhaustive();
    daemon.fail_next(Operation::SetAsksInInbox, "link unavailable");
    store.send(A::ToggleAsksInInbox, |s| {
        s.asks_in_inbox = Some(false);
        s.asks_saving = true;
    });
    store.receive_matching(
        "failure",
        |a| matches!(a, A::AsksSaved { result: Err(_), .. }),
        |_| {},
    );
    assert_eq!(store.state().asks_in_inbox, Some(true));
    assert!(!store.state().asks_saving);
    assert!(store.state().error.is_some());
    store.finish();
}

fn all_on() -> Vec<humata2_clients::AgentSwitch> {
    use humata2_clients::model::Agent;
    [Agent::Claude, Agent::Codex, Agent::Copilot, Agent::OpenCode]
        .map(|agent| humata2_clients::AgentSwitch {
            agent,
            enabled: true,
            install_when_missing: false,
            can_install: true,
            install_command: None,
        })
        .to_vec()
}
