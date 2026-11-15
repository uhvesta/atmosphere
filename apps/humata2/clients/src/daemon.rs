//! The daemon client: everything the app asks of the daemons, and the
//! events they push. One trait; [`FakeDaemon`] is the only implementation
//! until the real one is wired in over `api/proto`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tca::{BoxFuture, Clock};
use tokio::sync::mpsc;

use crate::fixture;
use crate::model::{
    Agent, Answer, Channel, ChannelId, Comment, ForkKind, Item, ItemId, ItemKind, ItemStatus, Line,
    MachineId, Recovery, RequestId, Scope, Session, SessionId, Speaker, Status, Transfer,
    TransferId, TransferState, Turn,
};
use crate::workspace::Workspace;

/// How many events a subscriber may fall behind before it is told to
/// reload instead (TR-8: queues are bounded).
pub const EVENT_QUEUE: usize = 256;

/// A daemon reply.
pub type Reply<T> = BoxFuture<Result<T, DaemonError>>;

/// The events a subscription delivers, in order.
pub type Events = mpsc::Receiver<Result<Event, DaemonError>>;

/// Something changed on a daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A session changed: its status, turn, title or flags.
    Session(Session),
    /// A session's terminal wrote a line.
    Output {
        /// The session.
        session: SessionId,
        /// The terminal tab: 0 is the agent's.
        tab: usize,
        /// The line.
        line: Line,
    },
    /// A request was added or answered.
    Request(crate::model::Request),
    /// A host went online or offline.
    Machine(crate::model::Machine),
    /// A transfer moved on.
    Transfer(Transfer),
    /// Events were dropped because the subscriber fell behind: reload.
    Resync,
}

/// The operations a fake can be told to fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operation {
    /// [`Daemon::load`].
    Load,
    /// [`Daemon::send`].
    Send,
    /// [`Daemon::answer`].
    Answer,
    /// [`Daemon::reconnect`].
    Reconnect,
    /// [`Daemon::fork`].
    Fork,
    /// [`Daemon::create_session`].
    CreateSession,
    /// [`Daemon::create_channel`].
    CreateChannel,
    /// [`Daemon::move_item`].
    MoveItem,
    /// [`Daemon::send_comments`].
    SendComments,
    /// [`Daemon::resume`].
    Resume,
}

/// Why a daemon call failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DaemonError {
    /// The host doesn't answer.
    #[error(
        "{machine} doesn't answer, so this can't be done there now. Reconnect it from Hosts; its sessions keep running and nothing is lost"
    )]
    Offline {
        /// The host.
        machine: MachineId,
    },
    /// What was asked about is gone.
    #[error(
        "{what} no longer exists. It may have been archived or deleted from another window; reload to see what is there now"
    )]
    NotFound {
        /// What was asked about.
        what: String,
    },
    /// The daemon refused it.
    #[error("{what} was refused: {reason}")]
    Refused {
        /// What was asked.
        what: String,
        /// Why, and what to do instead.
        reason: String,
    },
    /// A fake was told to fail this call.
    #[error("{operation:?} failed: {message}. Try again; a test made this fail on purpose")]
    Scripted {
        /// The call.
        operation: Operation,
        /// What the test said.
        message: String,
    },
}

