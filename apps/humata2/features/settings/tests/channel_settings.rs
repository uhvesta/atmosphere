//! SETTINGSFIX Slice 2: per-channel settings reducer — branch prefix override &
//! inherit, default agent & model, contain agents, and retention.

use std::sync::Arc;

use humata2_clients::model::ChannelId;
use humata2_clients::{FakeDaemon, fixture};
use humata2_settings::{
    ChannelSettingsAction as C, Settings, SettingsAction as A, SettingsState,
};
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<Settings>, FakeDaemon, ChannelId) {
    let ws = fixture::design();
    let ch = ws.channels[0].id.clone();
    let (clients, fakes) = humata2_clients::fake_clients(ws, Arc::new(TestClock::new()));
    let store = TestStore::new(SettingsState::default(), Settings::new(clients)).non_exhaustive();
    (store, fakes.daemon, ch)
}

#[test]
fn channel_branch_prefix_can_override_and_return_to_inherit() {
    let (mut store, _, ch) = setup();

    store.send(A::ChannelSettings(C::Load(ch.clone())), |_| {});
    store.receive_matching(
        "loaded",
        |a| matches!(a, A::ChannelSettings(C::Loaded { result: Ok(_), .. })),
        |_| {},
    );
    assert!(!store.state().channel_settings.branch_prefix_custom);

    // Switch to custom and save an override.
    store.send(
        A::ChannelSettings(C::SetBranchPrefixMode {
            channel: ch.clone(),
            custom: true,
        }),
        |_| {},
    );
    store.send(A::ChannelSettings(C::BranchPrefixChanged("ops/".into())), |_| {});
    store.send(A::ChannelSettings(C::SaveBranchPrefix(ch.clone())), |_| {});
    store.receive_matching(
        "saved prefix",
        |a| matches!(a, A::ChannelSettings(C::Saved { result: Ok(_), .. })),
        |_| {},
    );
    assert_eq!(
        store
            .state()
            .channel_settings
            .for_channel(&ch)
            .and_then(|s| s.branch_prefix.as_deref()),
        Some("ops/")
    );

    // Switch back to Inherit clears the channel override on the daemon.
    store.send(
        A::ChannelSettings(C::SetBranchPrefixMode {
            channel: ch.clone(),
            custom: false,
        }),
        |_| {},
    );
    store.receive_matching(
        "cleared prefix",
        |a| matches!(a, A::ChannelSettings(C::Saved { result: Ok(_), .. })),
        |_| {},
    );
    assert_eq!(
        store
            .state()
            .channel_settings
            .for_channel(&ch)
            .and_then(|s| s.branch_prefix.as_deref()),
        None
    );
    assert!(!store.state().channel_settings.branch_prefix_custom);
    store.finish();
}

#[test]
fn channel_agent_model_contain_and_retention_save_cleanly() {
    let (mut store, _, ch) = setup();

    store.send(A::ChannelSettings(C::Load(ch.clone())), |_| {});
    store.receive_matching(
        "loaded",
        |a| matches!(a, A::ChannelSettings(C::Loaded { result: Ok(_), .. })),
        |_| {},
    );

    store.send(
        A::ChannelSettings(C::SetDefaultAgent {
            channel: ch.clone(),
            agent: Some("codex".into()),
        }),
        |_| {},
    );
    store.receive_matching(
        "agent saved",
        |a| matches!(a, A::ChannelSettings(C::Saved { result: Ok(_), .. })),
        |_| {},
    );

    store.send(A::ChannelSettings(C::DefaultModelChanged("o3".into())), |_| {});
    store.send(A::ChannelSettings(C::SaveDefaultModel(ch.clone())), |_| {});
    store.receive_matching(
        "model saved",
        |a| matches!(a, A::ChannelSettings(C::Saved { result: Ok(_), .. })),
        |_| {},
    );

    store.send(
        A::ChannelSettings(C::SetContainAgents {
            channel: ch.clone(),
            value: Some(true),
        }),
        |_| {},
    );
    store.receive_matching(
        "contain saved",
        |a| matches!(a, A::ChannelSettings(C::Saved { result: Ok(_), .. })),
        |_| {},
    );

    store.send(A::ChannelSettings(C::ArchivePrDaysChanged("5".into())), |_| {});
    store.send(A::ChannelSettings(C::ArchiveIdleDaysChanged("10".into())), |_| {});
    store.send(A::ChannelSettings(C::SaveRetention(ch.clone())), |_| {});
    store.receive_matching(
        "retention saved",
        |a| matches!(a, A::ChannelSettings(C::Saved { result: Ok(_), .. })),
        |_| {},
    );

    let saved = store
        .state()
        .channel_settings
        .for_channel(&ch)
        .expect("saved channel settings");
    assert_eq!(
        saved.default_agent.as_ref().map(|a| (a.agent.as_str(), a.model.as_str())),
        Some(("codex", "o3"))
    );
    assert_eq!(saved.contain_agents, Some(true));
    assert_eq!(saved.retention.archive_pr_reviews_after_days, 5);
    assert_eq!(saved.retention.archive_idle_sessions_after_days, 10);
    store.finish();
}
