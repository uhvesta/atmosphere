//! The host role: runs sessions on this machine, on whichever backends it has.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use khodr_agent::{
    Agent, AgentError, ApprovalMode, CopilotOptions, CopilotProvider, CopilotRunner, Event,
    EventId, ForkOptions, LocalAgent, LocalModelConfig, LocalModelRunner, Script, ScriptedRunner,
    SendMode, SessionId, SessionRecord, StartOptions, Watch,
};
use khodr_wire::{Reply, Service, ServiceFuture, describe, error, pb};
use tokio::sync::{broadcast, mpsc};

use crate::convert;
use crate::error::HostError;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, HostError>> + Send + 'a>>;

/// [`Agent`] with its futures boxed, so the host can hold different backends.
trait Backend: Send + Sync {
    fn start(&self, options: StartOptions) -> BoxFuture<'_, SessionRecord>;
    fn send<'a>(
        &'a self,
        session: &'a SessionId,
        text: String,
        mode: SendMode,
    ) -> BoxFuture<'a, ()>;
    fn approve<'a>(
        &'a self,
        session: &'a SessionId,
        request: &'a str,
        approved: bool,
    ) -> BoxFuture<'a, ()>;
    fn set_approval_mode<'a>(
        &'a self,
        session: &'a SessionId,
        mode: ApprovalMode,
    ) -> BoxFuture<'a, ()>;
    fn abort<'a>(&'a self, session: &'a SessionId) -> BoxFuture<'a, ()>;
    fn fork<'a>(
        &'a self,
        session: &'a SessionId,
        options: ForkOptions,
    ) -> BoxFuture<'a, SessionRecord>;
    fn history<'a>(&'a self, session: &'a SessionId) -> BoxFuture<'a, Vec<Event>>;
    fn watch<'a>(&'a self, session: &'a SessionId) -> BoxFuture<'a, Watch>;
    fn sessions(&self) -> BoxFuture<'_, Vec<SessionRecord>>;
}

impl<A> Backend for A
where
    A: Agent,
    A::Error: Into<HostError>,
{
    fn start(&self, options: StartOptions) -> BoxFuture<'_, SessionRecord> {
        Box::pin(async move { Agent::start(self, options).await.map_err(Into::into) })
    }
    fn send<'a>(
        &'a self,
        session: &'a SessionId,
        text: String,
        mode: SendMode,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            Agent::send(self, session, text, mode)
                .await
                .map_err(Into::into)
        })
    }
    fn approve<'a>(
        &'a self,
        session: &'a SessionId,
        request: &'a str,
        approved: bool,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            Agent::approve(self, session, request, approved)
                .await
                .map_err(Into::into)
        })
    }
    fn set_approval_mode<'a>(
        &'a self,
        session: &'a SessionId,
        mode: ApprovalMode,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            Agent::set_approval_mode(self, session, mode)
                .await
                .map_err(Into::into)
        })
    }
    fn abort<'a>(&'a self, session: &'a SessionId) -> BoxFuture<'a, ()> {
        Box::pin(async move { Agent::abort(self, session).await.map_err(Into::into) })
    }
    fn fork<'a>(
        &'a self,
        session: &'a SessionId,
        options: ForkOptions,
    ) -> BoxFuture<'a, SessionRecord> {
        Box::pin(async move {
            Agent::fork(self, session, options)
                .await
                .map_err(Into::into)
        })
    }
    fn history<'a>(&'a self, session: &'a SessionId) -> BoxFuture<'a, Vec<Event>> {
        Box::pin(async move { Agent::history(self, session).await.map_err(Into::into) })
    }
    fn watch<'a>(&'a self, session: &'a SessionId) -> BoxFuture<'a, Watch> {
        Box::pin(async move { Agent::watch(self, session).await.map_err(Into::into) })
    }
    fn sessions(&self) -> BoxFuture<'_, Vec<SessionRecord>> {
        Box::pin(async move { Agent::sessions(self).await.map_err(Into::into) })
    }
}

