//! [`LocalAgent`]: the [`Agent`] for backends that run in this process.
//!
//! A backend only plays one turn ([`Runner`]); this keeps the sessions, their
//! history on disk, the queue of messages, steering, approvals, aborts and
//! forks.
//!
//! On disk, each session is a folder under `<root>/sessions/<id>` with
//! `session.json` (its [`SessionRecord`]) and `events.jsonl`.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::backup::{BackendFile, Backup};
use crate::error::AgentError;
use humata_workspace::{
    Changes, Checkout, Edit, Landed, Merged, Query, Repository, Snapshot, WorkspaceError,
    Workspaces,
};

use crate::event::{
    ApprovalMode, Event, EventKind, ForkOptions, Parent, PermissionKind, SendMode, SessionId,
    SessionRecord, SessionState, SessionUpdate, StartOptions,
};
use crate::import::{Importable, Imported};
use crate::log::EventLog;
use crate::summary::Summary;
use crate::{Agent, Model, Watch};

const RECORD_FILE: &str = "session.json";
const EVENTS_FILE: &str = "events.jsonl";

/// Plays one turn of a session.
pub trait Runner: Send + Sync + 'static {
    /// Recorded in the history as the turn's failure.
    type Error: std::error::Error + From<AgentError> + Send + 'static;

    /// Replies to `turn.text()`, recording what happens through `turn`. The
    /// turn's start and end are recorded around it.
    fn run_turn(
        self: Arc<Self>,
        turn: Turn,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Called when `fork` is made from `parent`, with the history it keeps,
    /// before it runs a turn. A runner that keeps its own state per session
    /// carries it over here.
    fn forked(
        &self,
        parent: &SessionId,
        fork: &SessionRecord,
        history: &[Event],
    ) -> Result<(), AgentError> {
        let _ = (parent, fork, history);
        Ok(())
    }

    /// The backend's own sessions on this host, started outside Humata,
    /// that aren't imported yet. A runner that keeps none has none.
    fn importable(&self) -> Result<Vec<Importable>, AgentError> {
        Ok(Vec::new())
    }

    /// The models the backend offers; with `config`, a session's backend
    /// config, those of the model server it names. A runner that picks none
    /// offers none.
    fn models(
        &self,
        config: Option<String>,
    ) -> impl Future<Output = Result<Vec<Model>, AgentError>> + Send {
        let _ = config;
        async { Ok(Vec::new()) }
    }

    /// The backend's own state of the session, such as its transcript, for
    /// [`Runner::restore`] to put back. None for a backend that keeps none.
    fn backup(&self, record: &SessionRecord) -> Result<Vec<BackendFile>, AgentError> {
        let _ = record;
        Ok(Vec::new())
    }

    /// Puts back what [`Runner::backup`] returned, for `record` as it is on
    /// this host now.
    fn restore(&self, record: &SessionRecord, files: &[BackendFile]) -> Result<(), AgentError> {
        let _ = (record, files);
        Ok(())
    }

    /// Deletes the backend's own state of `record`'s session, such as its
    /// transcript, as the session is deleted. A backend that keeps none has
    /// none to delete.
    fn forget(&self, record: &SessionRecord) -> Result<(), AgentError> {
        let _ = record;
        Ok(())
    }

    /// Reads the backend's session `external` as our history, and links it
    /// to our new session `session`, so its turns carry on the backend's.
    fn import(&self, external: &str, session: &SessionId) -> Result<Imported, AgentError> {
        let _ = (external, session);
        Err(AgentError::NotImportable)
    }
}

/// What a [`Runner`] uses to record a turn and to ask for permission.
pub struct Turn {
    session: Arc<Session>,
    workspaces: Option<Arc<Workspaces>>,
    text: String,
}

impl Turn {
    /// The message the turn replies to.
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn record(&self) -> SessionRecord {
        self.session.lock().record.clone()
    }

    /// Every event so far, this turn's included.
    pub fn history(&self) -> Vec<Event> {
        self.session.log.events()
    }

    pub fn emit(&self, kind: EventKind) -> Result<Event, AgentError> {
        let saving = matches!(kind, EventKind::ToolCompleted { success: true, .. });
        let event = self.session.log.append(Event::now(kind))?;
        if saving {
            save_soon(&self.session, self.workspaces.clone());
        }
        Ok(event)
    }

    /// Asks the person, and waits, unless the session's approval mode allows
    /// it. False if they deny it or the turn stops.
    pub async fn ask(
        &self,
        kind: PermissionKind,
        summary: impl Into<String>,
    ) -> Result<bool, AgentError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (answer, answered) = oneshot::channel();
        {
            let mut control = self.session.lock();
            self.session
                .log
                .append(Event::now(EventKind::PermissionRequested {
                    request_id: request_id.clone(),
                    kind,
                    summary: summary.into(),
                }))?;
            if control.record.approval.allows(kind) {
                self.session
                    .log
                    .append(Event::now(EventKind::PermissionCompleted {
                        request_id,
                        approved: true,
                        automatic: true,
                    }))?;
                return Ok(true);
            }
            control.pending.insert(request_id, Pending { kind, answer });
        }
        Ok(answered.await.unwrap_or(false))
    }

    /// Messages sent to steer this turn since the last call.
    pub fn take_steering(&self) -> Vec<String> {
        self.session.lock().steering.drain(..).collect()
    }
}

