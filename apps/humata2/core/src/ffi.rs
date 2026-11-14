//! What Swift calls: a [`Core`] it sends events to, and observers it's told
//! the state through, as plain records.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tca::{Clock, Observation, Store, StoreError, SystemClock};

use crate::chat::{Chat, ChatAction, ChatFeature};
use crate::daemon::{Daemon, LiveDaemon, Session};
use crate::feed::{ChatUpdate, Feed};
use crate::sessions::{Sessions, SessionsAction, SessionsFeature};

/// How often the list asks the daemon while it shows.
const POLL: Duration = Duration::from_secs(2);

/// How long the chat waits to follow its session again after it stopped.
const REWATCH: Duration = Duration::from_secs(1);

/// Why the core couldn't start.
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum CoreError {
    #[error("Humata 2 couldn't start its threads. Quit other apps and open it again.")]
    Start {
        #[source]
        source: StoreError,
    },
}

/// Holds the features' state, on threads of its own.
#[derive(uniffi::Object)]
pub struct Core {
    sessions: Store<SessionsFeature>,
    chat: Store<ChatFeature>,
}

/// What the person did on the session list.
#[derive(uniffi::Enum)]
pub enum SessionsEvent {
    Appeared,
    Disappeared,
    Refresh,
    Select { id: Option<String> },
}

impl From<SessionsEvent> for SessionsAction {
    fn from(event: SessionsEvent) -> Self {
        match event {
            SessionsEvent::Appeared => Self::Appeared,
            SessionsEvent::Disappeared => Self::Disappeared,
            SessionsEvent::Refresh => Self::Refresh,
            SessionsEvent::Select { id } => Self::Select(id),
        }
    }
}

/// The session list as Swift lays it out.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct SessionList {
    /// Newest first.
    pub rows: Vec<Session>,
    pub loading: bool,
    pub error: Option<String>,
    pub selected: Option<String>,
}

impl From<&Sessions> for SessionList {
    fn from(sessions: &Sessions) -> Self {
        Self {
            rows: sessions.list.iter().cloned().collect(),
            loading: sessions.loading,
            error: sessions.error.clone(),
            selected: sessions.selected.clone(),
        }
    }
}

/// What the person did in the chat.
#[derive(uniffi::Enum)]
pub enum ChatEvent {
    Send {
        text: String,
    },
    /// The unsent message was put back in the message field.
    TookUnsent,
    Stop,
}

impl From<ChatEvent> for ChatAction {
    fn from(event: ChatEvent) -> Self {
        match event {
            ChatEvent::Send { text } => Self::Send(text),
            ChatEvent::TookUnsent => Self::TookUnsent,
            ChatEvent::Stop => Self::Stop,
        }
    }
}

/// Told what changed in the chat, on any thread, in order: first all of
/// it, then each change.
#[uniffi::export(with_foreign)]
pub trait ChatObserver: Send + Sync {
    fn changed(&self, update: ChatUpdate);
}

/// An update made once per batch and handed over, not copied: two are
/// alike when their numbers are.
struct Told {
    number: u64,
    update: Mutex<Option<ChatUpdate>>,
}

impl PartialEq for Told {
    fn eq(&self, other: &Self) -> bool {
        self.number == other.number
    }
}

/// Told the session list now and each time it changes, on any thread.
#[uniffi::export(with_foreign)]
pub trait SessionsObserver: Send + Sync {
    fn changed(&self, list: SessionList);
}

/// Keeps an observer told; dropping it stops that.
#[derive(uniffi::Object)]
pub struct Watch {
    _observation: Observation,
}

#[uniffi::export]
impl Core {
    /// A core reaching the daemon at `socket`, or at its usual place.
    #[uniffi::constructor]
    pub fn new(socket: Option<String>) -> Result<Arc<Self>, CoreError> {
        let daemon: Arc<dyn Daemon> = Arc::new(LiveDaemon::new(socket.map(PathBuf::from)));
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let feature = SessionsFeature::new(daemon.clone(), clock.clone(), POLL);
        let sessions = Store::new(Sessions::default(), feature)
            .map_err(|source| CoreError::Start { source })?;
        let chat = Store::new(Chat::default(), ChatFeature::new(daemon, clock, REWATCH))
            .map_err(|source| CoreError::Start { source })?;
        Ok(Arc::new(Self { sessions, chat }))
    }

    /// Picking a session opens its chat.
    pub fn send(&self, event: SessionsEvent) {
        if let SessionsEvent::Select { id } = &event {
            self.chat.send(match id {
                Some(id) => ChatAction::Open(id.clone()),
                None => ChatAction::Close,
            });
        }
        self.sessions.send(event.into());
    }

    pub fn send_chat(&self, event: ChatEvent) {
        self.chat.send(event.into());
    }

    pub fn observe_chat(&self, observer: Arc<dyn ChatObserver>) -> Arc<Watch> {
        let feed = Mutex::new(Feed::new());
        let observation = self.chat.observe(
            move |chat: &Chat| {
                let update = feed
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .next(chat);
                Told {
                    number: update.number,
                    update: Mutex::new((!update.is_empty()).then_some(update)),
                }
            },
            move |told: &Told| {
                let update = told
                    .update
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                if let Some(update) = update {
                    observer.changed(update);
                }
            },
        );
        Arc::new(Watch {
            _observation: observation,
        })
    }

    pub fn observe(&self, observer: Arc<dyn SessionsObserver>) -> Arc<Watch> {
        let observation = self.sessions.observe(
            |sessions: &Sessions| SessionList::from(sessions),
            move |list| observer.changed(list.clone()),
        );
        Arc::new(Watch {
            _observation: observation,
        })
    }
}
