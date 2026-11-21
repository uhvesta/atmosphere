//! SETTINGSFIX Slice 2: global daemon-backed settings — default branch prefix
//! (set, reset, refresh GitHub login), preserving `default_agent.model` when
//! switching agents, security containment, retention defaults, terminal
//! font/scrollback, and notification quiet hours.

use std::sync::Arc;

use humata2_clients::daemon::{FullSettings, LongLinesMode};
use humata2_clients::model::Agent;
use humata2_clients::{FakeDaemon, fixture};
use humata2_settings::{
    GeneralAction as G, Settings, SettingsAction as A, SettingsState, Theme,
};
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<Settings>, FakeDaemon) {
    let (clients, fakes) =
        humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let store = TestStore::new(SettingsState::default(), Settings::new(clients)).non_exhaustive();
    (store, fakes.daemon)
}

#[test]
fn setting_resetting_and_refreshing_global_branch_prefix() {
    let (mut store, daemon) = setup();
    daemon.set_gh_default_prefix("alice/");

    // Refresh picks up the GitHub login default prefix.
    store.send(A::General(G::RefreshGhLogin), |_| {});
    store.receive_matching(
        "refreshed",
        |a| matches!(a, A::General(G::Loaded(Ok(_)))),
        |_| {},
    );
    assert_eq!(store.state().general.branch_prefix_draft, "alice/");
    assert_eq!(
        store.state().general.default_branch_prefix.as_deref(),
        Some("alice/")
    );

    // Invalid prefix is rejected locally before hitting the daemon.
    store.send(A::General(G::BranchPrefixChanged("bad prefix".into())), |_| {});
    store.send(A::General(G::SaveBranchPrefix), |_| {});
    assert!(store.state().general.error.is_some());

    // Custom prefix saves to daemon and marks branch_prefix_is_default = false.
    store.send(A::General(G::BranchPrefixChanged("team/".into())), |_| {});
    store.send(A::General(G::SaveBranchPrefix), |_| {});
    store.receive_matching(
        "saved custom",
        |a| matches!(a, A::General(G::Saved(Ok(_)))),
        |_| {},
    );
    assert_eq!(daemon.full_settings_snapshot().branch_prefix, "team/");
    assert!(!daemon.full_settings_snapshot().branch_prefix_is_default);

    // Resetting restores the remembered @user default prefix.
    store.send(A::General(G::ResetBranchPrefix), |_| {});
    store.receive_matching(
        "reset saved",
        |a| matches!(a, A::General(G::Saved(Ok(_)))),
        |_| {},
    );
    assert_eq!(daemon.full_settings_snapshot().branch_prefix, "alice/");
    assert_eq!(store.state().general.branch_prefix_draft, "alice/");
    store.finish();
}

#[test]
fn switching_default_agent_preserves_saved_default_model() {
    let (mut store, daemon) = setup();
    let mut initial = FullSettings::default();
    initial.default_agent.agent = "claude".to_owned();
    initial.default_agent.model = "claude-opus-4-6".to_owned();
    daemon.set_full_settings(initial);

    store.send(A::General(G::Load), |_| {});
    store.receive_matching(
        "loaded",
        |a| matches!(a, A::General(G::Loaded(Ok(_)))),
        |_| {},
    );
    assert_eq!(store.state().general.default_model_draft, "claude-opus-4-6");

    store.send(A::SetAgent(Agent::Codex), |_| {});
    store.receive(A::DefaultAgentSaved(Ok(())), |_| {});
    let snap = daemon.full_settings_snapshot();
    assert_eq!(snap.default_agent.agent, "codex");
    assert_eq!(snap.default_agent.model, "claude-opus-4-6");

    // Updating the default model keeps the selected agent.
    store.send(A::General(G::DefaultModelChanged("o3".into())), |_| {});
    store.send(A::General(G::SaveDefaultModel), |_| {});
    store.receive_matching(
        "model saved",
        |a| matches!(a, A::General(G::Saved(Ok(_)))),
        |_| {},
    );
    let snap = daemon.full_settings_snapshot();
    assert_eq!(snap.default_agent.agent, "codex");
    assert_eq!(snap.default_agent.model, "o3");
    store.finish();
}