/// What the app asks of the daemons.
pub trait Daemon: Send + Sync + 'static {
    /// Reads a text file in this session's own repository clone.
    fn read_file(&self, session: &SessionId, repo: &str, path: &str) -> Reply<String>;
    /// Everything there is, merged across hosts.
    fn load(&self) -> Reply<Workspace>;
    /// Follows what changes, from now on.
    fn events(&self) -> Reply<Events>;
    /// Types `text` into the session's agent and presses Enter.
    fn send(&self, session: &SessionId, text: &str) -> Reply<()>;
    /// Answers an Inbox request.
    fn answer(&self, request: &RequestId, answer: Answer) -> Reply<()>;
    /// Connects to a host again, and says what happened to each of its
    /// sessions (PER-26).
    fn reconnect(&self, machine: &MachineId) -> Reply<Vec<(SessionId, Recovery)>>;
    /// Starts an exited session's agent again on its conversation.
    fn resume(&self, session: &SessionId) -> Reply<()>;
    /// Forks a session.
    fn fork(&self, session: &SessionId, kind: ForkKind, title: &str) -> Reply<SessionId>;
    /// Archives a session; its forks keep running.
    fn archive(&self, session: &SessionId) -> Reply<()>;
    /// Pins or unpins a session.
    fn set_pinned(&self, session: &SessionId, pinned: bool) -> Reply<()>;
    /// Marks a session very important, or not.
    fn set_vip(&self, session: &SessionId, vip: bool) -> Reply<()>;
    /// Clears a session's unread mark and its recovery note.
    fn mark_read(&self, session: &SessionId) -> Reply<()>;
    /// Starts a session.
    fn create_session(
        &self,
        channel: &ChannelId,
        agent: Agent,
        machine: &MachineId,
        title: &str,
    ) -> Reply<SessionId>;
    /// Makes a channel.
    fn create_channel(
        &self,
        name: &str,
        repos: Vec<String>,
        machines: Vec<MachineId>,
    ) -> Reply<ChannelId>;
    /// Moves a board item to a column.
    fn move_item(&self, item: &ItemId, status: ItemStatus) -> Reply<()>;
    /// Adds a board item.
    fn create_item(&self, scope: Scope, title: &str) -> Reply<ItemId>;
    /// Makes `item` wait for `on`; refused when that would make a cycle.
    fn add_dependency(&self, item: &ItemId, on: &ItemId) -> Reply<()>;
    /// Sends review comments to the session's agent.
    fn send_comments(&self, session: &SessionId, comments: Vec<Comment>) -> Reply<()>;
    /// Retries a failed transfer.
    fn retry_transfer(&self, transfer: &TransferId) -> Reply<()>;
}

/// The terminals of sessions: typing into tabs other than the agent's.
/// Their output arrives as [`Event::Output`].
pub trait Terminals: Send + Sync + 'static {
    /// Starts a shell, or attaches the one terminal already running this agent.
    fn open(&self, session: &SessionId, agent: Agent) -> Reply<(usize, Session)>;

    /// Types `text` and Enter into tab `tab` of `session`.
    fn write(&self, session: &SessionId, tab: usize, text: &str) -> Reply<()>;
}

/// A daemon in memory, starting from [`fixture::standard`] or any
/// workspace. Every reply waits `latency` on the clock first, and the
/// agent's answer to a message streams in a line every `step`. Tests use a
/// `TestClock` and move it on; with zero durations nothing waits.
#[derive(Clone)]
pub struct FakeDaemon {
    shared: Arc<Mutex<Inner>>,
    clock: Arc<dyn Clock>,
    latency: Duration,
    step: Duration,
}

struct Inner {
    workspace: Workspace,
    subscribers: Vec<Subscriber>,
    failures: HashMap<Operation, String>,
    recoveries: HashMap<SessionId, Recovery>,
    calls: Vec<String>,
    next: u32,
}

struct Subscriber {
    events: mpsc::Sender<Result<Event, DaemonError>>,
    behind: bool,
}

impl FakeDaemon {
    /// A fake holding `workspace`, answering on `clock`.
    #[must_use]
    pub fn new(workspace: Workspace, clock: Arc<dyn Clock>) -> Self {
        Self {
            shared: Arc::new(Mutex::new(Inner {
                workspace,
                subscribers: Vec::new(),
                failures: HashMap::new(),
                recoveries: fixture::recoveries(),
                calls: Vec::new(),
                next: 1,
            })),
            clock,
            latency: Duration::ZERO,
            step: Duration::ZERO,
        }
    }

