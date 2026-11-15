use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tca::{BoxFuture, Clock};
use tokio::sync::mpsc;

use crate::daemon::{Daemon, DaemonError, Event, Events, Operation, Reply, Terminals};
use crate::fixture;
use crate::model::{
    Agent, Answer, ChannelId, Comment, ForkKind, ItemId, ItemStatus, Line, MachineId, Recovery,
    RequestId, Scope, Session, SessionId, Speaker, Status, TransferId,
};
use crate::workspace::Workspace;

mod boards;
mod channels;
mod inbox;
mod machines;
mod review;
mod sessions;
mod terminals;
mod transfers;

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
    archived: HashMap<SessionId, (Session, Vec<SessionId>)>,
    terminal_input: HashMap<(SessionId, usize), Vec<u8>>,
}

struct Subscriber {
    events: mpsc::Sender<Result<Event, DaemonError>>,
    behind: bool,
}

impl FakeDaemon {
    /// A fake holding `workspace`, answering on `clock`.
    #[must_use]
    pub fn new(mut workspace: Workspace, clock: Arc<dyn Clock>) -> Self {
        for session in &workspace.sessions {
            for tab in 0..=session.shells.len() {
                workspace
                    .terminal_bytes
                    .entry((session.id.clone(), tab))
                    .or_insert_with(|| fixture::terminal_screen(session, tab));
            }
        }
        Self {
            shared: Arc::new(Mutex::new(Inner {
                workspace,
                subscribers: Vec::new(),
                failures: HashMap::new(),
                recoveries: fixture::recoveries(),
                calls: Vec::new(),
                next: 1,
                archived: HashMap::new(),
                terminal_input: HashMap::new(),
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
            if !self.workspace.terminal_bytes.contains_key(&(id.clone(), 0)) {
                self.terminal_data(id, 0, fixture::terminal_screen(&session, 0));
            }
            self.emit(Event::Session(session));
        }
    }

    fn terminal_data(&mut self, session: &SessionId, tab: usize, bytes: Vec<u8>) {
        let output = self
            .workspace
            .terminal_bytes
            .entry((session.clone(), tab))
            .or_default();
        let offset = output.len();
        output.extend_from_slice(&bytes);
        self.emit(Event::TerminalData {
            session: session.clone(),
            tab,
            offset,
            bytes,
        });
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
        self.terminal_data(session, tab, fixture::terminal_line(&line));
        self.emit(Event::Output {
            session: session.clone(),
            tab,
            line,
        });
    }
}

impl Daemon for FakeDaemon {
    fn load(&self) -> Reply<Workspace> {
        sessions::load(self)
    }

    fn events(&self) -> Reply<Events> {
        sessions::events(self)
    }

    fn send(&self, session: &SessionId, text: &str) -> Reply<()> {
        sessions::send(self, session, text)
    }

    fn answer(&self, request: &RequestId, answer: Answer) -> Reply<()> {
        inbox::answer(self, request, answer)
    }

    fn reconnect(&self, machine: &MachineId) -> Reply<Vec<(SessionId, Recovery)>> {
        machines::reconnect(self, machine)
    }

    fn resume(&self, session: &SessionId) -> Reply<()> {
        sessions::resume(self, session)
    }

    fn fork(&self, session: &SessionId, kind: ForkKind, title: &str) -> Reply<SessionId> {
        sessions::fork(self, session, kind, title)
    }

    fn archive(&self, session: &SessionId) -> Reply<()> {
        sessions::archive(self, session)
    }

    fn add_machine(&self, address: &str) -> Reply<MachineId> {
        machines::add_machine(self, address)
    }

    fn restore_session(&self, session: &SessionId) -> Reply<()> {
        sessions::restore_session(self, session)
    }

    fn set_pinned(&self, session: &SessionId, pinned: bool) -> Reply<()> {
        sessions::set_pinned(self, session, pinned)
    }

    fn set_vip(&self, session: &SessionId, vip: bool) -> Reply<()> {
        sessions::set_vip(self, session, vip)
    }

    fn mark_read(&self, session: &SessionId) -> Reply<()> {
        sessions::mark_read(self, session)
    }

    fn create_session(
        &self,
        channel: &ChannelId,
        agent: Agent,
        machine: &MachineId,
        title: &str,
        options: crate::model::SessionOptions,
    ) -> Reply<SessionId> {
        sessions::create_session(self, channel, agent, machine, title, options)
    }

    fn create_channel(
        &self,
        name: &str,
        repos: Vec<String>,
        machines: Vec<MachineId>,
    ) -> Reply<ChannelId> {
        channels::create_channel(self, name, repos, machines)
    }

    fn move_item(&self, item: &ItemId, status: ItemStatus) -> Reply<()> {
        boards::move_item(self, item, status)
    }

    fn create_item(&self, scope: Scope, title: &str) -> Reply<ItemId> {
        boards::create_item(self, scope, title)
    }

    fn add_dependency(&self, item: &ItemId, on: &ItemId) -> Reply<()> {
        boards::add_dependency(self, item, on)
    }

    fn send_comments(&self, session: &SessionId, comments: Vec<Comment>) -> Reply<()> {
        review::send_comments(self, session, comments)
    }

    fn read_file(&self, session: &SessionId, repo: &str, path: &str) -> Reply<String> {
        review::read_file(self, session, repo, path)
    }

    fn retry_transfer(&self, transfer: &TransferId) -> Reply<()> {
        transfers::retry_transfer(self, transfer)
    }
}

impl Terminals for FakeDaemon {
    fn open(&self, session: &SessionId, agent: Agent) -> Reply<(usize, Session)> {
        terminals::open(self, session, agent)
    }

    fn write(&self, session: &SessionId, tab: usize, text: &str) -> Reply<()> {
        terminals::write(self, session, tab, text)
    }

    fn input(&self, session: &SessionId, tab: usize, bytes: Vec<u8>) -> Reply<()> {
        terminals::input(self, session, tab, bytes)
    }

    fn resize(&self, session: &SessionId, tab: usize, columns: u16, rows: u16) -> Reply<()> {
        terminals::resize(self, session, tab, columns, rows)
    }
}
