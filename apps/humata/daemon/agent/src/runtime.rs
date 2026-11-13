//! [`LocalAgent`]: the [`Agent`] for backends that run in this process.
//!
//! A backend only plays one turn ([`Runner`]); this keeps the sessions, their
//! history on disk, the queue of messages, steering, approvals, aborts and
//! forks.
//!
//! On disk, each session is a folder under `<root>/sessions/<id>` with
//! `session.json` (its [`SessionRecord`]) and `events.jsonl`.

use std::collections::{HashMap, HashSet, VecDeque};
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
    ApprovalMode, Event, EventId, EventKind, ForkOptions, Image, Message, Parent, PermissionKind,
    SendMode, SessionId, SessionRecord, SessionState, SessionUpdate, StartOptions,
};
use crate::import::{Importable, Imported};
use crate::log::EventLog;
use crate::summary::Summary;
use crate::{Agent, FileVersion, Model, Watch};

const RECORD_FILE: &str = "session.json";
const EVENTS_FILE: &str = "events.jsonl";
/// Under a session's folder: each rewind's cut history, by number.
const REWINDS_FOLDER: &str = "rewinds";

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

    /// Called when `session`'s history is cut back to `history`, as its
    /// `rewind`th rewind. A runner that keeps its own state per session
    /// cuts it back to match, keeping what it had for
    /// [`Runner::rewind_undone`].
    fn rewound(
        &self,
        session: &SessionRecord,
        history: &[Event],
        rewind: usize,
    ) -> Result<(), AgentError> {
        let _ = (session, history, rewind);
        Ok(())
    }

    /// Puts back what [`Runner::rewound`] kept for the `rewind`th rewind.
    fn rewind_undone(&self, session: &SessionRecord, rewind: usize) -> Result<(), AgentError> {
        let _ = (session, rewind);
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
    images: Vec<PathBuf>,
    /// Calls running of tools that only read, whose end saves nothing.
    reading: Mutex<HashSet<String>>,
}

impl Turn {
    fn lock_reading(&self) -> MutexGuard<'_, HashSet<String>> {
        self.reading.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The message the turn replies to.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The image files sent with the message, by their names'
    /// extensions png, jpg, gif or webp.
    pub fn images(&self) -> &[PathBuf] {
        &self.images
    }

    /// The folder every image sent to the session is in.
    pub(crate) fn images_folder(&self) -> PathBuf {
        crate::images::folder(&self.session.dir)
    }

    pub fn record(&self) -> SessionRecord {
        self.session.lock().record.clone()
    }

    /// What the session may do without asking, as it is now.
    pub fn approval(&self) -> ApprovalMode {
        self.session.lock().record.approval
    }

    /// Records a request of `kind` the agent's own judge let through, in
    /// [`ApprovalMode::Auto`].
    pub fn allowed_by_judge(
        &self,
        kind: PermissionKind,
        summary: impl Into<String>,
    ) -> Result<(), AgentError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let _control = self.session.lock();
        self.session
            .log
            .append(Event::now(EventKind::PermissionRequested {
                request_id: request_id.clone(),
                kind,
                summary: summary.into(),
            }))?;
        self.session
            .log
            .append(Event::now(EventKind::PermissionCompleted {
                request_id,
                approved: true,
                automatic: true,
                judged: true,
            }))?;
        Ok(())
    }

    /// Every event so far, this turn's included.
    pub fn history(&self) -> Vec<Event> {
        self.session.log.events()
    }

    pub fn emit(&self, kind: EventKind) -> Result<Event, AgentError> {
        // A checkpoint after each change, and where the agent's context
        // is cut, so a rewind to there has the files as they were.
        let saving = match &kind {
            EventKind::ToolStarted { call_id, tool, .. } => {
                if reads_only(tool) {
                    self.lock_reading().insert(call_id.clone());
                }
                false
            }
            EventKind::ToolCompleted {
                call_id, success, ..
            } => !self.lock_reading().remove(call_id) && *success,
            EventKind::Compacted => true,
            _ => false,
        };
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
                        judged: false,
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
    /// File names in the session's images folder.
    images: Vec<String>,
    /// Sent by the person: their next message joins it rather than
    /// queueing behind it. Notices Humata queues never take one in.
    joins: bool,
}