/// Runs sessions in this process, with `R` playing each turn.
pub struct LocalAgent<R> {
    root: PathBuf,
    runner: Arc<R>,
    sessions: Mutex<HashMap<SessionId, Arc<Session>>>,
    /// Where sessions that work in repositories get their copies of them.
    workspaces: Option<Arc<Workspaces>>,
    /// Sessions on disk that couldn't be opened, left as they are.
    skipped: Mutex<Vec<Skipped>>,
}

/// A session on disk that couldn't be opened, so it is left as it is until
/// it is fixed or removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// Its folder's name, which is its id.
    pub id: String,
    pub folder: PathBuf,
    /// Why it couldn't be opened, and what fixing it would take.
    pub reason: String,
}

struct Session {
    id: SessionId,
    dir: PathBuf,
    log: EventLog,
    control: Mutex<Control>,
    /// Held while the session's code is captured, so two captures of one
    /// worktree never run at once.
    capturing: tokio::sync::Mutex<()>,
    /// Whether a tool call finished since the code was last saved.
    unsaved: AtomicBool,
}

struct Control {
    record: SessionRecord,
    running: Option<Running>,
    /// Bumped for every driver, and on abort, so a stopped driver can tell.
    generation: u64,
    /// Between a turn's start and its end.
    turn_open: bool,
    queue: VecDeque<Queued>,
    steering: VecDeque<String>,
    pending: HashMap<String, Pending>,
}

/// A message waiting for the running turn to end.
struct Queued {
    id: String,
    text: String,
}

impl Session {
    /// Queues `text` at the back, or the front for a steer between turns,
    /// and says so in the history.
    fn queue(&self, control: &mut Control, text: String, front: bool) {
        let id = crate::event::EventId::random().to_string();
        self.note(EventKind::MessageQueued {
            queue_id: id.clone(),
            text: text.clone(),
        });
        let queued = Queued { id, text };
        if front {
            control.queue.push_front(queued);
        } else {
            control.queue.push_back(queued);
        }
    }

    /// The next queued message, which leaves the queue to run.
    fn next_queued(&self, control: &mut Control) -> Option<String> {
        let next = control.queue.pop_front()?;
        self.note(EventKind::Dequeued { queue_id: next.id });
        Some(next.text)
    }
}

/// A permission request waiting for its answer.
struct Pending {
    kind: PermissionKind,
    answer: oneshot::Sender<bool>,
}

/// The task that plays turns until the queue is empty.
struct Running {
    generation: u64,
    /// Taken by abort.
    handle: Option<JoinHandle<()>>,
}

impl Control {
    fn new(record: SessionRecord) -> Self {
        Self {
            record,
            running: None,
            generation: 0,
            turn_open: false,
            queue: VecDeque::new(),
            steering: VecDeque::new(),
            pending: HashMap::new(),
        }
    }

    fn is_current(&self, generation: u64) -> bool {
        self.running
            .as_ref()
            .is_some_and(|running| running.generation == generation)
    }
}

impl Session {
    fn lock(&self) -> MutexGuard<'_, Control> {
        self.control.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records an event the runtime itself adds. If the disk refuses it the
    /// session carries on: the event is still sent to watchers' next read of
    /// the log, and the failure goes to the program's log.
    fn note(&self, kind: EventKind) {
        if let Err(error) = self.log.append(Event::now(kind)) {
            eprintln!("humata: {}", describe(&error));
        }
    }
}

impl<R: Runner> LocalAgent<R> {
    /// Opens the sessions under `root`, creating it if needed. A session
    /// whose turn was cut off, because the program stopped, is closed with an
    /// aborted turn. One that can't be read is skipped, its folder left as
    /// it is, and listed with why by [`Agent::skipped`].
    pub fn open(root: impl Into<PathBuf>, runner: R) -> Result<Self, AgentError> {
        let root = root.into();
        let dir = root.join("sessions");
        std::fs::create_dir_all(&dir).map_err(|source| AgentError::CreateDirectory {
            path: dir.clone(),
            source,
        })?;
        let mut sessions = HashMap::new();
        let mut skipped = Vec::new();
        let entries = std::fs::read_dir(&dir).map_err(|source| AgentError::ReadDirectory {
            path: dir.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| AgentError::ReadDirectory {
                path: dir.clone(),
                source,
            })?;
            let path = entry.path();
            if !path.join(RECORD_FILE).is_file() {
                continue;
            }
            // One damaged session mustn't keep the others closed.
            let session = match load(&path) {
                Ok(session) => session,
                Err(error) => {
                    let reason = describe(&error);
                    eprintln!("humata: skipped a session: {reason}");
                    skipped.push(Skipped {
                        id: entry.file_name().to_string_lossy().into_owned(),
                        folder: path,
                        reason,
                    });
                    continue;
                }
            };
            for kind in closing(&session.log.events()) {
                session.log.append(Event::now(kind))?;
            }
            sessions.insert(session.id.clone(), Arc::new(session));
        }
        Ok(Self {
            root,
            runner: Arc::new(runner),
            sessions: Mutex::new(sessions),
            workspaces: None,
            skipped: Mutex::new(skipped),
        })
    }

    /// Lets sessions work in repositories, each in its own copy of them
    /// made by `workspaces`.
    #[must_use]
    pub fn with_workspaces(mut self, workspaces: Workspaces) -> Self {
        self.workspaces = Some(Arc::new(workspaces));
        self
    }