    /// Waits `latency` before every reply.
    #[must_use]
    pub fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = latency;
        self
    }

    /// Streams the agent's answer a line every `step`.
    #[must_use]
    pub fn with_step(mut self, step: Duration) -> Self {
        self.step = step;
        self
    }

    /// Makes the next call of `operation` fail with `message`.
    pub fn fail_next(&self, operation: Operation, message: impl Into<String>) {
        self.inner().failures.insert(operation, message.into());
    }

    /// Says what reconnecting will do to `session`.
    pub fn set_recovery(&self, session: &SessionId, recovery: Recovery) {
        self.inner().recoveries.insert(session.clone(), recovery);
    }

    /// The calls made so far, one line each, oldest first.
    #[must_use]
    pub fn calls(&self) -> Vec<String> {
        self.inner().calls.clone()
    }

    /// What it holds now.
    #[must_use]
    pub fn workspace(&self) -> Workspace {
        self.inner().workspace.clone()
    }

    /// Takes a host offline, as a closed lid would: its sessions show as
    /// offline and an event says so.
    pub fn take_offline(&self, machine: &MachineId) {
        let mut inner = self.inner();
        let mut events = Vec::new();
        if let Some(found) = inner
            .workspace
            .machines
            .iter_mut()
            .find(|found| &found.id == machine)
        {
            found.online = false;
            found.last_seen = "just now".to_owned();
            events.push(Event::Machine(found.clone()));
        }
        for session in inner
            .workspace
            .sessions
            .iter_mut()
            .filter(|session| &session.machine == machine)
        {
            session.status = Status::Offline;
            events.push(Event::Session(session.clone()));
        }
        for event in events {
            inner.emit(event);
        }
    }

    /// Pushes an event to every subscriber, as a daemon would.
    pub fn emit(&self, event: Event) {
        self.inner().emit(event);
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        match self.shared.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Runs `work` after the latency, unless the operation was told to fail.
    fn reply<T: Send + 'static>(
        &self,
        operation: Option<Operation>,
        call: String,
        work: impl FnOnce(&mut Inner) -> Result<T, DaemonError> + Send + 'static,
    ) -> Reply<T> {
        let shared = self.shared.clone();
        let wait = self.wait(self.latency);
        Box::pin(async move {
            wait.await;
            let mut inner = match shared.lock() {
                Ok(inner) => inner,
                Err(poisoned) => poisoned.into_inner(),
            };
            inner.calls.push(call);
            if let Some(operation) = operation
                && let Some(message) = inner.failures.remove(&operation)
            {
                return Err(DaemonError::Scripted { operation, message });
            }
            work(&mut inner)
        })
    }

    fn wait(&self, duration: Duration) -> BoxFuture<()> {
        if duration.is_zero() {
            Box::pin(async {})
        } else {
            self.clock.sleep(duration)
        }
    }
}

impl Inner {
    fn emit(&mut self, event: Event) {
        self.subscribers.retain_mut(|subscriber| {
            if subscriber.behind {
                match subscriber.events.try_send(Ok(Event::Resync)) {
                    Ok(()) => subscriber.behind = false,
                    Err(mpsc::error::TrySendError::Full(_)) => return true,
                    Err(mpsc::error::TrySendError::Closed(_)) => return false,
                }
                return true;
            }
            match subscriber.events.try_send(Ok(event.clone())) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    subscriber.behind = true;
                    true
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
        });
    }

