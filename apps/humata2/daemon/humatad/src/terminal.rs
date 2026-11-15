//! TerminalService: tabs, each a shell kept by a holder of its own, and
//! pasted images. Typing queued for a turn end isn't built yet.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_grpc::humata::v1 as pb;
use humata2_holder::{HolderError, HolderSpec};
use humata2_store::{SessionRecord, Store, StoreError, TabKind, TabRecord};
use pb::invalid_field::Problem;
use tokio::sync::watch;
use tonic::{Request, Response, Status, Streaming};

use crate::attach::{self, AttachStream};
use crate::{paste, status};

/// How long a holder gets to say what it holds before a tab is listed
/// without its offsets.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Results kept for retries with the same idempotency key.
const REMEMBERED_MAX: usize = 4096;

/// A new tab's size until a client attaches with its own.
const INITIAL_COLS: u16 = 80;
const INITIAL_ROWS: u16 = 24;

pub(crate) struct TerminalsConfig {
    pub(crate) store: Arc<Store>,
    pub(crate) machine: String,
    /// The `humata2_holder` binary.
    pub(crate) holder: PathBuf,
    pub(crate) shell: PathBuf,
    pub(crate) shell_args: Vec<OsString>,
    /// Where a machine shell starts.
    pub(crate) root: PathBuf,
    /// Where holders put their sockets.
    pub(crate) holders: PathBuf,
    /// Where holders write what goes wrong.
    pub(crate) logs: PathBuf,
    pub(crate) scrollback_bytes: usize,
}

pub(crate) struct Terminals {
    config: TerminalsConfig,
    remembered: Mutex<Remembered>,
    writers: Arc<Writers>,
}

impl Terminals {
    pub(crate) fn new(config: TerminalsConfig) -> Self {
        Self {
            config,
            remembered: Mutex::new(Remembered::default()),
            writers: Arc::new(Writers::default()),
        }
    }

    async fn session(&self, id: &str) -> Result<SessionRecord, Status> {
        let store = self.config.store.clone();
        let key = id.to_owned();
        match blocking(move || store.session(&key)).await? {
            Ok(Some(session)) => Ok(session),
            Ok(None) | Err(StoreError::InvalidId { .. }) => Err(no_session(id)),
            Err(error) => Err(status::store(error, &self.config.machine, "look up the session")),
        }
    }

    async fn tab(&self, id: &str, session: Option<&str>) -> Result<TabRecord, Status> {
        let store = self.config.store.clone();
        let key = id.to_owned();
        let found = match blocking(move || store.tab(&key)).await? {
            Ok(found) => found,
            Err(StoreError::InvalidId { .. }) => None,
            Err(error) => {
                return Err(status::store(error, &self.config.machine, "look up the tab"));
            }
        };
        match found {
            Some(tab) if session.is_none_or(|session| tab.session.as_deref() == Some(session)) => {
                Ok(tab)
            }
            _ => Err(no_tab(id)),
        }
    }

    async fn tabs(&self) -> Result<Vec<TabRecord>, Status> {
        let store = self.config.store.clone();
        blocking(move || store.tabs())
            .await?
            .map_err(|error| status::store(error, &self.config.machine, "list the tabs"))
    }

    /// A tab as the API shows it, with the offsets its holder reports.
    async fn describe(&self, tab: &TabRecord) -> pb::Tab {
        let offsets = tokio::time::timeout(
            PROBE_TIMEOUT,
            humata2_holder::attach(&tab.holder_socket, u64::MAX),
        )
        .await;
        let (earliest_offset, end_offset) = match offsets {
            Ok(Ok(attachment)) => (attachment.welcome.earliest, attachment.welcome.end),
            _ => (0, 0),
        };
        pb::Tab {
            id: Some(pb::TabId {
                value: tab.id.clone(),
            }),
            session: tab
                .session
                .clone()
                .map(|value| pb::SessionId { value }),
            kind: match tab.kind {
                TabKind::Agent => pb::tab::Kind::Agent,
                TabKind::Shell => pb::tab::Kind::Shell,
            } as i32,
            title: tab.title.clone(),
            end_offset,
            earliest_offset,
            machine: tab.session.is_none().then(|| pb::MachineId {
                value: self.config.machine.clone(),
            }),
        }
    }