    fn workspaces(&self, session: &SessionId) -> Result<&Workspaces, AgentError> {
        self.workspaces
            .as_deref()
            .ok_or_else(|| AgentError::NoWorkspaces {
                session: session.clone(),
            })
    }

    /// Snapshots and removes the session's own worktrees; a fork of the chat
    /// only leaves its parent's alone.
    async fn put_away(&self, session: &Session) -> Result<(), AgentError> {
        let record = session.lock().record.clone();
        let owned: Vec<_> = record
            .checkouts
            .iter()
            .filter(|checkout| checkout.owner == session.id.as_str())
            .cloned()
            .collect();
        if !owned.is_empty() {
            let workspaces = self.workspaces(&session.id)?;
            if let Some(snapshots) = checkpoint(session, Some(workspaces)).await? {
                session
                    .log
                    .append(Event::now(EventKind::Checkpoint { snapshots }))?;
            }
            workspaces
                .remove(&owned)
                .await
                .map_err(workspace_failed(&session.id))?;
        }
        let mut control = session.lock();
        let record = SessionRecord {
            checkouts: Vec::new(),
            ..control.record.clone()
        };
        save(&session.dir, &record)?;
        control.record = record;
        Ok(())
    }

    fn session(&self, id: &SessionId) -> Result<Arc<Session>, AgentError> {
        self.lock()
            .get(id)
            .cloned()
            .ok_or_else(|| AgentError::UnknownSession {
                session: id.clone(),
            })
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<SessionId, Arc<Session>>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn create(
        &self,
        record: SessionRecord,
        events: Vec<Event>,
    ) -> Result<Arc<Session>, AgentError> {
        let dir = self.root.join("sessions").join(record.id.as_str());
        std::fs::create_dir_all(&dir).map_err(|source| AgentError::CreateDirectory {
            path: dir.clone(),
            source,
        })?;
        save(&dir, &record)?;
        let log = EventLog::create(dir.join(EVENTS_FILE), events)?;
        let session = Arc::new(Session {
            id: record.id.clone(),
            dir,
            log,
            control: Mutex::new(Control::new(record)),
            capturing: tokio::sync::Mutex::default(),
            unsaved: AtomicBool::new(false),
        });
        self.lock().insert(session.id.clone(), session.clone());
        Ok(session)
    }

    /// Starts a driver for `text`. The caller holds `control` and has checked
    /// that nothing is running.
    fn start_driver(&self, session: &Arc<Session>, control: &mut Control, text: String) {
        control.generation += 1;
        let generation = control.generation;
        let handle = tokio::spawn(drive(
            session.clone(),
            self.runner.clone(),
            self.workspaces.clone(),
            generation,
            text,
        ));
        control.running = Some(Running {
            generation,
            handle: Some(handle),
        });
    }
}

/// Plays `text`, then each queued message, until the queue is empty or the
/// driver is stopped.
async fn drive<R: Runner>(
    session: Arc<Session>,
    runner: Arc<R>,
    workspaces: Option<Arc<Workspaces>>,
    generation: u64,
    text: String,
) {
    let mut text = text;
    loop {
        let prepared = prepare(&session, workspaces.as_deref()).await;
        {
            let mut control = session.lock();
            if !control.is_current(generation) {
                return;
            }
            control.turn_open = true;
            session.note(EventKind::UserMessage { text: text.clone() });
            session.note(EventKind::TurnStarted);
        }
        let result = match prepared {
            Ok(()) => runner
                .clone()
                .run_turn(Turn {
                    session: session.clone(),
                    workspaces: workspaces.clone(),
                    text,
                })
                .await
                .map_err(|error| describe(&error)),
            Err(error) => Err(describe(&error)),
        };
        let checkpoint = checkpoint(&session, workspaces.as_deref()).await;
        let mut control = session.lock();
        if !control.is_current(generation) {
            return;
        }
        if let Err(message) = result {
            session.note(EventKind::Failed { message });
        }
        note_checkpoint(&session, checkpoint);
        session.note(EventKind::TurnEnded { aborted: false });
        control.turn_open = false;
        control.steering.clear();
        control.pending.clear();
        match session.next_queued(&mut control) {
            Some(next) => text = next,
            None => {
                control.running = None;
                session.note(EventKind::Idle);
                return;
            }
        }
    }
}

impl<R: Runner> Agent for LocalAgent<R> {
    type Error = AgentError;

    async fn start(&self, options: StartOptions) -> Result<SessionRecord, AgentError> {
        let id = SessionId::random();
        let working_directory = if options.repositories.is_empty() {
            options.working_directory
        } else {
            self.workspaces(&id)?
                .working_directory(id.as_str(), &options.repositories)
        };
        let record = SessionRecord {
            id,
            name: options.name,
            working_directory,
            model: options.model,
            instructions: options.instructions,
            backend_config: options.backend_config,
            parent: None,
            approval: options.approval,
            repositories: options.repositories,
            checkouts: Vec::new(),
            archived: false,
            keeper: false,
        };
        self.create(record.clone(), Vec::new())?;
        Ok(record)
    }

