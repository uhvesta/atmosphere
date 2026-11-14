//! The list of sessions: kept up to date while it shows, newest first.

use std::sync::Arc;
use std::time::Duration;

use tca::{Clock, Effect, IdentifiedVec, Reducer};

use crate::daemon::{Daemon, Session};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sessions {
    /// Newest first, without archived ones.
    pub list: IdentifiedVec<Session>,
    /// Waiting for the first list.
    pub loading: bool,
    /// Why the last try failed, until one succeeds. The list stays as it
    /// was.
    pub error: Option<String>,
    pub selected: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum SessionsAction {
    /// The list showed: keep it up to date.
    Appeared,
    /// The list went away: stop.
    Disappeared,
    /// Ask again now.
    Refresh,
    Select(Option<String>),
    Loaded(Vec<Session>),
    Failed(String),
}

/// The session list's reducer, asking `daemon` every `every` while it shows.
pub struct SessionsFeature {
    daemon: Arc<dyn Daemon>,
    clock: Arc<dyn Clock>,
    every: Duration,
}

impl SessionsFeature {
    pub fn new(daemon: Arc<dyn Daemon>, clock: Arc<dyn Clock>, every: Duration) -> Self {
        Self {
            daemon,
            clock,
            every,
        }
    }
}

/// Asks `daemon` for the sessions once.
async fn ask(daemon: Arc<dyn Daemon>) -> SessionsAction {
    match daemon.sessions().await {
        Ok(sessions) => SessionsAction::Loaded(sessions),
        Err(error) => SessionsAction::Failed(error.to_string()),
    }
}

const POLL: &str = "sessions.poll";
const REFRESH: &str = "sessions.refresh";

impl Reducer for SessionsFeature {
    type State = Sessions;
    type Action = SessionsAction;

    fn reduce(&self, state: &mut Sessions, action: SessionsAction) -> Effect<SessionsAction> {
        match action {
            SessionsAction::Appeared => {
                state.loading = state.list.is_empty() && state.error.is_none();
                let daemon = self.daemon.clone();
                let clock = self.clock.clone();
                let every = self.every;
                Effect::run(move |send| async move {
                    loop {
                        send.send(ask(daemon.clone()).await);
                        clock.sleep(every).await;
                    }
                })
                .cancellable(POLL, true)
            }
            SessionsAction::Disappeared => Effect::cancel(POLL).and(Effect::cancel(REFRESH)),
            SessionsAction::Refresh => {
                Effect::future(ask(self.daemon.clone())).cancellable(REFRESH, true)
            }
            SessionsAction::Select(id) => {
                state.selected = id;
                Effect::none()
            }
            SessionsAction::Loaded(mut sessions) => {
                state.loading = false;
                state.error = None;
                sessions.retain(|session| !session.archived);
                sessions.sort_by(|a, b| {
                    b.last_active_unix_ms
                        .cmp(&a.last_active_unix_ms)
                        .then_with(|| a.name.cmp(&b.name))
                });
                let list: IdentifiedVec<Session> = sessions.into_iter().collect();
                if list != state.list {
                    state.list = list;
                }
                if let Some(selected) = &state.selected
                    && !state.list.contains(selected)
                {
                    state.selected = None;
                }
                Effect::none()
            }
            SessionsAction::Failed(error) => {
                state.loading = false;
                state.error = Some(error);
                Effect::none()
            }
        }
    }
}