    /// Starts a shell in `cwd` under a holder of its own and saves its tab.
    async fn open_shell(&self, session: Option<String>, cwd: PathBuf) -> Result<pb::Tab, Status> {
        let config = &self.config;
        let id = format!("tb_{}", uuid::Uuid::new_v4().simple());
        let socket = config.holders.join(format!("{id}.sock"));
        let log = config.logs.join(format!("{id}.log"));
        let spec = HolderSpec {
            holder: config.holder.clone(),
            socket: socket.clone(),
            program: config.shell.clone(),
            args: config.shell_args.clone(),
            cwd,
            env: Vec::new(),
            cols: INITIAL_COLS,
            rows: INITIAL_ROWS,
            scrollback_bytes: config.scrollback_bytes,
            log: log.clone(),
        };
        let spawned = blocking(move || humata2_holder::spawn(&spec))
            .await?
            .map_err(|error| holder_failed("start the tab's shell", &error, &log))?;
        let tab = TabRecord {
            id,
            session,
            kind: TabKind::Shell,
            title: config
                .shell
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "shell".to_owned()),
            holder_socket: socket,
            holder_pid: spawned.pid,
            created_unix_ms: unix_ms(),
        };
        let store = config.store.clone();
        let saved = tab.clone();
        if let Err(error) = blocking(move || store.put_tab(&saved)).await? {
            // Nothing could find the shell again, so it doesn't stay.
            let _ = humata2_holder::shutdown(&tab.holder_socket).await;
            return Err(status::store(error, &config.machine, "save the new tab"));
        }
        Ok(self.describe(&tab).await)
    }

    fn recall(&self, key: Option<&str>) -> Option<Outcome> {
        let key = key?;
        lock(&self.remembered).results.get(key).cloned()
    }

    fn remember(&self, key: Option<String>, outcome: Outcome) {
        let Some(key) = key else { return };
        let mut remembered = lock(&self.remembered);
        if remembered.results.insert(key.clone(), outcome).is_none() {
            remembered.order.push_back(key);
        }
        while remembered.order.len() > REMEMBERED_MAX {
            if let Some(oldest) = remembered.order.pop_front() {
                remembered.results.remove(&oldest);
            }
        }
    }
}

#[tonic::async_trait]
impl pb::terminal_service_server::TerminalService for Terminals {
    /// The tabs of `session`, or with no session, this machine's shells.
    async fn list_tabs(
        &self,
        request: Request<pb::ListTabsRequest>,
    ) -> Result<Response<pb::ListTabsResponse>, Status> {
        let session = id_of(request.into_inner().session.map(|session| session.value));
        if let Some(session) = &session {
            self.session(session).await?;
        }
        let mut tabs = Vec::new();
        for tab in self.tabs().await? {
            if tab.session == session {
                tabs.push(self.describe(&tab).await);
            }
        }
        Ok(Response::new(pb::ListTabsResponse { tabs }))
    }

    async fn open_shell_tab(
        &self,
        request: Request<pb::OpenShellTabRequest>,
    ) -> Result<Response<pb::Tab>, Status> {
        let request = request.into_inner();
        let key = idempotency("OpenShellTab", request.idempotency_key);
        if let Some(Outcome::Tab(tab)) = self.recall(key.as_deref()) {
            return Ok(Response::new(tab));
        }
        let session = id_of(request.session.map(|session| session.value))
            .ok_or_else(|| status::missing("session"))?;
        if request.repo.is_some() {
            return Err(status::invalid_field(
                "repo",
                Problem::NotAllowedHere,
                "starting a shell in one of the session's clones isn't built yet",
                "leave repo unset to start in the session folder, then cd into the clone",
            ));
        }
        let session = self.session(&session).await?;
        let tab = self.open_shell(Some(session.id), session.folder).await?;
        self.remember(key, Outcome::Tab(tab.clone()));
        Ok(Response::new(tab))
    }