    fn id(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}-{}", self.next);
        self.next += 1;
        id
    }

    fn session_mut(&mut self, id: &SessionId) -> Result<&mut Session, DaemonError> {
        let machines = &self.workspace.machines;
        let session = self
            .workspace
            .sessions
            .iter_mut()
            .find(|session| &session.id == id)
            .ok_or_else(|| DaemonError::NotFound {
                what: format!("Session {id}"),
            })?;
        let online = machines
            .iter()
            .find(|machine| machine.id == session.machine)
            .is_some_and(|machine| machine.online);
        if !online {
            return Err(DaemonError::Offline {
                machine: session.machine.clone(),
            });
        }
        Ok(session)
    }

    fn changed(&mut self, id: &SessionId) {
        if let Some(session) = self.workspace.session(id).cloned() {
            self.emit(Event::Session(session));
        }
    }

    fn output(&mut self, session: &SessionId, tab: usize, from: Speaker, text: &str) {
        let line = Line {
            from,
            text: text.to_owned(),
        };
        if tab == 0
            && let Some(found) = self
                .workspace
                .sessions
                .iter_mut()
                .find(|found| &found.id == session)
        {
            found.transcript.push(line.clone());
        }
        self.emit(Event::Output {
            session: session.clone(),
            tab,
            line,
        });
    }
}

/// The agent's answer to `text`, a line at a time.
fn answer_lines(text: &str) -> Vec<String> {
    vec![
        format!("Looking into “{}”.", text.trim()),
        "Read src/lib.rs (212 lines)".to_owned(),
        "Done. Anything else?".to_owned(),
    ]
}

fn would_cycle(items: &[Item], item: &ItemId, on: &ItemId) -> bool {
    // A cycle exists when `item` is already reachable from `on`.
    let mut stack = vec![on.clone()];
    let mut seen = Vec::new();
    while let Some(next) = stack.pop() {
        if &next == item {
            return true;
        }
        if seen.contains(&next) {
            continue;
        }
        if let Some(found) = items.iter().find(|found| found.id == next) {
            stack.extend(found.depends_on.iter().cloned());
        }
        seen.push(next);
    }
    false
}

impl Daemon for FakeDaemon {
    fn load(&self) -> Reply<Workspace> {
        self.reply(Some(Operation::Load), "load".to_owned(), |inner| {
            Ok(inner.workspace.clone())
        })
    }

    fn events(&self) -> Reply<Events> {
        self.reply(None, "events".to_owned(), |inner| {
            let (events, receiver) = mpsc::channel(EVENT_QUEUE);
            inner.subscribers.push(Subscriber {
                events,
                behind: false,
            });
            Ok(receiver)
        })
    }

