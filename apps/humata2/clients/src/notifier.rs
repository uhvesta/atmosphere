//! Desktop notifications: posting, withdrawing, and requesting permission.
//!
//! Kept free of OS dependencies so `humata2_client_api` stays `wasm32`-clean.

use serde::{Deserialize, Serialize};

use crate::model::{ItemId, RequestId, SessionId};

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
    /// The To-do, with the item an agent's To-do ask is about selected, or
    /// only the To-do when the ask suggests a new item (NOTIFY2).
    Todo {
        /// The item to select.
        item: Option<ItemId>,
    },
}

/// The sound a notification plays, when Play a sound is on (NOTIFY2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoticeSound {
    /// Two rising notes: an agent waits on you in the Inbox.
    Inbox,
    /// One lower, softer note: something to look at on the To-do.
    Todo,
}

impl NoticeSound {
    /// The sound file's name in the app bundle's Resources
    /// (`app/sounds/make_sounds.py` writes them).
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Inbox => "humata2-inbox.wav",
            Self::Todo => "humata2-todo.wav",
        }
    }
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
    /// The sound it plays, or `None` for a silent one.
    pub sound: Option<NoticeSound>,
}

/// OS notification authorization state reported by the platform notifier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionStatus {
    /// Permission state could not be determined.
    #[default]
    Unknown,
    /// Notifications are allowed by the user.
    Authorized,
    /// Notifications are explicitly denied by the user.
    Denied,
    /// The user has not yet been prompted for notification permission.
    NotDetermined,
    /// Notifications are provisionally authorized (quiet delivery).
    Provisional,
    /// Notifications are authorized for a temporary session.
    Ephemeral,
    /// Running outside a bundled app where OS notifications are unavailable.
    Unbundled,
}

/// Platform seam for posting and withdrawing desktop notifications.
pub trait Notifier: Send + Sync {
    /// Posts `notice`, replacing any existing notification with the same `id`.
    fn post(&self, notice: DesktopNotice);

    /// Withdraws any delivered or pending notifications matching `ids`.
    fn withdraw(&self, ids: &[String]);

    /// Requests OS notification permission if not yet granted.
    fn request_permission(&self);

    /// Queries the current OS notification authorization status.
    fn query_permission(&self) -> PermissionStatus;

    /// Plays a notification sound immediately.
    fn play_sound(&self, sound: NoticeSound);

    /// Updates the application's dock badge count, or clears it when `None`.
    fn set_dock_badge(&self, count: Option<usize>);
}

/// A notifier that drops every call. Used when notifications are unavailable
/// or disabled on the current platform.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopNotifier;

impl Notifier for NoopNotifier {
    fn post(&self, _notice: DesktopNotice) {}

    fn withdraw(&self, _ids: &[String]) {}

    fn request_permission(&self) {}

    fn query_permission(&self) -> PermissionStatus {
        PermissionStatus::Unknown
    }

    fn play_sound(&self, _sound: NoticeSound) {}

    fn set_dock_badge(&self, _count: Option<usize>) {}
}