    async fn open_machine_shell(
        &self,
        request: Request<pb::OpenMachineShellRequest>,
    ) -> Result<Response<pb::Tab>, Status> {
        let request = request.into_inner();
        let key = idempotency("OpenMachineShell", request.idempotency_key);
        if let Some(Outcome::Tab(tab)) = self.recall(key.as_deref()) {
            return Ok(Response::new(tab));
        }
        if let Some(machine) = id_of(request.machine.map(|machine| machine.value))
            && machine != self.config.machine
        {
            return Err(status::not_found(
                pb::EntityKind::Machine,
                &machine,
                format!("machine {machine} isn't this one, and this daemon can't open shells elsewhere yet"),
                "open the shell through that machine's own daemon",
            ));
        }
        let tab = self.open_shell(None, self.config.root.clone()).await?;
        self.remember(key, Outcome::Tab(tab.clone()));
        Ok(Response::new(tab))
    }

    async fn close_tab(
        &self,
        request: Request<pb::CloseTabRequest>,
    ) -> Result<Response<pb::CloseTabResponse>, Status> {
        let request = request.into_inner();
        let key = idempotency("CloseTab", request.idempotency_key);
        if let Some(Outcome::Closed) = self.recall(key.as_deref()) {
            return Ok(Response::new(pb::CloseTabResponse {}));
        }
        let id = id_of(request.tab.map(|tab| tab.value)).ok_or_else(|| status::missing("tab"))?;
        let session = id_of(request.session.map(|session| session.value));
        let tab = self.tab(&id, session.as_deref()).await?;
        if tab.kind == TabKind::Agent {
            return Err(status::invalid_field(
                "tab",
                Problem::NotAllowedHere,
                format!("tab {id} is the session's agent, which can't be closed on its own"),
                "stop the session instead; that ends the agent",
            ));
        }
        match humata2_holder::shutdown(&tab.holder_socket).await {
            // A holder that's gone already has nothing left to stop.
            Ok(()) | Err(HolderError::Connect { .. }) => {}
            Err(error) => {
                return Err(holder_failed("stop the tab's shell", &error, Path::new("")));
            }
        }
        let store = self.config.store.clone();
        blocking(move || store.remove_tab(&id))
            .await?
            .map_err(|error| status::store(error, &self.config.machine, "forget the closed tab"))?;
        self.writers.forget(&tab.id);
        self.remember(key, Outcome::Closed);
        Ok(Response::new(pb::CloseTabResponse {}))
    }

    type AttachStream = AttachStream;

    async fn attach(
        &self,
        request: Request<Streaming<pb::AttachClientMessage>>,
    ) -> Result<Response<Self::AttachStream>, Status> {
        let mut incoming = request.into_inner();
        let open = match incoming.message().await? {
            Some(pb::AttachClientMessage {
                message: Some(pb::attach_client_message::Message::Open(open)),
            }) => open,
            _ => {
                return Err(status::invalid_field(
                    "open",
                    Problem::Missing,
                    "an Attach must start with Open",
                    "send AttachOpen as the first message, naming the tab",
                ));
            }
        };
        let id = id_of(open.tab.clone().map(|tab| tab.value))
            .ok_or_else(|| status::missing("open.tab"))?;
        let session = id_of(open.session.clone().map(|session| session.value));
        let tab = self.tab(&id, session.as_deref()).await?;
        let stream = attach::start(&tab, &open, incoming, &self.writers).await?;
        Ok(Response::new(stream))
    }

