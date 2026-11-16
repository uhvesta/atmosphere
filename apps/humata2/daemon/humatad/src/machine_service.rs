//! MachineService (HO-1–HO-6, LD-10): the machines the host knows, adding
//! one over SSH, reconnecting it and removing it. The work is machines'
//! (`humata2_machines::Machines`); this wires it to the API: the access
//! checks, an `Operation` whose steps change as the work does (TR-7), and
//! the errors in the API's terms. A machine's id is its name until
//! machines get ids of their own (D61).
//!
//! What a machine's `reachability` says is what this daemon last saw: an
//! add or reconnect that worked, and then its peer link (`peer_client`)
//! coming up or going down, or a machine whose proof is wrong
//! (IDENTITY_MISMATCH). The add writes the enrollment key the link needs
//! (D83). A machine not connected since humatad2 started is UNSPECIFIED
//! until its link comes up. `daemon_version` is what the link's Hello
//! said, `last_seen_at` when an add, reconnect or link last reached the
//! machine (or a link was last known up), and `next_retry_at` when a down
//! link dials again.
//!
//! Each change to a machine as ListMachines shows it goes on the feed
//! (EventService), after the change is written.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use humata2_access::{Call, MachineAction, MachineActionKind, MachineId, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_machines::{
    Action, ConfigError, IdentityError, Provider, SetupError,
    AddSsh, Builds, Drive, EntryState, HostEntry, Machines, ManageError, OpStep, StepEvent,
    SystemRunner,
};
use tokio::runtime::Handle;
use tokio::task::AbortHandle;
use tonic::{Code, Request, Response, Status};

use crate::events::Feed;
use crate::guard::{self, Guard};
use crate::identity::{self, Caller};
use crate::operations::{Operations, Progress, Started};
use crate::peer_client::{self, CommandDial, Dial, LinkError, LinkEvent};
use crate::status;

/// How long a removal plan stays good.
const PLAN_LIFETIME: Duration = Duration::from_secs(10 * 60);

pub(crate) struct MachineSvc {
    guard: Arc<Guard>,
    work: Arc<Work>,
    operations: Arc<Operations>,
}

/// The wall clock, which tests set.
pub(crate) type Clock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

/// What this daemon last saw of a machine.
#[derive(Clone, Default)]
struct Seen {
    reach: pb::Reachability,
    daemon_version: String,
    last_seen_at: Option<SystemTime>,
    next_retry_at: Option<SystemTime>,
}

/// What the blocking work needs.
pub(crate) struct Work {
    pub(crate) machines: Machines,
    pub(crate) builds: Box<dyn Builds + Send + Sync>,
    /// What each machine looked like when last connected to.
    seen: Mutex<HashMap<String, Seen>>,
    feed: Arc<Feed>,
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
    pub(crate) skills: std::sync::OnceLock<Arc<crate::skills_service::SkillsHost>>,
}

