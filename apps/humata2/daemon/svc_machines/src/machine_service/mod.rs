//! MachineService (HO-1–HO-6, LD-10): the machines the host knows, adding
//! one over SSH, reconnecting it and removing it. The work is machines'
//! (`humata2_machines::Machines`); this wires it to the API: the access
//! checks, an `Operation` whose steps change as the work does (TR-7), and
//! the errors in the API's terms. A machine's id is its name until
//! machines get ids of their own (D61).
//!
//! What a machine's `reachability` says is what this daemon last saw: an
//! add or reconnect that worked, and then its peer link coming up or going
//! down, or a machine whose proof is wrong (IDENTITY_MISMATCH). The add
//! writes the enrollment key the link needs (D83). A machine not connected
//! since humatad2 started is UNSPECIFIED until its link comes up.
//! `daemon_version` is what the link's Hello said, `last_seen_at` when an
//! add, reconnect or link last reached the machine (or a link was last known
//! up), and `next_retry_at` when a down link dials again.
//!
//! Each change to a machine as ListMachines shows it goes on the feed
//! (EventService), after the change is written.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use humata2_access::{
    AccessRequest, Call, Capability, MachineAction, MachineActionKind, MachineId, Refusal, Target,
};
use humata2_daemon_ports::{Caller, Gated};
use humata2_grpc::humata::v1 as pb;
use humata2_machines::{
    Action, AddSsh, Builds, ConfigError, Drive, EntryState, HostEntry, IdentityError, Machines,
    ManageError, OpStep, Provider, SetupError, StepEvent, SystemRunner,
};
pub use humata2_rpc_kit::lock;
use humata2_rpc_kit::{
    chain, error, failed, failure, internal, invalid_field, missing, not_built_status,
};
use tokio::runtime::Handle;
use tokio::task::AbortHandle;
use tonic::{Code, Request, Response, Status};

use crate::ports::{
    Checked, LinkEvent, MachineChannelsPort, MachineFeedPort, MachineGuardPort,
    MachineHoldersPort, MachineHostProbePort, MachineHostVarsPort, MachineInboxPort,
    MachineLinksPort, MachineLoginPort, MachineOperationsPort, MachinePeerDialPort,
    MachinePeerReportsPort, MachineSkillsPort, MachineStopsPort, MachineStoragePort,
    MachineUpgradePort, OperationProgress, OperationStarted, RelayError, answer_gated_call, caller,
    gate, refused, store_error, user_only_call,
};

mod box_root;
mod box_storage;
mod change_root;
mod forget;
mod script_rpc;
mod scripts;
mod stack_settings;

pub use box_storage::BoxMachineSvc;
pub use change_root::start_layout;
pub use forget::{Forgetting, finish_forgets, follow_let_go};
pub use stack_settings::agents_changed;

/// What stopping or starting the host's own sessions through the machine
/// list is refused as.
#[derive(Debug, thiserror::Error)]
#[error("this is the machine the host runs on, not one of its boxes, so its sessions aren't stopped or started here")]
struct ThisMachine;

/// How long a removal plan stays good.
const PLAN_LIFETIME: Duration = Duration::from_secs(10 * 60);

#[derive(Clone)]
pub struct MachineSvc {
    pub(crate) guard: Arc<dyn MachineGuardPort>,
    pub(crate) work: Arc<Work>,
    pub(crate) operations: Arc<dyn MachineOperationsPort>,
    /// Where a call a rule sends to Ask me waits for the user (D38.6).
    pub(crate) inbox: Arc<dyn MachineInboxPort>,
    /// What this machine keeps, for the Storage page (LD-13 to LD-15).
    pub(crate) storage: Arc<dyn MachineStoragePort>,
    /// Channel folder migration for ChangeRoot.
    pub(crate) channels: Arc<dyn MachineChannelsPort>,
    /// Host environment variables.
    pub(crate) host_vars: Arc<dyn MachineHostVarsPort>,
    /// Running holders management (D784).
    pub(crate) holders: Arc<dyn MachineHoldersPort>,
}

/// The wall clock, which tests set.
pub type Clock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

/// What this daemon last saw of a machine.
#[derive(Clone, Default)]
struct Seen {
    reach: pb::Reachability,
    daemon_version: String,
    last_seen_at: Option<SystemTime>,
    next_retry_at: Option<SystemTime>,
}

/// What the blocking work needs.
pub struct Work {
    pub machines: Machines,
    pub builds: Box<dyn Builds + Send + Sync>,
    /// What each machine looked like when last connected to.
    seen: Mutex<HashMap<String, Seen>>,
    feed: Arc<dyn MachineFeedPort>,
    /// The machines as the feed last told them.
    shown: Mutex<BTreeMap<String, pb::Machine>>,
    /// Each machine's peer link supervisor, while one runs.
    links: Mutex<HashMap<String, AbortHandle>>,
    clock: Clock,
    /// Removal plans handed out: token to machine, and when. Delete on a
    /// provider's machine takes one too.
    plans: Mutex<HashMap<String, (String, Instant)>>,
    /// Skills sync, told each link that comes up or goes down (SK-3, SK-7);
    /// set once by `serve` before any link starts.
    pub skills: OnceLock<Arc<dyn MachineSkillsPort>>,
    /// The Inbox each link replays its machine's journal into (D226); set
    /// once by `serve` before any link starts.
    pub inbox: OnceLock<Arc<dyn MachineInboxPort>>,
    /// Each machine's live link, for the services that relay calls to it
    /// (D139). Told at the same hook as `skills`.
    pub(crate) relay_links: Arc<dyn MachineLinksPort>,
    /// What each machine last reported on its Uplink (D139, SL-3).
    pub(crate) reports: Arc<dyn MachinePeerReportsPort>,
    /// Peer link supervisor dialer.
    pub peer_dial: OnceLock<Arc<dyn MachinePeerDialPort>>,
    /// Login check for machines requiring interactive login.
    pub login: OnceLock<Arc<dyn MachineLoginPort>>,
    /// Local host probe reader.
    pub host_probe: OnceLock<Arc<dyn MachineHostProbePort>>,
    /// What each machine's last answered probe said (HO-3): shown, with its
    /// `read_at`, while the machine can't be asked. One per machine, kept
    /// only until the machine is removed; nothing polls (LD-14).
    probes: Mutex<HashMap<String, (pb::MachineHealth, Vec<pb::MachineFolder>)>>,
    /// Forget machine's plans and its part of the sessions (LD-11).
    pub forgetting: forget::Forgetful,
    /// The stops kept for other machines (D262); set once by `serve` before
    /// any link starts.
    pub stops: OnceLock<Arc<dyn MachineStopsPort>>,
    /// Posts or withdraws each machine's Migrate item at link-up (UP-1); set
    /// once by `serve` before any link starts.
    pub upgrade: OnceLock<Arc<dyn MachineUpgradePort>>,
    /// MACHINESCRIPTS (D681): the run engine, set once by `serve` before any link starts, and which
    /// machines were just added.
    scripts: scripts::Hook,
}

/// How long GetMachine waits for a machine's probe before it shows the last
/// one (TR-6, HO-5).
const PROBE_DEADLINE: Duration = Duration::from_secs(6);

/// The folders a probe measured under `~/Humata`, by name (LD-14). The
/// channel is left unset: the folder's name is all the machine knows.
fn folders_of(sizes: &HashMap<String, u64>) -> Vec<pb::MachineFolder> {
    let mut folders: Vec<pb::MachineFolder> = sizes
        .iter()
        .map(|(name, bytes)| pb::MachineFolder {
            channel: None,
            path: format!("~/Humata/{name}/"),
            bytes: *bytes,
        })
        .collect();
    folders.sort_by(|a, b| a.path.cmp(&b.path));
    folders
}

impl Work {
    /// The live link of every machine, for the services that relay calls to
    /// a machine (D139).
    pub fn links(&self) -> Arc<dyn MachineLinksPort> {
        self.relay_links.clone()
    }

    /// What each machine last reported, for whoever shows remote session
    /// status (D139).
    pub fn reports(&self) -> Arc<dyn MachinePeerReportsPort> {
        self.reports.clone()
    }

    /// `machine`'s link is up on `channel`: the report sink expects its
    /// snapshot, and relayed calls can go (D139).
    fn peer_up(&self, machine: &str, channel: &tonic::transport::Channel) {
        self.reports.link_up(machine);
        self.relay_links.up(machine, channel.clone());
    }

    /// `machine`'s link is down: relayed calls fail as offline, and what it
    /// reported stays readable, marked stale (HO-5).
    fn peer_down(&self, machine: &str) {
        let was_up = self.relay_links.channel(machine).is_ok();
        self.relay_links.down(machine);
        if was_up {
            self.reports.link_down(machine, (self.clock)());
        }
    }

    /// `machine` was removed: its watchers end and its reports go.
    fn forget_peer(&self, machine: &str) {
        self.forget_link(machine);
        self.reports.forget(machine);
    }

    /// As [`Self::forget_peer`], keeping what the machine last reported:
    /// a removal reads it when it archives the sessions (D783).
    fn forget_link(&self, machine: &str) {
        self.relay_links.forget(machine);
        lock(&self.probes).remove(machine);
        self.forget_stop(machine);
    }

    /// `machine` is gone or enrolled afresh: the stop kept for it goes too
    /// (D262), so a machine of the same name starts out running.
    fn forget_stop(&self, machine: &str) {
        if let Some(stops) = self.stops.get()
            && let Err(error) = stops.remove(machine)
        {
            humata2_rpc_kit::log::say!(
                "humatad2: couldn't drop the stop kept for {machine}: {}",
                chain(&error)
            );
        }
    }

    /// `machine` was removed or forgotten: the host's mark of how far its
    /// Inbox journal was applied goes, since the machine's journal starts
    /// over (D226), and so does its skills sync state, so a machine added
    /// again under the name gets a first sync that asks (SK-5, D760).
    pub async fn forget_journal(&self, machine: &str) {
        if let Some(inbox) = self.inbox.get() {
            inbox.forget_journal(machine).await;
        }
        if let Some(skills) = self.skills.get()
            && let Err(error) = skills.forget(machine).await
        {
            humata2_rpc_kit::log::say!("humatad2: {}", chain(&error));
        }
    }

    /// As [`Self::forget_journal`], from a thread that may block, for a
    /// machine enrolled afresh (no link up yet).
    pub fn forget_journal_now(&self, machine: &str) {
        self.forget_stop(machine);
        if let Some(inbox) = self.inbox.get() {
            inbox.forget_journal_now(machine);
        }
        if let Some(skills) = self.skills.get()
            && let Err(error) = skills.forget_now(machine)
        {
            humata2_rpc_kit::log::say!("humatad2: {}", chain(&error));
        }
    }

