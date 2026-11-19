use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tca::{BoxFuture, Clock};
use tokio::sync::{mpsc, watch};

use crate::daemon::{
    AgentSwitch, ArchivePlan, Daemon, DaemonError, Event, Events, ForkOptions, Operation, PastTurn, Reply,
    SwitchChange, TerminalNotice, Terminals,
};
use crate::fixture;
use crate::model::{
    Agent, Answer, Archive, Capability, ChannelId, DiffBase, GrantId, ItemId, ItemStatus, Line,
    MachineId, PushChoice, Recovery, RepoBases, RepoDiff, RequestId, RuleSetting, Scope, Session,
    SessionId, SetupId, Speaker, Status, TransferId,
};
use crate::workspace::Workspace;

mod access;
mod agent_asks;
mod agent_switches;
mod boards;
mod channels;
// REPOS (CH-6): Add a GitHub repo to a channel.
mod channel_repos;
mod desk;
mod docs;
mod inbox;
mod machines;
// REPOS (RL-1): the repo library and search.
mod repos;
mod review;
// D784: running holders, listed and ended.
mod running;
// REPOS (CH-21)
mod session_repos;
mod sessions;
mod setup;
mod stop;
mod terminal_link;
mod terminals;
mod transfers;
mod views;

pub use views::FakeViews;

/// A daemon in memory, starting from [`fixture::standard`] or any
/// workspace. Every reply waits `latency` on the clock first, and the
/// agent's answer to a message streams in a line every `step`. Tests use a
/// `TestClock` and move it on; with zero durations nothing waits.
#[derive(Clone)]
pub struct FakeDaemon {
    shared: Arc<Mutex<Inner>>,
    /// The operations held, each with the latch its replies wait on.
    holds: Arc<Mutex<HashMap<Operation, watch::Sender<bool>>>>,
    clock: Arc<dyn Clock>,
    latency: Duration,
    step: Duration,
}

