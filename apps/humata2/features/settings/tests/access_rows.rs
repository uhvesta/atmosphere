//! The grouped Agent access rows: 2-group layout, setting a row writes every
//! rule behind it, and a row whose rules differ reads Mixed.
use humata2_clients::model::{AccessRule, ChannelId, RuleSetting};
use humata2_clients::{fixture, FakeDaemon};
use humata2_settings::{
    AccessAction as A, ACCESS_GROUPS, RowSetting, Settings, SettingsAction, SettingsState,
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
fn two_primary_groups_with_expected_headings_and_rows() {
    assert_eq!(ACCESS_GROUPS.len(), 2);

    let local = &ACCESS_GROUPS[0];
    assert_eq!(local.title, "LOCAL & SAME-CHANNEL");
    assert_eq!(
        local.caption,
        "Runs automatically within scope \u{00b7} asks in Agent inbox outside scope"
    );
    assert_eq!(local.rows.len(), 7);
    assert!(local.rows.iter().all(|r| !r.scope_chip.is_empty()));

    let remote = &ACCESS_GROUPS[1];
    assert_eq!(remote.title, "LEAVES THIS MACHINE OR AFFECTS OTHERS");
    assert_eq!(
        remote.caption,
        "Always stops and asks in Agent inbox unless you allow it"
    );
    assert_eq!(remote.rows.len(), 5);
}

#[test]
fn push_rows_split_branch_push_from_pr_push() {
    let (_store, daemon, channel) = setup();
    let channel_rules = rules(&daemon, &channel);

    let branch_push = ACCESS_GROUPS[0]
        .rows
        .iter()
        .find(|r| r.key == "git-push-branch")
        .expect("git-push-branch is in group 1");
    assert_eq!(
        branch_push.setting(&channel_rules),
        RowSetting::Is(RuleSetting::Allow)
    );

    let pr_push = ACCESS_GROUPS[1]
        .rows
        .iter()
        .find(|r| r.key == "git-push-pr")
        .expect("git-push-pr is in group 2");
    assert_eq!(pr_push.capabilities, &["git_push"]);
}

#[test]
fn setting_a_row_writes_each_of_its_capabilities() {
    let (mut store, daemon, channel) = setup();
    let end = row("session-other-end");
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
    let end = row("session-other-end");
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

#[test]
fn the_global_pages_row_writes_the_global_rules_not_a_channels() {
    let (mut store, daemon, channel) = setup();
    let global = ChannelId::global_rules();
    let end = row("session-other-end");
    let channel_before = rules(&daemon, &channel);
    store.send(
        SettingsAction::Access(A::SetRow {
            channel: global.clone(),
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
    assert_eq!(
        end.setting(&rules(&daemon, &global)),
        RowSetting::Is(RuleSetting::Never)
    );
    assert_eq!(rules(&daemon, &channel), channel_before);
}