    /// `machine`'s removal is done: no channel lists it any more (D760),
    /// its sessions are archived, and its requests and Migrate items
    /// withdrawn (D783).
    /// From a thread that may block, never a runtime worker.
    pub fn removed(&self, machine: &str, runtime: &Handle) {
        if let Err(error) = runtime.block_on(self.take_out_of_channels(machine)) {
            humata2_rpc_kit::log::say!(
                "humatad2: {machine} was removed, but couldn't be taken out of its channels: {}. Take it out of each channel that still lists it",
                error.message()
            );
        }
        if let Err(error) = runtime.block_on(self.archive_removed(machine)) {
            humata2_rpc_kit::log::say!(
                "humatad2: {machine} was removed, but its sessions or Inbox requests couldn't all be let go of: {}. Archive the sessions still shown on {machine} and deny its requests still waiting, before adding another machine named {machine}; restarting Humata doesn't do it for you",
                chain(&error)
            );
        }
        // What it last reported is in the archives now (D783).
        self.reports.forget(machine);
    }

    /// `machine`'s health (HO-3, HO-4) and the sizes of its Humata folders
    /// (LD-14, measured now: GetMachine is the page opening): what its link
    /// answers to a probe now, or, when the link is down or slow, the last
    /// answer with its `read_at`; with skills sync's part (SK-3, SK-8) on
    /// either. No health when there is neither. Never waits past
    /// `PROBE_DEADLINE`.
    async fn health(&self, machine: &str) -> (Option<pb::MachineHealth>, Vec<pb::MachineFolder>) {
        let probed = match self.relay_links.channel(machine) {
            Ok(channel) => {
                let mut client = pb::peer_service_client::PeerServiceClient::new(channel);
                let call = client.probe(Request::new(pb::ProbeRequest { measure_storage: true }));
                match tokio::time::timeout(PROBE_DEADLINE, call).await {
                    Ok(Ok(reply)) => Some(reply.into_inner()),
                    _ => None,
                }
            }
            Err(_) => None,
        };
        self.health_from(machine, probed)
    }

    /// This Mac's health, read by this process itself: This Mac has no
    /// link to probe, so without this its page read "0 GB free of 0 GB",
    /// GitHub signed out and no agents (owner, 0.2.1). Its folders are the
    /// Storage page's, so none are measured here. Never waits past
    /// `PROBE_DEADLINE`.
    async fn own_health(&self, machine: &str) -> Option<pb::MachineHealth> {
        let Some(host_probe) = self.host_probe.get().cloned() else {
            return self.health_from(machine, None).0;
        };
        let read = tokio::task::spawn_blocking(move || host_probe.read_probe(false));
        let probed = match tokio::time::timeout(PROBE_DEADLINE, read).await {
            Ok(Ok(probe)) => Some(probe),
            _ => None,
        };
        self.health_from(machine, probed).0
    }

    /// The health `probed` says, or the last one kept for `machine` when
    /// there is no new one, with skills sync's part.
    fn health_from(
        &self,
        machine: &str,
        probed: Option<pb::ProbeResponse>,
    ) -> (Option<pb::MachineHealth>, Vec<pb::MachineFolder>) {
        let measured = probed.as_ref().map(|probe| folders_of(&probe.folder_bytes));
        let fresh = probed.map(|probe| pb::MachineHealth {
            read_at: Some(timestamp((self.clock)())),
            git_version: probe.git_version,
            gh_user: probe.gh_user,
            gh_scopes: probe.gh_scopes,
            gh_version: probe.gh_version,
            agents_installed: probe.agents_installed,
            agent_versions: probe.agent_versions,
            disk_free_bytes: probe.disk_free_bytes,
            disk_total_bytes: probe.disk_total_bytes,
            variable_names: probe.variable_names,
            linger_enabled: probe.linger_enabled,
            ..pb::MachineHealth::default()
        });
        let (mut health, folders) = match fresh {
            Some(fresh) => {
                let folders = measured.unwrap_or_default();
                lock(&self.probes).insert(machine.to_owned(), (fresh.clone(), folders.clone()));
                (Some(fresh), folders)
            }
            None => match lock(&self.probes).get(machine).cloned() {
                Some((last, folders)) => (Some(last), folders),
                None => (None, Vec::new()),
            },
        };
        if let Some(skills) = self.skills.get() {
            let (skills_auto_sync, skills_sync_problem) = skills.health(machine);
            let shown = health.get_or_insert_with(pb::MachineHealth::default);
            shown.skills_auto_sync = skills_auto_sync;
            shown.skills_sync_problem = skills_sync_problem;
        }
        (health, folders)
    }

    /// Uses the removal plan `token` made for `machine`, once.
    pub fn use_plan(&self, token: &str, machine: &str) -> Result<(), Status> {
        let planned = lock(&self.plans)
            .remove(token)
            .filter(|(planned, made)| planned == machine && made.elapsed() < PLAN_LIFETIME);
        if planned.is_some() {
            return Ok(());
        }
        Err(failure(
            Code::FailedPrecondition,
            error(
                format!("the removal plan for {machine} is unknown, used or older than ten minutes"),
                "ask for a new plan (PlanRemoveMachine), check what it says, then remove with its token",
                pb::error::Kind::PlanStale(pb::PlanStale { changes: Vec::new() }),
            ),
        ))
    }

    /// The provider commands `entry`'s provider defines, as the machine
    /// page lists them (MP-2); none for an SSH machine or a provider no
    /// longer configured.
    fn defined_actions(&self, entry: &HostEntry) -> Vec<i32> {
        let Some(record) = &entry.provider else {
            return Vec::new();
        };
        let Ok((_, providers)) = read_providers(self.machines.hosts_file()) else {
            return Vec::new();
        };
        let Some(provider) = providers.iter().find(|provider| provider.id() == record.id) else {
            return Vec::new();
        };
        [
            (Action::Start, pb::ProviderAction::Start),
            (Action::Stop, pb::ProviderAction::Stop),
            (Action::Status, pb::ProviderAction::Status),
            (Action::Delete, pb::ProviderAction::Delete),
        ]
        .into_iter()
        .filter(|(action, _)| provider.has(*action))
        .map(|(_, shown)| shown as i32)
        .collect()
    }

    pub fn new(
        machines: Machines,
        builds: Box<dyn Builds + Send + Sync>,
        feed: Arc<dyn MachineFeedPort>,
        relay_links: Arc<dyn MachineLinksPort>,
        reports: Arc<dyn MachinePeerReportsPort>,
        clock: Clock,
    ) -> Self {
        let work = Self {
            machines,
            builds,
            seen: Mutex::new(HashMap::new()),
            feed,
            shown: Mutex::new(BTreeMap::new()),
            links: Mutex::new(HashMap::new()),
            clock,
            plans: Mutex::new(HashMap::new()),
            skills: OnceLock::new(),
            inbox: OnceLock::new(),
            relay_links,
            reports,
            peer_dial: OnceLock::new(),
            login: OnceLock::new(),
            host_probe: OnceLock::new(),
            probes: Mutex::new(HashMap::new()),
            forgetting: forget::Forgetful::default(),
            stops: OnceLock::new(),
            upgrade: OnceLock::new(),
            scripts: scripts::Hook::default(),
        };
        *lock(&work.shown) = work.listed().unwrap_or_default();
        work
    }

    fn listed(&self) -> Result<BTreeMap<String, pb::Machine>, ManageError> {
        Ok(self
            .machines
            .list()?
            .iter()
            .map(|entry| (entry.name.clone(), self.machine(entry)))
            .collect())
    }

    /// Puts each machine that changed since the feed last told it on the
    /// feed: added, changed, or gone (with how it last looked).
    pub fn publish_machines(&self) {
        let mut shown = lock(&self.shown);
        let Ok(now) = self.listed() else {
            // The list will be read again at the next change.
            return;
        };
        for (name, machine) in &now {
            match shown.get(name) {
                None => self.feed.publish(
                    pb::Change::Created,
                    pb::event::Payload::Machine(machine.clone()),
                    None,
                ),
                Some(before) if before != machine => {
                    self.feed.publish(
                        pb::Change::Updated,
                        pb::event::Payload::Machine(machine.clone()),
                        None,
                    );
                }
                Some(_) => {}
            }
        }
        for (name, machine) in shown.iter() {
            if !now.contains_key(name) {
                self.feed.publish(
                    pb::Change::Deleted,
                    pb::event::Payload::Machine(machine.clone()),
                    None,
                );
            }
        }
        *shown = now;
    }

