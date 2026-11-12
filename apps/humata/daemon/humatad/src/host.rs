//! The host role: runs sessions on this machine, on whichever backends it has.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use humata_agent::{
    Agent, AgentError, ApprovalMode, ClaudeOptions, ClaudeRunner, CopilotOptions, CopilotProvider,
    CopilotRunner, Edit, Env, Event, EventId, ForkOptions, LocalAgent, LocalModelConfig,
    LocalModelRunner, McpServer, Query, Script, ScriptedRunner, SendMode, SessionId, SessionRecord,
    SessionUpdate, StartOptions, Watch, Workspaces,
};
use humata_wire::{Reply, Service, ServiceFuture, describe, error, pb};
use tokio::sync::{broadcast, mpsc};

use crate::convert;
use crate::error::HostError;
use crate::machine::Machine;

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
    fn update<'a>(
        &'a self,
        session: &'a SessionId,
        update: SessionUpdate,
    ) -> BoxFuture<'a, SessionRecord>;
    fn changes<'a>(
        &'a self,
        session: &'a SessionId,
        query: &'a Query,
    ) -> BoxFuture<'a, Vec<humata_agent::Changes>>;
    fn edit_changes<'a>(&'a self, session: &'a SessionId, edit: &'a Edit) -> BoxFuture<'a, ()>;
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
    fn update<'a>(
        &'a self,
        session: &'a SessionId,
        update: SessionUpdate,
    ) -> BoxFuture<'a, SessionRecord> {
        Box::pin(async move {
            Agent::update(self, session, update)
                .await
                .map_err(Into::into)
        })
    }
    fn changes<'a>(
        &'a self,
        session: &'a SessionId,
        query: &'a Query,
    ) -> BoxFuture<'a, Vec<humata_agent::Changes>> {
        Box::pin(async move {
            Agent::changes(self, session, query)
                .await
                .map_err(Into::into)
        })
    }
    fn edit_changes<'a>(&'a self, session: &'a SessionId, edit: &'a Edit) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            Agent::edit_changes(self, session, edit)
                .await
                .map_err(Into::into)
        })
    }
}

/// Runs sessions here and answers requests about them.
pub struct Host {
    backends: HashMap<pb::Backend, Arc<dyn Backend>>,
    /// Which backend runs each session.
    owners: Mutex<HashMap<SessionId, pb::Backend>>,
    machine: Machine,
    /// Where chats that work in no folder of the person's get one.
    chats: PathBuf,
    /// What this humatad is, read when it started, since an install may
    /// replace the executable while it runs.
    program_version: String,
}

impl Host {
    /// Opens the sessions kept under `state`, one folder per backend. Their
    /// copies of the code they work in go under `worktrees`. Agents that
    /// take MCP servers are given `mcp`.
    pub async fn open(
        state: &Path,
        worktrees: &Path,
        mcp: Option<McpServer>,
    ) -> Result<Self, HostError> {
        let workspaces = || Workspaces::new(worktrees);
        // Empty until the daemon sends the host's variables after connecting.
        let env = Env::default();
        let copilot_program = program("HUMATA_COPILOT", "copilot");
        let claude_program = program("HUMATA_CLAUDE", "claude");
        let scripted = LocalAgent::open(
            state.join("scripted"),
            ScriptedRunner::new(Script::default()),
        )?
        .with_workspaces(workspaces());
        // A session's own backend config can name another server.
        let local_model = LocalModelRunner::new(LocalModelConfig {
            base_url: std::env::var("HUMATA_LOCAL_URL").ok(),
            api_key: std::env::var("HUMATA_LOCAL_API_KEY").ok(),
        })
        .with_env(env.clone());
        let local =
            LocalAgent::open(state.join("local"), local_model)?.with_workspaces(workspaces());
        // Signed in with `copilot login`, it uses GitHub's models; given a
        // provider, by the session or here, a model server of its own.
        let copilot_state = state.join("copilot");
        let copilot_runner = CopilotRunner::new(CopilotOptions {
            program: copilot_program.clone(),
            state: copilot_state.join("backend"),
            env: env.clone(),
            provider: std::env::var("HUMATA_COPILOT_PROVIDER_URL")
                .ok()
                .map(|base_url| CopilotProvider {
                    kind: "openai".to_owned(),
                    base_url,
                    api_key: std::env::var("HUMATA_COPILOT_PROVIDER_API_KEY").ok(),
                }),
        });
        let copilot = LocalAgent::open(copilot_state.join("sessions"), copilot_runner)?
            .with_workspaces(workspaces());
        // Signed in with `claude` and `/login`, it runs on the person's
        // Claude subscription.
        let claude_state = state.join("claude");
        let claude_runner = ClaudeRunner::new(ClaudeOptions {
            program: claude_program.clone(),
            state: claude_state.join("backend"),
            env: env.clone(),
            mcp,
        });
        let claude = LocalAgent::open(claude_state.join("sessions"), claude_runner)?
            .with_workspaces(workspaces());
        let mut backends: HashMap<pb::Backend, Arc<dyn Backend>> = HashMap::new();
        backends.insert(pb::Backend::Scripted, Arc::new(scripted));
        backends.insert(pb::Backend::Local, Arc::new(local));
        backends.insert(pb::Backend::Copilot, Arc::new(copilot));
        backends.insert(pb::Backend::Claude, Arc::new(claude));
        let mut owners = HashMap::new();
        for (backend, agent) in &backends {
            for record in agent.sessions().await? {
                owners.insert(record.id, *backend);
            }
        }
        Ok(Self {
            backends,
            owners: Mutex::new(owners),
            machine: Machine::new(state.to_owned(), env, copilot_program, claude_program),
            chats: state.join("chats"),
            program_version: crate::build(),
        })
    }

