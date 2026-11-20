use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tca::{BoxFuture, Clock};
use tokio::sync::{mpsc, watch};

use crate::daemon::{
    AgentSwitch, ArchivePlan, Daemon, DaemonError, DeleteChannelPlan, DeleteChannelRequest,
    DeleteChannelResponse, Event, Events, ForkOptions, Operation, PastTurn, Reply, SwitchChange,
    TerminalNotice, Terminals,
};
use crate::fixture;
use crate::mass_delete::{DeleteScope, DeleteSessionsOutcome, DeleteSessionsPlan};
use crate::model::{
    Agent, Answer, Archive, Capability, ChannelId, DiffBase, GrantId, ItemId, ItemStatus, Line,
    MachineId, PushChoice, Recovery, RepoBases, RepoDiff, Request, RequestId, RuleSetting, Scope, Session,
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
mod mass_delete;
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
mod sync;
mod todo_automation;
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
    /// New subscribers hear `Event::Resync` first.
    resync_on_subscribe: bool,
    /// The next `events()` waits for this before it answers.
    subscribe_gate: Option<tokio::sync::oneshot::Receiver<()>>,
    /// The Desk as subscribers last heard it, to tell what changed (DESKWATCH).
    desk_told: crate::desk::DeskData,
    subscribers: Vec<Subscriber>,
    failures: HashMap<Operation, DaemonError>,
    recoveries: HashMap<SessionId, Recovery>,
    calls: Vec<String>,
    /// Every answer sent, in order, whether or not it worked.
    answers: Vec<(RequestId, Answer)>,
    /// Every request Ask why was sent for.
    asked_why: Vec<RequestId>,
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
    /// A stack layer's files in one repository, by `(session, repo,
    /// "base..head")`; a range not set here has none.
    ranges: HashMap<(SessionId, String, String), Vec<crate::model::FileDiff>>,
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
    /// FUSEFS: Settings › Sync and the mount switch.
    sync: crate::daemon::SyncSnapshot,
    /// TODO2WIRE: Settings › General › To-do.
    todo_automation: crate::daemon::TodoAutomation,
    github_repos: Vec<String>,
    /// Document versions (DV-2 to DV-8).
    docs: docs::Store,
    // REPOS (RL-1): the repo library, saved scope and search state.
    repos: repos::State,
    /// D784: the holders each machine lists, as a test seeded them.
    running: Vec<crate::daemon::RunningHolder>,
    /// MASSDELETE: the plan last given out, and whether the next delete changes.
    mass_delete: mass_delete::State,
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
                resync_on_subscribe: false,
                subscribe_gate: None,
                failures: HashMap::new(),
                recoveries: fixture::recoveries(),
                calls: Vec::new(),
                answers: Vec::new(),
                asked_why: Vec::new(),
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
                ranges: HashMap::new(),
                failing_repos: HashMap::new(),
                reviewed: HashMap::new(),
                looks: HashMap::new(),
                pushes: Vec::new(),
                pull_requests: HashMap::new(),
                next_pull_request: 1,
                codex_needs_trust: false,
                switches: agent_switches::defaults(),
                sync: crate::daemon::SyncSnapshot::default(),
                todo_automation: crate::daemon::TodoAutomation::default(),
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
                mass_delete: mass_delete::State::default(),
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

    /// Makes every later `events()` start with `Event::Resync`, as the live
    /// daemon's does once its subscription is up, so a follower that
    /// resubscribes after a failed load reloads.
    pub fn resync_on_subscribe(&self) {
        self.inner().resync_on_subscribe = true;
    }

    /// Makes the next `events()` wait until the returned sender sends (or is
    /// dropped), so a test orders a subscribe after other calls finish.
    #[must_use]
    pub fn hold_next_subscribe(&self) -> tokio::sync::oneshot::Sender<()> {
        let (release, gate) = tokio::sync::oneshot::channel();
        self.inner().subscribe_gate = Some(gate);
        release
    }

    /// Ends every open event stream, as a daemon that went away does.
    pub fn end_events(&self) {
        self.inner().subscribers.clear();
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

    /// Presets what Settings › Sync holds (FUSEFS); the next `sync_settings` gives this.
    pub fn set_sync_snapshot(&self, snapshot: crate::daemon::SyncSnapshot) {
        self.inner().sync = snapshot;
    }

    /// Makes the daemon's mount state read as `status` (D908) and tells the
    /// app, as the daemon does when the mount moves on its own.
    pub fn set_fusefs_status(&self, status: crate::daemon::FuseMountStatus) {
        self.inner().sync.fusefs_status = status;
        self.emit(Event::SettingsChanged);
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

    /// Says what `repo` of `session` changes between the revs `base` and
    /// `head`, as a stack layer shows it (D800).
    pub fn set_range_files(
        &self,
        session: &SessionId,
        repo: &str,
        base: &str,
        head: &str,
        files: Vec<crate::model::FileDiff>,
    ) {
        self.inner()
            .ranges
            .insert((session.clone(), repo.to_owned(), format!("{base}..{head}")), files);
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

    /// The next bulk delete answers Changed and deletes nothing (MASSDELETE).
    pub fn change_after_plan(&self) {
        mass_delete::change_after_plan(self);
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

    fn ask_why(&self, request: &RequestId) -> Reply<Request> {
        inbox::ask_why(self, request)
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

    fn sync_settings(&self) -> Reply<crate::daemon::SyncSnapshot> {
        sync::sync_settings(self)
    }

    fn set_sync_settings(&self, change: crate::daemon::SyncChange) -> Reply<crate::daemon::SyncSnapshot> {
        sync::set_sync_settings(self, change)
    }

    fn todo_automation(&self) -> Reply<crate::daemon::TodoAutomation> {
        todo_automation::todo_automation(self)
    }

    fn set_todo_automation(&self, section: crate::daemon::TodoAutomation) -> Reply<crate::daemon::TodoAutomation> {
        todo_automation::set_todo_automation(self, section)
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

    fn plan_delete_sessions(&self, scope: &DeleteScope) -> Reply<DeleteSessionsPlan> {
        mass_delete::plan(self, scope.clone())
    }

    fn delete_sessions(
        &self,
        token: &str,
        delete_unpushed_work: bool,
        typed_count: u32,
    ) -> Reply<DeleteSessionsOutcome> {
        mass_delete::delete(self, token, delete_unpushed_work, typed_count)
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

    fn plan_delete_channel(&self, channel: &ChannelId) -> Reply<DeleteChannelPlan> {
        channels::plan_delete_channel(self, channel)
    }

    fn delete_channel(&self, request: DeleteChannelRequest) -> Reply<DeleteChannelResponse> {
        channels::delete_channel(self, request)
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

    fn add_to_repo_library(&self, repo: &str) -> Reply<Vec<crate::repos::LibraryRepo>> {
        repos::add_to_repo_library(self, repo)
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

    fn open_agent_tab(
        &self,
        session: &SessionId,
        agent: Agent,
        prompt: &str,
    ) -> Reply<(usize, Session)> {
        let id = session.clone();
        let prompt = prompt.to_owned();
        let call = format!("open_agent_tab {session} {} {prompt:?}", agent.command());
        self.reply(Some(Operation::OpenAgentTab), call, move |inner| {
            if agent == Agent::Shell {
                return Err(DaemonError::Refused {
                    what: "Opening an agent tab".into(),
                    reason: "a plain shell is not an agent. Open a Terminal tab instead".into(),
                });
            }
            let session = inner.session_mut(&id)?;
            // D858: always a fresh extra tab, never attaching tab 0 or an earlier tab.
            session.shells.push(agent.command().to_owned());
            let tab = session.shells.len();
            session.terminal_agents.insert(tab, agent);
            let snapshot = session.clone();
            inner.changed(&id);
            for line in fixture::extra_terminal(agent, &snapshot.repo, &snapshot.branch) {
                inner.output(&id, tab, line.from, &line.text);
            }
            if !prompt.is_empty() {
                inner.output(&id, tab, Speaker::You, &prompt);
                inner.output(
                    &id,
                    tab,
                    Speaker::Agent,
                    &format!("Message received by the fake {} terminal.", agent.name()),
                );
            }
            Ok((tab, snapshot))
        })
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

#[cfg(test)]
mod open_agent_tab_tests {
    use super::*;
    use tca::TestClock;

    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(future)
    }

    #[test]
    fn open_agent_tab_creates_fresh_tabs_with_label_agent_and_prompt() {
        let daemon = FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()));
        let id = SessionId::new("ledger");
        let before = daemon.workspace().session(&id).expect("ledger").shells.len();

        let mut events = block_on(daemon.events()).expect("subscribe");
        let (first, snap1) = block_on(daemon.open_agent_tab(
            &id,
            Agent::Claude,
            "Review the diff without editing files.",
        ))
        .expect("first agent tab opens");
        assert_eq!(first, before + 1);
        assert_eq!(snap1.shells.last().map(String::as_str), Some("claude"));
        assert_eq!(snap1.terminal_agents.get(&first), Some(&Agent::Claude));

        // Picking the same agent a second time opens another fresh tab instead
        // of attaching tab 0 or the first reviewer tab.
        let (second, snap2) =
            block_on(daemon.open_agent_tab(&id, Agent::Claude, "")).expect("second agent tab opens");
        assert_eq!(second, first + 1);
        assert_eq!(snap2.terminal_agents.get(&first), Some(&Agent::Claude));
        assert_eq!(snap2.terminal_agents.get(&second), Some(&Agent::Claude));

        // Reloading the workspace keeps both labels and agent kinds.
        let reloaded = daemon.workspace();
        let kept = reloaded.session(&id).expect("ledger");
        assert_eq!(kept.shells.get(first - 1).map(String::as_str), Some("claude"));
        assert_eq!(kept.shells.get(second - 1).map(String::as_str), Some("claude"));
        assert_eq!(kept.terminal_agents.get(&first), Some(&Agent::Claude));
        assert_eq!(kept.terminal_agents.get(&second), Some(&Agent::Claude));

        // Prompt bytes were written to the first tab and not to the empty-prompt second tab.
        let first_bytes = String::from_utf8_lossy(&reloaded.terminal_bytes[&(id.clone(), first)]);
        assert!(first_bytes.contains("Review the diff without editing files."));
        let second_bytes = String::from_utf8_lossy(&reloaded.terminal_bytes[&(id.clone(), second)]);
        assert!(!second_bytes.contains("Review the diff without editing files."));

        // Subscribers received a session event carrying the agent tab metadata.
        let mut saw_session = false;
        while let Ok(Ok(event)) = events.try_recv() {
            if let Event::Session(changed) = event
                && changed.id == id
                && changed.terminal_agents.get(&first) == Some(&Agent::Claude)
            {
                saw_session = true;
            }
        }
        assert!(saw_session, "open_agent_tab publishes Event::Session with terminal_agents");
    }

    #[test]
    fn open_agent_tab_refuses_shell_and_honours_scripted_or_offline_errors() {
        let daemon = FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()));
        let id = SessionId::new("ledger");

        assert!(matches!(
            block_on(daemon.open_agent_tab(&id, Agent::Shell, "")),
            Err(DaemonError::Refused { .. })
        ));
        assert!(matches!(
            block_on(daemon.open_agent_tab(&SessionId::new("no-such"), Agent::Codex, "")),
            Err(DaemonError::NotFound { .. })
        ));

        daemon.fail_next(Operation::OpenAgentTab, "agent cli failed");
        assert!(matches!(
            block_on(daemon.open_agent_tab(&id, Agent::Codex, "check")),
            Err(DaemonError::Scripted {
                operation: Operation::OpenAgentTab,
                ..
            })
        ));

        let machine = daemon.workspace().session(&id).expect("ledger").machine.clone();
        daemon.take_offline(&machine);
        assert!(matches!(
            block_on(daemon.open_agent_tab(&id, Agent::Codex, "check")),
            Err(DaemonError::Offline { .. })
        ));
    }
}