    /// Runs `with` with a [`Drive`] that runs commands as child processes
    /// and hands each step's progress to `events`.
    pub fn drive<R>(
        &self,
        events: &mut dyn FnMut(StepEvent),
        cancelled: &dyn Fn() -> bool,
        with: impl FnOnce(&Machines, &mut Drive<'_>) -> R,
    ) -> R {
        let runner = SystemRunner::new(&());
        let mut tell = |event: StepEvent| {
            events(event);
            self.publish_machines();
        };
        let mut drive = Drive {
            runner: &runner,
            builds: &*self.builds,
            wait: &std::thread::sleep,
            events: &mut tell,
            cancelled,
        };
        let result = with(&self.machines, &mut drive);
        self.publish_machines();
        result
    }

    /// An add or reconnect reached `machine` (ONLINE) or failed.
    pub fn saw(&self, machine: &str, reach: pb::Reachability) {
        let now = (self.clock)();
        self.change(machine, |seen| {
            seen.reach = reach;
            if reach == pb::Reachability::Online {
                seen.last_seen_at = Some(now);
                seen.next_retry_at = None;
            }
        });
    }

    fn change(&self, machine: &str, with: impl FnOnce(&mut Seen)) {
        with(lock(&self.seen).entry(machine.to_owned()).or_default());
        self.publish_machines();
    }

    /// The link came up, or went down, dialling again after `next_retry_at`.
    fn link_changed(&self, machine: &str, event: &LinkEvent) {
        let now = (self.clock)();
        self.change(machine, |seen| match event {
            LinkEvent::Up { daemon_version, .. } => {
                seen.reach = pb::Reachability::Online;
                daemon_version.clone_into(&mut seen.daemon_version);
                seen.last_seen_at = Some(now);
                seen.next_retry_at = None;
            }
            LinkEvent::Down { next_retry_at, .. } => {
                if seen.reach == pb::Reachability::Online {
                    seen.last_seen_at = Some(now);
                }
                seen.reach = pb::Reachability::Offline;
                seen.next_retry_at = *next_retry_at;
            }
            LinkEvent::IdentityMismatch { .. } => {
                if seen.reach == pb::Reachability::Online {
                    seen.last_seen_at = Some(now);
                }
                seen.reach = pb::Reachability::IdentityMismatch;
                seen.next_retry_at = None;
            }
        });
    }

    /// Starts keeping `machine`'s link up via `abort` (e.g. from an interactive
    /// login one-shot pipe), replacing any existing link supervisor.
    pub fn start_link_handle(&self, machine: &str, abort: AbortHandle) {
        if let Some(before) = lock(&self.links).insert(machine.to_owned(), abort) {
            before.abort();
        }
    }

    /// Keeps `machine`'s link up through its link command, if the host
    /// holds its key (written when the machine was added or reconnected).
    pub fn link_up(self: &Arc<Self>, machine: &str, _runtime: &Handle) {
        if !self.machines.peer_key(machine).exists() {
            return;
        }
        let Some(dial) = self.peer_dial.get() else {
            return;
        };
        let entry = match self.machines.get(machine) {
            Ok(entry) => entry,
            Err(error) => {
                humata2_rpc_kit::log::say!(
                    "humatad2: couldn't read {machine} to start its peer link: {}",
                    chain(&error)
                );
                return;
            }
        };
        let work = Arc::downgrade(self);
        let name = machine.to_owned();
        let on_event = Arc::new(move |event: LinkEvent| tell_link(&work, &name, event));
        let abort = dial.spawn_supervisor(entry, on_event);
        if let Some(before) = lock(&self.links).insert(machine.to_owned(), abort) {
            before.abort();
        }
    }

    /// Stops keeping `machine`'s link up.
    pub fn link_down(&self, machine: &str) {
        if let Some(link) = lock(&self.links).remove(machine) {
            link.abort();
        }
        if let Some(skills) = self.skills.get() {
            skills.link_down(machine);
        }
        self.peer_down(machine);
    }

    pub fn machine(&self, entry: &HostEntry) -> pb::Machine {
        let seen = lock(&self.seen).get(&entry.name).cloned().unwrap_or_default();
        let reachability = match entry.state {
            EntryState::Adding | EntryState::Removing => pb::Reachability::Connecting,
            EntryState::Ready => seen.reach,
        };
        // A machine reached only by a login the person gives shows that it
        // needs them when its link is down (MP-16).
        let reachability = self
            .login
            .get()
            .map_or(reachability, |login| login.needs_login(entry, reachability));
        let stop = self.stops.get().and_then(|stops| stops.get(&entry.name));
        pb::Machine {
            id: Some(machine_id(&entry.name)),
            name: entry.name.clone(),
            address: entry.ssh_host.clone(),
            reachability: reachability as i32,
            os: entry.os.clone(),
            arch: entry.arch.clone(),
            daemon_version: seen.daemon_version,
            last_seen_at: seen.last_seen_at.map(timestamp),
            next_retry_at: seen.next_retry_at.map(timestamp),
            root: entry.root.clone(),
            provider: entry.provider.as_ref().map(|record| pb::ProviderId {
                value: record.id.clone(),
            }),
            sessions_stopped: stop.as_ref().map(|copy| copy.pb_record.clone()),
            stop_delivered: stop.is_some_and(|copy| copy.delivered),
            ..pb::Machine::default()
        }
    }
}

/// A link's change as the machine's reachability, and in the log.
fn tell_link(work: &Weak<Work>, machine: &str, event: LinkEvent) {
    let Some(work) = work.upgrade() else {
        return;
    };
    match &event {
        LinkEvent::Up { daemon_version, .. } => {
            humata2_rpc_kit::log::say!(
                "humatad2: the peer link to {machine} is up, running humatad2 {daemon_version}"
            );
        }
        LinkEvent::Down { reason, .. } | LinkEvent::IdentityMismatch { reason } => {
            humata2_rpc_kit::log::say!("humatad2: {reason}");
        }
    }
    // Skills hear of the link first, so the machine's channel is there by
    // the time the feed shows it online.
    if let Some(skills) = work.skills.get() {
        match &event {
            LinkEvent::Up { channel, .. } => skills.link_up(machine, channel.clone()),
            LinkEvent::Down { .. } | LinkEvent::IdentityMismatch { .. } => skills.link_down(machine),
        }
    }
    // So do the relay registry and the report sink (D139).
    match &event {
        LinkEvent::Up { channel, .. } => work.peer_up(machine, channel),
        LinkEvent::Down { .. } | LinkEvent::IdentityMismatch { .. } => work.peer_down(machine),
    }
    // The host's copy of the machine's stop agrees with the machine's own
    // before it shows online, and a stop it never got is sent (D262).
    if let LinkEvent::Up {
        channel,
        stopped,
        daemon_version,
    } = &event
    {
        if let Some(stops) = work.stops.get() {
            stops.link_up(machine, *stopped, channel.clone());
        }
        if let Some(upgrade) = work.upgrade.get() {
            upgrade.link_up(machine, daemon_version);
        }
        // The host's stack settings, when the machine's copy differs (D411).
        stack_settings::link_up(&work, machine);
        // MACHINESCRIPTS (D681): `added` for the first link-up after an add, else `connected`.
        work.fire_link_up(machine);
    }
    work.link_changed(machine, &event);
}

fn timestamp(at: SystemTime) -> humata2_grpc::Timestamp {
    let since = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    humata2_grpc::Timestamp {
        seconds: i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(since.subsec_nanos()).unwrap_or(0),
    }
}

/// At start: finishes or undoes what a restart cut short (PER-27), on a
/// thread of its own so serving doesn't wait for slow machines, then keeps
/// each ready machine's peer link up.
pub fn recover_in_background(work: Arc<Work>) {
    let runtime = Handle::try_current().ok();
    std::thread::spawn(move || {
        let mut ignore = |_: StepEvent| {};
        match work.drive(&mut ignore, &|| false, |machines, drive| machines.recover(drive)) {
            Ok(recovered) => {
                for machine in recovered {
                    if machine.found == EntryState::Removing
                        && machine.outcome.is_ok()
                        && let Some(runtime) = &runtime
                    {
                        work.removed(&machine.machine, runtime);
                    }
                    if let Err(error) = machine.outcome {
                        humata2_rpc_kit::log::say!(
                            "humatad2: {} was left {:?} by the last run and couldn't be finished yet: {}",
                            machine.machine,
                            machine.found,
                            chain(&error)
                        );
                    }
                }
            }
            Err(error) => humata2_rpc_kit::log::say!(
                "humatad2: couldn't finish the machine adds and removals the last run left: {}",
                chain(&error)
            ),
        }
        if let (Some(runtime), Ok(entries)) = (runtime, work.machines.list()) {
            for entry in entries.iter().filter(|entry| entry.state == EntryState::Ready) {
                work.link_up(&entry.name, &runtime);
            }
        }
    });
}

impl MachineSvc {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        guard: Arc<dyn MachineGuardPort>,
        work: Arc<Work>,
        operations: Arc<dyn MachineOperationsPort>,
        inbox: Arc<dyn MachineInboxPort>,
        storage: Arc<dyn MachineStoragePort>,
        channels: Arc<dyn MachineChannelsPort>,
        host_vars: Arc<dyn MachineHostVarsPort>,
        holders: Arc<dyn MachineHoldersPort>,
    ) -> Self {
        let svc = Self {
            guard,
            work,
            operations,
            inbox,
            storage,
            channels,
            host_vars,
            holders,
        };
        // A change of a box's folder the last run began and didn't end is asked again.
        svc.recover_box_roots();
        svc
    }

    /// The machines an agent's channel lists (TP-1), when its rules let it
    /// read that channel as GetChannel does; none for a session with no
    /// channel, or one whose rule would ask first.
    fn channel_machines(&self, caller: &Caller) -> Result<Vec<String>, Status> {
        let Caller::Agent(agent) = caller else {
            return Ok(Vec::new());
        };
        let channel = agent.session.channel.as_str();
        if channel.is_empty() {
            return Ok(Vec::new());
        }
        let here = self.guard.machine().to_owned();
        let Some(record) = self
            .storage
            .store()
            .channel(channel)
            .map_err(|error| store_error(error, &here, "read the channel's machines"))?
        else {
            return Ok(Vec::new());
        };
        let call = Call::Use {
            capability: Capability::ChannelRead,
            target: Target::Channel(humata2_access::ChannelId::new(channel)),
        };
        if !matches!(self.guard.check(caller, &call)?, Checked::Allowed) {
            return Ok(Vec::new());
        }
        Ok(record.machines)
    }

    /// An agent's ListMachines that its machine forwarded here (D768): its
    /// channel's machines, as here, with This Mac first when the channel
    /// has it. An agent on another machine can't name This Mac by its own
    /// daemon's "this machine", so the host names itself.
    pub async fn list_for_machine_agent(
        &self,
        request: Request<pb::ListMachinesRequest>,
    ) -> Result<Response<pb::ListMachinesResponse>, Status> {
        let who = caller(&request)?;
        let here = self.guard.machine().to_owned();
        let has_this = self.channel_machines(&who)?.iter().any(|machine| {
            machine.is_empty() || *machine == here || machine.as_str() == change_root::THIS_MAC_ALIAS
        });
        let mut listed =
            <Self as pb::machine_service_server::MachineService>::list_machines(self, request)
                .await?
                .into_inner();
        if has_this {
            listed.machines.insert(
                0,
                pb::Machine {
                    id: Some(machine_id(&here)),
                    name: "This Mac".to_owned(),
                    is_this_mac: true,
                    reachability: pb::Reachability::Online as i32,
                    ..pb::Machine::default()
                },
            );
        }
        Ok(Response::new(listed))
    }

    /// Whether `name` is this machine, by its id or as the CLI and app call it.
    fn is_this(&self, name: &str) -> bool {
        name == self.guard.machine() || name == change_root::THIS_MAC_ALIAS
    }

    /// The machine a root call is about, under the name its records use.
    fn root_machine(&self, name: String) -> String {
        if self.is_this(&name) {
            self.guard.machine().to_owned()
        } else {
            name
        }
    }

    /// A machine's root now, and what changing it reaches: its channels and
    /// their running sessions. This Mac's root is the one saved for it, else
    /// the folder the daemon keeps channels in; a box's is its registry entry's.
    async fn root_facts(&self, name: &str) -> Result<(String, change_root::Affected), Status> {
        let here = self.guard.machine().to_owned();
        if name == here {
            return self
                .storage_blocking(move |storage| change_root::own_facts(storage, &here))
                .await;
        }
        let box_root = self
            .work
            .machines
            .get(name)
            .map_err(|error| status_of(&error))?
            .root;
        let machine = name.to_owned();
        self.storage_blocking(move |storage| {
            let found = change_root::affected(storage.store(), &machine, &here)
                .map_err(|error| store_error(error, &here, "list the channels under the Humata folder"))?;
            Ok((box_root, found))
        })
        .await
    }