    /// A new empty folder for a chat that works in none of the person's.
    fn chat_folder(&self) -> Result<PathBuf, HostError> {
        let path = self.chats.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&path).map_err(|source| HostError::CreateChatFolder {
            path: path.clone(),
            source,
        })?;
        Ok(path)
    }

    /// The folder `session` works in.
    async fn folder(&self, session: &SessionId) -> Result<PathBuf, HostError> {
        let agent = self.owner(session)?.1;
        agent
            .sessions()
            .await?
            .into_iter()
            .find(|record| &record.id == session)
            .map(|record| record.working_directory)
            .ok_or_else(|| {
                HostError::Agent(AgentError::UnknownSession {
                    session: session.clone(),
                })
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
                let mut start = start;
                if start.working_directory.is_empty() && start.repositories.is_empty() {
                    start.working_directory = self.chat_folder()?.display().to_string();
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
                        repositories: start
                            .repositories
                            .into_iter()
                            .map(convert::repository)
                            .collect(),
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
            Request::CreateProject(_)
            | Request::ListProjects(_)
            | Request::CreateItem(_)
            | Request::UpdateItem(_)
            | Request::ListItems(_)
            | Request::ReadBoardLog(_)
            | Request::RevertBoardChange(_)
            | Request::ListInbox(_)
            | Request::ListHosts(_)
            | Request::AddHost(_)
            | Request::UpdateHost(_)
            | Request::RemoveHost(_)
            | Request::CheckHost(_)
            | Request::InstallHost(_)
            | Request::ListSshHosts(_)
            | Request::MarkViewed(_)
            | Request::MarkSeen(_)
            | Request::CreateThread(_)
            | Request::ReplyToThread(_)
            | Request::ResolveThread(_)
            | Request::DeleteComment(_)
            | Request::ListThreads(_)
            | Request::SendReview(_) => {
                return Err(HostError::NotForHosts);
            }
            Request::ConfigureHost(configure) => Response::HostConfigured(pb::HostConfigured {
                setup: Some(self.machine.configure(configure).await?),
            }),
            Request::ReadHostStatus(_) => {
                Response::HostStatus(self.machine.status(self.program_version.clone()).await)
            }
            Request::CloneRepositories(clone) => {
                Response::RepositoriesCloned(pb::RepositoriesCloned {
                    repositories: self.machine.clone(clone).await?,
                })
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
                    chat_only: fork.chat_only,
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
            Request::UpdateSession(update) => {
                let session = SessionId::new(update.session_id);
                let (backend, agent) = self.owner(&session)?;
                let changes = SessionUpdate {
                    name: update.name,
                    archived: update.archived,
                    keeper: update.keeper,
                };
                let record = agent.update(&session, changes).await?;
                Response::SessionUpdated(pb::SessionUpdated {
                    session: Some(self.info(backend, agent.as_ref(), &record).await?),
                })
            }
            Request::ReadChanges(read) => {
                let session = SessionId::new(read.session_id.clone());
                let query = convert::query(&read)?;
                let changes = self.owner(&session)?.1.changes(&session, &query).await?;
                Response::Changes(pb::Changes {
                    repositories: changes.iter().map(convert::changes).collect(),
                })
            }
            Request::EditChanges(edit) => {
                let session = SessionId::new(edit.session_id.clone());
                let edit = convert::edit(&edit)?;
                self.owner(&session)?
                    .1
                    .edit_changes(&session, &edit)
                    .await?;
                Response::ChangesEdited(pb::ChangesEdited {})
            }
            Request::ListFiles(list) => {
                let folder = self.folder(&SessionId::new(list.session_id)).await?;
                Response::Files(crate::files::list(&folder)?)
            }
            Request::ReadFile(read) => {
                let folder = self.folder(&SessionId::new(read.session_id)).await?;
                Response::FileContents(crate::files::read(&folder, &read.path)?)
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

/// An agent's program: the one `variable` names, else the first `name` on
/// PATH, else one in the folders installers use, which a shell's PATH may
/// have but humatad's, started by launchd or over SSH, lacks.
fn program(variable: &str, name: &str) -> PathBuf {
    if let Some(program) = std::env::var_os(variable) {
        return PathBuf::from(program);
    }
    let path = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let installed = home
        .iter()
        .flat_map(|home| [home.join(".local/bin"), home.join(".npm-global/bin")])
        .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    path.into_iter()
        .chain(installed)
        .map(|dir| dir.join(name))
        .find(|program| program.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}