impl Work {
    /// Uses the removal plan `token` made for `machine`, once.
    pub(crate) fn use_plan(&self, token: &str, machine: &str) -> Result<(), Status> {
        let planned = lock(&self.plans)
            .remove(token)
            .filter(|(planned, made)| planned == machine && made.elapsed() < PLAN_LIFETIME);
        if planned.is_some() {
            return Ok(());
        }
        Err(status::failure(
            Code::FailedPrecondition,
            status::error(
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

    pub(crate) fn new(machines: Machines, builds: Box<dyn Builds + Send + Sync>, feed: Arc<Feed>, clock: Clock) -> Self {
        let work = Self {
            machines,
            builds,
            seen: Mutex::new(HashMap::new()),
            feed,
            shown: Mutex::new(BTreeMap::new()),
            links: Mutex::new(HashMap::new()),
            clock,
            plans: Mutex::new(HashMap::new()),
            skills: std::sync::OnceLock::new(),
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
    pub(crate) fn publish_machines(&self) {
        let mut shown = lock(&self.shown);
        let Ok(now) = self.listed() else {
            // The list will be read again at the next change.
            return;
        };
        for (name, machine) in &now {
            match shown.get(name) {
                None => self.feed.publish(pb::Change::Created, pb::event::Payload::Machine(machine.clone()), None),
                Some(before) if before != machine => {
                    self.feed.publish(pb::Change::Updated, pb::event::Payload::Machine(machine.clone()), None);
                }
                Some(_) => {}
            }
        }
        for (name, machine) in shown.iter() {
            if !now.contains_key(name) {
                self.feed.publish(pb::Change::Deleted, pb::event::Payload::Machine(machine.clone()), None);
            }
        }
        *shown = now;
    }

    /// Runs `with` with a [`Drive`] that runs commands as child processes
    /// and hands each step's progress to `events`.
    pub(crate) fn drive<R>(
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
    pub(crate) fn saw(&self, machine: &str, reach: pb::Reachability) {
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

    /// The link came up, or went down with `error`, dialling again after
    /// `retry_in` unless that can't help.
    fn link_changed(&self, machine: &str, event: &LinkEvent<'_>) {
        let now = (self.clock)();
        self.change(machine, |seen| match event {
            LinkEvent::Up { daemon_version, .. } => {
                seen.reach = pb::Reachability::Online;
                (*daemon_version).clone_into(&mut seen.daemon_version);
                seen.last_seen_at = Some(now);
                seen.next_retry_at = None;
            }
            LinkEvent::Down { error, retry_in } => {
                if seen.reach == pb::Reachability::Online {
                    seen.last_seen_at = Some(now);
                }
                seen.reach = if identity_wrong(error) {
                    pb::Reachability::IdentityMismatch
                } else {
                    pb::Reachability::Offline
                };
                seen.next_retry_at = retry_in.map(|wait| now + wait);
            }
        });
    }

    /// Starts keeping `machine`'s peer link up over `dial`, in place of a
    /// link already kept. Each time it comes up or goes down the machine's
    /// reachability changes, on the feed.
    pub(crate) fn start_link(self: &Arc<Self>, machine: &str, dial: Box<dyn Dial>, runtime: &Handle) {
        let work = Arc::downgrade(self);
        let name = machine.to_owned();
        let report = Box::new(move |event: LinkEvent<'_>| tell_link(&work, &name, event));
        let key_file = self.machines.peer_key(machine);
        let task = runtime.spawn(peer_client::supervise(dial, key_file, machine.to_owned(), report));
        if let Some(before) = lock(&self.links).insert(machine.to_owned(), task.abort_handle()) {
            before.abort();
        }
    }

    /// Keeps `machine`'s link up through its link command, if the host
    /// holds its key (written when the machine was added or reconnected).
    pub(crate) fn link_up(self: &Arc<Self>, machine: &str, runtime: &Handle) {
        if !self.machines.peer_key(machine).exists() {
            return;
        }
        match self.machines.peer_link(machine) {
            Ok(invocation) => {
                self.start_link(machine, Box::new(CommandDial::new(machine.to_owned(), invocation)), runtime);
            }
            Err(error) => crate::log::say!(
                "humatad2: couldn't make the peer link command for {machine}: {}",
                status::chain(&error)
            ),
        }
    }

    /// Stops keeping `machine`'s link up.
    pub(crate) fn link_down(&self, machine: &str) {
        if let Some(link) = lock(&self.links).remove(machine) {
            link.abort();
        }
        if let Some(skills) = self.skills.get() {
            skills.link_down(machine);
        }
    }

    pub(crate) fn machine(&self, entry: &HostEntry) -> pb::Machine {
        let seen = lock(&self.seen).get(&entry.name).cloned().unwrap_or_default();
        let reachability = match entry.state {
            EntryState::Adding | EntryState::Removing => pb::Reachability::Connecting,
            EntryState::Ready => seen.reach,
        };
        // A machine reached only by a login the person gives shows that it
        // needs them when its link is down (MP-16).
        let reachability = crate::login_service::needs_login(self, entry, reachability);
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
            ..pb::Machine::default()
        }
    }
}

/// A link's change as the machine's reachability, and in the log.
fn tell_link(work: &Weak<Work>, machine: &str, event: LinkEvent<'_>) {
    let Some(work) = work.upgrade() else {
        return;
    };
    match &event {
        LinkEvent::Up { daemon_version, .. } => {
            crate::log::say!("humatad2: the peer link to {machine} is up, running humatad2 {daemon_version}");
        }
        LinkEvent::Down { error, .. } => {
            crate::log::say!("humatad2: {}; {}", status::chain(*error), error.fix());
        }
    }
    // Skills hear of the link first, so the machine's channel is there by
    // the time the feed shows it online.
    if let Some(skills) = work.skills.get() {
        match &event {
            LinkEvent::Up { channel, .. } => skills.link_up(machine, (*channel).clone()),
            LinkEvent::Down { .. } => skills.link_down(machine),
        }
    }
    work.link_changed(machine, &event);
}

/// Whether the machine and the host hold different keys: the machine's
/// proof is wrong, or it refused the host's (LD-12: add it again).
fn identity_wrong(error: &LinkError) -> bool {
    matches!(
        error,
        LinkError::MachineProofWrong { .. }
            | LinkError::MachineRefused {
                reason: pb::peer_unproven::Reason::HostProofWrong,
                ..
            }
    )
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
pub(crate) fn recover_in_background(work: Arc<Work>) {
    let runtime = Handle::try_current().ok();
    std::thread::spawn(move || {
        let mut ignore = |_: StepEvent| {};
        match work.drive(&mut ignore, &|| false, |machines, drive| machines.recover(drive)) {
            Ok(recovered) => {
                for machine in recovered {
                    if let Err(error) = machine.outcome {
                        crate::log::say!(
                            "humatad2: {} was left {:?} by the last run and couldn't be finished yet: {}",
                            machine.machine,
                            machine.found,
                            status::chain(&error)
                        );
                    }
                }
            }
            Err(error) => crate::log::say!(
                "humatad2: couldn't finish the machine adds and removals the last run left: {}",
                status::chain(&error)
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
    pub(crate) fn new(guard: Arc<Guard>, work: Arc<Work>, operations: Arc<Operations>) -> Self {
        Self {
            guard,
            work,
            operations,
        }
    }

    fn require(&self, caller: &Caller, machine: &str, action: MachineActionKind) -> Result<(), Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
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

    /// Runs `work` on a blocking thread, its progress going to `progress`,
    /// and finishes the operation with what it returns.
    /// Runs `job` for the operation on a blocking thread and answers with
    /// the operation as it starts; Retry runs `job` again.
    fn run(&self, progress: Progress, job: Job) -> pb::Operation {
        let operation = progress.operation();
        launch(self.work.clone(), progress, job);
        operation
    }
}

/// An operation's work, which may run more than once (Retry).
pub(crate) type Job = Arc<dyn Fn(&Machines, &mut Drive<'_>) -> Result<Option<pb::operation::Result>, ManageError> + Send + Sync>;

fn launch(shared: Arc<Work>, progress: Progress, job: Job) {
    launch_observed(shared, progress, job, None);
}

/// What sees each step event of an operation's work, besides the operation.
pub(crate) type Observer = Arc<dyn Fn(&StepEvent) + Send + Sync>;

/// As `launch`, with `observer` told each step event, on every run.
pub(crate) fn launch_observed(shared: Arc<Work>, progress: Progress, job: Job, observer: Option<Observer>) {
    let again = (shared.clone(), job.clone(), observer.clone());
    progress.rerun_with(Arc::new(move |progress| {
        launch_observed(again.0.clone(), progress, again.1.clone(), again.2.clone());
    }));
    let id = progress.operation().id.map(|id| id.value).unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        let mut events = |event: StepEvent| {
            if let Some(observer) = &observer {
                observer(&event);
            }
            progress.step(&event);
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
pub(crate) enum ProvidersFileError {
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
pub(crate) fn providers_path(hosts_file: &std::path::Path) -> std::path::PathBuf {
    hosts_file.with_file_name("providers.toml")
}

/// The providers file's text (empty when there is none) and the providers
/// it defines, without the built-in Lima example.
pub(crate) fn read_providers(hosts_file: &std::path::Path) -> Result<(String, Vec<Provider>), ProvidersFileError> {
    let path = providers_path(hosts_file);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => return Err(ProvidersFileError::Read { path, source }),
    };
    let providers = Provider::parse(&text).map_err(|source| ProvidersFileError::Invalid { source })?;
    Ok((text, providers))
}

pub(crate) fn machine_id(name: &str) -> pb::MachineId {
    pb::MachineId { value: name.to_owned() }
}

pub(crate) fn machine_of(id: Option<pb::MachineId>) -> Result<String, Status> {
    id.map(|id| id.value)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| status::missing("id"))
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The reachability a failed connect leaves.
pub(crate) fn reach_after(error: &ManageError) -> pb::Reachability {
    match error {
        ManageError::HostKeyMismatch { .. } => pb::Reachability::HostKeyChanged,
        ManageError::LoginNeeded { .. } => pb::Reachability::NeedsLogin,
        _ => pb::Reachability::Offline,
    }
}

/// `error` as the API's Error. `operation` names the operation it failed.
pub(crate) fn error_of(error: &ManageError, operation: Option<&str>) -> pb::Error {
    use pb::error::Kind;
    let message = status::chain(error);
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
        ManageError::NotFound { machine } => (
            "list the machines to see their names",
            Some(Kind::NotFound(pb::NotFound {
                kind: pb::EntityKind::Machine as i32,
                id: machine.clone(),
                in_trash: false,
            })),
        ),
        ManageError::Busy { .. } | ManageError::NotReady { .. } => {
            ("wait for the running add, reconnect or remove to finish, then try again", None)
        }
        ManageError::HostKeyUnconfirmed { machine, fingerprint } => (
            "compare the fingerprint with the one the machine itself shows (ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub on it), then add it again with that fingerprint",
            Some(Kind::HostKeyChanged(pb::HostKeyChanged {
                machine: Some(machine_id(machine)),
                expected_fingerprint: String::new(),
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
            "make key login work without a password or passphrase prompt (ssh-copy-id, or load the key into ssh-agent), then try again",
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
            "fix the identity command in Settings › Machine providers so that field is right; what was made is still there (use Delete what was made)",
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
            "the step's output says why the provider's command failed; fix it in Settings › Machine providers, then try again (Delete what was made removes anything it left)",
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
        ManageError::Step { step, .. } => (
            "the steps above say what failed; humatad2 was removed from the machine again, but anything a provider made is still there. Fix the cause and add the machine again",
            operation.map(|operation| step_failed(operation, *step, &message, "humatad2 and its files were removed from the machine again")),
        ),
        ManageError::UninstallFailed { .. } => (
            "reconnect when the machine is back to finish removing it",
            operation.map(|operation| {
                step_failed(operation, OpStep::Uninstall, &message, "the machine stays listed until humatad2 is removed from it")
            }),
        ),
        ManageError::Run { .. } | ManageError::Quote { .. } => {
            ("check that ssh is installed and on PATH for humatad2, then try again", None)
        }
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
            "remove `run_on` from the provider in Settings › Machine providers",
            None,
        ),
        ManageError::ProviderLoginNeeded { machine, .. } => (
            "log in to the machine interactively (Open interactively), or make the provider's connect command log in without a prompt",
            Some(Kind::LoginNeedsUser(pb::LoginNeedsUser {
                provider: None,
                machine: Some(machine_id(machine)),
            })),
        ),
        ManageError::NotProviderMachine { .. } => ("pick a machine the provider made", None),
        ManageError::ActionUndefined { .. } => ("add the command to the provider in Settings › Machine providers", None),
        ManageError::ActionFailed { .. } => (
            "the step's output says why; fix the command or the machine, then run it again",
            None,
        ),
        ManageError::DeleteFailed { .. } => (
            "delete what the provider made with its own tool; Humata no longer lists the machine",
            None,
        ),
        ManageError::ProviderLinkLost { .. } => ("remove the machine and add it again", None),
        ManageError::Hosts(_) | ManageError::KnownHosts(_) => {
            ("fix the file the message names, then try again", None)
        }
    };
    pb::Error {
        message,
        fix: fix.to_owned(),
        retry: None,
        kind,
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
pub(crate) fn status_of(error: &ManageError) -> Status {
    let code = match error {
        ManageError::InvalidSshHost { .. }
        | ManageError::BadName { .. }
        | ManageError::Params { .. }
        | ManageError::RunOnUnsupported { .. }
        | ManageError::ActionUndefined { .. }
        | ManageError::NotProviderMachine { .. } => Code::InvalidArgument,
        ManageError::NameTaken { .. } => Code::AlreadyExists,
        ManageError::NotFound { .. } => Code::NotFound,
        ManageError::Busy { .. } => Code::Aborted,
        ManageError::NotReady { .. }
        | ManageError::ProviderLoginNeeded { .. }
        | ManageError::HostKeyUnconfirmed { .. }
        | ManageError::HostKeyMismatch { .. }
        | ManageError::LoginNeeded { .. } => Code::FailedPrecondition,
        ManageError::Step { .. } | ManageError::UninstallFailed { .. } => Code::Unavailable,
        _ => Code::Internal,
    };
    status::failure(code, error_of(error, None))
}

#[tonic::async_trait]
impl pb::machine_service_server::MachineService for MachineSvc {
    /// The user sees every machine. An agent sees none yet: it may see
    /// its channel's, and channels don't list their machines yet.
    async fn list_machines(
        &self,
        request: Request<pb::ListMachinesRequest>,
    ) -> Result<Response<pb::ListMachinesResponse>, Status> {
        let caller = identity::caller(&request)?;
        let machines = match caller {
            Caller::Peer => return Err(status::refused(&Refusal::PeerCannotCall)),
            Caller::Agent(_) | Caller::Script(_) => Vec::new(),
            Caller::User(_) => {
                let work = self.work.clone();
                let entries = tokio::task::spawn_blocking(move || {
                    work.machines
                        .list()
                        .map(|entries| entries.iter().map(|entry| work.machine(entry)).collect())
                })
                .await
                .map_err(|error| status::internal(format!("listing the machines stopped: {error}"), ""))?;
                entries.map_err(|error| status_of(&error))?
            }
        };
        Ok(Response::new(pb::ListMachinesResponse { machines, page: None }))
    }

    async fn get_machine(
        &self,
        request: Request<pb::GetMachineRequest>,
    ) -> Result<Response<pb::MachineDetail>, Status> {
        let caller = identity::caller(&request)?;
        let name = machine_of(request.into_inner().id)?;
        let not_found = || status_of(&ManageError::NotFound { machine: name.clone() });
        match caller {
            Caller::Peer => return Err(status::refused(&Refusal::PeerCannotCall)),
            Caller::Agent(_) | Caller::Script(_) => return Err(not_found()),
            Caller::User(_) => {}
        }
        let entry = self.work.machines.get(&name).map_err(|error| status_of(&error))?;
        Ok(Response::new(pb::MachineDetail {
            machine: Some(self.work.machine(&entry)),
            actions: self.work.defined_actions(&entry),
            ..pb::MachineDetail::default()
        }))
    }

    /// Checks the request and answers with the running operation; the add
    /// runs on (connect, check, install, start, find_agents) and is undone
    /// if a step fails.
    async fn add_ssh_machine(
        &self,
        request: Request<pb::AddSshMachineRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let add = AddSsh {
            ssh_host: request.ssh_host,
            name: request.name,
            root: request.root,
            trust_fingerprint: Some(request.trust_host_key_fingerprint).filter(|value| !value.trim().is_empty()),
        };
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        // A retried call finds the operation it started, even after the
        // machine's name was taken by it.
        if let Some(operation) = self.operations.existing(&caller, request.idempotency_key.as_ref())? {
            return Ok(Response::new(operation));
        }
        let plan = self.work.machines.plan_add(&add).map_err(|error| status_of(&error))?;
        let name = plan.machine.as_str().to_owned();
        self.require(&caller, &name, MachineActionKind::Add)?;
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Machine(machine_id(&name))),
        };
        let progress = match self.operations.start(
            &caller,
            pb::OperationKind::AddMachine,
            target,
            &OpStep::ADD,
            true,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Response::new(operation)),
        };
        let work = self.work.clone();
        let operation = self.run(progress, Arc::new(move |machines, drive| {
            let added = machines.add(&add, plan.clone(), drive);
            match &added {
                Ok(entry) => {
                    work.saw(&entry.name, pb::Reachability::Online);
                    work.link_up(&entry.name, &Handle::current());
                }
                Err(error) => work.saw(&name, reach_after(error)),
            }
            added.map(|entry| Some(pb::operation::Result::Machine(machine_id(&entry.name))))
        }));
        Ok(Response::new(operation))
    }

    /// Connects again now (HO-6): installs what differs and starts the
    /// service. Answers when it is done, with the machine as it is.
    async fn reconnect_machine(
        &self,
        request: Request<pb::ReconnectMachineRequest>,
    ) -> Result<Response<pb::Machine>, Status> {
        let caller = identity::caller(&request)?;
        let name = machine_of(request.into_inner().id)?;
        self.require(&caller, &name, MachineActionKind::Reconnect)?;
        let work = self.work.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut ignore = |_: StepEvent| {};
            let result = work.drive(&mut ignore, &|| false, |machines, drive| machines.reconnect(&name, drive));
            match &result {
                Ok(entry) => {
                    work.saw(&entry.name, pb::Reachability::Online);
                    work.link_up(&entry.name, &Handle::current());
                }
                Err(ManageError::Busy { .. } | ManageError::NotFound { .. }) => {}
                Err(error) => work.saw(&name, reach_after(error)),
            }
            result.map(|entry| work.machine(&entry))
        })
        .await
        .map_err(|error| status::internal(format!("the reconnect stopped: {error}"), ""))?;
        result.map(Response::new).map_err(|error| status_of(&error))
    }

    /// What removing the machine does, and the token RemoveMachine takes.
    /// Sessions and channels on it aren't listed yet: neither is kept per
    /// machine in this build.
    async fn plan_remove_machine(
        &self,
        request: Request<pb::PlanRemoveMachineRequest>,
    ) -> Result<Response<pb::RemoveMachinePlan>, Status> {
        let caller = identity::caller(&request)?;
        let name = machine_of(request.into_inner().id)?;
        self.require(&caller, &name, MachineActionKind::Remove)?;
        self.work.machines.get(&name).map_err(|error| status_of(&error))?;
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
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let name = machine_of(request.id)?;
        self.require(&caller, &name, MachineActionKind::Remove)?;
        let principal = guard::principal(&caller);
        if self
            .guard
            .with_policy(|_, context| principal.present_user(context))
            .is_none()
        {
            return Err(status::refused(&Refusal::UserOnly {
                capability: Some(MachineActionKind::Remove.capability()),
            }));
        }
        if let Some(operation) = self.operations.existing(&caller, request.idempotency_key.as_ref())? {
            return Ok(Response::new(operation));
        }
        let token = request
            .plan
            .map(|plan| plan.value)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| status::missing("plan"))?;
        self.work.use_plan(&token, &name)?;
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Machine(machine_id(&name))),
        };
        let progress = match self.operations.start(
            &caller,
            pb::OperationKind::RemoveMachine,
            target,
            &OpStep::REMOVE,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Response::new(operation)),
        };
        self.work.link_down(&name);
        let operation = self.run(progress, Arc::new(move |machines, drive| {
            machines.remove(&name, drive).map(|()| None)
        }));
        Ok(Response::new(operation))
    }

    async fn plan_forget_machine(
        &self,
        _request: Request<pb::PlanForgetMachineRequest>,
    ) -> Result<Response<pb::ForgetMachinePlan>, Status> {
        Err(status::not_built("humata.v1.MachineService.PlanForgetMachine"))
    }

    async fn forget_machine(
        &self,
        _request: Request<pb::ForgetMachineRequest>,
    ) -> Result<Response<pb::ForgetMachineResponse>, Status> {
        Err(status::not_built("humata.v1.MachineService.ForgetMachine"))
    }

    async fn plan_change_root(
        &self,
        _request: Request<pb::PlanChangeRootRequest>,
    ) -> Result<Response<pb::ChangeRootPlan>, Status> {
        Err(status::not_built("humata.v1.MachineService.PlanChangeRoot"))
    }

    async fn change_root(
        &self,
        _request: Request<pb::ChangeRootRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        Err(status::not_built("humata.v1.MachineService.ChangeRoot"))
    }

    async fn set_vm_tier(
        &self,
        _request: Request<pb::SetVmTierRequest>,
    ) -> Result<Response<pb::Machine>, Status> {
        Err(status::not_built("humata.v1.MachineService.SetVmTier"))
    }

    async fn get_storage(
        &self,
        _request: Request<pb::GetStorageRequest>,
    ) -> Result<Response<pb::StorageReport>, Status> {
        Err(status::not_built("humata.v1.MachineService.GetStorage"))
    }

    async fn clear_build_cache(
        &self,
        _request: Request<pb::ClearBuildCacheRequest>,
    ) -> Result<Response<pb::ClearBuildCacheResponse>, Status> {
        Err(status::not_built("humata.v1.MachineService.ClearBuildCache"))
    }

    async fn trim_logs(
        &self,
        _request: Request<pb::TrimLogsRequest>,
    ) -> Result<Response<pb::TrimLogsResponse>, Status> {
        Err(status::not_built("humata.v1.MachineService.TrimLogs"))
    }

    async fn list_host_variables(
        &self,
        _request: Request<pb::ListHostVariablesRequest>,
    ) -> Result<Response<pb::ListHostVariablesResponse>, Status> {
        Err(status::not_built("humata.v1.MachineService.ListHostVariables"))
    }

    async fn set_host_variable(
        &self,
        _request: Request<pb::SetHostVariableRequest>,
    ) -> Result<Response<pb::SetHostVariableResponse>, Status> {
        Err(status::not_built("humata.v1.MachineService.SetHostVariable"))
    }

    async fn remove_host_variable(
        &self,
        _request: Request<pb::RemoveHostVariableRequest>,
    ) -> Result<Response<pb::SetHostVariableResponse>, Status> {
        Err(status::not_built("humata.v1.MachineService.RemoveHostVariable"))
    }

    async fn trust_new_host_key(
        &self,
        _request: Request<pb::TrustNewHostKeyRequest>,
    ) -> Result<Response<pb::Machine>, Status> {
        Err(status::not_built("humata.v1.MachineService.TrustNewHostKey"))
    }
}
