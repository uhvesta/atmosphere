//! The fake's Settings › Browser host discovery and install/uninstall (BROWSER).

use super::FakeDaemon;
use crate::daemon::{Operation, Reply};
use crate::model::{
    BrowserBridgeStatus, BrowserFamily, BrowserKind, ConnectedBrowserHost, DetectedBrowser,
};

/// Default detected browsers matching frame `b1-settings-browser`.
pub(super) fn defaults() -> Vec<DetectedBrowser> {
    vec![
        DetectedBrowser {
            kind: BrowserKind::Chrome,
            family: BrowserFamily::Chromium,
            display_name: "Chrome".to_owned(),
            binary_path: "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".to_owned(),
            native_messaging_dir:
                "~/Library/Application Support/Google/Chrome/NativeMessagingHosts".to_owned(),
            manifest_path:
                "~/Library/Application Support/Google/Chrome/NativeMessagingHosts/app.humata2.browser_host.json"
                    .to_owned(),
            host_installed: true,
            host_matches_current: true,
            connected: true,
            extension_path: "apps/humata2/browser_extension/chromium".to_owned(),
        },
        DetectedBrowser {
            kind: BrowserKind::Brave,
            family: BrowserFamily::Chromium,
            display_name: "Brave".to_owned(),
            binary_path: "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser".to_owned(),
            native_messaging_dir:
                "~/Library/Application Support/BraveSoftware/Brave-Browser/NativeMessagingHosts"
                    .to_owned(),
            manifest_path:
                "~/Library/Application Support/BraveSoftware/Brave-Browser/NativeMessagingHosts/app.humata2.browser_host.json"
                    .to_owned(),
            host_installed: true,
            host_matches_current: true,
            connected: false,
            extension_path: "apps/humata2/browser_extension/chromium".to_owned(),
        },
        DetectedBrowser {
            kind: BrowserKind::Edge,
            family: BrowserFamily::Chromium,
            display_name: "Edge".to_owned(),
            binary_path: "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"
                .to_owned(),
            native_messaging_dir:
                "~/Library/Application Support/Microsoft Edge/NativeMessagingHosts".to_owned(),
            manifest_path:
                "~/Library/Application Support/Microsoft Edge/NativeMessagingHosts/app.humata2.browser_host.json"
                    .to_owned(),
            host_installed: true,
            host_matches_current: false,
            connected: false,
            extension_path: "apps/humata2/browser_extension/chromium".to_owned(),
        },
        DetectedBrowser {
            kind: BrowserKind::Firefox,
            family: BrowserFamily::Firefox,
            display_name: "Firefox".to_owned(),
            binary_path: "/Applications/Firefox.app/Contents/MacOS/firefox".to_owned(),
            native_messaging_dir: "~/Library/Application Support/Mozilla/NativeMessagingHosts"
                .to_owned(),
            manifest_path:
                "~/Library/Application Support/Mozilla/NativeMessagingHosts/app.humata2.browser_host.json"
                    .to_owned(),
            host_installed: false,
            host_matches_current: false,
            connected: false,
            extension_path: "apps/humata2/browser_extension/firefox".to_owned(),
        },
    ]
}

pub(super) fn list_detected_browsers(daemon: &FakeDaemon) -> Reply<Vec<DetectedBrowser>> {
    daemon.reply(
        Some(Operation::ListDetectedBrowsers),
        "list_detected_browsers".to_owned(),
        |inner| Ok(inner.browsers.clone()),
    )
}

pub(super) fn install_browser_host(
    daemon: &FakeDaemon,
    kinds: Vec<BrowserKind>,
) -> Reply<Vec<DetectedBrowser>> {
    let call = format!("install_browser_host {kinds:?}");
    daemon.reply(Some(Operation::InstallBrowserHost), call, move |inner| {
        for browser in &mut inner.browsers {
            if kinds.is_empty() || kinds.contains(&browser.kind) {
                browser.host_installed = true;
                browser.host_matches_current = true;
            }
        }
        Ok(inner.browsers.clone())
    })
}

pub(super) fn uninstall_browser_host(
    daemon: &FakeDaemon,
    kinds: Vec<BrowserKind>,
) -> Reply<Vec<DetectedBrowser>> {
    let call = format!("uninstall_browser_host {kinds:?}");
    daemon.reply(Some(Operation::UninstallBrowserHost), call, move |inner| {
        for browser in &mut inner.browsers {
            if kinds.is_empty() || kinds.contains(&browser.kind) {
                browser.host_installed = false;
                browser.host_matches_current = false;
                browser.connected = false;
            }
        }
        Ok(inner.browsers.clone())
    })
}

pub(super) fn browser_bridge_status(daemon: &FakeDaemon) -> Reply<BrowserBridgeStatus> {
    daemon.reply(
        Some(Operation::BrowserBridgeStatus),
        "browser_bridge_status".to_owned(),
        |inner| {
            let hosts: Vec<ConnectedBrowserHost> = inner
                .browsers
                .iter()
                .enumerate()
                .filter(|(_, b)| b.connected)
                .map(|(i, b)| ConnectedBrowserHost {
                    extension_origin: match b.family {
                        BrowserFamily::Chromium => {
                            format!("chrome-extension://fake-{}/", b.kind.slug())
                        }
                        BrowserFamily::Firefox => b.manifest_path.clone(),
                    },
                    host_pid: 42_000 + u32::try_from(i).unwrap_or(0),
                    family: b.family,
                })
                .collect();
            Ok(BrowserBridgeStatus {
                connected: !hosts.is_empty(),
                hosts,
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tca::TestClock;

    use super::*;
    use crate::daemon::Daemon;
    use crate::fixture;

    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(future)
    }

    #[test]
    fn install_and_uninstall_update_detected_browsers_and_bridge_status() {
        let daemon = FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()));
        let listed = block_on(daemon.list_detected_browsers()).expect("list");
        assert_eq!(listed.len(), 4);
        assert!(listed.iter().any(|b| b.kind == BrowserKind::Chrome && b.connected));
        assert!(listed.iter().any(|b| b.kind == BrowserKind::Firefox && !b.host_installed));

        let installed = block_on(daemon.install_browser_host(vec![BrowserKind::Firefox]))
            .expect("install firefox");
        let firefox = installed
            .iter()
            .find(|b| b.kind == BrowserKind::Firefox)
            .expect("firefox");
        assert!(firefox.host_installed && firefox.host_matches_current);

        let all = block_on(daemon.install_browser_host(Vec::new())).expect("install all");
        assert!(all.iter().all(|b| b.host_installed && b.host_matches_current));

        let after_uninstall = block_on(daemon.uninstall_browser_host(vec![BrowserKind::Chrome]))
            .expect("uninstall chrome");
        let chrome = after_uninstall
            .iter()
            .find(|b| b.kind == BrowserKind::Chrome)
            .expect("chrome");
        assert!(!chrome.host_installed && !chrome.connected);

        let status = block_on(daemon.browser_bridge_status()).expect("status");
        assert!(!status.connected);
    }
}
