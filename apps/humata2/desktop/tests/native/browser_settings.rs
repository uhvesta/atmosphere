//! Settings › Browser and Browser read / write request cards (BROWSER,
//! frames `b1-settings-browser`, `b2-inbox-browser-read`, `b3-inbox-browser-write`).
//! Headless, on fakes.

use super::*;

use humata2_clients::model::{Answer, BrowserKind, RequestId};
use humata2_inbox::InboxAction;
use humata2_settings::{Section, SettingsAction};

#[test]
fn settings_browser_shows_detected_browsers_and_handles_install_and_uninstall() {
    let mut app = App::new(fixture::design());
    app.send(AppAction::Switch(Route::Settings));
    app.send(AppAction::Settings(SettingsAction::Section(Section::Browser)));

    assert!(app.contains("settings-browser-page"));
    assert!(app.contains("settings-browser-install-all"));
    assert!(app.contains("settings-browser-row-chrome"));
    assert!(app.contains("settings-browser-row-brave"));
    assert!(app.contains("settings-browser-row-edge"));
    assert!(app.contains("settings-browser-row-firefox"));

    // Chrome is set up & connected -> Uninstall only.
    assert!(app.contains("settings-browser-uninstall-chrome"));
    assert!(!app.contains("settings-browser-install-chrome"));

    // Edge is out of date -> Update host + Uninstall.
    assert!(app.contains("settings-browser-install-edge"));
    assert!(app.contains("settings-browser-uninstall-edge"));

    // Firefox is not set up -> Set up host only.
    assert!(app.contains("settings-browser-install-firefox"));
    assert!(!app.contains("settings-browser-uninstall-firefox"));

    // Extension copy/reveal links are present.
    assert!(app.contains("settings-browser-copy-chromium"));
    assert!(app.contains("settings-browser-reveal-chromium"));
    assert!(app.contains("settings-browser-copy-firefox"));
    assert!(app.contains("settings-browser-reveal-firefox"));

    // Clicking Set up host on Firefox installs its manifest.
    app.click("settings-browser-install-firefox");
    let firefox_installed = app.ui.read(|s| {
        s.settings
            .browser
            .browsers
            .as_ref()
            .and_then(|list| list.iter().find(|b| b.kind == BrowserKind::Firefox))
            .is_some_and(|b| b.host_installed && b.host_matches_current)
    });
    assert!(firefox_installed);
    assert!(app.contains("settings-browser-uninstall-firefox"));
    assert!(!app.contains("settings-browser-install-firefox"));

    // Clicking Uninstall on Chrome removes it.
    app.click("settings-browser-uninstall-chrome");
    let chrome_installed = app.ui.read(|s| {
        s.settings
            .browser
            .browsers
            .as_ref()
            .and_then(|list| list.iter().find(|b| b.kind == BrowserKind::Chrome))
            .is_some_and(|b| b.host_installed)
    });
    assert!(!chrome_installed);
    assert!(app.contains("settings-browser-install-chrome"));

    // Clicking Install all installs every browser.
    app.click("settings-browser-install-all");
    let all_installed = app.ui.read(|s| {
        s.settings
            .browser
            .browsers
            .as_ref()
            .is_some_and(|list| list.iter().all(|b| b.host_installed && b.host_matches_current))
    });
    assert!(all_installed);

    // Copy path puts the unpacked extension folder on the clipboard.
    app.click("settings-browser-copy-chromium");
    assert_eq!(
        app.fakes.clipboard.copied().as_deref(),
        Some("apps/humata2/browser_extension/chromium")
    );
}

#[test]
fn browser_read_card_offers_allow_once_allow_30m_and_deny_only() {
    let mut workspace = fixture::design();
    workspace.requests = vec![fixture::browser_read_request(
        "r-browser-read",
        "ledger",
        "payments",
    )];
    let mut app = App::new(workspace);
    app.send(AppAction::Switch(Route::Inbox));

    // Exactly three buttons: Allow once, Allow 30 min, Deny.
    assert!(app.contains("allow r-browser-read"));
    assert!(app.contains("allow 30m r-browser-read"));
    assert!(app.contains("deny r-browser-read"));
    // No Ask why, no Always allow box, no rule link.
    assert!(!app.contains("ask why r-browser-read"));
    assert!(!app.contains("always r-browser-read"));
    assert!(!app.contains("change rule r-browser-read"));

    // Clicking Allow 30 min sends Answer::AllowFor30Min.
    app.click("allow 30m r-browser-read");
    assert_eq!(
        app.fakes.daemon.answers(),
        vec![(RequestId::new("r-browser-read"), Answer::AllowFor30Min)]
    );
}

#[test]
fn browser_write_card_offers_allow_once_and_deny_only_with_url_and_target_preview() {
    let mut workspace = fixture::design();
    workspace.requests = vec![
        fixture::browser_write_request("r-browser-write", "ledger", "payments"),
        fixture::browser_read_request("r-browser-read", "ledger", "payments"),
    ];
    let mut app = App::new(workspace);
    app.send(AppAction::Switch(Route::Inbox));

    // Browser write: Allow once and Deny ONLY; exact URL and target preview shown.
    assert!(app.contains("allow r-browser-write"));
    assert!(app.contains("deny r-browser-write"));
    assert!(!app.contains("allow 30m r-browser-write"));
    assert!(!app.contains("ask why r-browser-write"));
    assert!(!app.contains("always r-browser-write"));
    assert!(app.contains("browser url r-browser-write"));
    assert!(app.contains("browser target r-browser-write"));

    // ApproveAll approves BrowserRead and skips BrowserWrite.
    app.send(AppAction::Inbox(InboxAction::ApproveAll));
    assert_eq!(
        app.fakes.daemon.answers(),
        vec![(RequestId::new("r-browser-read"), Answer::Approved)]
    );
    // Browser write is still waiting and can be approved once individually.
    assert!(app.contains("allow r-browser-write"));
    app.click("allow r-browser-write");
    assert_eq!(
        app.fakes.daemon.answers(),
        vec![
            (RequestId::new("r-browser-read"), Answer::Approved),
            (RequestId::new("r-browser-write"), Answer::Approved),
        ]
    );
}
