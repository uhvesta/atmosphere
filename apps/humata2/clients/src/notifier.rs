//! Desktop notifications: posting, withdrawing, and requesting permission.
//!
//! Kept free of OS dependencies so `humata2_client_api` stays `wasm32`-clean.

use serde::{Deserialize, Serialize};

use crate::model::{RequestId, SessionId};

/// Where clicking a desktop notification opens the app.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotifyTarget {
    /// A specific Agent inbox request in a session.
    Request {
        /// The session the request belongs to.
        session: SessionId,
        /// The request to select.
        request: RequestId,
    },
    /// A session's main view.
    Session(SessionId),
    /// The global Agent inbox.
    Inbox,
    /// The Home view.
    Home,
}

/// A desktop notification ready to hand to the platform notifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopNotice {
    /// Stable identifier used to replace in place or withdraw later.
    pub id: String,
    /// Primary notification heading.
    pub title: String,
    /// Secondary notification body.
    pub body: String,
    /// Where clicking the notification navigates.
    pub target: NotifyTarget,
}

/// Platform seam for posting and withdrawing desktop notifications.
pub trait Notifier: Send + Sync {
    /// Posts `notice`, replacing any existing notification with the same `id`.
    fn post(&self, notice: DesktopNotice);

    /// Withdraws any delivered or pending notifications matching `ids`.
    fn withdraw(&self, ids: &[String]);

    /// Requests OS notification permission if not yet granted.
    fn request_permission(&self);
}

/// A notifier that drops every call. Used when notifications are unavailable
/// or disabled on the current platform.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopNotifier;

impl Notifier for NoopNotifier {
    fn post(&self, _notice: DesktopNotice) {}

    fn withdraw(&self, _ids: &[String]) {}

    fn request_permission(&self) {}
}