impl Session {
    /// Queues `text` at the back, or the front for a steer between turns,
    /// and says so in the history.
    fn queue(
        &self,
        control: &mut Control,
        text: String,
        images: Vec<String>,
        front: bool,
        joins: bool,
    ) {
        let id = crate::event::EventId::random().to_string();
        self.note(EventKind::MessageQueued {
            queue_id: id.clone(),
            text: text.clone(),
            images: images.clone(),
        });
        let queued = Queued {
            id,
            text,
            images,
            joins,
        };
        if front {
            control.queue.push_front(queued);
        } else {
            control.queue.push_back(queued);
        }
    }

    /// Adds the person's `text` to their message waiting last in the queue,
    /// after a blank line, so what they send while a turn runs goes as one
    /// message. False when nothing of theirs waits last.
    fn join_queued(&self, control: &mut Control, text: &str, images: &[String]) -> bool {
        let Some(last) = control.queue.back_mut().filter(|last| last.joins) else {
            return false;
        };
        last.text = match (last.text.is_empty(), text.is_empty()) {
            (_, true) => last.text.clone(),
            (true, false) => text.to_owned(),
            (false, false) => format!("{}\n\n{text}", last.text),
        };
        last.images.extend_from_slice(images);
        let edited = EventKind::MessageQueued {
            queue_id: last.id.clone(),
            text: last.text.clone(),
            images: last.images.clone(),
        };
        self.note(edited);
        true
    }

    /// Puts `text` in place of the notice waiting in the queue that begins
    /// with the same line, which says what it's about, so only the latest
    /// of a notice sent again waits. False when none waits.
    fn replace_queued(&self, control: &mut Control, text: &str, images: &[String]) -> bool {
        let about = text.lines().next().unwrap_or_default();
        let Some(queued) = control.queue.iter_mut().find(|queued| {
            !queued.joins && queued.text.lines().next().unwrap_or_default() == about
        }) else {
            return false;
        };
        if queued.text == text && queued.images == images {
            return true;
        }
        queued.text = text.to_owned();
        queued.images = images.to_vec();
        let edited = EventKind::MessageQueued {
            queue_id: queued.id.clone(),
            text: queued.text.clone(),
            images: queued.images.clone(),
        };
        self.note(edited);
        true
    }