    fn send(&self, session: &SessionId, text: &str) -> Reply<()> {
        let id = session.clone();
        let text = text.to_owned();
        let shared = self.shared.clone();
        let step = self.step;
        let this = self.clone();
        let ack = self.reply(Some(Operation::Send), format!("send {session} {text:?}"), {
            let id = id.clone();
            let text = text.clone();
            move |inner| {
                let found = inner.session_mut(&id)?;
                if !found.status.accepts_input() {
                    return Err(DaemonError::Refused {
                        what: format!("Typing into {id}"),
                        reason: "the session has exited. Resume it first".to_owned(),
                    });
                }
                found.status = Status::Working;
                found.turn = Turn::Running;
                inner.changed(&id);
                inner.output(&id, 0, Speaker::You, &text);
                Ok(())
            }
        });
        Box::pin(async move {
            ack.await?;
            for line in answer_lines(&text) {
                this.wait(step).await;
                let mut inner = match shared.lock() {
                    Ok(inner) => inner,
                    Err(poisoned) => poisoned.into_inner(),
                };
                let from = if line.starts_with("Read ") {
                    Speaker::Tool
                } else {
                    Speaker::Agent
                };
                inner.output(&id, 0, from, &line);
            }
            let mut inner = match shared.lock() {
                Ok(inner) => inner,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(found) = inner
                .workspace
                .sessions
                .iter_mut()
                .find(|found| found.id == id)
            {
                found.status = Status::Idle;
                found.turn = Turn::Ended;
            }
            inner.changed(&id);
            Ok(())
        })
    }

    fn answer(&self, request: &RequestId, answer: Answer) -> Reply<()> {
        let id = request.clone();
        self.reply(
            Some(Operation::Answer),
            format!("answer {request} {answer:?}"),
            move |inner| {
                let found = inner
                    .workspace
                    .requests
                    .iter_mut()
                    .find(|found| found.id == id)
                    .ok_or_else(|| DaemonError::NotFound {
                        what: format!("Request {id}"),
                    })?;
                if found.answer.is_some() {
                    return Err(DaemonError::Refused {
                        what: format!("Answering “{}”", found.title),
                        reason: "it was already answered, perhaps from another window".to_owned(),
                    });
                }
                found.answer = Some(answer);
                let request = found.clone();
                let session = request.session.clone();
                inner.emit(Event::Request(request));
                let still = inner
                    .workspace
                    .waiting()
                    .any(|waiting| waiting.session == session);
                if !still
                    && let Some(found) = inner
                        .workspace
                        .sessions
                        .iter_mut()
                        .find(|found| found.id == session)
                    && found.status == Status::NeedsYou
                {
                    found.status = Status::Working;
                    inner.changed(&session);
                }
                Ok(())
            },
        )
    }

    fn reconnect(&self, machine: &MachineId) -> Reply<Vec<(SessionId, Recovery)>> {
        let id = machine.clone();
        self.reply(
            Some(Operation::Reconnect),
            format!("reconnect {machine}"),
            move |inner| {
                let found = inner
                    .workspace
                    .machines
                    .iter_mut()
                    .find(|found| found.id == id)
                    .ok_or_else(|| DaemonError::NotFound {
                        what: format!("Host {id}"),
                    })?;
                found.online = true;
                found.last_seen = "now".to_owned();
                let machine = found.clone();
                inner.emit(Event::Machine(machine));
                let mut outcomes = Vec::new();
                let recoveries = inner.recoveries.clone();
                let ids: Vec<SessionId> = inner
                    .workspace
                    .sessions
                    .iter()
                    .filter(|session| session.machine == id && !session.archived)
                    .map(|session| session.id.clone())
                    .collect();
                for session in ids {
                    let recovery = recoveries
                        .get(&session)
                        .cloned()
                        .unwrap_or(Recovery::Reattached);
                    if let Some(found) = inner
                        .workspace
                        .sessions
                        .iter_mut()
                        .find(|found| found.id == session)
                    {
                        found.status = match recovery {
                            Recovery::Reattached => Status::Working,
                            Recovery::Resumed => Status::Idle,
                            Recovery::Recoverable { .. } => Status::Exited { code: 137 },
                        };
                        found.turn = Turn::Ended;
                        found.recovery = Some(recovery.clone());
                    }
                    inner.changed(&session);
                    outcomes.push((session, recovery));
                }
                Ok(outcomes)
            },
        )
    }

    fn resume(&self, session: &SessionId) -> Reply<()> {
        let id = session.clone();
        self.reply(
            Some(Operation::Resume),
            format!("resume {session}"),
            move |inner| {
                let found = inner.session_mut(&id)?;
                found.status = Status::Idle;
                found.turn = Turn::Ended;
                found.recovery = None;
                let agent = found.agent;
                inner.changed(&id);
                inner.output(
                    &id,
                    0,
                    Speaker::System,
                    &format!("{} resumed on its conversation", agent.name()),
                );
                Ok(())
            },
        )
    }

    fn fork(&self, session: &SessionId, kind: ForkKind, title: &str) -> Reply<SessionId> {
        let parent = session.clone();
        let title = title.to_owned();
        self.reply(
            Some(Operation::Fork),
            format!("fork {session} {kind:?} {title:?}"),
            move |inner| {
                let source = inner.session_mut(&parent)?.clone();
                let id = SessionId::new(inner.id("s-fork"));
                let mut fork = source.clone();
                fork.id = id.clone();
                fork.parent = Some(parent.clone());
                fork.fork = Some(kind);
                fork.title = title.clone();
                fork.status = Status::Idle;
                fork.turn = Turn::Ended;
                fork.pinned = false;
                fork.vip = false;
                fork.unread = 0;
                fork.branch = format!("humata/{id}");
                fork.transcript = vec![Line {
                    from: Speaker::System,
                    text: format!("Forked from “{}” ({})", source.title, kind.name()),
                }];
                fork.shells = Vec::new();
                fork.recovery = None;
                let at = inner
                    .workspace
                    .sessions
                    .iter()
                    .position(|found| found.id == parent)
                    .map_or(inner.workspace.sessions.len(), |at| at + 1);
                if let Some(files) = inner.workspace.session_files.get(&parent).cloned() {
                    inner.workspace.session_files.insert(id.clone(), files);
                }
                inner.workspace.sessions.insert(at, fork);
                inner.changed(&id);
                Ok(id)
            },
        )
    }

    fn archive(&self, session: &SessionId) -> Reply<()> {
        let id = session.clone();
        self.reply(None, format!("archive {session}"), move |inner| {
            let parent = {
                let found = inner
                    .workspace
                    .sessions
                    .iter_mut()
                    .find(|found| found.id == id)
                    .ok_or_else(|| DaemonError::NotFound {
                        what: format!("Session {id}"),
                    })?;
                found.archived = true;
                found.pinned = false;
                found.parent.clone()
            };
            // FK-13: its forks move up to its parent.
            for child in inner
                .workspace
                .sessions
                .iter_mut()
                .filter(|child| child.parent.as_ref() == Some(&id))
            {
                child.parent.clone_from(&parent);
            }
            inner.emit(Event::Resync);
            Ok(())
        })
    }

    fn set_pinned(&self, session: &SessionId, pinned: bool) -> Reply<()> {
        let id = session.clone();
        self.reply(None, format!("pin {session} {pinned}"), move |inner| {
            let found = inner
                .workspace
                .sessions
                .iter_mut()
                .find(|found| found.id == id)
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("Session {id}"),
                })?;
            found.pinned = pinned;
            inner.changed(&id);
            Ok(())
        })
    }

    fn set_vip(&self, session: &SessionId, vip: bool) -> Reply<()> {
        let id = session.clone();
        self.reply(None, format!("vip {session} {vip}"), move |inner| {
            let found = inner
                .workspace
                .sessions
                .iter_mut()
                .find(|found| found.id == id)
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("Session {id}"),
                })?;
            found.vip = vip;
            inner.changed(&id);
            Ok(())
        })
    }

    fn mark_read(&self, session: &SessionId) -> Reply<()> {
        let id = session.clone();
        self.reply(None, format!("read {session}"), move |inner| {
            if let Some(found) = inner
                .workspace
                .sessions
                .iter_mut()
                .find(|found| found.id == id)
                && (found.unread > 0 || found.recovery.is_some())
            {
                found.unread = 0;
                found.recovery = None;
                inner.changed(&id);
            }
            Ok(())
        })
    }

    fn create_session(
        &self,
        channel: &ChannelId,
        agent: Agent,
        machine: &MachineId,
        title: &str,
    ) -> Reply<SessionId> {
        let channel = channel.clone();
        let machine = machine.clone();
        let title = title.to_owned();
        self.reply(
            Some(Operation::CreateSession),
            format!("create session {channel} {agent:?} {machine} {title:?}"),
            move |inner| {
                let online = inner
                    .workspace
                    .machine(&machine)
                    .is_some_and(|found| found.online);
                if !online {
                    return Err(DaemonError::Offline { machine });
                }
                let repo = inner
                    .workspace
                    .channel(&channel)
                    .and_then(|found| found.repos.first().cloned())
                    .unwrap_or_default();
                let id = SessionId::new(inner.id("s-new"));
                inner.workspace.sessions.push(Session {
                    id: id.clone(),
                    channel,
                    parent: None,
                    fork: None,
                    title: title.clone(),
                    agent,
                    machine,
                    status: Status::Idle,
                    turn: Turn::Ended,
                    pinned: false,
                    vip: false,
                    unread: 0,
                    repo,
                    branch: format!("humata/{id}"),
                    started: "just now".to_owned(),
                    active: "just now".to_owned(),
                    transcript: vec![Line {
                        from: Speaker::System,
                        text: format!("{} started · {title}", agent.name()),
                    }],
                    shells: Vec::new(),
                    terminal_agents: Default::default(),
                    archived: false,
                    recovery: None,
                });
                inner.changed(&id);
                Ok(id)
            },
        )
    }

    fn create_channel(
        &self,
        name: &str,
        repos: Vec<String>,
        machines: Vec<MachineId>,
    ) -> Reply<ChannelId> {
        let name = name.trim().to_lowercase().replace(' ', "-");
        self.reply(
            Some(Operation::CreateChannel),
            format!("create channel {name:?}"),
            move |inner| {
                if name.is_empty() {
                    return Err(DaemonError::Refused {
                        what: "Creating a channel".to_owned(),
                        reason: "it needs a name. Type one and try again".to_owned(),
                    });
                }
                if inner
                    .workspace
                    .channels
                    .iter()
                    .any(|found| found.name == name)
                {
                    return Err(DaemonError::Refused {
                        what: format!("Creating #{name}"),
                        reason: "a channel with that name exists. Pick another name".to_owned(),
                    });
                }
                let id = ChannelId::new(name.clone());
                inner.workspace.channels.push(Channel {
                    id: id.clone(),
                    name,
                    repos,
                    machines,
                    purpose: String::new(),
                });
                inner.emit(Event::Resync);
                Ok(id)
            },
        )
    }

    fn move_item(&self, item: &ItemId, status: ItemStatus) -> Reply<()> {
        let id = item.clone();
        self.reply(
            Some(Operation::MoveItem),
            format!("move {item} {status:?}"),
            move |inner| {
                let found = inner
                    .workspace
                    .items
                    .iter_mut()
                    .find(|found| found.id == id)
                    .ok_or_else(|| DaemonError::NotFound {
                        what: format!("Item {id}"),
                    })?;
                found.status = status;
                inner.emit(Event::Resync);
                Ok(())
            },
        )
    }

    fn create_item(&self, scope: Scope, title: &str) -> Reply<ItemId> {
        let title = title.trim().to_owned();
        self.reply(
            None,
            format!("create item {scope:?} {title:?}"),
            move |inner| {
                if title.is_empty() {
                    return Err(DaemonError::Refused {
                        what: "Adding an item".to_owned(),
                        reason: "it needs a title. Type one and try again".to_owned(),
                    });
                }
                let id = ItemId::new(inner.id("N"));
                inner.workspace.items.push(Item {
                    id: id.clone(),
                    scope,
                    kind: ItemKind::Task,
                    title,
                    status: ItemStatus::Todo,
                    depends_on: Vec::new(),
                    session: None,
                    parent: None,
                });
                inner.emit(Event::Resync);
                Ok(id)
            },
        )
    }

    fn add_dependency(&self, item: &ItemId, on: &ItemId) -> Reply<()> {
        let item = item.clone();
        let on = on.clone();
        self.reply(None, format!("depend {item} on {on}"), move |inner| {
            if item == on || would_cycle(&inner.workspace.items, &item, &on) {
                return Err(DaemonError::Refused {
                    what: format!("Making {item} wait for {on}"),
                    reason: format!(
                        "{on} already waits for {item}, so neither could start. Remove one of the two dependencies"
                    ),
                });
            }
            let found = inner
                .workspace
                .items
                .iter_mut()
                .find(|found| found.id == item)
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("Item {item}"),
                })?;
            if !found.depends_on.contains(&on) {
                found.depends_on.push(on);
            }
            inner.emit(Event::Resync);
            Ok(())
        })
    }

    fn send_comments(&self, session: &SessionId, comments: Vec<Comment>) -> Reply<()> {
        let id = session.clone();
        self.reply(
            Some(Operation::SendComments),
            format!("comments {session} {}", comments.len()),
            move |inner| {
                inner.session_mut(&id)?;
                let mut text = format!("{} review comments:", comments.len());
                for comment in &comments {
                    text.push_str(&format!(
                        " {}:{} {};",
                        comment.path, comment.line, comment.body
                    ));
                }
                for diff in inner
                    .workspace
                    .diffs
                    .iter_mut()
                    .filter(|diff| diff.session == id)
                {
                    for comment in &comments {
                        if diff.files.iter().any(|file| file.path == comment.path) {
                            let mut sent = comment.clone();
                            sent.sent = true;
                            diff.comments.push(sent);
                        }
                    }
                }
                inner.output(&id, 0, Speaker::You, &text);
                Ok(())
            },
        )
    }

    fn read_file(&self, session: &SessionId, repo: &str, path: &str) -> Reply<String> {
        let key = (session.clone(), repo.to_owned(), path.to_owned());
        self.reply(
            None,
            format!("read {session} {repo}/{path}"),
            move |inner| {
                inner.session_mut(&key.0)?;
                inner
                    .workspace
                    .file_contents
                    .get(&key)
                    .cloned()
                    .ok_or_else(|| DaemonError::NotFound {
                        what: format!("File {}/{} in {}", key.1, key.2, key.0),
                    })
            },
        )
    }

    fn retry_transfer(&self, transfer: &TransferId) -> Reply<()> {
        let id = transfer.clone();
        self.reply(None, format!("retry {transfer}"), move |inner| {
            let found = inner
                .workspace
                .transfers
                .iter_mut()
                .find(|found| found.id == id)
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("Transfer {id}"),
                })?;
            found.state = TransferState::Copying(0);
            let transfer = found.clone();
            inner.emit(Event::Transfer(transfer));
            Ok(())
        })
    }
}