struct Inner {
    workspace: Workspace,
    /// The Desk as subscribers last heard it, to tell what changed (DESKWATCH).
    desk_told: crate::desk::DeskData,
    subscribers: Vec<Subscriber>,
    failures: HashMap<Operation, DaemonError>,
    recoveries: HashMap<SessionId, Recovery>,
    calls: Vec<String>,
    /// Every answer sent, in order, whether or not it worked.
    answers: Vec<(RequestId, Answer)>,
    next: u32,
    archived: HashMap<SessionId, (Session, Vec<SessionId>)>,
    archive_frees: HashMap<SessionId, String>,
    unpushed: HashMap<SessionId, Vec<(String, String)>>,
    unsubmitted_drafts: HashMap<SessionId, u32>,
    /// Repositories whose clone has no default branch, until found.
    no_default: Vec<(SessionId, String)>,
    last_fork: Option<ForkOptions>,
    last_move: Option<(SessionId, MachineId)>,
    last_continue: Option<(SessionId, ChannelId, Option<u32>)>,
    last_archive: Option<(SessionId, bool)>,
    terminal_input: HashMap<(SessionId, usize), Vec<u8>>,
    sent_keys: std::collections::HashSet<String>,
    /// Shell tabs whose shell has exited, with its code.
    exited_tabs: HashMap<(SessionId, usize), i32>,
    /// How many images have been pasted, for their names.
    pasted_images: usize,
    /// The session the user views, whose output marks nothing (D392).
    viewing: Option<SessionId>,
    links: Vec<terminal_link::LinkTap>,
    diffs: HashMap<SessionId, Vec<RepoDiff>>,
    /// Repositories whose read fails, with the message (DS-35).
    failing_repos: HashMap<(SessionId, String), String>,
    reviewed: HashMap<SessionId, Vec<RepoDiff>>,
    looks: HashMap<SessionId, Vec<crate::model::LookedFile>>,
    pushes: Vec<(SessionId, PushChoice)>,
    pull_requests: HashMap<(SessionId, String), u32>,
    next_pull_request: u32,
    codex_needs_trust: bool,
    /// AGENTSET: the agent kinds' switches.
    switches: Vec<AgentSwitch>,
    github_repos: Vec<String>,
    /// Document versions (DV-2 to DV-8).
    docs: docs::Store,
    // REPOS (RL-1): the repo library, saved scope and search state.
    repos: repos::State,
    /// D784: the holders each machine lists, as a test seeded them.
    running: Vec<crate::daemon::RunningHolder>,
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
                desk_told: workspace.desk.clone(),
                workspace,
                subscribers: Vec::new(),
                failures: HashMap::new(),
                recoveries: fixture::recoveries(),
                calls: Vec::new(),
                answers: Vec::new(),
                next: 1,
                archived: HashMap::new(),
                archive_frees: HashMap::new(),
                unpushed: HashMap::new(),
                unsubmitted_drafts: HashMap::new(),
                no_default: Vec::new(),
                last_fork: None,
                last_move: None,
                last_continue: None,
                last_archive: None,
                terminal_input: HashMap::new(),
                sent_keys: std::collections::HashSet::new(),
                exited_tabs: HashMap::new(),
                pasted_images: 0,
                viewing: None,
                links: Vec::new(),
                diffs: HashMap::new(),
                failing_repos: HashMap::new(),
                reviewed: HashMap::new(),
                looks: HashMap::new(),
                pushes: Vec::new(),
                pull_requests: HashMap::new(),
                next_pull_request: 1,
                codex_needs_trust: false,
                switches: agent_switches::defaults(),
                docs: docs::Store::default(),
                github_repos: [
                    "acme/ledger",
                    "acme/api",
                    "acme/web",
                    "acme/infra",
                    "acme/docs",
                    "octo/notes",
                ]
                .map(str::to_owned)
                .to_vec(),
                // REPOS (RL-1)
                repos: repos::State::default(),
                running: Vec::new(),
            })),
            holds: Arc::new(Mutex::new(HashMap::new())),
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
        let error = DaemonError::Scripted { operation, message: message.into() };
        self.inner().failures.insert(operation, error);
    }

    /// Holds every reply of `operation` until [`Self::release`], for tests
    /// that need a read still in flight while they look. Holding twice is the
    /// same as once.
    pub fn hold(&self, operation: Operation) {
        let mut holds = match self.holds.lock() {
            Ok(holds) => holds,
            Err(poisoned) => poisoned.into_inner(),
        };
        holds.entry(operation).or_insert_with(|| watch::channel(false).0).send_replace(false);
    }

    /// Lets every reply of `operation` go, those already waiting and those
    /// that come later. Does nothing when it isn't held.
    pub fn release(&self, operation: Operation) {
        let mut holds = match self.holds.lock() {
            Ok(holds) => holds,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(latch) = holds.remove(&operation) {
            latch.send_replace(true);
        }
    }

    /// Makes `repo`'s read in `session` fail with `message` until
    /// [`Self::heal_repo`] (DS-35). The daemon reads every repository at
    /// once, so every `diffs` call for the session fails meanwhile, while
    /// `repo_diff` fails for this repository only.
    pub fn fail_repo(&self, session: &SessionId, repo: &str, message: impl Into<String>) {
        self.inner()
            .failing_repos
            .insert((session.clone(), repo.to_owned()), message.into());
    }

    /// Ends [`Self::fail_repo`] for `repo` in `session`.
    pub fn heal_repo(&self, session: &SessionId, repo: &str) {
        self.inner().failing_repos.remove(&(session.clone(), repo.to_owned()));
    }

    /// Makes the next call of `operation` fail with exactly `error`.
    pub fn fail_next_with(&self, operation: Operation, error: DaemonError) {
        self.inner().failures.insert(operation, error);
    }

    /// Says how much disk archiving `session` will free, as people read it ("1.4 GB").
    /// Makes the daemon say Codex's hooks still need trusting, or not (D202).
    pub fn set_codex_needs_trust(&self, on: bool) {
        self.inner().codex_needs_trust = on;
    }

    /// Presets the agents' switches (AGENTSET); the next `agent_switches` gives these.
    pub fn set_agent_switches(&self, switches: Vec<AgentSwitch>) {
        self.inner().switches = switches;
    }

    /// Says which GitHub repositories `gh` sees on the host, as `owner/name`,
    /// most recently pushed first (CH-3).
    pub fn set_github_repos(&self, repos: Vec<String>) {
        self.inner().github_repos = repos;
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

    /// Says how many pull request drafts of the review `session` were never
    /// submitted; archiving keeps them with the review (D513).
    pub fn set_unsubmitted_drafts(&self, session: &SessionId, count: u32) {
        self.inner().unsubmitted_drafts.insert(session.clone(), count);
    }

    /// Says `repo`'s clone in `session` has no default branch: reads that
    /// need it fail until [`Daemon::find_default_branch`] finds it.
    pub fn set_no_default_branch(&self, session: &SessionId, repo: &str) {
        let mut inner = self.inner();
        let held = (session.clone(), repo.to_owned());
        if !inner.no_default.contains(&held) {
            inner.no_default.push(held);
        }
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

    /// The session, channel and "before turn" of the last continue in
    /// another channel (CV-8).
    #[must_use]
    pub fn last_continue(&self) -> Option<(SessionId, ChannelId, Option<u32>)> {
        self.inner().last_continue.clone()
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

    /// Tells the views following a tab one of Humata's lines about it, as
    /// the daemon does when the tab's machine comes back.
    pub fn terminal_notice(&self, session: &SessionId, tab: usize, notice: TerminalNotice) {
        terminal_link::notice(&self.inner().links, session, tab, notice);
    }

    /// Prints `bytes` in a tab as its program would, to its saved screen and to
    /// the views following it.
    pub fn terminal_print(&self, session: &SessionId, tab: usize, bytes: &[u8]) {
        self.inner().terminal_data(session, tab, bytes.to_vec());
    }

    /// Ends the links of the views following a tab, as the daemon's end
    /// when the tab's program is stopped; a view opens a new one.
    pub fn end_terminal_links(&self, session: &SessionId, tab: usize) {
        terminal_link::end(&mut self.inner().links, session, tab);
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
        let holds = self.holds.clone();
        Box::pin(async move {
            wait.await;
            if let Some(operation) = operation {
                let latch = match holds.lock() {
                    Ok(holds) => holds.get(&operation).map(watch::Sender::subscribe),
                    Err(poisoned) => poisoned.into_inner().get(&operation).map(watch::Sender::subscribe),
                };
                if let Some(mut latch) = latch {
                    // A dropped latch is a release too.
                    let _ = latch.wait_for(|released| *released).await;
                }
            }
            let mut inner = match shared.lock() {
                Ok(inner) => inner,
                Err(poisoned) => poisoned.into_inner(),
            };
            inner.calls.push(call);
            if let Some(operation) = operation
                && let Some(error) = inner.failures.remove(&operation)
            {
                return Err(error);
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
    /// Tells every subscriber what changed on the Desk since it last did
    /// (DESKWATCH), whichever path changed it: every event goes through
    /// [`Self::emit`], which calls this first.
    fn tell_desk(&mut self) {
        if self.workspace.desk == self.desk_told {
            return;
        }
        let changes = self.workspace.desk.changes_since(&self.desk_told);
        self.desk_told = self.workspace.desk.clone();
        for change in changes {
            self.send(Event::Desk(change));
        }
    }

    fn emit(&mut self, event: Event) {
        self.tell_desk();
        self.send(event);
    }

    fn send(&mut self, event: Event) {
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
        output.extend_from_slice(&bytes);
        terminal_link::feed(&mut self.links, session, tab, &bytes);
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
        self.heard(session, tab);
    }

    /// CH-16 (D392): output on a session's agent tab that the user doesn't
    /// view marks the session unread, as the daemon does.
    fn heard(&mut self, session: &SessionId, tab: usize) {
        if tab != 0 || self.viewing.as_ref() == Some(session) {
            return;
        }
        let Some(found) = self
            .workspace
            .sessions
            .iter_mut()
            .find(|found| &found.id == session)
        else {
            return;
        };
        if found.unread > 0 {
            return;
        }
        found.unread = 1;
        self.changed(session);
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

    fn send_to_tab(&self, session: &SessionId, tab: usize, text: &str, key: &str) -> Reply<()> {
        if tab == 0 {
            return self.send_keyed(session, text, key);
        }
        sessions::send_to_tab(self, session, tab, text, key)
    }

    fn send_keyed(&self, session: &SessionId, text: &str, key: &str) -> Reply<()> {
        if self.inner().sent_keys.contains(key) {
            return Box::pin(async { Ok(()) });
        }
        let (this, key) = (self.clone(), key.to_owned());
        let sent = sessions::send(self, session, text);
        Box::pin(async move {
            sent.await?;
            this.inner().sent_keys.insert(key);
            Ok(())
        })
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


    fn set_default_agent(&self, agent: Agent) -> Reply<()> {
        agent_asks::set_default_agent(self, agent)
    }


    fn codex_needs_trust(&self) -> Reply<bool> {
        agent_asks::codex_needs_trust(self)
    }

    fn agent_switches(&self) -> Reply<Vec<AgentSwitch>> {
        agent_switches::agent_switches(self)
    }

    fn set_agent_switch(&self, kind: String, change: SwitchChange) -> Reply<AgentSwitch> {
        agent_switches::set_agent_switch(self, kind, change)
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

    fn continue_in_channel(
        &self,
        session: &SessionId,
        channel: &ChannelId,
        before_turn: Option<u32>,
    ) -> Reply<SessionId> {
        sessions::continue_in_channel(self, session, channel, before_turn)
    }

    fn rename_session(&self, session: &SessionId, title: &str, rename_branch: bool) -> Reply<()> {
        sessions::rename_session(self, session, title, rename_branch)
    }

    fn add_repo_to_session(&self, session: &SessionId, repo: &str) -> Reply<()> {
        session_repos::add_repo_to_session(self, session, repo)
    }

    fn stop_agent(&self, session: &SessionId) -> Reply<()> {
        sessions::stop_agent(self, session)
    }

    fn running_holders(&self, machine: &MachineId) -> Reply<Vec<crate::daemon::RunningHolder>> {
        running::running_holders(self, machine)
    }

    fn stop_running_holder(&self, machine: &MachineId, tab: &crate::daemon::HolderTab) -> Reply<()> {
        running::stop_running_holder(self, machine, tab)
    }

    fn resume_running_holder(&self, machine: &MachineId, tab: &crate::daemon::HolderTab) -> Reply<()> {
        running::resume_running_holder(self, machine, tab)
    }

    fn end_running_holder(&self, machine: &MachineId, tab: &crate::daemon::HolderTab) -> Reply<()> {
        running::end_running_holder(self, machine, tab)
    }

    fn adopt_running_holder(
        &self,
        machine: &MachineId,
        tab: &str,
        channel: Option<&crate::model::ChannelId>,
    ) -> Reply<SessionId> {
        running::adopt_running_holder(self, machine, tab, channel)
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

    fn restore_session_stopped(&self, session: &SessionId) -> Reply<()> {
        running::restore_session_stopped(self, session)
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

    fn view(&self, session: Option<&SessionId>) -> Reply<()> {
        sessions::view(self, session)
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

    fn github_repos(&self, query: &str) -> Reply<Vec<String>> {
        channels::github_repos(self, query)
    }

    fn add_repo_to_channel(&self, channel: &ChannelId, repo: &str) -> Reply<()> {
        channel_repos::add_repo_to_channel(self, channel, repo)
    }

    // REPOS (RL-5): the repo library and the one repo search.
    fn search_repos(
        &self,
        query: &str,
        scope: Option<crate::repos::RepoScope>,
    ) -> Reply<crate::repos::RepoSearch> {
        repos::search_repos(self, query, scope)
    }

    fn repo_library(&self) -> Reply<Vec<crate::repos::LibraryRepo>> {
        repos::repo_library(self)
    }

    fn remove_from_repo_library(&self, repo: &str) -> Reply<Vec<crate::repos::LibraryRepo>> {
        repos::remove_from_repo_library(self, repo)
    }

    fn forget_repo_library(&self) -> Reply<u32> {
        repos::forget_repo_library(self)
    }

    fn repo_scope(&self) -> Reply<crate::repos::RepoScope> {
        repos::repo_scope(self)
    }

    fn save_repo_scope(&self, scope: crate::repos::RepoScope) -> Reply<crate::repos::RepoScope> {
        repos::save_repo_scope(self, scope)
    }

    fn repo_owners(&self) -> Reply<Vec<String>> {
        repos::repo_owners(self)
    }

    fn now_ms(&self) -> i64 {
        repos::now_ms(self)
    }

    fn cancel_setup(&self, setup: &SetupId) -> Reply<()> {
        setup::cancel_setup(self, setup)
    }

    fn retry_setup(&self, setup: &SetupId) -> Reply<ChannelId> {
        setup::retry_setup(self, setup)
    }

    fn move_item(&self, item: &ItemId, status: ItemStatus, seen: Option<u64>) -> Reply<()> {
        boards::move_item(self, item, status, seen)
    }

    fn create_item_in(&self, scope: Scope, title: &str, status: ItemStatus) -> Reply<ItemId> {
        boards::create_item(self, scope, title, status)
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

    fn find_files(&self, session: &SessionId) -> Reply<Vec<crate::model::SessionFile>> {
        let id = session.clone();
        self.reply(None, format!("find_files {session}"), move |inner| {
            inner.session_mut(&id)?;
            let files = inner.workspace.session_files.get(&id).cloned().unwrap_or_default();
            Ok(crate::daemon::fair_share(files, crate::daemon::SESSION_FILES_CAP))
        })
    }

    fn diffs(&self, session: &SessionId, base: &DiffBase) -> Reply<Vec<RepoDiff>> {
        review::diffs(self, session, base)
    }

    fn repo_diff(&self, session: &SessionId, base: &DiffBase, repo: &str) -> Reply<Vec<RepoDiff>> {
        review::repo_diff(self, session, base, repo)
    }

    fn bases(&self, session: &SessionId, repo: &str) -> Reply<RepoBases> {
        review::bases(self, session, repo)
    }

    fn find_default_branch(&self, session: &SessionId, repo: &str) -> Reply<String> {
        review::find_default_branch(self, session, repo)
    }

    fn mark_reviewed(&self, session: &SessionId) -> Reply<()> {
        review::mark_reviewed(self, session)
    }

    fn looks(&self, session: &SessionId) -> Reply<Vec<crate::model::LookedFile>> {
        review::looks(self, session)
    }

    fn save_looks(&self, session: &SessionId, looks: Vec<crate::model::LookedFile>) -> Reply<()> {
        review::save_looks(self, session, looks)
    }

    fn plan_push(&self, session: &SessionId) -> Reply<crate::model::PushPlan> {
        review::plan_push(self, session)
    }

    fn push(
        &self,
        session: &SessionId,
        _token: &str,
        choices: Vec<PushChoice>,
    ) -> Reply<Vec<crate::model::PushedPullRequest>> {
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

    fn open_in(&self, session: &SessionId, agent: Agent, folder: &str) -> Reply<(usize, Session)> {
        terminals::open_in(self, session, agent, folder)
    }

    fn write(&self, session: &SessionId, tab: usize, text: &str) -> Reply<()> {
        terminals::write(self, session, tab, text)
    }

    fn link(&self, session: &SessionId, tab: usize) -> Reply<crate::daemon::TerminalLink> {
        terminal_link::link(self, session, tab)
    }

    fn paste_image(
        &self,
        session: &SessionId,
        kind: crate::daemon::ImageKind,
        bytes: Vec<u8>,
    ) -> Reply<String> {
        terminals::paste_image(self, session, kind, bytes)
    }
}