#[test]
fn security_retention_layout_and_quiet_hours_sync_with_daemon() {
    let (mut store, daemon) = setup();
    store.send(A::General(G::Load), |_| {});
    store.receive_matching(
        "loaded",
        |a| matches!(a, A::General(G::Loaded(Ok(_)))),
        |_| {},
    );

    // Security › Contain agents
    store.send(A::General(G::ToggleContainAgents), |_| {});
    store.receive_matching(
        "contain saved",
        |a| matches!(a, A::General(G::Saved(Ok(_)))),
        |_| {},
    );
    assert!(daemon.full_settings_snapshot().security.contain_agents);

    // Git worktrees: default true, toggles off and back on.
    assert!(daemon.full_settings_snapshot().use_worktrees);
    store.send(A::General(G::ToggleUseWorktrees), |_| {});
    store.receive_matching(
        "worktrees off",
        |a| matches!(a, A::General(G::Saved(Ok(_)))),
        |_| {},
    );
    assert!(!daemon.full_settings_snapshot().use_worktrees);
    store.send(A::General(G::ToggleUseWorktrees), |_| {});
    store.receive_matching(
        "worktrees on",
        |a| matches!(a, A::General(G::Saved(Ok(_)))),
        |_| {},
    );
    assert!(daemon.full_settings_snapshot().use_worktrees);

    // Retention defaults
    store.send(A::General(G::ArchivePrDaysChanged("7".into())), |_| {});
    store.send(A::General(G::ArchiveIdleDaysChanged("14".into())), |_| {});
    store.send(A::General(G::SaveRetention), |_| {});
    store.receive_matching(
        "retention saved",
        |a| matches!(a, A::General(G::Saved(Ok(_)))),
        |_| {},
    );
    let snap = daemon.full_settings_snapshot();
    assert_eq!(snap.retention_defaults.archive_pr_reviews_after_days, 7);
    assert_eq!(snap.retention_defaults.archive_idle_sessions_after_days, 14);

    // Layout: theme, font size, wrap, font family, scrollback
    store.send(A::SetTheme(Theme::Dark), |_| {});
    store.send(A::SetFontSize(15), |_| {});
    store.send(A::ToggleWrap, |_| {});
    store.send(A::General(G::TerminalFontChanged("JetBrains Mono".into())), |_| {});
    store.send(A::General(G::SaveTerminalFont), |_| {});
    store.receive_matching(
        "font saved",
        |a| matches!(a, A::General(G::Saved(Ok(_)))),
        |_| {},
    );
    store.send(A::General(G::ScrollbackChanged("25000".into())), |_| {});
    store.send(A::General(G::SaveScrollback), |_| {});
    store.receive_matching(
        "scrollback saved",
        |a| matches!(a, A::General(G::Saved(Ok(_)))),
        |_| {},
    );
    let snap = daemon.full_settings_snapshot();
    assert_eq!(snap.terminal.theme, "dark");
    assert_eq!(snap.terminal.font_size, 15);
    assert_eq!(snap.terminal.font, "JetBrains Mono");
    assert_eq!(snap.terminal.scrollback_lines, 25_000);
    assert_eq!(snap.long_lines, LongLinesMode::Scroll);

    // Notifications: quiet hours + VIP breaks quiet hours
    store.send(A::General(G::QuietFromChanged("22:00".into())), |_| {});
    store.send(A::General(G::QuietUntilChanged("07:30".into())), |_| {});
    store.send(A::General(G::SaveQuietHours), |_| {});
    store.receive_matching(
        "quiet hours saved",
        |a| matches!(a, A::General(G::Saved(Ok(_)))),
        |_| {},
    );
    store.send(A::General(G::ToggleVipBreaksQuietHours), |_| {});
    store.receive_matching(
        "vip saved",
        |a| matches!(a, A::General(G::Saved(Ok(_)))),
        |_| {},
    );
    let snap = daemon.full_settings_snapshot();
    assert_eq!(snap.notifications.quiet_from, "22:00");
    assert_eq!(snap.notifications.quiet_until, "07:30");
    assert!(snap.notifications.vip_breaks_quiet_hours);
    store.finish();
}