/// Runs sessions here and answers requests about them.
pub struct Host {
    backends: HashMap<pb::Backend, Arc<dyn Backend>>,
    /// Which backend runs each session.
    owners: Mutex<HashMap<SessionId, pb::Backend>>,
}

impl Host {
    /// Opens the sessions kept under `state`, one folder per backend.
    pub async fn open(state: &Path) -> Result<Self, HostError> {
        let scripted = LocalAgent::open(
            state.join("scripted"),
            ScriptedRunner::new(Script::default()),
        )?;
        // A session's own backend config can name another server.
        let local_model = LocalModelRunner::new(LocalModelConfig {
            base_url: std::env::var("KHODR_LOCAL_URL").ok(),
            api_key: std::env::var("KHODR_LOCAL_API_KEY").ok(),
        });
        let local = LocalAgent::open(state.join("local"), local_model)?;
        // Signed in with `copilot login`, it uses GitHub's models; given a
        // provider, by the session or here, a model server of its own.
        let copilot_state = state.join("copilot");
        let copilot_runner = CopilotRunner::new(CopilotOptions {
            program: std::env::var_os("KHODR_COPILOT")
                .map_or_else(|| PathBuf::from("copilot"), PathBuf::from),
            state: copilot_state.join("backend"),
            env: Vec::new(),
            provider: std::env::var("KHODR_COPILOT_PROVIDER_URL")
                .ok()
                .map(|base_url| CopilotProvider {
                    kind: "openai".to_owned(),
                    base_url,
                    api_key: std::env::var("KHODR_COPILOT_PROVIDER_API_KEY").ok(),
                }),
        });
        let copilot = LocalAgent::open(copilot_state.join("sessions"), copilot_runner)?;
        let mut backends: HashMap<pb::Backend, Arc<dyn Backend>> = HashMap::new();
        backends.insert(pb::Backend::Scripted, Arc::new(scripted));
        backends.insert(pb::Backend::Local, Arc::new(local));
        backends.insert(pb::Backend::Copilot, Arc::new(copilot));
        let mut owners = HashMap::new();
        for (backend, agent) in &backends {
            for record in agent.sessions().await? {
                owners.insert(record.id, *backend);
            }
        }
        Ok(Self {
            backends,
            owners: Mutex::new(owners),
        })
    }

    fn backend(&self, backend: pb::Backend) -> Result<&Arc<dyn Backend>, HostError> {
        self.backends
            .get(&backend)
            .ok_or(HostError::UnknownBackend {
                backend: backend.as_str_name(),
            })
    }