    /// Storage is the user's page: agents and scripts are refused. Returns
    /// the other machine the call names, which the caller relays to; none
    /// for this machine.
    fn storage_call(
        &self,
        caller: &Caller,
        machine: Option<&pb::MachineId>,
    ) -> Result<Option<String>, Status> {
        match caller {
            Caller::User(_) => {}
            Caller::Peer => return Err(refused(&Refusal::PeerCannotCall)),
            Caller::Agent(_) | Caller::Script(_) => {
                return Err(refused(&Refusal::UserOnly { capability: None }));
            }
        }
        match machine
            .map(|id| id.value.as_str())
            .filter(|name| !name.is_empty())
        {
            Some(name) if name != self.guard.machine() => Ok(Some(name.to_owned())),
            _ => Ok(None),
        }
    }

    /// D784: Stop, End and Adopt of a running holder end or take over a
    /// terminal, so an agent may make them only under the machine-destroy
    /// switch (Never by default), as it may remove a machine; the user
    /// always may, and a script or peer never. Returns the other machine
    /// the call names, as [`MachineSvc::storage_call`] does.
    fn destroy_call(
        &self,
        caller: &Caller,
        machine: Option<&pb::MachineId>,
    ) -> Result<Option<String>, Status> {
        if let Caller::Agent(_) = caller {
            let named = machine
                .map(|id| id.value.as_str())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| self.guard.machine())
                .to_owned();
            self.require(caller, &named, MachineActionKind::Delete)?;
            return Ok(Some(named).filter(|name| name != self.guard.machine()));
        }
        self.storage_call(caller, machine)
    }

    /// The machine service on `machine`'s live link: a machine the host
    /// doesn't know is not found, one it can't reach is offline at once
    /// (TR-6). Requests made with it are built from their message alone.
    fn storage_client(
        &self,
        machine: &str,
    ) -> Result<pb::machine_service_client::MachineServiceClient<tonic::transport::Channel>, Status> {
        self.work
            .machines
            .get(machine)
            .map_err(|error| status_of(&error))?;
        let channel = self.work.relay_links.channel(machine).map_err(Status::from)?;
        Ok(pb::machine_service_client::MachineServiceClient::new(channel))
    }

    /// The plan to change a box's Humata folder, made by the box's own
    /// daemon, which knows its channels and running sessions; its token is
    /// the box's. The folder shown as the old one is the one the host keeps
    /// for the box, which is what the machine's page shows. A box that can't
    /// be reached answers offline at once (TR-6).
    async fn plan_box_root(&self, name: &str, new_root: String) -> Result<pb::ChangeRootPlan, Status> {
        let mut client = self.storage_client(name)?;
        let shown = self
            .work
            .machines
            .get(name)
            .map_err(|error| status_of(&error))?
            .root;
        let mut plan = client
            .plan_change_root(Request::new(pb::PlanChangeRootRequest {
                id: None,
                root: new_root,
            }))
            .await?
            .into_inner();
        plan.machine = Some(machine_id(name));
        plan.old_root = shown;
        Ok(plan)
    }

    /// The checks every host-variable call makes: the user's alone (an agent
    /// or script is refused, and so is a peer), and the machine is this one
    /// (`None`) or a known box, whose name is returned (HO-7, HO-13).
    fn variables_call(
        &self,
        caller: &Caller,
        id: Option<pb::MachineId>,
    ) -> Result<Option<String>, Status> {
        if matches!(caller, Caller::Agent(_)) {
            return Err(user_only_call(
                Capability::SettingsChange,
                "reading or changing host variables",
                "ask the user to set the variable in Settings › Machines › Providers, under the Commands of the provider that reads it; humata provider ls says which variables each provider reads and whether they are set",
            ));
        }
        user_only(caller)?;
        match id.map(|id| id.value).filter(|name| !name.is_empty()) {
            Some(name) if name != self.guard.machine() => {
                self.work
                    .machines
                    .get(&name)
                    .map_err(|error| status_of(&error))?;
                Ok(Some(name))
            }
            _ => Ok(None),
        }
    }

    /// Runs `work` on this machine's variables on a blocking thread.
    async fn on_variables<R: Send + 'static>(
        &self,
        work: impl FnOnce(&dyn MachineHostVarsPort) -> Result<R, crate::ports::HostVarsError> + Send + 'static,
    ) -> Result<R, Status> {
        let host_vars = self.host_vars.clone();
        tokio::task::spawn_blocking(move || work(host_vars.as_ref()))
            .await
            .map_err(|error| internal(format!("the host variable work stopped: {error}"), ""))?
            .map_err(|error| error.status())
    }

    /// A client of `machine`'s own MachineService over its live link. A
    /// machine whose link is down is offline at once, never waited for (HO-5).
    fn box_client(
        &self,
        machine: &str,
    ) -> Result<pb::machine_service_client::MachineServiceClient<tonic::transport::Channel>, Status> {
        let channel = self.work.links().channel(machine).map_err(Status::from)?;
        Ok(pb::machine_service_client::MachineServiceClient::new(channel))
    }

    /// Runs `work` on a blocking thread, as `Storage` does file work.
    async fn storage_blocking<R: Send + 'static>(
        &self,
        work: impl FnOnce(&dyn MachineStoragePort) -> Result<R, Status> + Send + 'static,
    ) -> Result<R, Status> {
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || work(storage.as_ref()))
            .await
            .map_err(|error| internal(format!("the storage work stopped: {error}"), ""))?
    }

    fn require(
        &self,
        caller: &Caller,
        machine: &str,
        action: MachineActionKind,
    ) -> Result<(), Status> {
        if matches!(caller, Caller::Peer) {
            return Err(refused(&Refusal::PeerCannotCall));
        }
        self.guard.require(
            caller,
            &Call::Use {
                capability: action.capability(),
                target: Target::Machine(MachineAction {
                    machine: MachineId::new(machine),
                    action,
                }),
            },
        )
    }

    /// Checks the call as [`MachineSvc::require`] does, but a rule of Ask me
    /// answers `Some(request)` for the caller to put in the Inbox rather than
    /// refusing (D38.6). An `approved` call goes on.
    fn gate(
        &self,
        caller: &Caller,
        machine: &str,
        action: MachineActionKind,
        approved: bool,
    ) -> Result<Option<Box<AccessRequest>>, Status> {
        if matches!(caller, Caller::Peer) {
            return Err(refused(&Refusal::PeerCannotCall));
        }
        gate(
            self.guard.as_ref(),
            caller,
            &Call::Use {
                capability: action.capability(),
                target: Target::Machine(MachineAction {
                    machine: MachineId::new(machine),
                    action,
                }),
            },
            approved,
        )
    }

    /// The checks for stopping or starting a box's sessions: the machine is
    /// a known box, the caller may (an agent only when its rule allows it),
    /// and a user is present (D263).
    fn sessions_call(
        &self,
        caller: &Caller,
        machine: &str,
        action: MachineActionKind,
    ) -> Result<(), Status> {
        self.require(caller, machine, action)?;
        if matches!(caller, Caller::User(_)) && !self.guard.is_present_user(caller) {
            return Err(refused(&Refusal::UserOnly {
                capability: Some(action.capability()),
            }));
        }
        if machine == self.guard.machine() {
            return Err(failed(
                Code::FailedPrecondition,
                &ThisMachine,
                "run `humata stop` or `humata start` on this machine itself",
                None,
            ));
        }
        self.work
            .machines
            .get(machine)
            .map_err(|error| status_of(&error))?;
        Ok(())
    }

    /// The body of AddSshMachine; `approved` once the user said yes to it
    /// in the Inbox (D38.6, TP-13).
    async fn add_ssh(
        &self,
        caller: &Caller,
        request: pb::AddSshMachineRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        let add = AddSsh {
            ssh_host: request.ssh_host,
            name: request.name,
            root: request.root,
            trust_fingerprint: Some(request.trust_host_key_fingerprint)
                .filter(|value| !value.trim().is_empty()),
        };
        if matches!(caller, Caller::Peer) {
            return Err(refused(&Refusal::PeerCannotCall));
        }
        // A retried call finds the operation it started, even after the
        // machine's name was taken by it.
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(operation));
        }
        let plan = self
            .work
            .machines
            .plan_add(&add)
            .map_err(|error| status_of(&error))?;
        let name = plan.machine.as_str().to_owned();
        if let Some(asked) = self.gate(caller, &name, MachineActionKind::Add, approved)? {
            return Ok(Gated::Ask(asked));
        }
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Machine(machine_id(&name))),
        };
        let progress = match self.operations.start_op(
            caller,
            pb::OperationKind::AddMachine,
            target,
            &OpStep::ADD,
            true,
            request.idempotency_key,
        )? {
            OperationStarted::Started(progress) => progress,
            OperationStarted::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let work = self.work.clone();
        let operation = self.run(
            progress,
            Arc::new(move |machines, drive| {
                let added = machines.add(&add, plan.clone(), drive);
                match &added {
                    Ok(entry) => {
                        work.saw(&entry.name, pb::Reachability::Online);
                        // A machine added afresh starts its journal over.
                        work.forget_journal_now(&entry.name);
                        // MACHINESCRIPTS (D681): its first link-up is the `added` trigger.
                        work.note_added(&entry.name);
                        work.link_up(&entry.name, &Handle::current());
                    }
                    Err(error) => work.saw(&name, reach_after(error)),
                }
                added.map(|entry| Some(pb::operation::Result::Machine(machine_id(&entry.name))))
            }),
        );
        Ok(Gated::Done(operation))
    }

    /// The body of ReconnectMachine; `approved` once the user said yes to it
    /// in the Inbox (D38.6, TP-13).
    async fn reconnect(
        &self,
        caller: &Caller,
        request: pb::ReconnectMachineRequest,
        approved: bool,
    ) -> Result<Gated<pb::Machine>, Status> {
        let name = machine_of(request.id)?;
        if let Some(asked) = self.gate(caller, &name, MachineActionKind::Reconnect, approved)? {
            return Ok(Gated::Ask(asked));
        }
        let work = self.work.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut ignore = |_: StepEvent| {};
            let result = work.drive(&mut ignore, &|| false, |machines, drive| {
                machines.reconnect(&name, drive)
            });
            match &result {
                Ok(entry) => {
                    work.saw(&entry.name, pb::Reachability::Online);
                    work.link_up(&entry.name, &Handle::current());
                }
                Err(ManageError::Busy { .. }) => {}
                // A reconnect finishes a removal left `Removing` and then
                // finds no machine (D760).
                Err(ManageError::NotFound { .. }) => work.removed(&name, &Handle::current()),
                Err(error) => work.saw(&name, reach_after(error)),
            }
            result.map(|entry| work.machine(&entry))
        })
        .await
        .map_err(|error| internal(format!("the reconnect stopped: {error}"), ""))?;
        result.map(Gated::Done).map_err(|error| status_of(&error))
    }

    /// Runs `job` for the operation on a blocking thread and answers with
    /// the operation as it starts; Retry runs `job` again.
    fn run(&self, progress: Arc<dyn OperationProgress>, job: Job) -> pb::Operation {
        let operation = progress.operation();
        launch(self.work.clone(), progress, job);
        operation
    }
}

