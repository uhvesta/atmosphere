//! The Agent access tab calls the daemon, ends not pending, and reports
//! failures until dismissed.
use humata2_clients::model::{Capability, ChannelId, RuleSetting};
use humata2_clients::{Clients, FakeDaemon, Operation, fixture};
use humata2_settings::{AccessAction as A, Settings, SettingsAction, SettingsState};
use std::sync::Arc;
use tca::{TestClock, TestStore};

/// A store over the design fixture, with the first channel and rule it has.
fn setup() -> (
    TestStore<Settings>,
    FakeDaemon,
    ChannelId,
    Capability,
) {
    setup_with(|_, _| SettingsState::default())
}

/// Like [`setup`], with the store starting from `state` for the row picked.
fn setup_with(
    state: impl FnOnce(&ChannelId, &Capability) -> SettingsState,
) -> (
    TestStore<Settings>,
    FakeDaemon,
    ChannelId,
    Capability,
) {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let workspace = fakes.daemon.workspace();
    let (channel, rules) = workspace
        .rules
        .iter()
        .next()
        .expect("the design fixture has rules for a channel");
    // The first row (own clones, IN-16) has no setting to change.
    let capability = rules
        .iter()
        .find(|rule| rule.setting.is_some())
        .expect("the channel has a rule with a setting")
        .capability
        .clone();
    let store = TestStore::new(state(channel, &capability), Settings::new(clients));
    (store, fakes.daemon, channel.clone(), capability)
}

fn set(channel: &ChannelId, capability: &Capability) -> SettingsAction {
    SettingsAction::Access(A::Set {
        channel: channel.clone(),
        capability: capability.clone(),
        setting: RuleSetting::Never,
    })
}

fn done(
    channel: &ChannelId,
    capability: &Capability,
    result: Result<(), humata2_clients::DaemonError>,
) -> SettingsAction {
    SettingsAction::Access(A::Done {
        channel: channel.clone(),
        capability: capability.clone(),
        result,
    })
}

#[test]
fn setting_a_rule_calls_the_daemon_and_ends_not_pending() {
    let (mut store, daemon, channel, capability) = setup();
    store.send(set(&channel, &capability), |s| {
        s.access.pending.insert((channel.clone(), capability.clone()));
    });
    store.receive(done(&channel, &capability, Ok(())), |s| {
        s.access.pending.clear();
    });
    let rules = &daemon.workspace().rules[&channel];
    let rule = rules.iter().find(|r| r.capability == capability);
    assert_eq!(rule.and_then(|r| r.setting), Some(RuleSetting::Never));
    store.finish();
}

#[test]
fn a_second_set_for_the_same_row_is_ignored_while_pending() {
    // The fake answers at once, so the store starts with the row's first
    // change still on its way.
    let (mut store, daemon, channel, capability) = setup_with(|channel, capability| {
        let mut state = SettingsState::default();
        state
            .access
            .pending
            .insert((channel.clone(), capability.clone()));
        state
    });
    let before = daemon.workspace().rules[&channel].clone();
    store.send(set(&channel, &capability), |_| {});
    assert_eq!(daemon.workspace().rules[&channel], before);
    store.finish();
}

#[test]
fn a_failed_set_shows_an_error_until_dismissed() {
    let (store, daemon, channel, capability) = setup();
    // The error text is whatever the daemon error says; only that one shows.
    let mut store = store.non_exhaustive();
    daemon.fail_next(Operation::SetRule, "link unavailable");
    store.send(set(&channel, &capability), |s| {
        s.access.pending.insert((channel.clone(), capability.clone()));
    });
    store.receive_matching(
        "failure",
        |a| matches!(a, SettingsAction::Access(A::Done { result: Err(_), .. })),
        |_| {},
    );
    assert!(store.state().access.error.is_some());
    assert!(store.state().access.pending.is_empty());
    store.send(SettingsAction::Access(A::DismissError), |s| {
        s.access.error = None;
    });
    store.finish();
}

#[test]
fn removing_a_grant_runs_and_clears() {
    let (mut store, daemon, _, _) = setup();
    let grant = daemon
        .workspace()
        .rules
        .values()
        .flatten()
        .flat_map(|r| r.grants.iter())
        .map(|g| g.id.clone())
        .next()
        .expect("the fixture has a grant");
    store.send(SettingsAction::Access(A::RemoveGrant(grant.clone())), |s| {
        s.access.removing.insert(grant.clone());
    });
    store.receive(
        SettingsAction::Access(A::GrantRemoved {
            grant: grant.clone(),
            result: Ok(()),
        }),
        |s| {
            s.access.removing.clear();
        },
    );
    store.finish();
}
