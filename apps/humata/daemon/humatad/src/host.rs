//! The host role: runs sessions on this machine, on whichever backends it has.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use humata_agent::{
    Agent, AgentError, ApprovalMode, BackendFile, Backup, BackupSince, Checkout, ClaudeOptions,
    ClaudeRunner, CopilotOptions, CopilotProvider, CopilotRunner, Edit, Env, Event, EventId,
    ForkOptions, Held, Image, Importable, Landed, LocalAgent, LocalModelConfig, LocalModelRunner,
    McpServer, Merged, Message, ModelServer, Query, Repository, Script, ScriptedRunner, SendMode,
    SessionId, SessionRecord, SessionUpdate, StartOptions, Summary, Watch, Workspaces, bundle,
};
use humata_wire::{Reply, Service, ServiceFuture, describe, error, pb};
use tokio::sync::{broadcast, mpsc};

use crate::convert;
use crate::error::HostError;
use crate::machine::Machine;
use crate::ports::Ports;
use crate::relay::Relay;
use crate::terminals::{Attached, Chunk, Opening, Terminals};

/// How long a session here is idle before its history is sealed, as the
/// Mac's daily cleanup does with its own.
const SEAL_IDLE: std::time::Duration = std::time::Duration::from_secs(3 * 24 * 60 * 60);

/// How often a host puts its old histories to rest.
const REST_EVERY: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, HostError>> + Send + 'a>>;