/// An operation's work, which may run more than once (Retry).
pub type Job = Arc<
    dyn Fn(&Machines, &mut Drive<'_>) -> Result<Option<pb::operation::Result>, ManageError>
        + Send
        + Sync,
>;

fn launch(shared: Arc<Work>, progress: Arc<dyn OperationProgress>, job: Job) {
    launch_observed(shared, progress, job, None);
}

/// What sees each step event of an operation's work, besides the operation.
pub type Observer = Arc<dyn Fn(&StepEvent) + Send + Sync>;

/// As `launch`, with `observer` told each step event, on every run.
pub fn launch_observed(
    shared: Arc<Work>,
    progress: Arc<dyn OperationProgress>,
    job: Job,
    observer: Option<Observer>,
) {
    let again = (shared.clone(), job.clone(), observer.clone());
    progress.rerun_with(Arc::new(move |progress| {
        launch_observed(again.0.clone(), progress, again.1.clone(), again.2.clone());
    }));
    let id = progress
        .operation()
        .id
        .map(|id| id.value)
        .unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        let mut events = |event: StepEvent| {
            if let Some(observer) = &observer {
                observer(&event);
            }
            progress.step_event(&event);
        };
        let cancelled = || progress.cancelled();
        let result = shared.drive(&mut events, &cancelled, |machines, drive| job(machines, drive));
        match result {
            Ok(result) => progress.succeed(result),
            Err(ManageError::Cancelled { .. }) => progress.end_cancelled(),
            Err(error) => progress.fail(error_of(&error, Some(&id))),
        }
    });
}

/// Why the providers file couldn't be read.
#[derive(Debug, thiserror::Error)]
pub enum ProvidersFileError {
    #[error(
        "couldn't read the providers file {}: {source}. Check that it is yours and readable, then try again",
        .path.display()
    )]
    Read {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the providers file on disk isn't valid: {source}")]
    Invalid {
        #[source]
        source: ConfigError,
    },
}

/// Where the providers file is: beside `hosts.json` (MP-2).
pub fn providers_path(hosts_file: &std::path::Path) -> std::path::PathBuf {
    hosts_file.with_file_name("providers.toml")
}

/// The providers file's text (empty when there is none) and the providers
/// it defines, without the built-in Lima example.
pub fn read_providers(
    hosts_file: &std::path::Path,
) -> Result<(String, Vec<Provider>), ProvidersFileError> {
    let path = providers_path(hosts_file);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => return Err(ProvidersFileError::Read { path, source }),
    };
    let providers =
        Provider::parse(&text).map_err(|source| ProvidersFileError::Invalid { source })?;
    Ok((text, providers))
}

pub fn machine_id(name: &str) -> pb::MachineId {
    pb::MachineId {
        value: name.to_owned(),
    }
}

/// A Humata folder the user typed: an absolute path or one under `~/`, with
/// no trailing slash.
fn root_of(root: &str) -> Result<String, Status> {
    let root = root.trim();
    if root.is_empty() {
        return Err(missing("root"));
    }
    let root = root.trim_end_matches('/');
    let under_home = root.strip_prefix("~/").is_some_and(|rest| !rest.is_empty());
    if (!root.starts_with('/') && !under_home) || root == "/" || root.chars().any(char::is_control)
    {
        return Err(invalid_field(
            "root",
            pb::invalid_field::Problem::BadFormat,
            format!("`{root}` can't be a Humata folder: it must be a full path or start with ~/"),
            "type a path such as ~/Humata or /srv/humata",
        ));
    }
    Ok(root.to_owned())
}

/// Changing a machine's root is the user's call alone.
fn user_only(caller: &Caller) -> Result<(), Status> {
    match caller {
        Caller::User(_) => Ok(()),
        Caller::Peer => Err(refused(&Refusal::PeerCannotCall)),
        Caller::Agent(_) | Caller::Script(_) => {
            Err(refused(&Refusal::UserOnly { capability: None }))
        }
    }
}

/// What a change-root plan is filed under: the machine and the root it
/// was made for. A machine name has no colon, so this is no machine's name.
fn root_plan_key(machine: &str, root: &str) -> String {
    format!("root:{machine}:{root}")
}

pub fn machine_of(id: Option<pb::MachineId>) -> Result<String, Status> {
    id.map(|id| id.value)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| missing("id"))
}

/// The reachability a failed connect leaves.
pub fn reach_after(error: &ManageError) -> pb::Reachability {
    match error {
        ManageError::HostKeyMismatch { .. }
        | ManageError::Step {
            source: SetupError::HostKeyChanged { .. },
            ..
        } => pb::Reachability::HostKeyChanged,
        ManageError::LoginNeeded { .. } => pb::Reachability::NeedsLogin,
        _ => pb::Reachability::Offline,
    }
}

/// `error` as the API's Error. `operation` names the operation it failed.
pub fn error_of(error: &ManageError, operation: Option<&str>) -> pb::Error {
    use pb::error::Kind;
    let message = chain(error);
    let (fix, kind) = match error {
        ManageError::InvalidSshHost { .. } => (
            "type an alias from ~/.ssh/config, or user@host, or user@host:port",
            Some(Kind::InvalidField(pb::InvalidField {
                field: "ssh_host".to_owned(),
                problem: pb::invalid_field::Problem::BadFormat as i32,
            })),
        ),
        ManageError::BadName { .. } => (
            "use letters, digits and dashes, or leave the name empty to take it from the host",
            Some(Kind::InvalidField(pb::InvalidField {
                field: "name".to_owned(),
                problem: pb::invalid_field::Problem::BadFormat as i32,
            })),
        ),
        ManageError::NameTaken { machine } => (
            "pick another name, or remove the machine that has it first",
            Some(Kind::NameTaken(pb::NameTaken {
                kind: pb::EntityKind::Machine as i32,
                name: machine.clone(),
                suggestion: format!("{machine}-2"),
            })),
        ),
        ManageError::BeingRemoved { .. } => (
            "wait for the removal to finish (the machine leaves Hosts when it does), then add it again. A removal waiting for its machine finishes once the machine is back or Humata restarts",
            None,
        ),
        ManageError::NotFound { machine } => (
            "list the machines to see their names",
            Some(Kind::NotFound(pb::NotFound {
                kind: pb::EntityKind::Machine as i32,
                id: machine.clone(),
                in_trash: false,
            })),
        ),
        ManageError::Busy { .. } | ManageError::NotReady { .. } => (
            "wait for the running add, reconnect or remove to finish, then try again",
            None,
        ),
        ManageError::HostKeyUnconfirmed {
            machine,
            algorithm,
            fingerprint,
        } => (
            "compare the fingerprint with the one the machine itself shows (ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub on it), then add it again with that fingerprint",
            Some(Kind::HostKeyUnknown(pb::HostKeyUnknown {
                machine: Some(machine_id(machine)),
                algorithm: algorithm.clone(),
                presented_fingerprint: fingerprint.clone(),
            })),
        ),
        ManageError::HostKeyMismatch {
            machine,
            expected,
            presented,
        } => (
            "if the machine wasn't rebuilt, someone may be intercepting the connection: check the key on the machine itself before trusting another one",
            Some(Kind::HostKeyChanged(pb::HostKeyChanged {
                machine: Some(machine_id(machine)),
                expected_fingerprint: expected.clone(),
                presented_fingerprint: presented.clone(),
            })),
        ),
        ManageError::LoginNeeded { machine, .. } => (
            "make key login work without a password or passphrase prompt (ssh-copy-id, or load the key into ssh-agent), then try again. If this machine needs a login each time (a code, a web sign-in), add it through its provider instead, which can open the login for you",
            Some(Kind::SshFailed(pb::SshFailed {
                problem: pb::ssh_failed::Problem::AuthFailed as i32,
                machine: Some(machine_id(machine)),
                address: String::new(),
            })),
        ),
        ManageError::Step {
            step: OpStep::Identity,
            source: SetupError::Identity(IdentityError::Field { field, .. }),
            ..
        } => (
            "fix the identity command in Settings › Machines › Providers so that field is right; what was made is still there (use Delete what was made)",
            Some(Kind::ProviderIdentityInvalid(pb::ProviderIdentityInvalid {
                provider: None,
                field: field.clone(),
                problem: pb::invalid_field::Problem::BadFormat as i32,
            })),
        ),
        ManageError::Step {
            step: step @ (OpStep::Create | OpStep::Identity),
            output_tail,
            source: SetupError::CommandFailed { exit, .. },
            ..
        } => (
            "the step's output says why the provider's command failed; fix it in Settings › Machines › Providers, then try again (Delete what was made removes anything it left)",
            Some(Kind::ProviderStepFailed(pb::ProviderStepFailed {
                provider: None,
                step: match step {
                    OpStep::Create => pb::provider_step_failed::Step::Create,
                    _ => pb::provider_step_failed::Step::Identity,
                } as i32,
                exit_code: match exit {
                    humata2_machines::Exit::Code(code) => *code,
                    _ => -1,
                },
                output_tail: output_tail.clone(),
            })),
        ),
        ManageError::Step {
            source: SetupError::Builds(humata2_machines::BuildsError::HostNeedsMigrate { .. }),
            ..
        } => (
            "Migrate the daemon on This Mac first (the Inbox item), then add the machine again; nothing was installed on the machine",
            None,
        ),
        ManageError::Step {
            source:
                SetupError::HostKeyChanged {
                    machine,
                    expected_fingerprint,
                    presented_fingerprint,
                },
            ..
        } => (
            "if the machine wasn't rebuilt, someone may be intercepting the connection: check the key on the machine itself before trusting another one",
            Some(Kind::HostKeyChanged(pb::HostKeyChanged {
                machine: Some(machine_id(machine.as_str())),
                expected_fingerprint: expected_fingerprint.clone().unwrap_or_default(),
                presented_fingerprint: presented_fingerprint.clone().unwrap_or_default(),
            })),
        ),
        ManageError::Step { step, .. } => (
            "the steps above say what failed; humatad2 was removed from the machine again, but anything a provider made is still there. Fix the cause and add the machine again",
            operation.map(|operation| {
                step_failed(
                    operation,
                    *step,
                    &message,
                    "humatad2 and its files were removed from the machine again",
                )
            }),
        ),
        ManageError::UninstallFailed { .. } => (
            "reconnect when the machine is back to finish removing it",
            operation.map(|operation| {
                step_failed(
                    operation,
                    OpStep::Uninstall,
                    &message,
                    "the machine stays listed until humatad2 is removed from it",
                )
            }),
        ),
        ManageError::Run { .. } | ManageError::Quote { .. } => (
            "check that ssh is installed and on PATH for humatad2, then try again",
            None,
        ),
        ManageError::Cancelled { .. } => ("add the machine again when you want it", None),
        ManageError::SavePeerKey { .. } | ManageError::DeletePeerKey { .. } => (
            "fix the permissions of the peer-keys folder in the state folder, then try again",
            None,
        ),
        ManageError::Params { source } => {
            let field = match source {
                humata2_machines::ValuesError::Missing { param, .. }
                | humata2_machines::ValuesError::Undeclared { param, .. }
                | humata2_machines::ValuesError::Nul { param } => format!("params[{param}]"),
            };
            (
                "give exactly the params the provider declares, each without a NUL byte",
                Some(Kind::InvalidField(pb::InvalidField {
                    field,
                    problem: pb::invalid_field::Problem::BadFormat as i32,
                })),
            )
        }
        ManageError::RunOnUnsupported { .. } => (
            "remove `run_on` from the provider in Settings › Machines › Providers",
            None,
        ),
        ManageError::ProviderLoginNeeded {
            machine, reason, ..
        } => {
            let (prompt_line, hung) = match reason {
                humata2_machines::LoginReason::Prompted(prompt) => (prompt.line.clone(), false),
                humata2_machines::LoginReason::Hung => (String::new(), true),
            };
            (
                "log in to the machine interactively (Open interactively), or make the provider's connect command log in without a prompt",
                Some(Kind::LoginNeedsUser(pb::LoginNeedsUser {
                    provider: None,
                    machine: Some(machine_id(machine)),
                    prompt_kind: pb::LoginPromptKind::Unspecified as i32,
                    prompt_line,
                    hung,
                })),
            )
        }
        ManageError::NotProviderMachine { .. } => ("pick a machine the provider made", None),
        ManageError::ActionUndefined { .. } => (
            "add the command to the provider in Settings › Machines › Providers",
            None,
        ),
        ManageError::ActionFailed { .. } => (
            "the step's output says why; fix the command or the machine, then run it again",
            None,
        ),
        ManageError::DeleteFailed { .. } => (
            "delete what the provider made with its own tool; Humata no longer lists the machine",
            None,
        ),
        ManageError::ProviderLinkLost { .. } => ("remove the machine and add it again", None),
        ManageError::BadEnvName { .. } => (
            "name script variables with capital letters, digits and `_`, not starting with a digit",
            None,
        ),
        ManageError::Hosts(_) | ManageError::KnownHosts(_) => {
            ("fix the file the message names, then try again", None)
        }
    };
    // MACHINESCRIPTS w4c: a host behind the app has its own kind, which the
    // app turns into the inline Migrate This Mac action.
    let kind = kind.or_else(|| match error {
        ManageError::Step {
            source: SetupError::Builds(humata2_machines::BuildsError::HostNeedsMigrate { platform }),
            ..
        } => Some(Kind::HostNeedsMigrate(pb::HostNeedsMigrate {
            platform: platform.to_string(),
        })),
        _ => None,
    });
    pb::Error {
        message,
        fix: fix.to_owned(),
        retry: None,
        kind,
    }
}

