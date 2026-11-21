//! Settings switch and live status for the host daemon's loopback To-do web
//! server (WEBTODO, D905).

/// What the daemon holds for the To-do web UI switch and its live listener.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebTodoSnapshot {
    /// Whether the host daemon serves the To-do UI at a local loopback URL.
    /// Off by default.
    pub enabled: bool,
    /// Live state of the server (output only: writes ignore it).
    pub status: WebTodoStatus,
}

/// Live state of the host daemon's To-do web server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum WebTodoStatus {
    /// Disabled by Settings (the default).
    #[default]
    Off,
    /// Starting up and binding its loopback socket.
    Starting,
    /// Listening on loopback `url` (e.g. `http://127.0.0.1:17420/`).
    Listening { url: String },
    /// Wanted, but could not start.
    Failed { reason: String },
}

impl WebTodoStatus {
    /// Returns the loopback URL when the server is listening.
    #[must_use]
    pub fn url(&self) -> Option<&str> {
        match self {
            Self::Listening { url } => Some(url.as_str()),
            _ => None,
        }
    }
}