    fn owner(&self, session: &SessionId) -> Result<(pb::Backend, &Arc<dyn Backend>), HostError> {
        let backend = self
            .owners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session)
            .copied()
            .ok_or_else(|| AgentError::UnknownSession {
                session: session.clone(),
            })?;
        Ok((backend, self.backend(backend)?))
    }

    async fn info(
        &self,
        backend: pb::Backend,
        agent: &dyn Backend,
        record: &SessionRecord,
    ) -> Result<pb::SessionInfo, HostError> {
        let history = agent.history(&record.id).await?;
        Ok(convert::session(record, backend, &history))
    }

    async fn answer(&self, request: pb::request::Kind) -> Result<Reply, HostError> {
        use pb::request::Kind as Request;
        use pb::response::Kind as Response;
        Ok(Reply::Response(match request {
            Request::StartSession(start) => {
                let backend = start.backend();
                let approval = convert::approval_mode(start.approval_mode());
                if start.working_directory.is_empty() {
                    return Err(HostError::NoWorkingDirectory);
                }
                let agent = self.backend(backend)?;
                let record = agent
                    .start(StartOptions {
                        name: start.name,
                        working_directory: PathBuf::from(start.working_directory),
                        model: convert::optional(start.model),
                        instructions: convert::optional(start.instructions),
                        backend_config: convert::optional(start.backend_config),
                        approval,
                    })
                    .await?;
                self.owners
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(record.id.clone(), backend);
                Response::SessionStarted(pb::SessionStarted {
                    session: Some(self.info(backend, agent.as_ref(), &record).await?),
                })
            }
            Request::SendMessage(message) => {
                let mode = convert::send_mode(message.mode());
                let session = SessionId::new(message.session_id);
                self.owner(&session)?
                    .1
                    .send(&session, message.text, mode)
                    .await?;
                Response::MessageSent(pb::MessageSent {})
            }
            Request::Approve(approve) => {
                let session = SessionId::new(approve.session_id);
                self.owner(&session)?
                    .1
                    .approve(&session, &approve.permission_request_id, approve.approved)
                    .await?;
                Response::Approved(pb::Approved {})
            }
            Request::SetApprovalMode(set) => {
                let mode = convert::approval_mode(set.mode());
                let session = SessionId::new(set.session_id);
                self.owner(&session)?
                    .1
                    .set_approval_mode(&session, mode)
                    .await?;
                Response::ApprovalModeSet(pb::ApprovalModeSet {})
            }
            Request::CreateProject(_) | Request::ListProjects(_) => {
                return Err(HostError::NotForHosts);
            }
            Request::AbortTurn(abort) => {
                let session = SessionId::new(abort.session_id);
                self.owner(&session)?.1.abort(&session).await?;
                Response::TurnAborted(pb::TurnAborted {})
            }
            Request::ForkSession(fork) => {
                let session = SessionId::new(fork.session_id);
                let (backend, agent) = self.owner(&session)?;
                let options = ForkOptions {
                    before: convert::optional(fork.before_event_id).map(EventId::new),
                    name: convert::optional(fork.name),
                    working_directory: None,
                };
                let record = agent.fork(&session, options).await?;
                self.owners
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(record.id.clone(), backend);
                Response::SessionForked(pb::SessionForked {
                    session: Some(self.info(backend, agent.as_ref(), &record).await?),
                })
            }
            Request::ReadHistory(read) => {
                let session = SessionId::new(read.session_id);
                let events = self.owner(&session)?.1.history(&session).await?;
                Response::History(pb::History {
                    events: events.iter().map(convert::event).collect(),
                })
            }
            Request::ListSessions(_) => {
                let mut sessions = Vec::new();
                for (backend, agent) in &self.backends {
                    for record in agent.sessions().await? {
                        sessions.push(self.info(*backend, agent.as_ref(), &record).await?);
                    }
                }
                sessions.sort_by(|a, b| a.id.cmp(&b.id));
                Response::Sessions(pb::Sessions { sessions })
            }
            Request::WatchSession(watch) => {
                let session = SessionId::new(watch.session_id);
                let watch = self.owner(&session)?.1.watch(&session).await?;
                return Ok(stream(watch));
            }
        }))
    }
}

/// The history, then each new event, until the watcher goes away.
fn stream(watch: Watch) -> Reply {
    let (items, receiver) = mpsc::channel(64);
    let Watch { history, mut live } = watch;
    tokio::spawn(async move {
        let item = |event: &Event| pb::StreamItem {
            kind: Some(pb::stream_item::Kind::SessionEvent(convert::event(event))),
        };
        for event in &history {
            if items.send(Ok(item(event))).await.is_err() {
                return;
            }
        }
        loop {
            let next = tokio::select! {
                next = live.recv() => next,
                () = items.closed() => return,
            };
            let sent = match next {
                Ok(event) => items.send(Ok(item(&event))).await,
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    let error = HostError::FellBehind { missed };
                    let _ = items
                        .send(Err(pb::Error {
                            code: error.code().into(),
                            message: describe(&error),
                        }))
                        .await;
                    return;
                }
                Err(broadcast::error::RecvError::Closed) => return,
            };
            if sent.is_err() {
                return;
            }
        }
    });
    Reply::Stream {
        response: pb::response::Kind::Watching(pb::Watching {}),
        items: receiver,
    }
}

impl Service for Host {
    fn handle(self: Arc<Self>, request: pb::request::Kind) -> ServiceFuture {
        Box::pin(async move {
            match self.answer(request).await {
                Ok(reply) => reply,
                Err(failure) => Reply::Response(error(failure.code(), describe(&failure))),
            }
        })
    }
}