    async fn send(&self, id: &SessionId, text: String, mode: SendMode) -> Result<(), AgentError> {
        let session = self.session(id)?;
        let mut control = session.lock();
        if control.record.archived {
            return Err(AgentError::Archived {
                session: id.clone(),
            });
        }
        if control.running.is_none() {
            self.start_driver(&session, &mut control, text);
            return Ok(());
        }
        match mode {
            SendMode::Enqueue => session.queue(&mut control, text, false),
            SendMode::EnqueueOnce if control.queue.iter().any(|queued| queued.text == text) => {}
            SendMode::EnqueueOnce => session.queue(&mut control, text, false),
            SendMode::Immediate if control.turn_open => {
                session
                    .log
                    .append(Event::now(EventKind::UserMessage { text: text.clone() }))?;
                control.steering.push_back(text);
            }
            // Between turns, or while one is being stopped: it goes next.
            SendMode::Immediate => session.queue(&mut control, text, true),
        }
        Ok(())
    }

    async fn unqueue(&self, id: &SessionId, queue_id: &str) -> Result<(), AgentError> {
        let session = self.session(id)?;
        let mut control = session.lock();
        let position = control
            .queue
            .iter()
            .position(|queued| queued.id == queue_id)
            .ok_or_else(|| AgentError::UnknownQueuedMessage {
                session: id.clone(),
                queued: queue_id.to_owned(),
            })?;
        control.queue.remove(position);
        session.note(EventKind::Dequeued {
            queue_id: queue_id.to_owned(),
        });
        Ok(())
    }

    async fn approve(
        &self,
        id: &SessionId,
        request_id: &str,
        approved: bool,
    ) -> Result<(), AgentError> {
        let session = self.session(id)?;
        let answer = {
            let mut control = session.lock();
            let Pending { answer, .. } = control.pending.remove(request_id).ok_or_else(|| {
                AgentError::UnknownPermissionRequest {
                    session: id.clone(),
                    request: request_id.to_owned(),
                }
            })?;
            let recorded = session
                .log
                .append(Event::now(EventKind::PermissionCompleted {
                    request_id: request_id.to_owned(),
                    approved,
                    automatic: false,
                }));
            if let Err(error) = recorded {
                // Unrecorded, it must not go ahead.
                let _ = answer.send(false);
                return Err(error);
            }
            answer
        };
        // The turn may have stopped meanwhile; then nobody is waiting.
        let _ = answer.send(approved);
        Ok(())
    }

    async fn set_approval_mode(
        &self,
        id: &SessionId,
        mode: ApprovalMode,
    ) -> Result<(), AgentError> {
        let session = self.session(id)?;
        let mut control = session.lock();
        let record = SessionRecord {
            approval: mode,
            ..control.record.clone()
        };
        save(&session.dir, &record)?;
        control.record = record;
        let allowed: Vec<String> = control
            .pending
            .iter()
            .filter(|(_, pending)| mode.allows(pending.kind))
            .map(|(request, _)| request.clone())
            .collect();
        for request_id in allowed {
            let Some(Pending { answer, .. }) = control.pending.remove(&request_id) else {
                continue;
            };
            let recorded = session
                .log
                .append(Event::now(EventKind::PermissionCompleted {
                    request_id,
                    approved: true,
                    automatic: true,
                }));
            // Unrecorded, it must not go ahead; the turn sees a refusal.
            let approved = recorded.is_ok();
            let _ = answer.send(approved);
            recorded?;
        }
        Ok(())
    }

    async fn abort(&self, id: &SessionId) -> Result<(), AgentError> {
        let session = self.session(id)?;
        let handle = {
            let mut control = session.lock();
            control.generation += 1;
            let generation = control.generation;
            let Some(running) = control.running.as_mut() else {
                return Ok(());
            };
            // Already being stopped.
            let Some(handle) = running.handle.take() else {
                return Ok(());
            };
            running.generation = generation;
            control.pending.clear();
            handle.abort();
            handle
        };
        // Cancelled, or it finished first: either way it has stopped.
        let _ = handle.await;
        let open = session.lock().turn_open;
        let checkpoint = if open {
            checkpoint(&session, self.workspaces.as_deref()).await
        } else {
            Ok(None)
        };
        let mut control = session.lock();
        if control.turn_open {
            note_checkpoint(&session, checkpoint);
            session.note(EventKind::TurnEnded { aborted: true });
            control.turn_open = false;
        }
        control.steering.clear();
        match session.next_queued(&mut control) {
            Some(next) => {
                control.running = None;
                self.start_driver(&session, &mut control, next);
            }
            None => {
                control.running = None;
                session.note(EventKind::Idle);
            }
        }
        Ok(())
    }

    async fn fork(
        &self,
        id: &SessionId,
        options: ForkOptions,
    ) -> Result<SessionRecord, AgentError> {
        let parent = self.session(id)?;
        let mut events = parent.log.events();
        if let Some(before) = &options.before {
            let index = events
                .iter()
                .position(|event| &event.id == before)
                .ok_or_else(|| AgentError::UnknownEvent {
                    session: id.clone(),
                    event: before.clone(),
                })?;
            events.truncate(index);
        }
        events.extend(closing(&events).into_iter().map(Event::now));
        let from = parent.lock().record.clone();
        let fork = SessionId::random();
        let repositories = if from.repositories.is_empty() {
            options.repositories
        } else {
            from.repositories.clone()
        };
        let (working_directory, checkouts) = if repositories.is_empty() {
            (from.working_directory.clone(), Vec::new())
        } else if from.repositories.is_empty() {
            // A fork of a chat that works in code from here: its copy is
            // made from each repository's HEAD before its first turn.
            let workspaces = self.workspaces(&fork)?;
            (
                workspaces.working_directory(fork.as_str(), &repositories),
                Vec::new(),
            )
        } else if options.chat_only {
            (from.working_directory.clone(), from.checkouts.clone())
        } else {
            // The code as it was where the fork starts; without a snapshot
            // there, its copy is made before its first turn.
            let workspaces = self.workspaces(id)?;
            let snapshots = last_checkpoint(&events);
            let checkouts = if snapshots.is_empty() {
                Vec::new()
            } else {
                workspaces
                    .create(fork.as_str(), &from.repositories, &snapshots)
                    .await
                    .map_err(workspace_failed(&fork))?
            };
            (
                workspaces.working_directory(fork.as_str(), &from.repositories),
                checkouts,
            )
        };
        let record = SessionRecord {
            id: fork,
            name: options.name.unwrap_or(from.name),
            working_directory,
            parent: Some(Parent {
                session: id.clone(),
                before: options.before,
            }),
            instructions: options.instructions.or(from.instructions),
            repositories,
            checkouts,
            archived: false,
            keeper: false,
            ..from
        };
        self.runner.forked(id, &record, &events)?;
        self.create(record.clone(), events)?;
        Ok(record)
    }