/// [`Agent`] with its futures boxed, so the host can hold different backends.
trait Backend: Send + Sync {
    fn start(&self, options: StartOptions) -> BoxFuture<'_, SessionRecord>;
    fn send<'a>(
        &'a self,
        session: &'a SessionId,
        message: Message,
        mode: SendMode,
    ) -> BoxFuture<'a, ()>;
    fn read_image<'a>(&'a self, session: &'a SessionId, name: &'a str) -> BoxFuture<'a, Image>;
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
    fn drop_queue<'a>(&'a self, session: &'a SessionId) -> BoxFuture<'a, ()>;
    fn rewind<'a>(
        &'a self,
        session: &'a SessionId,
        before: &'a EventId,
        files: bool,
    ) -> BoxFuture<'a, ()>;
    fn undo_rewind<'a>(&'a self, session: &'a SessionId) -> BoxFuture<'a, ()>;
    fn unqueue<'a>(&'a self, session: &'a SessionId, queue_id: &'a str) -> BoxFuture<'a, ()>;
    fn edit_queued<'a>(
        &'a self,
        session: &'a SessionId,
        queue_id: &'a str,
        text: String,
    ) -> BoxFuture<'a, ()>;
    fn fork<'a>(
        &'a self,
        session: &'a SessionId,
        options: ForkOptions,
    ) -> BoxFuture<'a, SessionRecord>;
    fn history<'a>(&'a self, session: &'a SessionId) -> BoxFuture<'a, Vec<Event>>;
    fn summary<'a>(&'a self, session: &'a SessionId) -> BoxFuture<'a, Summary>;
    fn watch_reporting<'a>(
        &'a self,
        session: &'a SessionId,
        progress: humata_agent::Progress,
    ) -> BoxFuture<'a, Watch>;
    fn rest(
        &self,
        seal: HashSet<SessionId>,
        progress: Box<dyn FnMut(usize, usize) + Send>,
    ) -> BoxFuture<'_, usize>;
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
    fn file_versions<'a>(
        &'a self,
        session: &'a SessionId,
        path: &'a str,
    ) -> BoxFuture<'a, Vec<humata_agent::FileVersion>>;
    fn file_at<'a>(
        &'a self,
        session: &'a SessionId,
        path: &'a str,
        turn: usize,
    ) -> BoxFuture<'a, String>;
    fn land<'a>(
        &'a self,
        session: &'a SessionId,
        branch: &'a str,
        message: &'a str,
    ) -> BoxFuture<'a, Vec<Landed>>;
    fn update_from_base<'a>(
        &'a self,
        session: &'a SessionId,
    ) -> BoxFuture<'a, Vec<(Checkout, String, Merged)>>;
    fn importable(&self) -> BoxFuture<'_, Vec<Importable>>;
    fn skipped(&self) -> BoxFuture<'_, Vec<humata_agent::Skipped>>;
    fn remove_skipped<'a>(&'a self, id: &'a str) -> BoxFuture<'a, ()>;
    fn models(&self, backend_config: Option<String>) -> BoxFuture<'_, Vec<humata_agent::Model>>;
    fn import<'a>(
        &'a self,
        external: &'a str,
        name: Option<String>,
    ) -> BoxFuture<'a, SessionRecord>;
    fn backup_since<'a>(&'a self, session: &'a SessionId, held: Held)
    -> BoxFuture<'a, BackupSince>;
    fn delete<'a>(&'a self, session: &'a SessionId) -> BoxFuture<'a, ()>;
    fn restore(
        &self,
        backup: Backup,
        repositories: Vec<Repository>,
    ) -> BoxFuture<'_, SessionRecord>;
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
        message: Message,
        mode: SendMode,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            Agent::send(self, session, message, mode)
                .await
                .map_err(Into::into)
        })
    }
    fn read_image<'a>(&'a self, session: &'a SessionId, name: &'a str) -> BoxFuture<'a, Image> {
        Box::pin(async move {
            Agent::read_image(self, session, name)
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
    fn drop_queue<'a>(&'a self, session: &'a SessionId) -> BoxFuture<'a, ()> {
        Box::pin(async move { Agent::drop_queue(self, session).await.map_err(Into::into) })
    }
    fn rewind<'a>(
        &'a self,
        session: &'a SessionId,
        before: &'a EventId,
        files: bool,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            Agent::rewind(self, session, before, files)
                .await
                .map_err(Into::into)
        })
    }
    fn undo_rewind<'a>(&'a self, session: &'a SessionId) -> BoxFuture<'a, ()> {
        Box::pin(async move { Agent::undo_rewind(self, session).await.map_err(Into::into) })
    }
    fn unqueue<'a>(&'a self, session: &'a SessionId, queue_id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            Agent::unqueue(self, session, queue_id)
                .await
                .map_err(Into::into)
        })
    }
    fn edit_queued<'a>(
        &'a self,
        session: &'a SessionId,
        queue_id: &'a str,
        text: String,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            Agent::edit_queued(self, session, queue_id, text)
                .await
                .map_err(Into::into)
        })
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
    fn summary<'a>(&'a self, session: &'a SessionId) -> BoxFuture<'a, Summary> {
        Box::pin(async move { Agent::summary(self, session).await.map_err(Into::into) })
    }
    fn watch_reporting<'a>(
        &'a self,
        session: &'a SessionId,
        progress: humata_agent::Progress,
    ) -> BoxFuture<'a, Watch> {
        Box::pin(async move {
            Agent::watch_reporting(self, session, progress)
                .await
                .map_err(Into::into)
        })
    }
    fn rest(
        &self,
        seal: HashSet<SessionId>,
        progress: Box<dyn FnMut(usize, usize) + Send>,
    ) -> BoxFuture<'_, usize> {
        Box::pin(async move { Agent::rest(self, seal, progress).await.map_err(Into::into) })
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
    fn file_versions<'a>(
        &'a self,
        session: &'a SessionId,
        path: &'a str,
    ) -> BoxFuture<'a, Vec<humata_agent::FileVersion>> {
        Box::pin(async move {
            Agent::file_versions(self, session, path)
                .await
                .map_err(Into::into)
        })
    }
    fn file_at<'a>(
        &'a self,
        session: &'a SessionId,
        path: &'a str,
        turn: usize,
    ) -> BoxFuture<'a, String> {
        Box::pin(async move {
            Agent::file_at(self, session, path, turn)
                .await
                .map_err(Into::into)
        })
    }
    fn land<'a>(
        &'a self,
        session: &'a SessionId,
        branch: &'a str,
        message: &'a str,
    ) -> BoxFuture<'a, Vec<Landed>> {
        Box::pin(async move {
            Agent::land(self, session, branch, message)
                .await
                .map_err(Into::into)
        })
    }
    fn update_from_base<'a>(
        &'a self,
        session: &'a SessionId,
    ) -> BoxFuture<'a, Vec<(Checkout, String, Merged)>> {
        Box::pin(async move {
            Agent::update_from_base(self, session)
                .await
                .map_err(Into::into)
        })
    }
    fn skipped(&self) -> BoxFuture<'_, Vec<humata_agent::Skipped>> {
        Box::pin(async move { Agent::skipped(self).await.map_err(Into::into) })
    }
    fn remove_skipped<'a>(&'a self, id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move { Agent::remove_skipped(self, id).await.map_err(Into::into) })
    }
    fn importable(&self) -> BoxFuture<'_, Vec<Importable>> {
        Box::pin(async move { Agent::importable(self).await.map_err(Into::into) })
    }
    fn models(&self, backend_config: Option<String>) -> BoxFuture<'_, Vec<humata_agent::Model>> {
        Box::pin(async move {
            Agent::models(self, backend_config)
                .await
                .map_err(Into::into)
        })
    }
    fn import<'a>(
        &'a self,
        external: &'a str,
        name: Option<String>,
    ) -> BoxFuture<'a, SessionRecord> {
        Box::pin(async move {
            Agent::import(self, external, name)
                .await
                .map_err(Into::into)
        })
    }
    fn backup_since<'a>(
        &'a self,
        session: &'a SessionId,
        held: Held,
    ) -> BoxFuture<'a, BackupSince> {
        Box::pin(async move {
            Agent::backup_since(self, session, held)
                .await
                .map_err(Into::into)
        })
    }
    fn delete<'a>(&'a self, session: &'a SessionId) -> BoxFuture<'a, ()> {
        Box::pin(async move { Agent::delete(self, session).await.map_err(Into::into) })
    }
    fn restore(
        &self,
        backup: Backup,
        repositories: Vec<Repository>,
    ) -> BoxFuture<'_, SessionRecord> {
        Box::pin(async move {
            Agent::restore(self, backup, repositories)
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
    /// The client ids of each session's latest sends, newest last.
    sent: Mutex<HashMap<SessionId, std::collections::VecDeque<String>>>,
    machine: Machine,
    /// Where chats that work in no folder of the person's get one.
    chats: PathBuf,
    /// What this humatad is, read when it started, since an install may
    /// replace the executable while it runs.
    program_version: String,
    /// Where this host's agents' humata tool calls wait for the daemon.
    relay: Arc<Relay>,
    /// The socket a detached host serves on, removed when it is asked to
    /// stop.
    socket: std::sync::OnceLock<PathBuf>,
    stop: tokio::sync::Notify,
    /// The agents' variables, which its terminals get too.
    env: Env,
    terminals: Terminals,
    /// Connections to its ports, for the Mac's forwards.
    ports: Arc<Ports>,
    /// Where the app's model server is, which local-model sessions use
    /// unless told otherwise.
    model_server: ModelServer,
}

impl Host {
    /// Opens the sessions kept under `state`, one folder per backend. Their
    /// copies of the code they work in go under `worktrees`. Agents that
    /// take MCP servers are given `mcp`.
    /// Sets the variables for this host's agents, as the daemon does for
    /// the Mac it runs on.
    pub fn set_env(&self, env: std::collections::BTreeMap<String, String>) {
        self.machine.set_env(env);
    }

    /// Points this host's local-model sessions at the app's model server
    /// on `port`, kept for when it starts again. Failing to keep it isn't
    /// fatal: sessions use it until the host stops.
    pub fn set_model_server(&self, port: u16) {
        if let Err(error) = self.model_server.set_port(port) {
            eprintln!(
                "humatad: couldn't keep the model server's port {port} for after a restart ({error}). \
                 Sessions use it until humatad stops; check that its state folder is writable."
            );
        }
    }

    /// Where the app's model server is, as last set, shared with what
    /// reaches the hosts.
    pub fn model_server(&self) -> ModelServer {
        self.model_server.clone()
    }

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
        let model_server = ModelServer::open(state.join("model-server"));
        let local_model = LocalModelRunner::new(LocalModelConfig {
            base_url: std::env::var("HUMATA_LOCAL_URL").ok(),
            api_key: std::env::var("HUMATA_LOCAL_API_KEY").ok(),
        })
        .with_env(env.clone())
        .with_server(model_server.clone())
        .with_mcp(mcp.clone());
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
            mcp: mcp.clone(),
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
            sent: Mutex::default(),
            machine: Machine::new(
                state.to_owned(),
                env.clone(),
                copilot_program,
                claude_program,
            ),
            chats: state.join("chats"),
            program_version: crate::build(),
            relay: Arc::new(Relay::open(state.join("queued-tool-calls"))),
            socket: std::sync::OnceLock::new(),
            stop: tokio::sync::Notify::new(),
            env,
            terminals: Terminals::default(),
            ports: Arc::default(),
            model_server,
        })
    }

    /// Drops the histories of idle sessions no one watches from memory,
    /// sealing those of `seal` on disk; `report` is told the sessions done
    /// of the total. Says how many it sealed.
    pub async fn rest(
        &self,
        seal: &HashSet<String>,
        report: Arc<dyn Fn(usize, usize) + Send + Sync>,
    ) -> Result<usize, HostError> {
        let total = self
            .owners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        let mut start = 0;
        let mut sealed = 0;
        for agent in self.backends.values() {
            let seal = seal.iter().cloned().map(SessionId::new).collect();
            let counted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let (told, report) = (Arc::clone(&counted), Arc::clone(&report));
            let progress = Box::new(move |done: usize, of: usize| {
                told.store(of, std::sync::atomic::Ordering::Relaxed);
                report(start + done, total.max(start + of));
            });
            sealed += agent.rest(seal, progress).await?;
            start += counted.load(std::sync::atomic::Ordering::Relaxed);
        }
        Ok(sealed)
    }

    /// Rests this host's histories, sealing those of sessions archived or
    /// idle since `before` (in Unix milliseconds). Says how many it sealed.
    pub(crate) async fn rest_idle_since(&self, before: i64) -> Result<usize, HostError> {
        let mut seal = HashSet::new();
        for (backend, agent) in &self.backends {
            for record in agent.sessions().await? {
                let info = self.info(*backend, agent.as_ref(), &record).await?;
                if info.archived || info.last_active_unix_ms < before {
                    seal.insert(info.id);
                }
            }
        }
        self.rest(&seal, Arc::new(|_: usize, _: usize| {})).await
    }

    /// Once a day, rests this host's histories and seals the old ones, as
    /// the Mac's daemon does for the sessions on the Mac: a host serves
    /// sessions on its own, so it does this itself.
    pub(crate) fn rest_on_schedule(self: &Arc<Self>) {
        let me = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(REST_EVERY).await;
                let Some(host) = me.upgrade() else {
                    return;
                };
                let idle = i64::try_from(SEAL_IDLE.as_millis()).unwrap_or(i64::MAX);
                let before = crate::store::now_unix_ms().saturating_sub(idle);
                if let Err(error) = host.rest_idle_since(before).await {
                    eprintln!(
                        "humatad: couldn't seal this host's old session histories; they stay as they were, and it tries again in a day: {}",
                        describe(&error)
                    );
                }
            }
        });
    }

    /// Marks this host as serving on `socket`, which a [`pb::StopHost`]
    /// removes.
    pub fn serve_on(&self, socket: &Path) {
        let _ = self.socket.set(socket.to_owned());
    }

    /// Resolves once the daemon asks this host to stop.
    pub async fn stopped(&self) {
        self.stop.notified().await;
    }

    pub fn relay(&self) -> Arc<Relay> {
        self.relay.clone()
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
    /// The session's folder if one is given, else `folder`, else the home
    /// folder.
    async fn root(&self, session: &str, folder: &str) -> Result<PathBuf, HostError> {
        if !session.is_empty() {
            return self.folder(&SessionId::new(session.to_owned())).await;
        }
        if !folder.is_empty() {
            return Ok(PathBuf::from(folder));
        }
        Ok(std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from))
    }

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

    /// Whether a send named `client_id` is new to `session`, keeping it if
    /// so. An empty id is always new.
    fn take_send(&self, session: &SessionId, client_id: &str) -> bool {
        const KEPT: usize = 32;
        if client_id.is_empty() {
            return true;
        }
        let mut sent = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
        let ids = sent.entry(session.clone()).or_default();
        if ids.iter().any(|id| id == client_id) {
            return false;
        }
        if ids.len() == KEPT {
            ids.pop_front();
        }
        ids.push_back(client_id.to_owned());
        true
    }

    /// Forgets a send that failed, so it can be tried again.
    fn forget_send(&self, session: &SessionId, client_id: &str) {
        if let Some(ids) = self
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(session)
        {
            ids.retain(|id| id != client_id);
        }
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
        let summary = agent.summary(&record.id).await?;
        Ok(convert::session(record, backend, &summary))
    }

    /// The session's recovery copy, with its snapshots' commits in each
    /// repository that `haves` doesn't have.
    async fn back_up(&self, request: pb::BackupSession) -> Result<pb::SessionBackup, HostError> {
        let session = SessionId::new(request.session_id);
        let (backend, agent) = self.owner(&session)?;
        // Only the events after the ones the daemon has, when its last is
        // still where it was; otherwise, as after a rewind, all of them.
        let held = Held {
            events: usize::try_from(request.events_have).unwrap_or(usize::MAX),
            last_event_id: request.last_event_id,
        };
        let backup = agent.backup_since(&session, held).await?;
        let mut bundles = Vec::new();
        for repository in &backup.record.repositories {
            let haves = request
                .haves
                .iter()
                .find(|haves| haves.name == repository.name)
                .map(|haves| haves.commits.as_slice())
                .unwrap_or_default();
            let made = bundle::create(&repository.path, session.as_str(), haves).await;
            let made = made.map_err(|source| HostError::Snapshots {
                name: repository.name.clone(),
                source,
            })?;
            if let Some(bundle) = made {
                bundles.push(pb::RepositoryBundle {
                    name: repository.name.clone(),
                    bundle,
                });
            }
        }
        // Backend files the daemon has the same of are only named.
        let (mut backend_files, mut backend_unchanged) = (Vec::new(), Vec::new());
        for file in backup.backend {
            let sha256 = <sha2::Sha256 as sha2::Digest>::digest(&file.contents);
            let unchanged = request
                .backend_haves
                .iter()
                .any(|have| have.name == file.name && have.sha256 == sha256.as_slice());
            if unchanged {
                backend_unchanged.push(file.name);
            } else {
                backend_files.push(pb::BackendFile {
                    name: file.name,
                    contents: file.contents,
                });
            }
        }
        let encode = |source| HostError::EncodeBackup { source };
        Ok(pb::SessionBackup {
            backend: backend.into(),
            record: serde_json::to_vec(&backup.record).map_err(encode)?,
            events: backup.events,
            events_from: u32::try_from(backup.events_from).unwrap_or(u32::MAX),
            backend_files,
            backend_unchanged,
            bundles,
            turns: u32::try_from(backup.turns).unwrap_or(u32::MAX),
        })
    }

    /// Sets a session up again here from its recovery copy.
    async fn restore(&self, request: pb::RestoreBackup) -> Result<pb::SessionRestored, HostError> {
        let backup = request.backup.unwrap_or_default();
        let backend = backup.backend();
        let agent = self.backend(backend)?;
        let mut record: SessionRecord =
            serde_json::from_slice(&backup.record).map_err(|source| HostError::BadBackup {
                part: "session record",
                source,
            })?;
        // A chat in no folder of the person's had an empty one of its own
        // where it was; it gets one here, as when it started, or it couldn't
        // run.
        if record.repositories.is_empty()
            && !record.working_directory.starts_with(&self.chats)
            && record.working_directory.parent().and_then(Path::file_name)
                == Some(OsStr::new("chats"))
        {
            record.working_directory = self.chat_folder()?;
        }
        let events: Vec<Event> =
            serde_json::from_slice(&backup.events).map_err(|source| HostError::BadBackup {
                part: "history",
                source,
            })?;
        let repositories: Vec<Repository> = request
            .repositories
            .into_iter()
            .map(convert::repository)
            .collect();
        for bundle in &backup.bundles {
            let clone = repositories
                .iter()
                .find(|repository| repository.name == bundle.name)
                .ok_or_else(|| HostError::NoClone {
                    name: bundle.name.clone(),
                })?;
            bundle::apply(&clone.path, &bundle.bundle)
                .await
                .map_err(|source| HostError::Snapshots {
                    name: bundle.name.clone(),
                    source,
                })?;
        }
        let backup = Backup {
            record,
            events,
            backend: backup
                .backend_files
                .into_iter()
                .map(|file| BackendFile {
                    name: file.name,
                    contents: file.contents,
                })
                .collect(),
        };
        let record = agent.restore(backup, repositories).await?;
        self.owners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(record.id.clone(), backend);
        Ok(pb::SessionRestored {
            session: Some(self.info(backend, agent.as_ref(), &record).await?),
        })
    }

    async fn answer(&self, request: pb::request::Kind) -> Result<Reply, HostError> {
        use pb::request::Kind as Request;
        use pb::response::Kind as Response;
        Ok(Reply::Response(match request {
            Request::StopHost(pb::StopHost {}) => {
                // Gone before the answer, so the daemon's next connection
                // can't reach this host and starts the new one.
                if let Some(socket) = self.socket.get() {
                    let _ = std::fs::remove_file(socket);
                }
                // Kept until the server waits, if it isn't yet.
                self.stop.notify_one();
                Response::HostStopping(pb::HostStopping {})
            }
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
                        plugins: plugin_choices(backend, &start.plugins),
                        effort: convert::optional(start.effort),
                        experimental: start.experimental,
                        notices: start.notices,
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
                let agent = self.owner(&session)?.1;
                // One it took already, whose answer was lost, isn't run again.
                if self.take_send(&session, &message.client_id) {
                    let sent = Message {
                        text: message.text,
                        images: message.images.into_iter().map(convert::image).collect(),
                    };
                    if let Err(error) = agent.send(&session, sent, mode).await {
                        self.forget_send(&session, &message.client_id);
                        return Err(error.into());
                    }
                }
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
            | Request::DeleteProject(_)
            | Request::CreateItem(_)
            | Request::UpdateItem(_)
            | Request::DeleteItem(_)
            | Request::ListItems(_)
            | Request::ReadBoardLog(_)
            | Request::RevertBoardChange(_)
            | Request::ListInbox(_)
            | Request::AskPerson(_)
            | Request::RequestMove(_)
            | Request::AnswerRequest(_)
            | Request::ListHosts(_)
            | Request::ListRecoveries(_)
            | Request::RestoreSession(_)
            | Request::RestoreHost(_)
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
            | Request::SendReview(_)
            | Request::ForwardPort(_)
            | Request::ListForwards(_)
            | Request::StopForward(_)
            | Request::ListProposals(_)
            | Request::ApplyProposal(_)
            | Request::DeclineProposal(_)
            | Request::Propose(_)
            | Request::UpdateProject(_)
            | Request::ReadSpec(_)
            | Request::ReadSkill(_)
            | Request::LinkCommands(_)
            | Request::Search(_)
            | Request::ListPullRequests(_)
            | Request::ImportIssues(_)
            | Request::WaitForApproval(_)
            | Request::ListApprovals(_)
            | Request::DecideApprovals(_)
            | Request::RevokeGrant(_)
            | Request::GetStorage(_)
            | Request::SetStorageSettings(_)
            | Request::CleanUpStorage(_)
            | Request::GetPullRefresh(_)
            | Request::SetPullRefresh(_)
            | Request::SetModelServer(_)
            | Request::ListSkills(_)
            | Request::PickSkill(_)
            | Request::ListPlugins(_)
            | Request::SetProjectPlugin(_)
            | Request::GetSessionDefaults(_)
            | Request::SetSessionDefaults(_) => {
                return Err(HostError::NotForHosts);
            }
            Request::SetSpecs(set) => {
                self.relay.set_specs(set);
                Response::SpecsSet(pb::SpecsSet {})
            }
            Request::ConfigureHost(configure) => {
                if let Ok(port) = u16::try_from(configure.model_server_port)
                    && port != 0
                {
                    self.set_model_server(port);
                }
                Response::HostConfigured(pb::HostConfigured {
                    setup: Some(self.machine.configure(configure).await?),
                })
            }
            Request::ReadHostStatus(_) => {
                Response::HostStatus(self.machine.status(self.program_version.clone()).await)
            }
            Request::CloneRepositories(clone) => {
                Response::RepositoriesCloned(self.machine.clone(clone).await?)
            }
            Request::ScanSkills(scan) => {
                Response::SkillsScanned(self.machine.scan_skills(scan).await?)
            }
            Request::ScanPlugins(pb::ScanPlugins {}) => {
                Response::PluginsScanned(self.machine.scan_plugins().await?)
            }
            Request::Unqueue(unqueue) => {
                let session = SessionId::new(unqueue.session_id);
                self.owner(&session)?
                    .1
                    .unqueue(&session, &unqueue.queue_id)
                    .await?;
                Response::Unqueued(pb::Unqueued {})
            }
            Request::ReadImage(read) => {
                let session = SessionId::new(read.session_id);
                let image = self
                    .owner(&session)?
                    .1
                    .read_image(&session, &read.name)
                    .await?;
                Response::ImageData(pb::ImageData {
                    image: Some(pb::Image {
                        media_type: image.media_type,
                        data: image.data,
                    }),
                })
            }
            Request::EditQueued(edit) => {
                let session = SessionId::new(edit.session_id);
                self.owner(&session)?
                    .1
                    .edit_queued(&session, &edit.queue_id, edit.text)
                    .await?;
                Response::QueuedEdited(pb::QueuedEdited {})
            }
            Request::AbortTurn(abort) => {
                let session = SessionId::new(abort.session_id);
                let owner = self.owner(&session)?.1;
                if abort.drop_queued {
                    owner.drop_queue(&session).await?;
                }
                owner.abort(&session).await?;
                Response::TurnAborted(pb::TurnAborted {})
            }
            Request::RewindSession(rewind) => {
                let session = SessionId::new(rewind.session_id);
                let before = EventId::new(rewind.before_event_id);
                self.owner(&session)?
                    .1
                    .rewind(&session, &before, rewind.files)
                    .await?;
                Response::SessionRewound(pb::SessionRewound {})
            }
            Request::UndoRewind(undo) => {
                let session = SessionId::new(undo.session_id);
                self.owner(&session)?.1.undo_rewind(&session).await?;
                Response::RewindUndone(pb::RewindUndone {})
            }
            Request::ListImportable(_) => {
                let mut sessions = Vec::new();
                for backend in [pb::Backend::Claude, pb::Backend::Copilot] {
                    for found in self.backend(backend)?.importable().await? {
                        sessions.push(pb::ImportableSession {
                            backend: backend.into(),
                            id: found.id,
                            working_directory: found.working_directory.display().to_string(),
                            summary: found.summary,
                            updated_unix_ms: found.updated_unix_ms,
                        });
                    }
                }
                sessions.sort_by_key(|found| std::cmp::Reverse(found.updated_unix_ms));
                Response::Importable(pb::Importable { sessions })
            }
            Request::ListSkipped(_) => {
                let mut sessions = Vec::new();
                for backend in [
                    pb::Backend::Claude,
                    pb::Backend::Copilot,
                    pb::Backend::Local,
                    pb::Backend::Scripted,
                ] {
                    let Some(agent) = self.backends.get(&backend) else {
                        continue;
                    };
                    for skipped in agent.skipped().await? {
                        sessions.push(pb::SkippedSession {
                            backend: backend.into(),
                            id: skipped.id,
                            folder: skipped.folder.display().to_string(),
                            reason: skipped.reason,
                        });
                    }
                }
                Response::Skipped(pb::Skipped { sessions })
            }
            Request::RemoveSkipped(remove) => {
                self.backend(remove.backend())?
                    .remove_skipped(&remove.id)
                    .await?;
                Response::SkippedRemoved(pb::SkippedRemoved {})
            }
            Request::ListModels(list) => {
                let models = self
                    .backend(list.backend())?
                    .models(convert::optional(list.backend_config))
                    .await?;
                Response::Models(pb::Models {
                    models: models
                        .into_iter()
                        .map(|model| pb::ModelChoice {
                            id: model.id,
                            name: model.name,
                            detail: model.detail.unwrap_or_default(),
                            reads_images: model.reads_images,
                            efforts: model.efforts,
                            default_effort: model.default_effort.unwrap_or_default(),
                        })
                        .collect(),
                })
            }
            Request::BackupSession(backup) => Response::SessionBackup(self.back_up(backup).await?),
            Request::RestoreBackup(restore) => {
                Response::SessionRestored(self.restore(restore).await?)
            }
            Request::ImportSession(import) => {
                let backend = import.backend();
                let agent = self.backend(backend)?;
                let record = agent
                    .import(&import.external_id, convert::optional(import.name))
                    .await?;
                self.owners
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(record.id.clone(), backend);
                Response::SessionStarted(pb::SessionStarted {
                    session: Some(self.info(backend, agent.as_ref(), &record).await?),
                })
            }
            Request::ForkSession(fork) => {
                let session = SessionId::new(fork.session_id);
                let (backend, agent) = self.owner(&session)?;
                let options = ForkOptions {
                    before: convert::optional(fork.before_event_id).map(EventId::new),
                    name: convert::optional(fork.name),
                    chat_only: fork.chat_only,
                    repositories: fork
                        .repositories
                        .into_iter()
                        .map(convert::repository)
                        .collect(),
                    instructions: fork.instructions,
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
                    effort: update.effort.map(convert::optional),
                    experimental: update.experimental,
                };
                let record = agent.update(&session, changes).await?;
                Response::SessionUpdated(pb::SessionUpdated {
                    session: Some(self.info(backend, agent.as_ref(), &record).await?),
                })
            }
            Request::DeleteSession(delete) => {
                let session = SessionId::new(delete.session_id);
                let agent = self.owner(&session)?.1;
                if delete.only_if_held {
                    let record = record(agent.as_ref(), &session).await?;
                    for checkout in &record.checkouts {
                        // A fork's checkouts are its parent's.
                        if checkout.owner == session.as_str()
                            && !humata_agent::held_elsewhere(&checkout.repository, &checkout.branch)
                                .await
                        {
                            return Ok(Reply::Response(Response::SessionDeleted(
                                pb::SessionDeleted { kept: true },
                            )));
                        }
                    }
                }
                agent.delete(&session).await?;
                self.owners
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&session);
                Response::SessionDeleted(pb::SessionDeleted { kept: false })
            }
            Request::ReadChanges(read) => {
                let session = SessionId::new(read.session_id.clone());
                let query = convert::query(&read)?;
                let changes = self.owner(&session)?.1.changes(&session, &query).await?;
                Response::Changes(pb::Changes {
                    repositories: changes.iter().map(convert::changes).collect(),
                })
            }
            Request::LandSession(land) => {
                let session = SessionId::new(land.session_id);
                let agent = self.owner(&session)?.1;
                let landed = agent.land(&session, &land.branch, &land.message).await?;
                let mut repositories = Vec::with_capacity(landed.len());
                for landed in landed {
                    let checkout = landed.checkout;
                    let push = land.push || land.open_pull_requests;
                    if push {
                        self.machine
                            .push(&checkout.path, &checkout.name, &checkout.branch)
                            .await?;
                    }
                    repositories.push(pb::LandedRepository {
                        origin_url: crate::machine::origin(&checkout.path).await,
                        name: checkout.name,
                        branch: checkout.branch,
                        commit: landed.commit.unwrap_or_default(),
                        base: landed.base,
                        pushed: push,
                    });
                }
                Response::SessionLanded(pb::SessionLanded {
                    repositories,
                    pull_requests: Vec::new(),
                })
            }
            Request::UpdateFromBase(update) => {
                let session = SessionId::new(update.session_id);
                let agent = self.owner(&session)?.1;
                for checkout in record(agent.as_ref(), &session).await?.checkouts {
                    self.machine.fetch(&checkout.path, &checkout.name).await?;
                }
                let merged = agent.update_from_base(&session).await?;
                let conflicting: Vec<String> = merged
                    .iter()
                    .flat_map(|(checkout, _, merged)| match merged {
                        Merged::Conflicts(files) => files
                            .iter()
                            .map(|file| format!("{}/{file}", checkout.name))
                            .collect(),
                        Merged::UpToDate | Merged::Merged => Vec::new(),
                    })
                    .collect();
                let sent_to_agent = !conflicting.is_empty();
                if sent_to_agent {
                    let text = format!(
                        "I merged the base branch into your copy and these files conflict:\n\n{}\n\n\
                         Resolve the conflicts in each, keeping what both sides meant, then commit \
                         the merge with git commit --no-edit.",
                        conflicting
                            .iter()
                            .map(|file| format!("- {file}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    );
                    agent.send(&session, text.into(), SendMode::Enqueue).await?;
                }
                Response::BaseUpdated(pb::BaseUpdated {
                    repositories: merged
                        .into_iter()
                        .map(|(checkout, upstream, merged)| {
                            let (outcome, conflicts) = match merged {
                                Merged::UpToDate => (pb::MergeOutcome::UpToDate, Vec::new()),
                                Merged::Merged => (pb::MergeOutcome::Merged, Vec::new()),
                                Merged::Conflicts(files) => (pb::MergeOutcome::Conflicts, files),
                            };
                            pb::MergedRepository {
                                name: checkout.name,
                                upstream,
                                outcome: outcome.into(),
                                conflicts,
                            }
                        })
                        .collect(),
                    sent_to_agent,
                })
            }
            Request::DeleteBranches(delete) => {
                let session = SessionId::new(delete.session_id);
                let agent = self.owner(&session)?.1;
                let record = record(agent.as_ref(), &session).await?;
                if delete.archive {
                    // Work since the merge, committed or not, is the
                    // person's to keep or drop, so it stays where it is.
                    for branch in &delete.branches {
                        let checkout = record.checkouts.iter().find(|checkout| {
                            checkout.name == branch.repository && checkout.branch == branch.branch
                        });
                        if let Some(checkout) = checkout
                            && !humata_agent::holds_only(checkout, &branch.merged_tip).await
                        {
                            return Ok(Reply::Response(Response::BranchesDeleted(
                                pb::BranchesDeleted { kept: true },
                            )));
                        }
                    }
                    let archive = SessionUpdate {
                        archived: Some(true),
                        ..SessionUpdate::default()
                    };
                    agent.update(&session, archive).await?;
                }
                for branch in delete.branches {
                    let Some(repository) = record
                        .repositories
                        .iter()
                        .find(|repository| repository.name == branch.repository)
                    else {
                        continue;
                    };
                    humata_agent::delete_branch(&repository.path, &branch.branch)
                        .await
                        .map_err(|source| AgentError::Workspace {
                            session: session.clone(),
                            source,
                        })?;
                }
                Response::BranchesDeleted(pb::BranchesDeleted { kept: false })
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
                Response::Files(crate::files::list_apart(folder).await?)
            }
            Request::ReadFile(read) => match read.turn {
                Some(turn) if !read.session_id.is_empty() => {
                    let session = SessionId::new(read.session_id);
                    let text = self
                        .owner(&session)?
                        .1
                        .file_at(&session, &read.path, turn as usize)
                        .await?;
                    Response::FileContents(crate::files::version(&read.path, text))
                }
                _ => {
                    let root = self.root(&read.session_id, &read.root).await?;
                    Response::FileContents(crate::files::read_apart(root, read.path).await?)
                }
            },
            Request::ListFileVersions(list) => {
                let session = SessionId::new(list.session_id);
                let versions = self
                    .owner(&session)?
                    .1
                    .file_versions(&session, &list.path)
                    .await?;
                Response::FileVersions(pb::FileVersions {
                    versions: versions
                        .into_iter()
                        .map(|version| pb::FileVersion {
                            turn: u32::try_from(version.turn).unwrap_or(u32::MAX),
                            time_unix_ms: version.time_unix_ms,
                        })
                        .collect(),
                })
            }
            Request::RestoreFileVersion(restore) => {
                let session = SessionId::new(restore.session_id.clone());
                let text = self
                    .owner(&session)?
                    .1
                    .file_at(&session, &restore.path, restore.turn as usize)
                    .await?;
                let folder = self.folder(&session).await?;
                crate::files::write_apart(folder.clone(), restore.path.clone(), text).await?;
                Response::FileVersionRestored(pb::FileVersionRestored {
                    contents: Some(crate::files::read_apart(folder, restore.path).await?),
                })
            }
            Request::ListFolder(list) => {
                let root = self.root(&list.session_id, &list.root).await?;
                Response::Folder(crate::files::folder(&root, &list.directory)?)
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
            Request::WatchToolCalls(_) => return Ok(self.relay.watch()),
            Request::AnswerToolCall(answer) => {
                self.relay.answer(answer)?;
                Response::ToolCallAnswered(pb::ToolCallAnswered {})
            }
            Request::WatchSession(watch) => {
                let session = SessionId::new(watch.session_id);
                let backend = Arc::clone(self.owner(&session)?.1);
                return Ok(stream(backend, session));
            }
            Request::OpenTerminal(open) => {
                let folder = self.root(&open.session_id, &open.folder).await?;
                let terminal = self.terminals.open(Opening {
                    session_id: open.session_id,
                    folder: &folder,
                    rows: dimension(open.rows, 24),
                    columns: dimension(open.columns, 80),
                    env: self.env.vars(),
                })?;
                Response::TerminalOpened(pb::TerminalOpened {
                    terminal: Some(terminal),
                })
            }
            Request::AttachTerminal(attach) => {
                let (_, attached) = self.terminals.attach(&attach.terminal_id)?;
                return Ok(terminal_stream(attached));
            }
            Request::TerminalInput(input) => {
                self.terminals.input(&input.terminal_id, input.data).await?;
                Response::TerminalInputSent(pb::TerminalInputSent {})
            }
            Request::ResizeTerminal(resize) => {
                self.terminals.resize(
                    &resize.terminal_id,
                    dimension(resize.rows, 24),
                    dimension(resize.columns, 80),
                )?;
                Response::TerminalResized(pb::TerminalResized {})
            }
            Request::ListTerminals(_) => Response::Terminals(pb::Terminals {
                terminals: self.terminals.list(),
            }),
            Request::CloseTerminal(close) => {
                self.terminals.close(&close.terminal_id)?;
                Response::TerminalClosed(pb::TerminalClosed {})
            }
            Request::ConnectPort(connect) => {
                let (id, chunks) = self.ports.connect(connect.port).await?;
                return Ok(port_stream(id, chunks));
            }
            Request::WritePort(write) => {
                self.ports.write(&write.connection_id, &write.data).await?;
                Response::PortWritten(pb::PortWritten {})
            }
            Request::ClosePort(close) => {
                self.ports.close(&close.connection_id).await;
                Response::PortClosed(pb::PortClosed {})
            }
        }))
    }
}

/// How much of the history is read while it's read from disk, the history,
/// a marker that the history is all sent, then each new event, until the
/// watcher goes away.
fn stream(backend: Arc<dyn Backend>, session: SessionId) -> Reply {
    let (items, receiver) = mpsc::channel(64);
    tokio::spawn(async move {
        let told = items.clone();
        // A watcher that's behind misses a step of the progress, not an event.
        let progress: humata_agent::Progress = Box::new(move |read, total| {
            let _ = told.try_send(Ok(pb::StreamItem {
                kind: Some(pb::stream_item::Kind::HistoryLoading(pb::HistoryLoading {
                    read,
                    total,
                })),
            }));
        });
        let watch = match backend.watch_reporting(&session, progress).await {
            Ok(watch) => watch,
            Err(error) => {
                let _ = items
                    .send(Err(pb::Error {
                        code: error.code().into(),
                        message: describe(&error),
                    }))
                    .await;
                return;
            }
        };
        let Watch { history, mut live } = watch;
        let item = |event: &Event| pb::StreamItem {
            kind: Some(pb::stream_item::Kind::SessionEvent(convert::event(event))),
        };
        for event in &history {
            if items.send(Ok(item(event))).await.is_err() {
                return;
            }
        }
        drop(history);
        // Sent for an empty history too, so the watcher knows nothing more
        // is being read.
        let loaded = pb::StreamItem {
            kind: Some(pb::stream_item::Kind::HistoryLoaded(pb::HistoryLoaded {})),
        };
        if items.send(Ok(loaded)).await.is_err() {
            return;
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

/// What the server on a port sends, until it closes the connection.
fn port_stream(id: String, mut chunks: mpsc::Receiver<Vec<u8>>) -> Reply {
    let (items, receiver) = mpsc::channel(16);
    tokio::spawn(async move {
        loop {
            let next = tokio::select! {
                next = chunks.recv() => next,
                () = items.closed() => return,
            };
            let Some(data) = next else { return };
            let item = pb::StreamItem {
                kind: Some(pb::stream_item::Kind::PortData(pb::PortData { data })),
            };
            if items.send(Ok(item)).await.is_err() {
                return;
            }
        }
    });
    Reply::Stream {
        response: pb::response::Kind::PortConnected(pb::PortConnected { connection_id: id }),
        items: receiver,
    }
}

/// A terminal's size from the request, or `default` when it's unset.
fn dimension(value: u32, default: u16) -> u16 {
    match u16::try_from(value) {
        Ok(0) => default,
        Ok(value) => value,
        Err(_) => u16::MAX,
    }
}

/// The terminal's recent output, then what it prints, until it exits or
/// the watcher goes away.
fn terminal_stream(mut attached: Attached) -> Reply {
    let (items, receiver) = mpsc::channel(64);
    tokio::spawn(async move {
        let item = |output: pb::TerminalOutput| {
            Ok(pb::StreamItem {
                kind: Some(pb::stream_item::Kind::TerminalOutput(output)),
            })
        };
        let exit = |code: i32| pb::TerminalOutput {
            exited: true,
            exit_code: code,
            ..Default::default()
        };
        let mut replay = std::mem::take(&mut attached.replay);
        let mut exited = attached.exited;
        loop {
            let replayed = pb::TerminalOutput {
                data: std::mem::take(&mut replay),
                replay: true,
                ..Default::default()
            };
            if items.send(item(replayed)).await.is_err() {
                return;
            }
            if let Some(code) = exited {
                let _ = items.send(item(exit(code))).await;
                return;
            }
            loop {
                let next = tokio::select! {
                    next = attached.live.recv() => next,
                    () = items.closed() => return,
                };
                let output = match next {
                    Ok(Chunk::Output(data)) => pb::TerminalOutput {
                        data,
                        ..Default::default()
                    },
                    Ok(Chunk::Exited(code)) => {
                        let _ = items.send(item(exit(code))).await;
                        return;
                    }
                    // Starts over from what's kept, which has what it missed.
                    Err(broadcast::error::RecvError::Lagged(_)) => break,
                    Err(broadcast::error::RecvError::Closed) => return,
                };
                if items.send(item(output)).await.is_err() {
                    return;
                }
            }
            (replay, exited) = attached.again();
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

/// The record of `session`, which `agent` runs.
async fn record(agent: &dyn Backend, session: &SessionId) -> Result<SessionRecord, HostError> {
    agent
        .sessions()
        .await?
        .into_iter()
        .find(|record| &record.id == session)
        .ok_or_else(|| {
            AgentError::UnknownSession {
                session: session.clone(),
            }
            .into()
        })
}

/// The plugins turned on or off for a session of `backend`, by its CLI's
/// name for each; another agent's don't apply.
fn plugin_choices(
    backend: pb::Backend,
    choices: &[pb::PluginChoice],
) -> std::collections::BTreeMap<String, bool> {
    let agent = match backend {
        pb::Backend::Claude => pb::PluginAgent::Claude,
        pb::Backend::Copilot => pb::PluginAgent::Copilot,
        _ => return std::collections::BTreeMap::new(),
    };
    choices
        .iter()
        .filter(|choice| choice.agent() == agent)
        .map(|choice| (choice.id.clone(), choice.enabled))
        .collect()
}
