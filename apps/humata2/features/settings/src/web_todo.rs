//! Settings › To-do web UI switch (WEBTODO, D905): turns the host daemon's
//! loopback To-do web server on and off (off by default) and displays the live
//! local URL next to the switch. Follows Settings › Sync's optimistic save and
//! rollback-on-refusal flow.

use humata2_clients::daemon::WebTodoSnapshot;
use humata2_clients::{Clients, DaemonError};
use tca::Effect;

/// What the daemon last said about the To-do web UI switch and its live URL.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebTodoState {
    /// `None` until the daemon has answered.
    pub snapshot: Option<WebTodoSnapshot>,
}

impl WebTodoState {
    /// Whether the switch is currently on (defaults to `false`).
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.snapshot.as_ref().is_some_and(|s| s.enabled)
    }

    /// Loopback URL to show next to the switch when the server is listening.
    #[must_use]
    pub fn url(&self) -> Option<&str> {
        self.snapshot.as_ref().and_then(|s| s.status.url())
    }
}

/// What happens on the To-do web UI switch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebTodoAction {
    /// Read the switch and live server status from the daemon.
    Load,
    /// The daemon answered with the current switch and status, or failed.
    Loaded(Result<WebTodoSnapshot, DaemonError>),
    /// Flip the switch on or off.
    ToggleEnabled,
    /// The daemon saved the change, or refused.
    Saved {
        /// Snapshot shown before the toggle, restored on refusal.
        before: WebTodoSnapshot,
        /// Snapshot returned by the daemon, or the refusal error.
        result: Result<WebTodoSnapshot, DaemonError>,
    },
}

/// Applies `action`, calling the daemon through `clients`.
pub(crate) fn reduce(
    clients: &Clients,
    state: &mut WebTodoState,
    error: &mut Option<String>,
    action: WebTodoAction,
) -> Effect<WebTodoAction> {
    match action {
        WebTodoAction::Load => {
            let daemon = clients.daemon.clone();
            Effect::future(async move { WebTodoAction::Loaded(daemon.web_todo_settings().await) })
        }
        WebTodoAction::Loaded(result) => {
            match result {
                Ok(snapshot) => state.snapshot = Some(snapshot),
                Err(failure) => *error = Some(failure.to_string()),
            }
            Effect::none()
        }
        WebTodoAction::ToggleEnabled => {
            let Some(snapshot) = state.snapshot.as_mut() else {
                return Effect::none();
            };
            let before = snapshot.clone();
            snapshot.enabled = !snapshot.enabled;
            let wanted = snapshot.enabled;
            let daemon = clients.daemon.clone();
            Effect::future(async move {
                WebTodoAction::Saved {
                    before,
                    result: daemon.set_web_todo_enabled(wanted).await,
                }
            })
        }
        WebTodoAction::Saved { before, result } => {
            match result {
                Ok(saved) => {
                    state.snapshot = Some(saved);
                    *error = None;
                }
                Err(failure) => {
                    state.snapshot = Some(before);
                    *error = Some(failure.to_string());
                }
            }
            Effect::none()
        }
    }
}