    async fn skipped(&self) -> Result<Vec<Skipped>, AgentError> {
        let mut skipped = self
            .skipped
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        skipped.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(skipped)
    }

    async fn remove_skipped(&self, id: &str) -> Result<(), AgentError> {
        let mut skipped = self.skipped.lock().unwrap_or_else(PoisonError::into_inner);
        let index = skipped
            .iter()
            .position(|session| session.id == id)
            .ok_or_else(|| AgentError::UnknownSkipped { id: id.to_owned() })?;
        let folder = &skipped[index].folder;
        match std::fs::remove_dir_all(folder) {
            Ok(()) => {}
            // Removed by hand already: nothing is left to remove.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(AgentError::RemoveSkipped {
                    path: folder.clone(),
                    source,
                });
            }
        }
        skipped.remove(index);
        Ok(())
    }

    async fn delete(&self, id: &SessionId) -> Result<(), AgentError> {
        let session = self.session(id)?;
        let record = session.lock().record.clone();
        if !record.archived {
            return Err(AgentError::DeleteUnarchived {
                session: id.clone(),
            });
        }
        let fork = self.lock().values().find_map(|other| {
            let forked = other.lock().record.parent.as_ref()?.session == *id;
            forked.then(|| other.id.clone())
        });
        if let Some(fork) = fork {
            return Err(AgentError::DeleteForked {
                session: id.clone(),
                fork,
            });
        }
        // Archived, so only a worktree an archive left part way is here.
        let owned: Vec<_> = record
            .checkouts
            .iter()
            .filter(|checkout| checkout.owner == id.as_str())
            .cloned()
            .collect();
        if !owned.is_empty() {
            self.workspaces(id)?
                .remove(&owned)
                .await
                .map_err(workspace_failed(id))?;
        }
        for repository in &record.repositories {
            humata_workspace::forget(&repository.path, id.as_str())
                .await
                .map_err(workspace_failed(id))?;
        }
        self.runner.forget(&record)?;
        match std::fs::remove_dir_all(&session.dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(AgentError::DeleteSession {
                    session: id.clone(),
                    path: session.dir.clone(),
                    source,
                });
            }
        }
        self.lock().remove(id);
        Ok(())
    }

    async fn importable(&self) -> Result<Vec<Importable>, AgentError> {
        let mut found = self.runner.importable()?;
        found.sort_by(|a, b| b.updated_unix_ms.cmp(&a.updated_unix_ms));
        Ok(found)
    }

    async fn models(&self, backend_config: Option<String>) -> Result<Vec<Model>, AgentError> {
        self.runner.models(backend_config).await
    }

    async fn import(
        &self,
        external: &str,
        name: Option<String>,
    ) -> Result<SessionRecord, AgentError> {
        let id = SessionId::random();
        let imported = self.runner.import(external, &id)?;
        let record = SessionRecord {
            id,
            name: name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(imported.summary),
            working_directory: imported.working_directory,
            model: None,
            instructions: None,
            backend_config: None,
            parent: None,
            approval: ApprovalMode::default(),
            repositories: Vec::new(),
            checkouts: Vec::new(),
            archived: false,
            keeper: false,
        };
        self.create(record.clone(), imported.events)?;
        Ok(record)
    }

    async fn backup(&self, id: &SessionId) -> Result<Backup, AgentError> {
        let session = self.session(id)?;
        let record = session.lock().record.clone();
        let backend = self.runner.backup(&record)?;
        Ok(Backup {
            record,
            events: session.log.events(),
            backend,
        })
    }

    async fn restore(
        &self,
        backup: Backup,
        repositories: Vec<Repository>,
    ) -> Result<SessionRecord, AgentError> {
        let mut record = backup.record;
        let id = record.id.clone();
        let folder = self.root.join("sessions").join(id.as_str());
        if self.lock().contains_key(&id) || folder.exists() {
            return Err(AgentError::SessionExists { session: id });
        }
        if !record.repositories.is_empty() {
            let mut here = Vec::with_capacity(record.repositories.len());
            for wanted in &record.repositories {
                let found = repositories
                    .iter()
                    .find(|repository| repository.name == wanted.name)
                    .ok_or_else(|| AgentError::MissingRepository {
                        session: id.clone(),
                        name: wanted.name.clone(),
                    })?;
                here.push(found.clone());
            }
            record.working_directory = self.workspaces(&id)?.working_directory(id.as_str(), &here);
            record.repositories = here;
        }
        // Its worktrees are made again from its last snapshot.
        record.checkouts = Vec::new();
        let mut events = backup.events;
        events.extend(closing(&events).into_iter().map(Event::now));
        self.runner.restore(&record, &backup.backend)?;
        let session = self.create(record, events)?;
        let archived = session.lock().record.archived;
        if !archived && let Err(error) = prepare(&session, self.workspaces.as_deref()).await {
            // Nothing half made stays, so the restore can be tried again.
            self.lock().remove(&id);
            let record = session.lock().record.clone();
            if let Err(forgetting) = self.runner.forget(&record) {
                eprintln!("humata: couldn't forget the failed restore of {id}: {forgetting}");
            }
            if let Err(removing) = std::fs::remove_dir_all(&session.dir) {
                eprintln!("humata: couldn't remove the failed restore of {id}: {removing}");
            }
            return Err(error);
        }
        let record = session.lock().record.clone();
        Ok(record)
    }

    async fn history(&self, id: &SessionId) -> Result<Vec<Event>, AgentError> {
        Ok(self.session(id)?.log.events())
    }

    async fn summary(&self, id: &SessionId) -> Result<Summary, AgentError> {
        Ok(self.session(id)?.log.summary())
    }

    async fn watch(&self, id: &SessionId) -> Result<Watch, AgentError> {
        let (history, live) = self.session(id)?.log.watch();
        Ok(Watch { history, live })
    }

    async fn sessions(&self) -> Result<Vec<SessionRecord>, AgentError> {
        let mut records: Vec<_> = self
            .lock()
            .values()
            .map(|session| session.lock().record.clone())
            .collect();
        records.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(records)
    }

    async fn update(
        &self,
        id: &SessionId,
        update: SessionUpdate,
    ) -> Result<SessionRecord, AgentError> {
        let session = self.session(id)?;
        let archiving = {
            let mut control = session.lock();
            // Archiving again puts away worktrees an earlier archive that
            // failed half way left behind.
            let owns = control
                .record
                .checkouts
                .iter()
                .any(|checkout| checkout.owner == id.as_str());
            let archiving = update.archived == Some(true) && (!control.record.archived || owns);
            if archiving && control.running.is_some() {
                return Err(AgentError::ArchiveWhileWorking {
                    session: id.clone(),
                });
            }
            let from = control.record.clone();
            let record = SessionRecord {
                name: update.name.unwrap_or(from.name.clone()),
                archived: update.archived.unwrap_or(from.archived),
                keeper: update.keeper.unwrap_or(from.keeper),
                ..from
            };
            save(&session.dir, &record)?;
            control.record = record;
            archiving
        };
        if archiving {
            self.put_away(&session).await?;
        }
        Ok(session.lock().record.clone())
    }

    async fn changes(&self, id: &SessionId, query: &Query) -> Result<Vec<Changes>, AgentError> {
        let session = self.session(id)?;
        let record = session.lock().record.clone();
        if record.repositories.is_empty() {
            return Ok(Vec::new());
        }
        let checkpoints: Vec<Vec<Snapshot>> = session
            .log
            .events()
            .iter()
            .filter_map(|event| event.kind.snapshots())
            .map(<[Snapshot]>::to_vec)
            .collect();
        self.workspaces(id)?
            .changes(
                id.as_str(),
                &record.repositories,
                &checkpoints,
                &record.checkouts,
                query,
            )
            .await
            .map_err(workspace_failed(id))
    }

    async fn edit_changes(&self, id: &SessionId, edit: &Edit) -> Result<(), AgentError> {
        let session = self.session(id)?;
        let checkouts = session.lock().record.checkouts.clone();
        self.workspaces(id)?
            .edit(&checkouts, edit)
            .await
            .map_err(workspace_failed(id))
    }

    async fn land(
        &self,
        id: &SessionId,
        branch: &str,
        message: &str,
    ) -> Result<Vec<Landed>, AgentError> {
        let session = self.session(id)?;
        let _capturing = session.capturing.lock().await;
        let owned = between_turns(
            &session,
            AgentError::LandWhileWorking {
                session: id.clone(),
            },
        )?;
        let landed = humata_workspace::land(&owned, branch, message).await;
        // A land can fail after renaming some branches, so the record takes
        // each worktree's branch from git either way.
        let mut current = Vec::with_capacity(owned.len());
        for checkout in &owned {
            if let Some(name) = humata_workspace::branch_of(&checkout.path).await {
                current.push((checkout.path.clone(), name));
            }
        }
        let mut control = session.lock();
        let mut record = control.record.clone();
        for checkout in &mut record.checkouts {
            if let Some((_, name)) = current.iter().find(|(path, _)| *path == checkout.path) {
                checkout.branch = name.clone();
            }
        }
        save(&session.dir, &record)?;
        control.record = record;
        landed.map_err(workspace_failed(id))
    }

    async fn update_from_base(
        &self,
        id: &SessionId,
    ) -> Result<Vec<(Checkout, String, Merged)>, AgentError> {
        let session = self.session(id)?;
        let _capturing = session.capturing.lock().await;
        let owned = between_turns(
            &session,
            AgentError::UpdateWhileWorking {
                session: id.clone(),
            },
        )?;
        let mut merged = Vec::with_capacity(owned.len());
        for checkout in owned {
            let upstream = humata_workspace::upstream(&checkout)
                .await
                .map_err(workspace_failed(id))?;
            let outcome = humata_workspace::merge(&checkout, &upstream)
                .await
                .map_err(workspace_failed(id))?;
            merged.push((checkout, upstream, outcome));
        }
        Ok(merged)
    }
}