#[cfg(test)]
mod host_needs_migrate_tests {
    use super::*;
    use humata2_machines::{BuildsError, Platform};

    fn step_error(source: SetupError) -> ManageError {
        ManageError::Step {
            machine: "devbox".to_owned(),
            step: OpStep::Install,
            output_tail: String::new(),
            remedies: Vec::new(),
            source,
        }
    }

    #[test]
    fn a_host_behind_the_app_names_its_own_kind_and_platform() {
        let platform = Platform::SHIPPED[3];
        let error = error_of(
            &step_error(SetupError::Builds(BuildsError::HostNeedsMigrate { platform })),
            Some("op-1"),
        );
        match error.kind {
            Some(pb::error::Kind::HostNeedsMigrate(needs)) => {
                assert_eq!(needs.platform, platform.to_string())
            }
            other => panic!("expected host_needs_migrate, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_build_is_not_a_host_that_needs_migrating() {
        let error = error_of(
            &step_error(SetupError::Builds(BuildsError::NoBuild {
                platform: Platform::SHIPPED[3],
                shipped: String::new(),
            })),
            Some("op-1"),
        );
        assert!(!matches!(
            error.kind,
            Some(pb::error::Kind::HostNeedsMigrate(_))
        ));
    }
}

fn step_failed(operation: &str, step: OpStep, cause: &str, left: &str) -> pb::error::Kind {
    pb::error::Kind::StepFailed(Box::new(pb::StepFailed {
        operation: Some(pb::OperationId {
            value: operation.to_owned(),
        }),
        step: step.key().to_owned(),
        cause: Some(Box::new(pb::Error {
            message: cause.to_owned(),
            fix: String::new(),
            retry: None,
            kind: None,
        })),
        left_as_is: left.to_owned(),
    }))
}

/// `error` as the status a call returns.
pub fn status_of(error: &ManageError) -> Status {
    let code = match error {
        ManageError::InvalidSshHost { .. }
        | ManageError::BadName { .. }
        | ManageError::Params { .. }
        | ManageError::RunOnUnsupported { .. }
        | ManageError::ActionUndefined { .. }
        | ManageError::NotProviderMachine { .. } => Code::InvalidArgument,
        ManageError::NameTaken { .. } => Code::AlreadyExists,
        ManageError::NotFound { .. } => Code::NotFound,
        ManageError::Busy { .. } | ManageError::BeingRemoved { .. } => Code::Aborted,
        ManageError::NotReady { .. }
        | ManageError::ProviderLoginNeeded { .. }
        | ManageError::HostKeyUnconfirmed { .. }
        | ManageError::HostKeyMismatch { .. }
        | ManageError::LoginNeeded { .. } => Code::FailedPrecondition,
        ManageError::Step { .. } | ManageError::UninstallFailed { .. } => Code::Unavailable,
        _ => Code::Internal,
    };
    failure(code, error_of(error, None))
}

#[tonic::async_trait]
impl pb::machine_service_server::MachineService for MachineSvc {
    /// The user sees every machine. An agent sees its channel's (TP-1),
    /// so `humata cp … <machine>` and `humata host show` find them; a
    /// script sees none.
    async fn list_machines(
        &self,
        request: Request<pb::ListMachinesRequest>,
    ) -> Result<Response<pb::ListMachinesResponse>, Status> {
        let who = caller(&request)?;
        let seen = match &who {
            Caller::Peer => return Err(refused(&Refusal::PeerCannotCall)),
            Caller::Script(_) => return Ok(Response::new(pb::ListMachinesResponse::default())),
            Caller::Agent(_) => Some(self.channel_machines(&who)?),
            Caller::User(_) => None,
        };
        let work = self.work.clone();
        let entries = tokio::task::spawn_blocking(move || {
            work.machines.list().map(|entries| {
                entries
                    .iter()
                    .filter(|entry| seen.as_ref().is_none_or(|seen| seen.contains(&entry.name)))
                    .map(|entry| work.machine(entry))
                    .collect()
            })
        })
        .await
        .map_err(|error| internal(format!("listing the machines stopped: {error}"), ""))?;
        let machines = entries.map_err(|error| status_of(&error))?;
        Ok(Response::new(pb::ListMachinesResponse {
            machines,
            page: None,
        }))
    }

    async fn get_machine(
        &self,
        request: Request<pb::GetMachineRequest>,
    ) -> Result<Response<pb::MachineDetail>, Status> {
        let who = caller(&request)?;
        let name = machine_of(request.into_inner().id)?;
        let not_found = || status_of(&ManageError::NotFound { machine: name.clone() });
        match &who {
            Caller::Peer => return Err(refused(&Refusal::PeerCannotCall)),
            Caller::Script(_) => return Err(not_found()),
            // TP-1: an agent may look at its channel's machines, and at
            // this one, which it may be on.
            Caller::Agent(_) => {
                if !self.is_this(&name) && !self.channel_machines(&who)?.contains(&name) {
                    return Err(not_found());
                }
            }
            Caller::User(_) => {}
        }
        if self.is_this(&name) {
            let name = self.guard.machine().to_owned();
            let (root, _) = self.root_facts(&name).await?;
            // D261: This Mac's own stop is in force once it is recorded.
            let sessions_stopped = self.work.stops.get().and_then(|stops| stops.this_machine());
            let health = self.work.own_health(&name).await;
            return Ok(Response::new(pb::MachineDetail {
                machine: Some(pb::Machine {
                    id: Some(machine_id(&name)),
                    name: "This Mac".to_owned(),
                    is_this_mac: true,
                    reachability: pb::Reachability::Online as i32,
                    // Named as a peer's Hello names them, so This Mac's
                    // page fills its system line and Health "Humata" row.
                    os: std::env::consts::OS.replace("macos", "darwin"),
                    arch: std::env::consts::ARCH.to_owned(),
                    daemon_version: option_env!("CARGO_PKG_VERSION")
                        .unwrap_or("0.0.0")
                        .to_owned(),
                    root,
                    stop_delivered: sessions_stopped.is_some(),
                    sessions_stopped,
                    ..pb::Machine::default()
                }),
                health,
                ..pb::MachineDetail::default()
            }));
        }
        let entry = self
            .work
            .machines
            .get(&name)
            .map_err(|error| status_of(&error))?;
        let (health, folders) = self.work.health(&name).await;
        Ok(Response::new(pb::MachineDetail {
            machine: Some(self.work.machine(&entry)),
            health,
            folders,
            actions: self.work.defined_actions(&entry),
            ..pb::MachineDetail::default()
        }))
    }

    /// Checks the request and answers with the running operation; the add
    /// runs on (connect, check, install, start, find_agents) and is undone
    /// if a step fails. An agent's add waits in the Inbox when its rule says
    /// Ask me, and runs once the user approves it (D38.6).
    async fn add_ssh_machine(
        &self,
        request: Request<pb::AddSshMachineRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let who = caller(&request)?;
        let request = request.into_inner();
        let key = request.idempotency_key.as_ref().map(|key| key.value.clone());
        let first = self.add_ssh(&who, request.clone(), false).await;
        let (me, caller_copy) = (self.clone(), who.clone());
        answer_gated_call(
            &self.inbox,
            &who,
            key,
            "humata.v1.Operation",
            first,
            move || async move { me.add_ssh(&caller_copy, request, true).await },
        )
        .await
    }

    /// Connects again now (HO-6): installs what differs and starts the
    /// service. Answers when it is done, with the machine as it is. An
    /// agent's reconnect waits in the Inbox when its rule says Ask me.
    async fn reconnect_machine(
        &self,
        request: Request<pb::ReconnectMachineRequest>,
    ) -> Result<Response<pb::Machine>, Status> {
        let who = caller(&request)?;
        let request = request.into_inner();
        let first = self.reconnect(&who, request.clone(), false).await;
        let (me, caller_copy) = (self.clone(), who.clone());
        answer_gated_call(
            &self.inbox,
            &who,
            None,
            "humata.v1.Machine",
            first,
            move || async move { me.reconnect(&caller_copy, request, true).await },
        )
        .await
    }

    /// What removing the machine does, and the token RemoveMachine takes.
    async fn plan_remove_machine(
        &self,
        request: Request<pb::PlanRemoveMachineRequest>,
    ) -> Result<Response<pb::RemoveMachinePlan>, Status> {
        let who = caller(&request)?;
        let name = machine_of(request.into_inner().id)?;
        self.require(&who, &name, MachineActionKind::Remove)?;
        self.work
            .machines
            .get(&name)
            .map_err(|error| status_of(&error))?;
        let token = format!("pl_{}", uuid::Uuid::new_v4().simple());
        let mut plans = lock(&self.work.plans);
        plans.retain(|_, (_, made)| made.elapsed() < PLAN_LIFETIME);
        plans.insert(token.clone(), (name.clone(), Instant::now()));
        Ok(Response::new(pb::RemoveMachinePlan {
            machine: Some(machine_id(&name)),
            sessions: Vec::new(),
            channels: Vec::new(),
            runs_provider_delete: false,
            token: Some(pb::PlanToken { value: token }),
        }))
    }

    /// Removes the machine the plan was made for: the user only, present.
    async fn remove_machine(
        &self,
        request: Request<pb::RemoveMachineRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let who = caller(&request)?;
        let request = request.into_inner();
        let name = machine_of(request.id)?;
        self.require(&who, &name, MachineActionKind::Remove)?;
        if !self.guard.is_present_user(&who) {
            return Err(refused(&Refusal::UserOnly {
                capability: Some(MachineActionKind::Remove.capability()),
            }));
        }
        if let Some(operation) = self
            .operations
            .existing(&who, request.idempotency_key.as_ref())?
        {
            return Ok(Response::new(operation));
        }
        let token = request
            .plan
            .map(|plan| plan.value)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| missing("plan"))?;
        self.work.use_plan(&token, &name)?;
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Machine(machine_id(&name))),
        };
        let progress = match self.operations.start_op(
            &who,
            pb::OperationKind::RemoveMachine,
            target,
            &OpStep::REMOVE,
            false,
            request.idempotency_key,
        )? {
            OperationStarted::Started(progress) => progress,
            OperationStarted::Existing(operation) => return Ok(Response::new(operation)),
        };
        // An add of the name until the removal ends is told it is being
        // removed, not that the name is taken (D760).
        self.work.machines.removing(&name);
        // While its link is still up, so the machine itself ends them (D784).
        let left = self.work.end_removed_sessions(&name).await;
        if !left.is_empty() {
            // The removal goes on; what is left running says how to end it.
            progress.set_step("end-sessions", pb::StepState::Failed, Some(left.join("\n")));
        }
        self.work.link_down(&name);
        // Its reports stay until its sessions are archived with what they
        // last said (`removed`), so the archive keeps the lost work (D783).
        self.work.forget_link(&name);
        self.work.forget_journal(&name).await;
        let work = self.work.clone();
        let operation = self.run(
            progress,
            Arc::new(move |machines, drive| {
                machines.remove_holding_name(&name, drive)?;
                // A test holds here to show an add of the name waits (D783).
                humata2_daemon_runtime::fault::hold_blocking("remove.name_held");
                work.removed(&name, &Handle::current());
                machines.removal_done(&name);
                Ok(None)
            }),
        );
        Ok(Response::new(operation))
    }

    async fn plan_forget_machine(
        &self,
        request: Request<pb::PlanForgetMachineRequest>,
    ) -> Result<Response<pb::ForgetMachinePlan>, Status> {
        self.plan_forget_of(request).await
    }

    async fn forget_machine(
        &self,
        request: Request<pb::ForgetMachineRequest>,
    ) -> Result<Response<pb::ForgetMachineResponse>, Status> {
        self.forget_of(request).await
    }

    async fn plan_change_root(
        &self,
        request: Request<pb::PlanChangeRootRequest>,
    ) -> Result<Response<pb::ChangeRootPlan>, Status> {
        let who = caller(&request)?;
        let request = request.into_inner();
        let name = self.root_machine(machine_of(request.id)?);
        let new_root = root_of(&request.root)?;
        user_only(&who)?;
        if name != self.guard.machine() {
            return self.plan_box_root(&name, new_root).await.map(Response::new);
        }
        let (old_root, found) = self.root_facts(&name).await?;
        let ids = |ids: Vec<String>| {
            ids.into_iter()
                .map(|value| pb::ChannelId { value })
                .collect::<Vec<_>>()
        };
        let token = change_root::issue_plan(&self.work.plans, &name, &new_root);
        Ok(Response::new(pb::ChangeRootPlan {
            machine: Some(machine_id(&name)),
            old_root,
            new_root,
            channels: ids(found.channels),
            running: found
                .running
                .into_iter()
                .map(|value| pb::SessionId { value })
                .collect(),
            token: Some(pb::PlanToken { value: token }),
        }))
    }

    async fn change_root(
        &self,
        request: Request<pb::ChangeRootRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let who = caller(&request)?;
        let request = request.into_inner();
        let name = self.root_machine(machine_of(request.id)?);
        let new_root = root_of(&request.root)?;
        user_only(&who)?;
        if name != self.guard.machine() {
            return self
                .change_box_root(
                    &who,
                    &name,
                    new_root,
                    request.plan,
                    request.move_channels,
                    request.idempotency_key,
                )
                .await
                .map(Response::new);
        }
        if request.move_channels {
            let (_, found) = self.root_facts(&name).await?;
            if let Some(refusal) = change_root::busy_refusal(&name, &found.running) {
                return Err(refusal);
            }
        }
        if let Some(operation) = self
            .operations
            .existing(&who, request.idempotency_key.as_ref())?
        {
            return Ok(Response::new(operation));
        }
        let token = request
            .plan
            .map(|plan| plan.value)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| missing("plan"))?;
        change_root::take_plan(&self.work.plans, &token, &name, &new_root)?;
        let progress = match change_root::start_operation(
            self.operations.as_ref(),
            &who,
            &name,
            request.move_channels,
            request.idempotency_key,
        )? {
            OperationStarted::Started(progress) => progress,
            OperationStarted::Existing(operation) => return Ok(Response::new(operation)),
        };
        let started = change_root::apply_here(
            progress,
            self.storage.clone(),
            self.channels.clone(),
            name,
            new_root,
            request.move_channels,
        )
        .await?;
        Ok(Response::new(started))
    }

    async fn set_vm_tier(
        &self,
        _request: Request<pb::SetVmTierRequest>,
    ) -> Result<Response<pb::Machine>, Status> {
        Err(not_built_status("humata.v1.MachineService.SetVmTier"))
    }

    async fn get_storage(
        &self,
        request: Request<pb::GetStorageRequest>,
    ) -> Result<Response<pb::StorageReport>, Status> {
        let who = caller(&request)?;
        if let Some(machine) = self.storage_call(&who, request.get_ref().id.as_ref())? {
            let mut client = self.storage_client(&machine)?;
            let mut report = client
                .get_storage(Request::new(pb::GetStorageRequest { id: None }))
                .await?
                .into_inner();
            report.machine = Some(pb::MachineId { value: machine });
            return Ok(Response::new(report));
        }
        let report = self.storage_blocking(|storage| storage.report(Vec::new())).await?;
        Ok(Response::new(report))
    }

    async fn list_running_holders(
        &self,
        request: Request<pb::ListRunningHoldersRequest>,
    ) -> Result<Response<pb::ListRunningHoldersResponse>, Status> {
        let who = caller(&request)?;
        if let Some(machine) = self.storage_call(&who, request.get_ref().machine.as_ref())? {
            let mut client = self.storage_client(&machine)?;
            let mut listed = client
                .list_running_holders(Request::new(pb::ListRunningHoldersRequest { machine: None }))
                .await?
                .into_inner();
            for holder in &mut listed.holders {
                holder.machine = Some(machine_id(&machine));
            }
            return Ok(Response::new(listed));
        }
        let here = self.guard.machine().to_owned();
        let holders = self.holders.list(&here).await?;
        Ok(Response::new(pb::ListRunningHoldersResponse { holders }))
    }

    async fn stop_running_holder(
        &self,
        request: Request<pb::StopRunningHolderRequest>,
    ) -> Result<Response<pb::StopRunningHolderResponse>, Status> {
        let who = caller(&request)?;
        let remote = self.destroy_call(&who, request.get_ref().machine.as_ref())?;
        let tab = running_tab(request.get_ref().tab.as_ref())?;
        if let Some(machine) = remote {
            let mut client = self.storage_client(&machine)?;
            let tab = Some(pb::TabId { value: tab });
            return client
                .stop_running_holder(Request::new(pb::StopRunningHolderRequest {
                    machine: None,
                    tab,
                }))
                .await;
        }
        let here = self.guard.machine().to_owned();
        self.holders.stop(&here, &tab).await?;
        Ok(Response::new(pb::StopRunningHolderResponse {}))
    }

    async fn resume_running_holder(
        &self,
        request: Request<pb::ResumeRunningHolderRequest>,
    ) -> Result<Response<pb::ResumeRunningHolderResponse>, Status> {
        let who = caller(&request)?;
        let remote = self.storage_call(&who, request.get_ref().machine.as_ref())?;
        let tab = running_tab(request.get_ref().tab.as_ref())?;
        if let Some(machine) = remote {
            let mut client = self.storage_client(&machine)?;
            let tab = Some(pb::TabId { value: tab });
            return client
                .resume_running_holder(Request::new(pb::ResumeRunningHolderRequest {
                    machine: None,
                    tab,
                }))
                .await;
        }
        let here = self.guard.machine().to_owned();
        self.holders.resume(&here, &who, &tab).await?;
        Ok(Response::new(pb::ResumeRunningHolderResponse {}))
    }

    async fn end_running_holder(
        &self,
        request: Request<pb::EndRunningHolderRequest>,
    ) -> Result<Response<pb::EndRunningHolderResponse>, Status> {
        let who = caller(&request)?;
        let remote = self.destroy_call(&who, request.get_ref().machine.as_ref())?;
        let tab = running_tab(request.get_ref().tab.as_ref())?;
        if let Some(machine) = remote {
            let mut client = self.storage_client(&machine)?;
            let tab = Some(pb::TabId { value: tab });
            return client
                .end_running_holder(Request::new(pb::EndRunningHolderRequest {
                    machine: None,
                    tab,
                }))
                .await;
        }
        let here = self.guard.machine().to_owned();
        self.holders.end(&here, &tab).await?;
        Ok(Response::new(pb::EndRunningHolderResponse {}))
    }

    async fn adopt_running_holder(
        &self,
        request: Request<pb::AdoptRunningHolderRequest>,
    ) -> Result<Response<pb::AdoptRunningHolderResponse>, Status> {
        let who = caller(&request)?;
        let remote = self.destroy_call(&who, request.get_ref().machine.as_ref())?;
        let tab = running_tab(request.get_ref().tab.as_ref())?;
        let channel = request
            .get_ref()
            .channel
            .as_ref()
            .map(|id| id.value.clone())
            .unwrap_or_default();
        if let Some(machine) = remote {
            let mut client = self.storage_client(&machine)?;
            let tab = Some(pb::TabId { value: tab });
            let channel = Some(pb::ChannelId { value: channel }).filter(|id| !id.value.is_empty());
            return client
                .adopt_running_holder(Request::new(pb::AdoptRunningHolderRequest {
                    machine: None,
                    tab,
                    channel,
                }))
                .await;
        }
        let here = self.guard.machine().to_owned();
        let session = self.holders.adopt(&here, &who, &tab, &channel).await?;
        Ok(Response::new(pb::AdoptRunningHolderResponse {
            session: Some(pb::SessionId { value: session }),
        }))
    }

    async fn let_go_of_host(
        &self,
        _request: Request<pb::LetGoOfHostRequest>,
    ) -> Result<Response<pb::LetGoOfHostResponse>, Status> {
        Err(failure(
            Code::FailedPrecondition,
            pb::Error {
                message: "this daemon is the host: it keeps its sessions, so it has no host to let go of".to_owned(),
                fix: "to free another machine's holders, forget or remove that machine from Hosts; it lets go of this host's sessions itself and its holders keep running".to_owned(),
                retry: None,
                kind: None,
            },
        ))
    }

    async fn clear_build_cache(
        &self,
        request: Request<pb::ClearBuildCacheRequest>,
    ) -> Result<Response<pb::ClearBuildCacheResponse>, Status> {
        let who = caller(&request)?;
        self.storage_call(&who, None)?;
        let request = request.into_inner();
        let session = request
            .session
            .as_ref()
            .map(|session| session.value.clone())
            .unwrap_or_default();
        let elsewhere = self
            .storage_blocking(move |storage| storage.machine_of_session(&session))
            .await?;
        if let Some(machine) = elsewhere {
            let mut client = self.storage_client(&machine)?;
            return client.clear_build_cache(Request::new(request)).await;
        }
        let freed = self
            .storage_blocking(move |storage| storage.clear_build_cache(&request))
            .await?;
        Ok(Response::new(freed))
    }

    async fn trim_logs(
        &self,
        request: Request<pb::TrimLogsRequest>,
    ) -> Result<Response<pb::TrimLogsResponse>, Status> {
        let who = caller(&request)?;
        let other = self.storage_call(&who, request.get_ref().id.as_ref())?;
        let mut request = request.into_inner();
        if let Some(machine) = other {
            let mut client = self.storage_client(&machine)?;
            request.id = None;
            return client.trim_logs(Request::new(request)).await;
        }
        let freed = self
            .storage_blocking(move |storage| storage.trim_logs(&request))
            .await?;
        Ok(Response::new(freed))
    }

    async fn list_host_variables(
        &self,
        request: Request<pb::ListHostVariablesRequest>,
    ) -> Result<Response<pb::ListHostVariablesResponse>, Status> {
        let who = caller(&request)?;
        let request = request.into_inner();
        let Some(machine) = self.variables_call(&who, request.id.clone())? else {
            let names = self.on_variables(|variables| variables.names()).await?;
            return Ok(Response::new(pb::ListHostVariablesResponse { names }));
        };
        let mut client = self.box_client(&machine)?;
        let call = client.list_host_variables(Request::new(request));
        match tokio::time::timeout(PROBE_DEADLINE, call).await {
            Ok(answer) => answer.map(|answer| {
                let mut names = answer.into_inner().names;
                names.sort();
                Response::new(pb::ListHostVariablesResponse { names })
            }),
            Err(_) => Err(RelayError { machine }.into()),
        }
    }

    async fn set_host_variable(
        &self,
        request: Request<pb::SetHostVariableRequest>,
    ) -> Result<Response<pb::SetHostVariableResponse>, Status> {
        let who = caller(&request)?;
        let request = request.into_inner();
        let target = self.variables_call(&who, request.id.clone())?;
        self.host_vars
            .check(&request.name, &request.value)
            .map_err(|error| error.status())?;
        match target {
            None => {
                self.on_variables(move |variables| variables.set(&request.name, &request.value))
                    .await?;
                let running = self
                    .storage_blocking(|storage| storage.running_agents())
                    .await?;
                Ok(Response::new(pb::SetHostVariableResponse {
                    running_sessions_keep_old_value: running,
                }))
            }
            Some(machine) => {
                self.box_client(&machine)?
                    .set_host_variable(Request::new(request))
                    .await
            }
        }
    }

    async fn remove_host_variable(
        &self,
        request: Request<pb::RemoveHostVariableRequest>,
    ) -> Result<Response<pb::SetHostVariableResponse>, Status> {
        let who = caller(&request)?;
        let request = request.into_inner();
        match self.variables_call(&who, request.id.clone())? {
            None => {
                self.on_variables(move |variables| variables.remove(&request.name))
                    .await?;
                let running = self
                    .storage_blocking(|storage| storage.running_agents())
                    .await?;
                Ok(Response::new(pb::SetHostVariableResponse {
                    running_sessions_keep_old_value: running,
                }))
            }
            Some(machine) => {
                self.box_client(&machine)?
                    .remove_host_variable(Request::new(request))
                    .await
            }
        }
    }

    async fn trust_new_host_key(
        &self,
        request: Request<pb::TrustNewHostKeyRequest>,
    ) -> Result<Response<pb::Machine>, Status> {
        let who = caller(&request)?;
        let request = request.into_inner();
        let name = machine_of(request.id)?;
        user_only(&who)?;
        if matches!(who, Caller::User(_)) && !self.guard.is_present_user(&who) {
            return Err(refused(&Refusal::UserOnly {
                capability: Some(Capability::MachineSettings),
            }));
        }
        let fingerprint = request.new_fingerprint.trim().to_owned();
        if fingerprint.is_empty() {
            return Err(missing("new_fingerprint"));
        }
        let (work, machine) = (self.work.clone(), name.clone());
        tokio::task::spawn_blocking(move || {
            work.machines
                .trust_new_host_key(&machine, &fingerprint, &SystemRunner::new(&()))
        })
        .await
        .map_err(|error| internal(format!("trusting the host key stopped: {error}"), ""))?
        .map_err(|error| status_of(&error))?;
        let was_changed = lock(&self.work.seen)
            .get(&name)
            .is_some_and(|seen| seen.reach == pb::Reachability::HostKeyChanged);
        if was_changed {
            self.work.saw(&name, pb::Reachability::Unspecified);
        }
        let entry = self
            .work
            .machines
            .get(&name)
            .map_err(|error| status_of(&error))?;
        Ok(Response::new(self.work.machine(&entry)))
    }

    async fn stop_machine_sessions(
        &self,
        request: Request<pb::StopMachineSessionsRequest>,
    ) -> Result<Response<pb::StopMachineSessionsResponse>, Status> {
        let who = caller(&request)?;
        let request = request.into_inner();
        let name = machine_of(request.machine)?;
        self.sessions_call(&who, &name, MachineActionKind::StopSessions)?;
        let via = pb::StopVia::try_from(request.via).unwrap_or(pb::StopVia::Unspecified);
        let Some(stops) = self.work.stops.get() else {
            return Err(internal("stops service is not wired", "restart humatad2"));
        };
        let reach = stops
            .stop_remote(
                &name,
                request.stop_id,
                request.reason,
                via,
                self.guard.machine().to_owned(),
            )
            .await?;
        Ok(Response::new(pb::StopMachineSessionsResponse {
            stopped: Some(reach.pb_record),
            delivered: reach.delivered,
        }))
    }

    async fn start_machine_sessions(
        &self,
        request: Request<pb::StartMachineSessionsRequest>,
    ) -> Result<Response<pb::StartMachineSessionsResponse>, Status> {
        let who = caller(&request)?;
        let name = machine_of(request.into_inner().machine)?;
        self.sessions_call(&who, &name, MachineActionKind::StartSessions)?;
        let by_user = self
            .work
            .stops
            .get()
            .and_then(|stops| stops.get(&name))
            .is_some_and(|copy| copy.by_user);
        if by_user && !matches!(who, Caller::User(_)) {
            return Err(refused(&Refusal::UserOnly {
                capability: Some(MachineActionKind::StartSessions.capability()),
            }));
        }
        let Some(stops) = self.work.stops.get() else {
            return Err(internal("stops service is not wired", "restart humatad2"));
        };
        let cancelled = stops.start_remote(&name).await?;
        Ok(Response::new(pb::StartMachineSessionsResponse { cancelled }))
    }

    async fn list_machine_scripts(
        &self,
        request: Request<pb::ListMachineScriptsRequest>,
    ) -> Result<Response<pb::ListMachineScriptsResponse>, Status> {
        self.rpc_list_machine_scripts(request).await
    }

    async fn check_machine_scripts(
        &self,
        request: Request<pb::CheckMachineScriptsRequest>,
    ) -> Result<Response<pb::CheckMachineScriptsResponse>, Status> {
        self.rpc_check_machine_scripts(request).await
    }

    async fn run_machine_script(
        &self,
        request: Request<pb::RunMachineScriptRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        self.rpc_run_machine_script(request).await
    }

    async fn list_machine_script_runs(
        &self,
        request: Request<pb::ListMachineScriptRunsRequest>,
    ) -> Result<Response<pb::ListMachineScriptRunsResponse>, Status> {
        self.rpc_list_machine_script_runs(request).await
    }
}

/// The tab a D784 call names, which it must.
pub(crate) fn running_tab(tab: Option<&pb::TabId>) -> Result<String, Status> {
    tab.map(|tab| tab.value.clone())
        .filter(|tab| !tab.is_empty())
        .ok_or_else(|| missing("tab"))
}
