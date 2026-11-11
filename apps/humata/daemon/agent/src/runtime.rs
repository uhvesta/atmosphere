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
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::error::AgentError;
use humata_workspace::{Changes, Snapshot, WorkspaceError, Workspaces};

use crate::event::{
    ApprovalMode, Event, EventKind, ForkOptions, Parent, PermissionKind, SendMode, SessionId,
    SessionRecord, SessionState, SessionUpdate, StartOptions,
};
use crate::log::EventLog;
use crate::{Agent, Watch};

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
}

/// What a [`Runner`] uses to record a turn and to ask for permission.
pub struct Turn {
    session: Arc<Session>,
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
        self.session.log.append(Event::now(kind))
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
}

struct Session {
    id: SessionId,
    dir: PathBuf,
    log: EventLog,
    control: Mutex<Control>,
}

struct Control {
    record: SessionRecord,
    running: Option<Running>,
    /// Bumped for every driver, and on abort, so a stopped driver can tell.
    generation: u64,
    /// Between a turn's start and its end.
    turn_open: bool,
    queue: VecDeque<String>,
    steering: VecDeque<String>,
    pending: HashMap<String, Pending>,
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
    /// aborted turn.
    pub fn open(root: impl Into<PathBuf>, runner: R) -> Result<Self, AgentError> {
        let root = root.into();
        let dir = root.join("sessions");
        std::fs::create_dir_all(&dir).map_err(|source| AgentError::CreateDirectory {
            path: dir.clone(),
            source,
        })?;
        let mut sessions = HashMap::new();
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
            let session = load(&path)?;
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
        match control.queue.pop_front() {
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
            SendMode::Enqueue => control.queue.push_back(text),
            SendMode::Immediate if control.turn_open => {
                session
                    .log
                    .append(Event::now(EventKind::UserMessage { text: text.clone() }))?;
                control.steering.push_back(text);
            }
            // Between turns, or while one is being stopped: it goes next.
            SendMode::Immediate => control.queue.push_front(text),
        }
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
        match control.queue.pop_front() {
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
        let (working_directory, checkouts) = if from.repositories.is_empty() {
            (from.working_directory.clone(), Vec::new())
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
            checkouts,
            archived: false,
            keeper: false,
            ..from
        };
        self.runner.forked(id, &record, &events)?;
        self.create(record.clone(), events)?;
        Ok(record)
    }

    async fn history(&self, id: &SessionId) -> Result<Vec<Event>, AgentError> {
        Ok(self.session(id)?.log.events())
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
            let archiving = update.archived == Some(true) && !control.record.archived;
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

    async fn changes(&self, id: &SessionId) -> Result<Vec<Changes>, AgentError> {
        let session = self.session(id)?;
        let record = session.lock().record.clone();
        if record.repositories.is_empty() {
            return Ok(Vec::new());
        }
        let history = session.log.events();
        let base = history
            .iter()
            .find_map(|event| event.kind.snapshots())
            .map(<[Snapshot]>::to_vec)
            .unwrap_or_default();
        self.workspaces(id)?
            .changes(
                id.as_str(),
                &record.repositories,
                &base,
                &last_checkpoint(&history),
                &record.checkouts,
            )
            .await
            .map_err(workspace_failed(id))
    }
}

fn workspace_failed(session: &SessionId) -> impl FnOnce(WorkspaceError) -> AgentError + '_ {
    move |source| AgentError::Workspace {
        session: session.clone(),
        source,
    }
}

/// The snapshots of the last checkpoint in `events`; none if there is none.
fn last_checkpoint(events: &[Event]) -> Vec<Snapshot> {
    events
        .iter()
        .rev()
        .find_map(|event| event.kind.snapshots())
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
    let turn = checkpoints(&session.log.events());
    workspaces
        .snapshot(session.id.as_str(), turn, &checkouts)
        .await
        .map(Some)
        .map_err(workspace_failed(&session.id))
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

/// What closes a history that stops inside a turn: the turn ends, aborted,
/// and the session goes idle.
fn closing(events: &[Event]) -> Vec<EventKind> {
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
fn describe(error: &dyn std::error::Error) -> String {
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
