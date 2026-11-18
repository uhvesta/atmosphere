//! A scripted [`ViewsClient`]: it keeps the views an agent offered and
//! tells every watcher about each change, as the daemon's event feed does.
//! Clones share them, so two clones stand for two apps open at once. No
//! timers, no real IO.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::mpsc;

use crate::daemon::{DaemonError, Reply};
use crate::model::{PreparedView, SessionId, ViewId, ViewState};
use crate::views::{ViewContent, ViewEvents, ViewRecord, ViewsClient};
use crate::workspace::Workspace;

/// Records a watcher can fall behind by before the fake drops it.
const WATCH_BUFFER: usize = 64;

/// Who the fake says prepared a view when nobody is named.
const PREPARED_BY: &str = "Claude Code";

#[derive(Default)]
struct Inner {
    views: Vec<ViewRecord>,
    watchers: Vec<mpsc::Sender<ViewRecord>>,
    /// What each idempotency key answered the first time it was used.
    answered: HashMap<String, ViewRecord>,
    next: u32,
}

impl Inner {
    fn tell(&mut self, record: &ViewRecord) {
        self.watchers.retain(|watcher| watcher.try_send(record.clone()).is_ok());
    }
}

/// Views that start empty. A view the agent offers is waiting for the
/// person; every change raises its revision by one and goes to every
/// watcher before the call that made it answers. Using an idempotency key a
/// second time answers what the first use did and changes nothing.
#[derive(Clone, Default)]
pub struct FakeViews {
    shared: Arc<Mutex<Inner>>,
}

impl FakeViews {
    /// Views with nothing offered.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Views as the workspace has them: each one a record, in order,
    /// waiting when the person has not accepted it, else kept. Nobody is
    /// told; nobody watches yet.
    #[must_use]
    pub fn seeded(workspace: &Workspace) -> Self {
        let views = Self::new();
        {
            let mut inner = views.inner();
            for view in &workspace.views {
                inner.next += 1;
                let id = ViewId::new(format!("vw_{}", inner.next));
                inner.views.push(ViewRecord {
                    id,
                    view: view.clone(),
                    state: if view.accepted { ViewState::Kept } else { ViewState::Waiting },
                    revision: 1,
                    prepared_by: PREPARED_BY.to_owned(),
                    content: ViewContent::Summary,
                });
            }
        }
        views
    }

    /// The agent of `session` offers a view: it waits for the person, and
    /// every watcher is told. Answers its id.
    pub fn offer(&self, session: &SessionId, title: &str, body: &str) -> ViewId {
        self.offer_view(session, title, body, PREPARED_BY, ViewContent::Summary)
    }

    /// The agent `prepared_by` of `session` offers a view with `content`:
    /// it waits for the person, and every watcher is told. Answers its id.
    pub fn offer_view(
        &self,
        session: &SessionId,
        title: &str,
        summary: &str,
        prepared_by: &str,
        content: ViewContent,
    ) -> ViewId {
        let mut inner = self.inner();
        inner.next += 1;
        let id = ViewId::new(format!("vw_{}", inner.next));
        let record = ViewRecord {
            id: id.clone(),
            view: PreparedView {
                title: title.to_owned(),
                session: session.clone(),
                body: summary.to_owned(),
                accepted: false,
                note: summary.to_owned(),
                prepared: "now".to_owned(),
            },
            state: ViewState::Waiting,
            revision: 1,
            prepared_by: prepared_by.to_owned(),
            content,
        };
        inner.views.push(record.clone());
        inner.tell(&record);
        id
    }

    /// The content the daemon reads for `id` from now on, as when it checks
    /// a review plan's notes against the files at each list: the revision
    /// stays and nobody is told. Does nothing for a view it doesn't hold.
    pub fn read_as(&self, id: &ViewId, content: ViewContent) {
        if let Some(record) = self.inner().views.iter_mut().find(|record| &record.id == id) {
            record.content = content;
        }
    }

    /// Answers a view with the caller's own idempotency `key`, as a retry
    /// of one user action would.
    pub fn set_state_with_key(&self, id: &ViewId, key: &str, state: ViewState) -> Result<ViewRecord, DaemonError> {
        let mut inner = self.inner();
        if let Some(first) = inner.answered.get(key) {
            return Ok(first.clone());
        }
        let found = inner
            .views
            .iter_mut()
            .find(|record| &record.id == id)
            .ok_or_else(|| DaemonError::NotFound { what: format!("View {id}") })?;
        let moved = found.state != state;
        if moved {
            found.state = state;
            found.revision += 1;
            found.view.accepted = matches!(
                state,
                ViewState::Opened | ViewState::OpenedAndClosed | ViewState::Kept
            );
        }
        let changed = found.clone();
        inner.answered.insert(key.to_owned(), changed.clone());
        if moved {
            inner.tell(&changed);
        }
        Ok(changed)
    }
}

impl ViewsClient for FakeViews {
    fn list(&self, session: &SessionId) -> Reply<Vec<ViewRecord>> {
        let views = self
            .inner()
            .views
            .iter()
            .filter(|record| &record.view.session == session)
            .cloned()
            .collect();
        Box::pin(std::future::ready(Ok(views)))
    }

    fn list_offered(&self) -> Reply<Vec<ViewRecord>> {
        let views = self
            .inner()
            .views
            .iter()
            .rev()
            .filter(|record| record.state != ViewState::Draft)
            .cloned()
            .collect();
        Box::pin(std::future::ready(Ok(views)))
    }

    fn set_state(&self, id: &ViewId, state: ViewState) -> Reply<ViewRecord> {
        let key = {
            let mut inner = self.inner();
            inner.next += 1;
            format!("fake-key-{}", inner.next)
        };
        Box::pin(std::future::ready(self.set_state_with_key(id, &key, state)))
    }

    fn watch(&self) -> Reply<ViewEvents> {
        let (send, receive) = mpsc::channel(WATCH_BUFFER);
        self.inner().watchers.push(send);
        Box::pin(std::future::ready(Ok(receive)))
    }
}