/// The checkouts `session` made, refusing with `working` while a turn runs
/// and when it has no copy of its code.
fn between_turns(session: &Session, working: AgentError) -> Result<Vec<Checkout>, AgentError> {
    let control = session.lock();
    if control.running.is_some() {
        return Err(working);
    }
    let owned: Vec<Checkout> = control
        .record
        .checkouts
        .iter()
        .filter(|checkout| checkout.owner == session.id.as_str())
        .cloned()
        .collect();
    if owned.is_empty() {
        return Err(AgentError::NoCodeToLand {
            session: session.id.clone(),
        });
    }
    Ok(owned)
}

fn workspace_failed(session: &SessionId) -> impl FnOnce(WorkspaceError) -> AgentError + '_ {
    move |source| AgentError::Workspace {
        session: session.clone(),
        source,
    }
}

/// The snapshots of the last checkpoint or save in `events`; none if there
/// is none.
fn last_checkpoint(events: &[Event]) -> Vec<Snapshot> {
    events
        .iter()
        .rev()
        .find_map(|event| event.kind.code())
        .map(<[Snapshot]>::to_vec)
        .unwrap_or_default()
}

/// Makes the session's copy of its repositories, if it works in some and
/// has none: from its last checkpoint, for a fork or a session unarchived,
/// or else from each repository's HEAD, recorded as its first checkpoint.
async fn prepare(session: &Session, workspaces: Option<&Workspaces>) -> Result<(), AgentError> {
    let record = session.lock().record.clone();
    let ready = !record.checkouts.is_empty()
        && record
            .checkouts
            .iter()
            .all(|checkout| checkout.path.is_dir());
    if record.repositories.is_empty() || ready {
        return Ok(());
    }
    let workspaces = workspaces.ok_or_else(|| AgentError::NoWorkspaces {
        session: session.id.clone(),
    })?;
    let failed = workspace_failed(&session.id);
    let history = session.log.events();
    let from = last_checkpoint(&history);
    let made = workspaces
        .create(session.id.as_str(), &record.repositories, &from)
        .await;
    let checkouts = made.map_err(failed)?;
    {
        let mut control = session.lock();
        let record = SessionRecord {
            working_directory: workspaces
                .working_directory(session.id.as_str(), &record.repositories),
            checkouts: checkouts.clone(),
            ..control.record.clone()
        };
        save(&session.dir, &record)?;
        control.record = record;
    }
    if from.is_empty() {
        let turn = checkpoints(&history);
        let snapshots = workspaces
            .snapshot(session.id.as_str(), turn, &checkouts)
            .await
            .map_err(workspace_failed(&session.id))?;
        session
            .log
            .append(Event::now(EventKind::Checkpoint { snapshots }))?;
    }
    Ok(())
}