impl Terminals for FakeDaemon {
    fn open(&self, session: &SessionId, agent: Agent) -> Reply<(usize, Session)> {
        let id = session.clone();
        self.reply(
            None,
            format!("open {session} {}", agent.command()),
            move |inner| {
                let session = inner.session_mut(&id)?;
                let existing = if agent == Agent::Shell {
                    None
                } else if session.agent == agent {
                    Some(0)
                } else {
                    session
                        .terminal_agents
                        .iter()
                        .find_map(|(tab, found)| (*found == agent).then_some(*tab))
                };
                let tab = existing.unwrap_or_else(|| {
                    session.shells.push(agent.command().to_owned());
                    let tab = session.shells.len();
                    if agent != Agent::Shell {
                        session.terminal_agents.insert(tab, agent);
                    }
                    tab
                });
                let snapshot = session.clone();
                inner.changed(&id);
                if existing.is_none() {
                    for line in fixture::extra_terminal(agent, &snapshot.repo, &snapshot.branch) {
                        inner.output(&id, tab, line.from, &line.text);
                    }
                }
                Ok((tab, snapshot))
            },
        )
    }

    fn write(&self, session: &SessionId, tab: usize, text: &str) -> Reply<()> {
        let id = session.clone();
        let text = text.to_owned();
        self.reply(
            None,
            format!("write {session} {tab} {text:?}"),
            move |inner| {
                inner.session_mut(&id)?;
                inner.output(&id, tab, Speaker::You, &format!("$ {text}"));
                let echo = match text.trim() {
                    "ls" => "notes.md  todo.txt".to_owned(),
                    "pwd" => "~/humata".to_owned(),
                    "" => String::new(),
                    other => format!("zsh: command not found: {other}"),
                };
                if !echo.is_empty() {
                    inner.output(&id, tab, Speaker::Tool, &echo);
                }
                Ok(())
            },
        )
    }
}
