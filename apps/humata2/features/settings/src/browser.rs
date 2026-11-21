//! Settings › Browser (BROWSER): detected browsers on this Mac, native
//! messaging host install / update / uninstall, and copying or revealing the
//! unpacked extension folder.

use humata2_clients::model::{BrowserKind, DetectedBrowser};
use humata2_clients::{Clients, DaemonError};
use tca::Effect;

/// What the daemon last said about detected browsers on this Mac.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrowserState {
    /// `None` until the daemon has answered.
    pub browsers: Option<Vec<DetectedBrowser>>,
    /// Whether an install or uninstall call is in flight.
    pub busy: bool,
    /// The extension path most recently copied to the clipboard, if any.
    pub copied_path: Option<String>,
}

/// What happens on Settings › Browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserAction {
    /// Read detected browsers from the daemon.
    Load,
    /// The daemon answered with detected browsers, or failed.
    Loaded(Result<Vec<DetectedBrowser>, DaemonError>),
    /// Set up or update Humata's native messaging host for one browser.
    Install(BrowserKind),
    /// Set up or update Humata's native messaging host for every detected browser.
    InstallAll,
    /// Remove Humata's native messaging host manifest for one browser.
    Uninstall(BrowserKind),
    /// Copy an unpacked extension folder path to the clipboard.
    CopyExtensionPath(String),
    /// Reveal an unpacked extension folder in Finder.
    RevealExtensionPath(String),
    /// The daemon answered an install or uninstall call.
    Changed(Result<Vec<DetectedBrowser>, DaemonError>),
}

/// Applies `action`, calling the daemon through `clients`. `error` is the
/// settings page's error line.
pub(crate) fn reduce(
    clients: &Clients,
    state: &mut BrowserState,
    error: &mut Option<String>,
    action: BrowserAction,
) -> Effect<BrowserAction> {
    match action {
        BrowserAction::Load => {
            let daemon = clients.daemon.clone();
            Effect::future(async move {
                BrowserAction::Loaded(daemon.list_detected_browsers().await)
            })
        }
        BrowserAction::Loaded(result) => {
            match result {
                Ok(browsers) => {
                    state.browsers = Some(browsers);
                    *error = None;
                }
                Err(failure) => *error = Some(failure.to_string()),
            }
            Effect::none()
        }
        BrowserAction::Install(kind) => {
            if state.busy {
                return Effect::none();
            }
            state.busy = true;
            let daemon = clients.daemon.clone();
            Effect::future(async move {
                BrowserAction::Changed(daemon.install_browser_host(vec![kind]).await)
            })
        }
        BrowserAction::InstallAll => {
            if state.busy {
                return Effect::none();
            }
            state.busy = true;
            let daemon = clients.daemon.clone();
            Effect::future(async move {
                BrowserAction::Changed(daemon.install_browser_host(Vec::new()).await)
            })
        }
        BrowserAction::Uninstall(kind) => {
            if state.busy {
                return Effect::none();
            }
            state.busy = true;
            let daemon = clients.daemon.clone();
            Effect::future(async move {
                BrowserAction::Changed(daemon.uninstall_browser_host(vec![kind]).await)
            })
        }
        BrowserAction::CopyExtensionPath(path) => {
            clients.clipboard.copy(&path);
            state.copied_path = Some(path);
            Effect::none()
        }
        BrowserAction::RevealExtensionPath(path) => {
            if let Err(failure) = clients.reveal.reveal(&path) {
                *error = Some(failure.to_string());
            }
            Effect::none()
        }
        BrowserAction::Changed(result) => {
            state.busy = false;
            match result {
                Ok(browsers) => {
                    state.browsers = Some(browsers);
                    *error = None;
                }
                Err(failure) => *error = Some(failure.to_string()),
            }
            Effect::none()
        }
    }
}
