//! NOTIFY (D889): desktop notifications default off, request OS notification
//! permission when toggled on, keep `permission_request_alerts` independent,
//! and fall back cleanly when deserializing older preferences.

use humata2_clients::fixture;
use humata2_notify::NotifyConfig;
use humata2_settings::{
    Preferences, Row, Section, Settings, SettingsAction as A, SettingsState,
};
use std::sync::Arc;
use tca::{TestClock, TestStore};

#[test]
fn desktop_notifications_default_off() {
    let prefs = Preferences::default();
    assert!(!prefs.notify.enabled);
    assert_eq!(prefs.notify, NotifyConfig::default());
    assert!(!prefs.permission_request_alerts);
}

#[test]
fn toggling_on_requests_permission_and_toggling_off_does_not() {
    let (clients, fakes) =
        humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(SettingsState::default(), Settings::new(clients));

    assert_eq!(fakes.notifier.permission_requests(), 0);

    store.send(A::ToggleDesktopNotifications, |s| {
        s.preferences.notify.enabled = true;
    });
    assert!(!store.state().preferences.permission_request_alerts);

    store.send(A::ToggleDesktopNotifications, |s| {
        s.preferences.notify.enabled = false;
    });

    store.send(A::ToggleDesktopNotifications, |s| {
        s.preferences.notify.enabled = true;
    });
    store.finish();
    assert_eq!(fakes.notifier.permission_requests(), 2);
}

#[test]
fn toggling_on_once_requests_permission_once() {
    let (clients, fakes) =
        humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(SettingsState::default(), Settings::new(clients));

    store.send(A::ToggleDesktopNotifications, |s| {
        s.preferences.notify.enabled = true;
    });
    store.send(A::ToggleDesktopNotifications, |s| {
        s.preferences.notify.enabled = false;
    });
    store.finish();
    assert_eq!(fakes.notifier.permission_requests(), 1);
}