fn checkpoints(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|event| event.kind.snapshots().is_some())
        .count()
}

/// Snapshots the session's copy of its code; None if it has none.
async fn checkpoint(
    session: &Session,
    workspaces: Option<&Workspaces>,
) -> Result<Option<Vec<Snapshot>>, AgentError> {
    let checkouts = session.lock().record.checkouts.clone();
    if checkouts.is_empty() {
        return Ok(None);
    }
    let workspaces = workspaces.ok_or_else(|| AgentError::NoWorkspaces {
        session: session.id.clone(),
    })?;
    let _capturing = session.capturing.lock().await;
    // The checkpoint has whatever a save still waiting would have.
    session.unsaved.store(false, Ordering::SeqCst);
    let turn = checkpoints(&session.log.events());
    workspaces
        .snapshot(session.id.as_str(), turn, &checkouts)
        .await
        .map(Some)
        .map_err(workspace_failed(&session.id))
}

/// Saves the session's code in the background after a tool call, part way
/// through its turn. Calls that finish while one is being saved are saved
/// together once it's done.
fn save_soon(session: &Arc<Session>, workspaces: Option<Arc<Workspaces>>) {
    let Some(workspaces) = workspaces else {
        return;
    };
    if session.lock().record.checkouts.is_empty() {
        return;
    }
    session.unsaved.store(true, Ordering::SeqCst);
    let session = session.clone();
    tokio::spawn(async move {
        let _capturing = session.capturing.lock().await;
        if !session.unsaved.swap(false, Ordering::SeqCst) {
            return;
        }
        let checkouts = session.lock().record.checkouts.clone();
        let saved = session
            .log
            .events()
            .iter()
            .filter(|event| matches!(event.kind, EventKind::Saved { .. }))
            .count();
        match workspaces
            .save(session.id.as_str(), saved, &checkouts)
            .await
        {
            Ok(snapshots) => session.note(EventKind::Saved { snapshots }),
            // The turn's end still checkpoints the code; a save that
            // couldn't be taken only loses the mid-turn copy.
            Err(source) => eprintln!(
                "humata: {}",
                describe(&AgentError::Workspace {
                    session: session.id.clone(),
                    source,
                })
            ),
        }
    });
}

/// Records a turn's checkpoint, or why it couldn't be taken.
fn note_checkpoint(session: &Session, checkpoint: Result<Option<Vec<Snapshot>>, AgentError>) {
    match checkpoint {
        Ok(Some(snapshots)) => session.note(EventKind::Checkpoint { snapshots }),
        Ok(None) => {}
        Err(error) => session.note(EventKind::Failed {
            message: describe(&error),
        }),
    }
}

