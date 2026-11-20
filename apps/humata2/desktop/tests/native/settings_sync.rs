//! Settings > Sync (FUSEFSWIRE, D904): the page shows what the daemon holds,
//! a machine's Default / On / Off menu changes only the field picked, and
//! the page is drawn for review. Headless, on fakes.
use super::*;

use humata2_clients::daemon::{FuseMountStatus, MachineSyncSettings, SyncSnapshot};
use humata2_settings::{Section, SettingsAction};

/// Master on, the mount off, and gpu-box with its own Sync switched Off.
fn held() -> SyncSnapshot {
    let mut held = SyncSnapshot::default();
    held.sync.machines.insert(
        "gpu-box".to_owned(),
        MachineSyncSettings { enabled: Some(false), ..MachineSyncSettings::default() },
    );
    // Frame 211: devbox-1 picks On for MCP servers.
    held.sync.machines.insert(
        "devbox-1".to_owned(),
        MachineSyncSettings { mcp: Some(true), ..MachineSyncSettings::default() },
    );
    held
}

fn open_sync() -> App {
    let mut app = App::new(fixture::design());
    app.fakes.daemon.set_sync_snapshot(held());
    app.send(AppAction::Switch(Route::Settings));
    app.send(AppAction::Settings(SettingsAction::Section(Section::Sync)));
    app
}

#[test]
fn the_sync_page_shows_the_switches_and_a_machine_menu_changes_one_field() {
    let mut app = open_sync();
    assert!(app.contains("settings-sync-master"));
    assert!(app.contains("settings-fusefs-enabled"));
    assert!(app.contains("settings-sync-machine-gpu-box-skills"));
    assert!(!app.contains("settings-sync-machine-gpu-box-skills-menu"));

    app.click("settings-sync-machine-gpu-box-skills");
    assert!(app.contains("settings-sync-machine-gpu-box-skills-menu"));
    app.click("settings-sync-machine-gpu-box-skills-on");

    assert!(!app.contains("settings-sync-machine-gpu-box-skills-menu"));
    let departures = app.ui.read(|state| {
        state
            .settings
            .sync
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.sync.machines.get("gpu-box").copied())
    });
    assert_eq!(
        departures,
        Some(MachineSyncSettings {
            enabled: Some(false),
            skills: Some(true),
            ..MachineSyncSettings::default()
        })
    );
}

/// Frame 211 on the real page: master on, gpu-box's Sync Off, mount off.
#[test]
fn the_sync_page_is_drawn() {
    let mut app = open_sync();
    app.cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.snapshot("settings-sync");
}

/// The mount switched on, then the daemon's state moves: the page follows
/// without a reload (D908).
#[test]
fn the_mount_card_follows_the_daemons_mount_state() {
    let mut app = App::new(fixture::design());
    let mut on = held();
    on.fusefs_enabled = true;
    app.fakes.daemon.set_sync_snapshot(on);
    app.send(AppAction::Switch(Route::Settings));
    app.send(AppAction::Settings(SettingsAction::Section(Section::Sync)));
    app.settle();
    assert!(!app.contains("settings-fusefs-status"));
    assert!(!app.contains("settings-fusefs-problem"));

    app.fakes.daemon.set_fusefs_status(FuseMountStatus::Starting);
    app.settle();
    assert!(app.contains("settings-fusefs-status"));
    assert!(!app.contains("settings-fusefs-problem"));

    app.fakes.daemon.set_fusefs_status(FuseMountStatus::Mounted { path: "/mnt/Humata".to_owned() });
    app.settle();
    assert!(app.contains("settings-fusefs-status"));
    assert!(!app.contains("settings-fusefs-problem"));

    app.fakes.daemon.set_fusefs_status(FuseMountStatus::CannotSetUp {
        reason: "This Mac has neither FSKit nor /dev/fuse.".to_owned(),
    });
    app.settle();
    assert!(app.contains("settings-fusefs-problem"));
    assert!(!app.contains("settings-fusefs-status"));

    app.fakes.daemon.set_fusefs_status(FuseMountStatus::Off);
    app.settle();
    assert!(!app.contains("settings-fusefs-problem"));
    assert!(!app.contains("settings-fusefs-status"));
}

/// A machine that can't mount shows nothing while the switch is off.
#[test]
fn a_mount_problem_is_not_shown_with_the_switch_off() {
    let mut app = open_sync();
    app.fakes.daemon.set_fusefs_status(FuseMountStatus::CannotSetUp { reason: "No FUSE.".to_owned() });
    app.settle();
    assert!(!app.contains("settings-fusefs-problem"));
}
