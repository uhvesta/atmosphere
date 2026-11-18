//! Picking the default agent in Settings keeps it locally and tells the
//! daemon (ST-15, D770), so each channel can say which agent a start takes.
use humata2_clients::model::Agent;
use humata2_clients::{Clients, fixture};
use humata2_settings::{Settings, SettingsAction as A, SettingsState};
use std::sync::Arc;
use tca::{TestClock, TestStore};

#[test]
fn picking_the_default_agent_is_sent_to_the_daemon() {
    let (clients, fakes) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(SettingsState::default(), Settings::new(clients));
    store.send(A::SetAgent(Agent::Codex), |s| s.preferences.default_agent = Agent::Codex);
    store.receive(A::DefaultAgentSaved(Ok(())), |_| {});
    store.finish();
    assert!(fakes.daemon.calls().iter().any(|call| call == "set_default_agent codex"));
}
