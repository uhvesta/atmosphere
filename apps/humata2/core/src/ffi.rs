//! What Swift calls: a [`Core`] it sends events to, and observers it's told
//! the state through, as plain records.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tca::{Observation, Store, StoreError, SystemClock};

use crate::daemon::{LiveDaemon, Session};
use crate::sessions::{Sessions, SessionsAction, SessionsFeature};

/// How often the list asks the daemon while it shows.
const POLL: Duration = Duration::from_secs(2);

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
        let daemon = Arc::new(LiveDaemon::new(socket.map(PathBuf::from)));
        let feature = SessionsFeature::new(daemon, Arc::new(SystemClock::new()), POLL);
        let sessions = Store::new(Sessions::default(), feature)
            .map_err(|source| CoreError::Start { source })?;
        Ok(Arc::new(Self { sessions }))
    }

    pub fn send(&self, event: SessionsEvent) {
        self.sessions.send(event.into());
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
