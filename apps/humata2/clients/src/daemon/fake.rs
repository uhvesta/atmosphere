use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tca::{BoxFuture, Clock};
use tokio::sync::mpsc;

use crate::daemon::{
    ArchivePlan, Daemon, DaemonError, Event, Events, ForkOptions, Operation, PastTurn, Reply,
    Terminals,
};
use crate::fixture;
use crate::model::{
    Agent, Answer, Archive, Capability, ChannelId, DiffBase, ForkKind, GrantId, ItemId, ItemStatus,
    Line, MachineId, PushChoice, Recovery, RepoDiff, RequestId, RuleSetting, Scope, Session,
    SessionId, SetupId, Speaker, Status, TransferId,
};
use crate::workspace::Workspace;

mod access;
mod agent_asks;
mod boards;
mod channels;
mod desk;
mod inbox;
mod machines;
mod review;
mod sessions;
mod setup;
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
    archive_frees: HashMap<SessionId, String>,
    unpushed: HashMap<SessionId, Vec<(String, String)>>,
    last_fork: Option<ForkOptions>,
    last_move: Option<(SessionId, MachineId)>,
    last_archive: Option<(SessionId, bool)>,
    terminal_input: HashMap<(SessionId, usize), Vec<u8>>,
    diffs: HashMap<SessionId, Vec<RepoDiff>>,
    reviewed: HashMap<SessionId, Vec<RepoDiff>>,
    pushes: Vec<(SessionId, PushChoice)>,
    pull_requests: HashMap<(SessionId, String), u32>,
    next_pull_request: u32,
    asks_in_inbox: bool,
    codex_needs_trust: bool,
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
                archive_frees: HashMap::new(),
                unpushed: HashMap::new(),
                last_fork: None,
                last_move: None,
                last_archive: None,
                terminal_input: HashMap::new(),
                diffs: HashMap::new(),
                reviewed: HashMap::new(),
                pushes: Vec::new(),
                pull_requests: HashMap::new(),
                next_pull_request: 1,
                asks_in_inbox: true,
                codex_needs_trust: false,
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

    /// Says how much disk archiving `session` will free, as people read it ("1.4 GB").
    /// Makes the daemon say Codex's hooks still need trusting, or not (D202).
    pub fn set_codex_needs_trust(&self, on: bool) {
        self.inner().codex_needs_trust = on;
    }

    pub fn set_archive_frees(&self, session: &SessionId, frees: impl Into<String>) {
        self.inner()
            .archive_frees
            .insert(session.clone(), frees.into());
    }

    /// Says what work in each repository (its name, then what is unpushed)
    /// archiving `session` would delete.
    pub fn set_unpushed(&self, session: &SessionId, work: Vec<(String, String)>) {
        self.inner().unpushed.insert(session.clone(), work);
    }

    /// What the last fork asked for.
    #[must_use]
    pub fn last_fork(&self) -> Option<ForkOptions> {
        self.inner().last_fork.clone()
    }

    /// The session and machine of the last move.
    #[must_use]
    pub fn last_move(&self) -> Option<(SessionId, MachineId)> {
        self.inner().last_move.clone()
    }

    /// The session of the last archive, and whether it asked to commit,
    /// push and open pull requests first.
    #[must_use]
    pub fn last_archive(&self) -> Option<(SessionId, bool)> {
        self.inner().last_archive.clone()
    }

    /// Says what `session`'s changes against origin/main are now.
    pub fn set_diffs(&self, session: &SessionId, diffs: Vec<RepoDiff>) {
        let mut merged: Vec<RepoDiff> = Vec::new();
        for diff in diffs {
            match merged.iter_mut().find(|have| have.repo == diff.repo) {
                Some(have) => have.files.extend(diff.files),
                None => merged.push(diff),
            }
        }
        self.inner().diffs.insert(session.clone(), merged);
    }

    /// Says which pull request is open for `session`'s branch in `repo`.
    pub fn set_open_pull_request(&self, session: &SessionId, repo: &str, number: u32) {
        self.inner()
            .pull_requests
            .insert((session.clone(), repo.to_owned()), number);
    }

    /// Every push choice the app sent, oldest first, with its session.
    #[must_use]
    pub fn pushes(&self) -> Vec<(SessionId, PushChoice)> {
        self.inner().pushes.clone()
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

    fn set_rule(
        &self,
        channel: &ChannelId,
        capability: &Capability,
        setting: RuleSetting,
    ) -> Reply<()> {
        access::set_rule(self, channel, capability, setting)
    }

    fn remove_grant(&self, grant: &GrantId) -> Reply<()> {
        access::remove_grant(self, grant)
    }

    fn asks_in_inbox(&self) -> Reply<bool> {
        agent_asks::asks_in_inbox(self)
    }

    fn set_asks_in_inbox(&self, inbox: bool) -> Reply<bool> {
        agent_asks::set_asks_in_inbox(self, inbox)
    }

    fn codex_needs_trust(&self) -> Reply<bool> {
        agent_asks::codex_needs_trust(self)
    }

    fn reconnect(&self, machine: &MachineId) -> Reply<Vec<(SessionId, Recovery)>> {
        machines::reconnect(self, machine)
    }

    fn resume(&self, session: &SessionId) -> Reply<()> {
        sessions::resume(self, session)
    }

    fn start_fresh(&self, session: &SessionId) -> Reply<()> {
        sessions::start_fresh(self, session)
    }

    fn fork(&self, session: &SessionId, options: ForkOptions) -> Reply<SessionId> {
        sessions::fork(self, session, options)
    }

    fn list_turns(&self, session: &SessionId) -> Reply<Vec<PastTurn>> {
        sessions::list_turns(self, session)
    }

    fn move_session(&self, session: &SessionId, to: &MachineId) -> Reply<()> {
        sessions::move_session(self, session, to)
    }

    fn plan_archive(&self, session: &SessionId) -> Reply<ArchivePlan> {
        sessions::plan_archive(self, session)
    }

    fn archive(&self, session: &SessionId, push_first: bool) -> Reply<Archive> {
        sessions::archive(self, session, push_first)
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

    fn cancel_setup(&self, setup: &SetupId) -> Reply<()> {
        setup::cancel_setup(self, setup)
    }

    fn retry_setup(&self, setup: &SetupId) -> Reply<ChannelId> {
        setup::retry_setup(self, setup)
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

    fn remove_dependency(&self, item: &ItemId, on: &ItemId) -> Reply<()> {
        boards::remove_dependency(self, item, on)
    }

    fn set_item_session(&self, item: &ItemId, session: &SessionId, working: bool) -> Reply<()> {
        boards::set_item_session(self, item, session, working)
    }

    fn item_history(&self, item: &ItemId) -> Reply<Vec<String>> {
        boards::item_history(self, item)
    }

    fn read_file(&self, session: &SessionId, repo: &str, path: &str) -> Reply<String> {
        review::read_file(self, session, repo, path)
    }

    fn diffs(&self, session: &SessionId, base: &DiffBase) -> Reply<Vec<RepoDiff>> {
        review::diffs(self, session, base)
    }

    fn mark_reviewed(&self, session: &SessionId) -> Reply<()> {
        review::mark_reviewed(self, session)
    }

    fn plan_push(&self, session: &SessionId) -> Reply<crate::model::PushPlan> {
        review::plan_push(self, session)
    }

    fn push(
        &self,
        session: &SessionId,
        _token: &str,
        choices: Vec<PushChoice>,
    ) -> Reply<Option<u32>> {
        review::push(self, session, choices)
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
