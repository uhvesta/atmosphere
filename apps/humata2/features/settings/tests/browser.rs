//! Settings › Browser (BROWSER): opening the section loads detected browsers,
//! install / uninstall / install-all update state, and errors surface cleanly.

use std::sync::Arc;

use humata2_clients::model::{BrowserFamily, BrowserKind, DetectedBrowser};
use humata2_clients::{FakeDaemon, Operation, fixture};
use humata2_settings::{
    BrowserAction, Section, Settings, SettingsAction as A, SettingsState,
};
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<Settings>, FakeDaemon) {
    let (clients, fakes) =
        humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let store = TestStore::new(SettingsState::default(), Settings::new(clients));
    (store, fakes.daemon)
}

fn browsers() -> Vec<DetectedBrowser> {
    vec![
        DetectedBrowser {
            kind: BrowserKind::Chrome,
            family: BrowserFamily::Chromium,
            display_name: "Chrome".to_owned(),
            binary_path: "/Applications/Google Chrome.app".to_owned(),
            native_messaging_dir: "/tmp/chrome".to_owned(),
            manifest_path: "/tmp/chrome/app.humata2.browser_host.json".to_owned(),
            host_installed: true,
            host_matches_current: true,
            connected: true,
            extension_path: "apps/humata2/browser_extension/chromium".to_owned(),
        },
        DetectedBrowser {
            kind: BrowserKind::Firefox,
            family: BrowserFamily::Firefox,
            display_name: "Firefox".to_owned(),
            binary_path: "/Applications/Firefox.app".to_owned(),
            native_messaging_dir: "/tmp/firefox".to_owned(),
            manifest_path: "/tmp/firefox/app.humata2.browser_host.json".to_owned(),
            host_installed: false,
            host_matches_current: false,
            connected: false,
            extension_path: "apps/humata2/browser_extension/firefox".to_owned(),
        },
    ]
}

#[test]
fn opening_browser_section_loads_detected_browsers() {
    let (mut store, daemon) = setup();
    let expected = browsers();
    daemon.set_detected_browsers(expected.clone());

    assert!(Section::ALL.contains(&Section::Browser));
    assert_eq!(Section::Browser.title(), "Browser");

    store.send(A::Section(Section::Browser), |s| {
        s.section = Section::Browser;
    });
    let shown = expected.clone();
    store.receive(A::Browser(BrowserAction::Loaded(Ok(expected))), move |s| {
        s.browser.browsers = Some(shown);
    });
    store.finish();
}

#[test]
fn install_uninstall_and_install_all_update_browsers() {
    let (mut store, daemon) = setup();
    let initial = browsers();
    daemon.set_detected_browsers(initial.clone());

    store.send(A::Browser(BrowserAction::Load), |_| {});
    let shown = initial.clone();
    store.receive(A::Browser(BrowserAction::Loaded(Ok(initial.clone()))), move |s| {
        s.browser.browsers = Some(shown);
    });

    // Install Firefox.
    let mut after_install = initial.clone();
    after_install[1].host_installed = true;
    after_install[1].host_matches_current = true;
    store.send(A::Browser(BrowserAction::Install(BrowserKind::Firefox)), |s| {
        s.browser.busy = true;
    });
    let shown = after_install.clone();
    store.receive(
        A::Browser(BrowserAction::Changed(Ok(after_install.clone()))),
        move |s| {
            s.browser.busy = false;
            s.browser.browsers = Some(shown);
        },
    );

    // Uninstall Chrome.
    let mut after_uninstall = after_install.clone();
    after_uninstall[0].host_installed = false;
    after_uninstall[0].host_matches_current = false;
    after_uninstall[0].connected = false;
    store.send(A::Browser(BrowserAction::Uninstall(BrowserKind::Chrome)), |s| {
        s.browser.busy = true;
    });
    let shown = after_uninstall.clone();
    store.receive(
        A::Browser(BrowserAction::Changed(Ok(after_uninstall.clone()))),
        move |s| {
            s.browser.busy = false;
            s.browser.browsers = Some(shown);
        },
    );

    // Install all brings Chrome back.
    let mut after_all = after_uninstall;
    after_all[0].host_installed = true;
    after_all[0].host_matches_current = true;
    store.send(A::Browser(BrowserAction::InstallAll), |s| {
        s.browser.busy = true;
    });
    let shown = after_all.clone();
    store.receive(A::Browser(BrowserAction::Changed(Ok(after_all))), move |s| {
        s.browser.busy = false;
        s.browser.browsers = Some(shown);
    });
    store.finish();
}

#[test]
fn failed_install_clears_busy_and_sets_error() {
    let (store, daemon) = setup();
    let mut store = store.non_exhaustive();
    daemon.set_detected_browsers(browsers());
    daemon.fail_next(Operation::InstallBrowserHost, "permission denied");

    store.send(A::Browser(BrowserAction::Install(BrowserKind::Firefox)), |s| {
        s.browser.busy = true;
    });
    store.receive_matching(
        "install failed",
        |a| matches!(a, A::Browser(BrowserAction::Changed(Err(_)))),
        |_| {},
    );
    assert!(!store.state().browser.busy);
    assert!(store.state().error.is_some());
    store.finish();
}
