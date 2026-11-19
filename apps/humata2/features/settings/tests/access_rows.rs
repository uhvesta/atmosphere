//! The grouped Agent access rows: setting a row writes every rule behind
//! it, a row whose rules differ reads Mixed, and to-do rows ask in To-do.
use humata2_clients::model::{AccessRule, ChannelId, RuleSetting};
use humata2_clients::{fixture, FakeDaemon};
use humata2_settings::{
    AccessAction as A, ACCESS_GROUPS, AskLands, RowSetting, Settings, SettingsAction,
    SettingsState,
};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<Settings>, FakeDaemon, ChannelId) {
    let (clients, fakes) =
        humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let channel = fakes
        .daemon
        .workspace()
        .rules
        .keys()
        .next()
        .expect("the design fixture has rules for a channel")
        .clone();
    let store = TestStore::new(SettingsState::default(), Settings::new(clients)).non_exhaustive();
    (store, fakes.daemon, channel)
}

fn row(key: &str) -> &'static humata2_settings::AccessRow {
    ACCESS_GROUPS
        .iter()
        .flat_map(|g| g.rows)
        .find(|r| r.key == key)
        .expect("the row exists")
}

fn rules(daemon: &FakeDaemon, channel: &ChannelId) -> Vec<AccessRule> {
    daemon.workspace().rules[channel].clone()
}

#[test]
fn setting_a_row_writes_each_of_its_capabilities() {
    let (mut store, daemon, channel) = setup();
    let end = row("session-end");
    store.send(
        SettingsAction::Access(A::SetRow {
            channel: channel.clone(),
            row: end.key.to_owned(),
            setting: RuleSetting::Never,
        }),
        |_| {},
    );
    for _ in end.capabilities {
        store.receive_matching(
            "a rule written",
            |a| matches!(a, SettingsAction::Access(A::Done { result: Ok(()), .. })),
            |_| {},
        );
    }
    let after = rules(&daemon, &channel);
    for id in end.capabilities {
        let rule = after.iter().find(|r| r.capability.as_str() == *id);
        assert_eq!(rule.and_then(|r| r.setting), Some(RuleSetting::Never), "{id}");
    }
    assert_eq!(end.setting(&after), RowSetting::Is(RuleSetting::Never));
    assert!(store.state().access.pending.is_empty());
}

#[test]
fn a_row_whose_rules_differ_reads_mixed() {
    let (_store, daemon, channel) = setup();
    let mut rules = rules(&daemon, &channel);
    let end = row("session-end");
    for rule in &mut rules {
        if rule.capability.as_str() == end.capabilities[0] {
            rule.setting = Some(RuleSetting::Allow);
        } else if end.capabilities.contains(&rule.capability.as_str()) {
            rule.setting = Some(RuleSetting::Never);
        }
    }
    assert_eq!(end.setting(&rules), RowSetting::Mixed);
}

#[test]
fn to_do_rows_ask_in_to_do_and_the_rest_in_the_agent_inbox() {
    let todo = &ACCESS_GROUPS[0];
    for r in todo.rows {
        assert_eq!(r.ask_lands, AskLands::Todo);
        assert!(r.note(RowSetting::Is(RuleSetting::Ask)).contains("To-do"));
    }
    let files = &ACCESS_GROUPS[1];
    for r in files.rows {
        assert!(r.note(RowSetting::Is(RuleSetting::Ask)).contains("Agent inbox"));
    }
}

#[test]
fn every_capability_a_row_writes_is_in_the_design_fixture() {
    let (_store, daemon, channel) = setup();
    let rules = rules(&daemon, &channel);
    for r in ACCESS_GROUPS.iter().flat_map(|g| g.rows) {
        for id in r.capabilities {
            assert!(
                rules.iter().any(|x| x.capability.as_str() == *id && x.setting.is_some()),
                "{id}"
            );
        }
    }
}

#[test]
fn the_other_rules_list_toggles() {
    let (mut store, _daemon, _channel) = setup();
    store.send(SettingsAction::Access(A::ToggleAll), |s| {
        s.access.show_all = true;
    });
    store.send(SettingsAction::Access(A::ToggleAll), |s| {
        s.access.show_all = false;
    });
}