    /// The next queued message, which leaves the queue to run.
    fn next_queued(&self, control: &mut Control) -> Option<(String, Vec<String>)> {
        let next = control.queue.pop_front()?;
        self.note(EventKind::Dequeued {
            queue_id: next.id,
            dropped: false,
        });
        Some((next.text, next.images))
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
            for kind in stopped(&session.log.events()) {
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
    fn start_driver(
        &self,
        session: &Arc<Session>,
        control: &mut Control,
        text: String,
        images: Vec<String>,
    ) {
        control.generation += 1;
        let generation = control.generation;
        let handle = tokio::spawn(drive(
            session.clone(),
            self.runner.clone(),
            self.workspaces.clone(),
            generation,
            text,
            images,
        ));
        control.running = Some(Running {
            generation,
            handle: Some(handle),
        });
    }
}

/// Plays `text` and `images`, then each queued message, until the queue is empty or the
/// driver is stopped.
async fn drive<R: Runner>(
    session: Arc<Session>,
    runner: Arc<R>,
    workspaces: Option<Arc<Workspaces>>,
    generation: u64,
    text: String,
    images: Vec<String>,
) {
    let (mut text, mut images) = (text, images);
    loop {
        let prepared = prepare(&session, workspaces.as_deref()).await;
        {
            let mut control = session.lock();
            if !control.is_current(generation) {
                return;
            }
            control.turn_open = true;
            session.note(EventKind::UserMessage {
                text: text.clone(),
                images: images.clone(),
            });
            session.note(EventKind::TurnStarted);
        }
        let result = match prepared {
            Ok(()) => runner
                .clone()
                .run_turn(Turn {
                    session: session.clone(),
                    workspaces: workspaces.clone(),
                    text,
                    images: images
                        .iter()
                        .map(|name| crate::images::folder(&session.dir).join(name))
                        .collect(),
                    reading: Mutex::default(),
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
            Some(next) => (text, images) = next,
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
            plugins: options.plugins,
            effort: options.effort.filter(|effort| !effort.is_empty()),
            experimental: options.experimental,
        };
        self.create(record.clone(), Vec::new())?;
        Ok(record)
    }

    async fn send(
        &self,
        id: &SessionId,
        message: Message,
        mode: SendMode,
    ) -> Result<(), AgentError> {
        let session = self.session(id)?;
        // Written before the lock, so a large image never holds up the
        // session's other calls.
        let images = crate::images::save(id, &session.dir, &message.images)?;
        let text = message.text;
        let mut control = session.lock();
        if control.record.archived {
            return Err(AgentError::Archived {
                session: id.clone(),
            });
        }
        if control.running.is_none() {
            self.start_driver(&session, &mut control, text, images);
            return Ok(());
        }
        match mode {
            SendMode::Enqueue => {
                if !session.join_queued(&mut control, &text, &images) {
                    session.queue(&mut control, text, images, false, true);
                }
            }
            SendMode::EnqueueOnce => {
                if !session.replace_queued(&mut control, &text, &images) {
                    session.queue(&mut control, text, images, false, false);
                }
            }
            SendMode::Immediate if control.turn_open && images.is_empty() => {
                session.log.append(Event::now(EventKind::UserMessage {
                    text: text.clone(),
                    images: Vec::new(),
                }))?;
                control.steering.push_back(text);
            }
            // Between turns, while one is being stopped, or with images a
            // running turn can't take: it goes next.
            SendMode::Immediate => session.queue(&mut control, text, images, true, false),
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
            dropped: false,
        });
        Ok(())
    }

    async fn edit_queued(
        &self,
        id: &SessionId,
        queue_id: &str,
        text: String,
    ) -> Result<(), AgentError> {
        let session = self.session(id)?;
        let mut control = session.lock();
        let queued = control
            .queue
            .iter_mut()
            .find(|queued| queued.id == queue_id)
            .ok_or_else(|| AgentError::UnknownQueuedMessage {
                session: id.clone(),
                queued: queue_id.to_owned(),
            })?;
        queued.text = text.clone();
        let images = queued.images.clone();
        session.note(EventKind::MessageQueued {
            queue_id: queue_id.to_owned(),
            text,
            images,
        });
        Ok(())
    }

    async fn read_image(&self, id: &SessionId, name: &str) -> Result<Image, AgentError> {
        let session = self.session(id)?;
        crate::images::read(id, &session.dir, name)
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
                    judged: false,
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
                    judged: false,
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
            Some((next, images)) => {
                control.running = None;
                self.start_driver(&session, &mut control, next, images);
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
        copy_images(
            &parent.dir,
            &self.root.join("sessions").join(record.id.as_str()),
        )?;
        self.create(record.clone(), events)?;
        Ok(record)
    }

    async fn rewind(
        &self,
        id: &SessionId,
        before: &EventId,
        files: bool,
    ) -> Result<(), AgentError> {
        let session = self.session(id)?;
        {
            let mut control = session.lock();
            if control.record.archived {
                return Err(AgentError::Archived {
                    session: id.clone(),
                });
            }
            // What waits to be sent goes with what is cut.
            let queued: Vec<String> = control.queue.drain(..).map(|queued| queued.id).collect();
            for queue_id in queued {
                session.note(EventKind::Dequeued {
                    queue_id,
                    dropped: true,
                });
            }
        }
        self.abort(id).await?;
        let events = session.log.events();
        let index = events
            .iter()
            .position(|event| &event.id == before)
            .ok_or_else(|| AgentError::UnknownEvent {
                session: id.clone(),
                event: before.clone(),
            })?;
        let EventKind::UserMessage { text, images } = events[index].kind.clone() else {
            return Err(AgentError::NotAMessage {
                session: id.clone(),
                event: before.clone(),
            });
        };
        let mut kept = events[..index].to_vec();
        kept.extend(closing(&kept).into_iter().map(Event::now));
        let rewinds = session.dir.join(REWINDS_FOLDER);
        let rewind = count_entries(&rewinds)? + 1;
        let backup = rewinds.join(rewind.to_string());
        std::fs::create_dir_all(&backup).map_err(|source| AgentError::CreateDirectory {
            path: backup.clone(),
            source,
        })?;
        drop(EventLog::create(backup.join(EVENTS_FILE), events)?);
        let record = session.lock().record.clone();
        let snapshots = if files && !record.checkouts.is_empty() {
            if record
                .checkouts
                .iter()
                .any(|checkout| checkout.owner != id.as_str())
            {
                return Err(AgentError::SharedFiles {
                    session: id.clone(),
                });
            }
            let workspaces = self.workspaces(id)?;
            let _capturing = session.capturing.lock().await;
            let saved = workspaces
                .save_rewound(id.as_str(), rewind, &record.checkouts)
                .await
                .map_err(workspace_failed(id))?;
            workspaces
                .restore(&record.checkouts, &last_checkpoint(&kept))
                .await
                .map_err(workspace_failed(id))?;
            saved
        } else {
            Vec::new()
        };
        self.runner.rewound(&record, &kept, rewind)?;
        kept.push(Event::now(EventKind::Rewound {
            rewind,
            text,
            images,
            files,
            snapshots,
        }));
        session.log.replace(kept)
    }

    async fn undo_rewind(&self, id: &SessionId) -> Result<(), AgentError> {
        let session = self.session(id)?;
        let events = session.log.events();
        let nothing = || AgentError::NothingToUndo {
            session: id.clone(),
        };
        let last = events
            .iter()
            .rev()
            .take_while(|event| {
                !matches!(
                    event.kind,
                    EventKind::UserMessage { .. } | EventKind::RewindUndone
                )
            })
            .find_map(|event| match &event.kind {
                EventKind::Rewound {
                    rewind, snapshots, ..
                } => Some((*rewind, snapshots.clone())),
                _ => None,
            });
        let (rewind, snapshots) = last.ok_or_else(nothing)?;
        if session.lock().running.is_some() {
            return Err(nothing());
        }
        let backup = session
            .dir
            .join(REWINDS_FOLDER)
            .join(rewind.to_string())
            .join(EVENTS_FILE);
        let mut restored = EventLog::open(backup)?.events();
        let record = session.lock().record.clone();
        if !snapshots.is_empty() {
            let workspaces = self.workspaces(id)?;
            let _capturing = session.capturing.lock().await;
            workspaces
                .restore(&record.checkouts, &snapshots)
                .await
                .map_err(workspace_failed(id))?;
        }
        self.runner.rewind_undone(&record, rewind)?;
        restored.push(Event::now(EventKind::RewindUndone));
        session.log.replace(restored)
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
            plugins: Default::default(),
            effort: None,
            experimental: false,
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
        events.extend(stopped(&events).into_iter().map(Event::now));
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
                effort: match update.effort {
                    Some(effort) => effort.filter(|effort| !effort.is_empty()),
                    None => from.effort.clone(),
                },
                experimental: update.experimental.unwrap_or(from.experimental),
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

    async fn file_versions(
        &self,
        id: &SessionId,
        path: &str,
    ) -> Result<Vec<FileVersion>, AgentError> {
        let session = self.session(id)?;
        let repositories = session.lock().record.repositories.clone();
        let Some((repository, path)) = in_repository(&repositories, path) else {
            return Ok(Vec::new());
        };
        let (times, checkpoints): (Vec<i64>, Vec<Vec<Snapshot>>) = session
            .log
            .events()
            .iter()
            .filter_map(|event| {
                let snapshots = event.kind.snapshots()?;
                Some((event.time_unix_ms, snapshots.to_vec()))
            })
            .unzip();
        let turns = humata_workspace::versions(repository, &checkpoints, path)
            .await
            .map_err(workspace_failed(id))?;
        Ok(turns
            .into_iter()
            .map(|turn| FileVersion {
                turn,
                time_unix_ms: times.get(turn).copied().unwrap_or_default(),
            })
            .collect())
    }

    async fn file_at(&self, id: &SessionId, path: &str, turn: usize) -> Result<String, AgentError> {
        let session = self.session(id)?;
        let repositories = session.lock().record.repositories.clone();
        let (repository, relative) =
            in_repository(&repositories, path).ok_or_else(|| AgentError::NotSnapshotted {
                session: id.clone(),
                path: path.to_owned(),
            })?;
        let checkpoints: Vec<Vec<Snapshot>> = session
            .log
            .events()
            .iter()
            .filter_map(|event| event.kind.snapshots())
            .map(<[Snapshot]>::to_vec)
            .collect();
        humata_workspace::file_at(repository, &checkpoints, turn, relative)
            .await
            .map_err(workspace_failed(id))
    }
}

/// The repository a path from the session's folder is in, and the path in
/// it: the folder is its one repository's worktree, or holds a worktree of
/// each.
fn in_repository<'a>(
    repositories: &'a [Repository],
    path: &'a str,
) -> Option<(&'a Repository, &'a str)> {
    match repositories {
        [] => None,
        [only] => Some((only, path)),
        _ => {
            let (name, rest) = path.split_once('/')?;
            let repository = repositories
                .iter()
                .find(|repository| repository.name == name)?;
            Some((repository, rest))
        }
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

/// Whether the tool named `tool` only reads, whatever backend's it is, so
/// the code needn't be saved after it.
fn reads_only(tool: &str) -> bool {
    const READING: &[&str] = &[
        // Claude Code's.
        "Read",
        "Grep",
        "Glob",
        "LS",
        "WebFetch",
        "WebSearch",
        "TodoWrite",
        "NotebookRead",
        // Copilot's.
        "view",
        "grep",
        "glob",
        "web_fetch",
        "report_intent",
        "think",
        // The local model's.
        "read_file",
        "list_folder",
        "read_skill",
    ];
    READING.contains(&tool)
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
    let turn = session.log.summary().checkpoints;
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
        let summary = session.log.summary();
        match workspaces
            .save(
                session.id.as_str(),
                summary.saves,
                &checkouts,
                &summary.code,
            )
            .await
        {
            Ok(Some(snapshots)) => session.note(EventKind::Saved { snapshots }),
            // Nothing changed since the last save: there's nothing to keep.
            Ok(None) => {}
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
            EventKind::MessageQueued { queue_id, .. } if !queued.contains(&queue_id.as_str()) => {
                queued.push(queue_id)
            }
            EventKind::Dequeued { queue_id, .. } => queued.retain(|id| id != queue_id),
            _ => {}
        }
    }
    let mut kinds: Vec<EventKind> = queued
        .into_iter()
        .map(|id| EventKind::Dequeued {
            queue_id: id.to_owned(),
            dropped: true,
        })
        .collect();
    kinds.extend(closing_turn(events));
    kinds
}

/// As `closing`, for a session whose turn Humata stopped running, at a
/// restart or a move: a cut turn ends as failed, so the inbox says it
/// stopped rather than that the agent replied.
fn stopped(events: &[Event]) -> Vec<EventKind> {
    let mut kinds = closing(events);
    if let Some(at) = kinds
        .iter()
        .position(|kind| *kind == EventKind::TurnEnded { aborted: true })
    {
        kinds.insert(
            at,
            EventKind::Failed {
                message: "Humata stopped while this turn ran, so it was cut short. Send a message to carry on from where it stopped; check its last changes first, as a tool may have been cut off partway.".to_owned(),
            },
        );
    }
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

/// Copies the images of the session in `from` to the one in `to`, so a
/// fork's messages show theirs; a session sent none has nothing to copy.
/// How many entries the folder at `path` has; none if it isn't there.
fn count_entries(path: &Path) -> Result<usize, AgentError> {
    match std::fs::read_dir(path) {
        Ok(entries) => Ok(entries.count()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(source) => Err(AgentError::ReadDirectory {
            path: path.to_owned(),
            source,
        }),
    }
}

fn copy_images(from: &Path, to: &Path) -> Result<(), AgentError> {
    let from = crate::images::folder(from);
    let entries = match std::fs::read_dir(&from) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(AgentError::ReadImage { path: from, source }),
    };
    let to = crate::images::folder(to);
    std::fs::create_dir_all(&to).map_err(|source| AgentError::SaveImage {
        path: to.clone(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| AgentError::ReadImage {
            path: from.clone(),
            source,
        })?;
        let target = to.join(entry.file_name());
        std::fs::copy(entry.path(), &target).map_err(|source| AgentError::SaveImage {
            path: target,
            source,
        })?;
    }
    Ok(())
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
            Event::now(EventKind::UserMessage {
                text: "hi".into(),
                images: Vec::new(),
            }),
            Event::now(EventKind::TurnStarted),
        ];
        assert_eq!(
            closing(&events),
            vec![EventKind::TurnEnded { aborted: true }, EventKind::Idle]
        );
    }

    #[test]
    fn a_turn_cut_by_a_stop_ends_failed() {
        let mut events = vec![
            Event::now(EventKind::UserMessage {
                text: "hi".into(),
                images: Vec::new(),
            }),
            Event::now(EventKind::TurnStarted),
        ];
        events.extend(stopped(&events).into_iter().map(Event::now));
        assert_eq!(SessionState::after(&events), SessionState::Failed);
        let finished = vec![
            Event::now(EventKind::TurnStarted),
            Event::now(EventKind::TurnEnded { aborted: false }),
            Event::now(EventKind::Idle),
        ];
        assert!(stopped(&finished).is_empty());
    }

    #[test]
    fn a_message_queued_before_a_cut_is_dropped() {
        let events = vec![
            Event::now(EventKind::UserMessage {
                text: "hi".into(),
                images: Vec::new(),
            }),
            Event::now(EventKind::TurnStarted),
            Event::now(EventKind::TurnEnded { aborted: false }),
            Event::now(EventKind::Idle),
            Event::now(EventKind::MessageQueued {
                queue_id: "q1".into(),
                text: "next".into(),
                images: Vec::new(),
            }),
        ];
        assert_eq!(
            closing(&events),
            vec![EventKind::Dequeued {
                queue_id: "q1".into(),
                dropped: true,
            }]
        );
    }

    #[test]
    fn a_history_cut_between_turns_needs_nothing() {
        let events = vec![
            Event::now(EventKind::UserMessage {
                text: "hi".into(),
                images: Vec::new(),
            }),
            Event::now(EventKind::TurnStarted),
            Event::now(EventKind::TurnEnded { aborted: false }),
            Event::now(EventKind::Idle),
        ];
        assert!(closing(&events).is_empty());
        assert!(closing(&[]).is_empty());
    }
}