    async fn r#type(
        &self,
        _request: Request<pb::TypeRequest>,
    ) -> Result<Response<pb::TypeResponse>, Status> {
        Err(status::not_built("humata.v1.TerminalService.Type"))
    }

    async fn cancel_queued_typing(
        &self,
        _request: Request<pb::CancelQueuedTypingRequest>,
    ) -> Result<Response<pb::CancelQueuedTypingResponse>, Status> {
        Err(status::not_built("humata.v1.TerminalService.CancelQueuedTyping"))
    }

    async fn list_queued(
        &self,
        _request: Request<pb::ListQueuedRequest>,
    ) -> Result<Response<pb::ListQueuedResponse>, Status> {
        Err(status::not_built("humata.v1.TerminalService.ListQueued"))
    }

    async fn send_queued_now(
        &self,
        _request: Request<pb::SendQueuedNowRequest>,
    ) -> Result<Response<pb::SendQueuedNowResponse>, Status> {
        Err(status::not_built("humata.v1.TerminalService.SendQueuedNow"))
    }

    async fn paste_image(
        &self,
        request: Request<pb::PasteImageRequest>,
    ) -> Result<Response<pb::PasteImageResponse>, Status> {
        let request = request.into_inner();
        let key = idempotency("PasteImage", request.idempotency_key);
        if let Some(Outcome::Paste(pasted)) = self.recall(key.as_deref()) {
            return Ok(Response::new(pasted));
        }
        let session = id_of(request.session.map(|session| session.value))
            .ok_or_else(|| status::missing("session"))?;
        let extension = paste::check(request.format, &request.data)?;
        let size_bytes = request.data.len() as u64;
        let store = self.config.store.clone();
        let folder = session.clone();
        let saved = blocking(move || store.save_paste(&folder, extension, &request.data)).await?;
        let path = match saved {
            Ok(path) => path,
            Err(StoreError::NoSession { .. } | StoreError::InvalidId { .. }) => {
                return Err(no_session(&session));
            }
            Err(error) => {
                return Err(status::store(error, &self.config.machine, "save the pasted image"));
            }
        };
        let Some(path) = path.to_str().map(str::to_owned) else {
            return Err(status::internal(
                format!("the pasted image was saved at {}, a path that isn't UTF-8", path.display()),
                "",
            ));
        };
        let pasted = pb::PasteImageResponse { path, size_bytes };
        self.remember(key, Outcome::Paste(pasted.clone()));
        Ok(Response::new(pasted))
    }
}

/// What a mutation returned, for a retry with the same key.
#[derive(Clone)]
enum Outcome {
    Tab(pb::Tab),
    Closed,
    Paste(pb::PasteImageResponse),
}

#[derive(Default)]
struct Remembered {
    results: HashMap<String, Outcome>,
    order: VecDeque<String>,
}

/// How many attaches that can type each tab has (SE-24). Attaches watch
/// their tab's count to say how many others there are.
#[derive(Default)]
pub(crate) struct Writers {
    tabs: Mutex<HashMap<String, watch::Sender<u32>>>,
}

impl Writers {
    fn sender(&self, tab: &str) -> watch::Sender<u32> {
        lock(&self.tabs)
            .entry(tab.to_owned())
            .or_insert_with(|| watch::channel(0).0)
            .clone()
    }

    pub(crate) fn watch(&self, tab: &str) -> watch::Receiver<u32> {
        self.sender(tab).subscribe()
    }

    /// Counts an attach that can type until the guard is dropped.
    pub(crate) fn join(&self, tab: &str) -> WriterGuard {
        let sender = self.sender(tab);
        sender.send_modify(|count| *count += 1);
        WriterGuard { sender }
    }

    fn forget(&self, tab: &str) {
        lock(&self.tabs).remove(tab);
    }
}

pub(crate) struct WriterGuard {
    sender: watch::Sender<u32>,
}

impl Drop for WriterGuard {
    fn drop(&mut self) {
        self.sender
            .send_modify(|count| *count = count.saturating_sub(1));
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs file work off the async threads.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, Status> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| status::internal(format!("a background task failed: {error}"), ""))
}

/// An id field's value, or `None` when it's unset or empty.
fn id_of(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

fn idempotency(method: &str, key: Option<pb::IdempotencyKey>) -> Option<String> {
    id_of(key.map(|key| key.value)).map(|key| format!("{method}/{key}"))
}

fn no_session(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Session,
        id,
        format!("no session {id} is on this machine"),
        "check the session id; it may have been removed or be on another machine",
    )
}

pub(crate) fn no_tab(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Tab,
        id,
        format!("no tab {id} is open on this machine"),
        "list the session's tabs for the ones still open",
    )
}

fn holder_failed(doing: &str, error: &HolderError, log: &Path) -> Status {
    status::internal(
        format!("couldn't {doing}: {}", status::chain(error)),
        &log.display().to_string(),
    )
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