#[test]
fn deserialized_preferences_default_notify_and_preserve_explicit_values() {
    let omitted: Preferences = serde_json::from_str(r#"{"font_size":14}"#).unwrap();
    assert_eq!(omitted.font_size, 14);
    assert_eq!(omitted.notify, NotifyConfig::default());
    assert!(!omitted.notify.enabled);

    let enabled: Preferences = serde_json::from_str(r#"{"notify":{"enabled":true}}"#).unwrap();
    assert!(enabled.notify.enabled);
    assert!(enabled.notify.inbox_requests);
    assert!(enabled.notify.agent_questions);
    assert!(enabled.notify.failed_resumes);
    assert!(!enabled.notify.finished_turns);
    assert!(!enabled.notify.migrate_items);
    assert!(enabled.notify.quiet_when_frontmost);
    assert_eq!(enabled.notify.cooldown_secs, 30);
}

#[test]
fn row_is_in_notifications_before_permission_alerts_and_reachable_from_palette() {
    assert_eq!(Row::DesktopNotifications.title(), "Desktop notifications");
    assert_eq!(Row::DesktopNotifications.section(), Section::Notifications);
    assert_eq!(Row::PlaySound.title(), "Play a sound");
    assert_eq!(Row::PlaySound.section(), Section::Notifications);

    let desktop_pos = Row::ALL
        .iter()
        .position(|&r| r == Row::DesktopNotifications)
        .expect("Row::DesktopNotifications in Row::ALL");
    let sound_pos = Row::ALL
        .iter()
        .position(|&r| r == Row::PlaySound)
        .expect("Row::PlaySound in Row::ALL");
    let alerts_pos = Row::ALL
        .iter()
        .position(|&r| r == Row::PermissionRequestAlerts)
        .expect("Row::PermissionRequestAlerts in Row::ALL");
    assert!(
        desktop_pos < sound_pos && sound_pos < alerts_pos,
        "Row::ALL order should be DesktopNotifications < PlaySound < PermissionRequestAlerts"
    );

    let (clients, _) =
        humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(SettingsState::default(), Settings::new(clients));
    store.send(A::Reveal(Row::PlaySound), |s| {
        s.section = Section::Notifications;
        s.revealed = Some(Row::PlaySound);
    });
    store.receive_matching(
        "general loaded",
        |a| matches!(a, A::General(humata2_settings::GeneralAction::Loaded(Ok(_)))),
        |s| {
            s.general.settings = Some(humata2_clients::daemon::FullSettings::default());
            s.general.terminal_font_draft = "Menlo".to_owned();
            s.general.scrollback_draft = "10000".to_owned();
            s.general.archive_pr_days_draft = "0".to_owned();
            s.general.archive_idle_days_draft = "0".to_owned();
        },
    );
    store.finish();
}

#[test]
fn play_sound_defaults_off_and_is_decoupled_from_master_switch() {
    let (clients, _) =
        humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(SettingsState::default(), Settings::new(clients));

    assert!(!store.state().preferences.notify.play_sound);
    assert!(!store.state().preferences.notify.enabled);

    // Master is off: toggling play_sound still works (decoupled for in-app toasts).
    store.send(A::TogglePlaySound, |s| {
        s.preferences.notify.play_sound = true;
    });
    assert!(store.state().preferences.notify.play_sound);

    // Toggling master on and off preserves play_sound = true.
    store.send(A::ToggleDesktopNotifications, |s| {
        s.preferences.notify.enabled = true;
    });
    assert!(store.state().preferences.notify.play_sound);

    store.send(A::ToggleDesktopNotifications, |s| {
        s.preferences.notify.enabled = false;
    });
    assert!(store.state().preferences.notify.play_sound);

    // Turning play_sound off while master is off also works.
    store.send(A::TogglePlaySound, |s| {
        s.preferences.notify.play_sound = false;
    });
    assert!(!store.state().preferences.notify.play_sound);
    store.finish();
}

#[test]
fn in_app_toasts_test_notification_and_permission_check() {
    let (clients, fakes) =
        humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    fakes
        .notifier
        .set_permission(humata2_clients::PermissionStatus::Authorized);
    let mut store = TestStore::new(SettingsState::default(), Settings::new(clients));

    assert!(store.state().preferences.notify.in_app_toasts);
    store.send(A::ToggleInAppToasts, |s| {
        s.preferences.notify.in_app_toasts = false;
    });
    store.send(A::ToggleInAppToasts, |s| {
        s.preferences.notify.in_app_toasts = true;
    });

    store.send(A::SendTestNotification, |_| {});
    store.send(A::TogglePlaySound, |s| {
        s.preferences.notify.play_sound = true;
    });
    store.send(A::SendTestNotification, |_| {});

    store.send(A::RequestNotificationPermission, |_| {});
    store.receive(
        A::NotificationPermissionChecked(humata2_settings::PermissionStatus::Authorized),
        |s| {
            s.notification_permission = humata2_settings::PermissionStatus::Authorized;
        },
    );
    store.finish();

    let posted = fakes.notifier.posted();
    assert_eq!(posted.len(), 2);
    assert_eq!(posted[0].id, "humata.test");
    assert_eq!(posted[0].sound, None);
    assert_eq!(posted[1].id, "humata.test");
    assert_eq!(posted[1].sound, Some(humata2_clients::NoticeSound::Inbox));
    assert_eq!(fakes.notifier.permission_requests(), 1);
}

#[test]
fn restore_requests_permission_when_notifications_already_enabled() {
    let (clients, fakes) =
        humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(SettingsState::default(), Settings::new(clients));

    let mut prefs = Preferences::default();
    prefs.notify.enabled = true;
    store.send(A::Restore(prefs.clone()), |s| {
        s.preferences = prefs;
    });
    store.finish();
    assert_eq!(fakes.notifier.permission_requests(), 1);
}
