//! Settings › Agents › Codex's trust step (D202): opening the section asks
//! the daemon whether Codex's hooks still need trusting in its `/hooks`.
//! The step shows only while the daemon says so, and has no dismiss.
use humata2_clients::{Clients, FakeDaemon, fixture};
use humata2_settings::{Section, Settings, SettingsAction as A, SettingsState};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<Settings>, FakeDaemon) {
    let (clients, fakes) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    let store = TestStore::new(SettingsState::default(), Settings::new(clients));
    (store, fakes.daemon)
}

#[test]
fn trusted_or_unused_hooks_show_no_step() {
    let (mut store, _) = setup();
    store.send(A::Section(Section::Agents), |s| s.section = Section::Agents);
    store.receive(A::AsksShown(Ok(true)), |s| s.asks_in_inbox = Some(true));
    store.receive(A::CodexTrustShown(Ok(false)), |_| {});
    store.finish();
}

#[test]
fn hooks_not_yet_heard_show_the_step_until_the_daemon_hears_one() {
    let (mut store, daemon) = setup();
    daemon.set_codex_needs_trust(true);
    store.send(A::Section(Section::Agents), |s| s.section = Section::Agents);
    store.receive(A::AsksShown(Ok(true)), |s| s.asks_in_inbox = Some(true));
    store.receive(A::CodexTrustShown(Ok(true)), |s| s.codex_needs_trust = true);
    // The user trusted them in Codex and a hook came in: reopening clears it.
    daemon.set_codex_needs_trust(false);
    store.send(A::Section(Section::Agents), |_| {});
    store.receive(A::AsksShown(Ok(true)), |_| {});
    store.receive(A::CodexTrustShown(Ok(false)), |s| s.codex_needs_trust = false);
    store.finish();
}