/// What closes a history that stops inside a turn: messages still queued
/// leave the queue, since it didn't outlive the process, the turn ends,
/// aborted, and the session goes idle.
fn closing(events: &[Event]) -> Vec<EventKind> {
    let mut queued: Vec<&str> = Vec::new();
    for event in events {
        match &event.kind {
            EventKind::MessageQueued { queue_id, .. } => queued.push(queue_id),
            EventKind::Dequeued { queue_id } => queued.retain(|id| id != queue_id),
            _ => {}
        }
    }
    let mut kinds: Vec<EventKind> = queued
        .into_iter()
        .map(|id| EventKind::Dequeued {
            queue_id: id.to_owned(),
        })
        .collect();
    kinds.extend(closing_turn(events));
    kinds
}

fn closing_turn(events: &[Event]) -> Vec<EventKind> {
    match SessionState::after(events) {
        SessionState::Working | SessionState::WaitingForApproval => {
            vec![EventKind::TurnEnded { aborted: true }, EventKind::Idle]
        }
        SessionState::Idle | SessionState::Failed => {
            if matches!(
                events.last(),
                Some(Event {
                    kind: EventKind::TurnEnded { .. },
                    ..
                })
            ) {
                vec![EventKind::Idle]
            } else {
                Vec::new()
            }
        }
    }
}

fn load(dir: &Path) -> Result<Session, AgentError> {
    let record_path = dir.join(RECORD_FILE);
    let json = std::fs::read(&record_path).map_err(|source| AgentError::Read {
        path: record_path.clone(),
        source,
    })?;
    let record: SessionRecord =
        serde_json::from_slice(&json).map_err(|source| AgentError::CorruptRecord {
            path: record_path,
            source,
        })?;
    let log = EventLog::open(dir.join(EVENTS_FILE))?;
    Ok(Session {
        id: record.id.clone(),
        dir: dir.to_owned(),
        log,
        control: Mutex::new(Control::new(record)),
        capturing: tokio::sync::Mutex::default(),
        unsaved: AtomicBool::new(false),
    })
}

/// Writes the session's record in `dir`, all or nothing.
fn save(dir: &Path, record: &SessionRecord) -> Result<(), AgentError> {
    let path = dir.join(RECORD_FILE);
    let partial = dir.join(format!("{RECORD_FILE}.partial"));
    let json = serde_json::to_vec_pretty(record).map_err(|source| AgentError::Encode { source })?;
    std::fs::write(&partial, json).map_err(|source| AgentError::Write {
        path: partial.clone(),
        source,
    })?;
    std::fs::rename(&partial, &path).map_err(|source| AgentError::Write { path, source })
}

/// The error and its causes, on one line.
pub(crate) fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Waits until the session's history satisfies `done`, and returns it.
    pub(crate) async fn wait_for<A: Agent>(
        agent: &A,
        id: &SessionId,
        done: impl Fn(&[Event]) -> bool,
    ) -> Vec<Event> {
        let mut watch = agent.watch(id).await.expect("the session exists");
        let mut events = watch.history;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(&events) {
            let next = tokio::time::timeout_at(deadline, watch.live.recv()).await;
            match next {
                Ok(Ok(event)) => events.push(event),
                Ok(Err(error)) => panic!("watch failed: {error}"),
                Err(_) => panic!("timed out; events so far: {events:#?}"),
            }
        }
        events
    }

    pub(crate) fn idle_turns(count: usize) -> impl Fn(&[Event]) -> bool {
        move |events| {
            events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::Idle))
                .count()
                >= count
        }
    }

    pub(crate) fn waiting(events: &[Event]) -> bool {
        SessionState::after(events) == SessionState::WaitingForApproval
    }

    pub(crate) fn permission_request(events: &[Event]) -> Option<String> {
        events.iter().rev().find_map(|event| match &event.kind {
            EventKind::PermissionRequested { request_id, .. } => Some(request_id.clone()),
            _ => None,
        })
    }

    #[test]
    fn a_history_cut_inside_a_turn_is_closed_as_aborted() {
        let events = vec![
            Event::now(EventKind::UserMessage { text: "hi".into() }),
            Event::now(EventKind::TurnStarted),
        ];
        assert_eq!(
            closing(&events),
            vec![EventKind::TurnEnded { aborted: true }, EventKind::Idle]
        );
    }

    #[test]
    fn a_message_queued_before_a_cut_is_dropped() {
        let events = vec![
            Event::now(EventKind::UserMessage { text: "hi".into() }),
            Event::now(EventKind::TurnStarted),
            Event::now(EventKind::TurnEnded { aborted: false }),
            Event::now(EventKind::Idle),
            Event::now(EventKind::MessageQueued {
                queue_id: "q1".into(),
                text: "next".into(),
            }),
        ];
        assert_eq!(
            closing(&events),
            vec![EventKind::Dequeued {
                queue_id: "q1".into()
            }]
        );
    }

    #[test]
    fn a_history_cut_between_turns_needs_nothing() {
        let events = vec![
            Event::now(EventKind::UserMessage { text: "hi".into() }),
            Event::now(EventKind::TurnStarted),
            Event::now(EventKind::TurnEnded { aborted: false }),
            Event::now(EventKind::Idle),
        ];
        assert!(closing(&events).is_empty());
        assert!(closing(&[]).is_empty());
    }
}
