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
