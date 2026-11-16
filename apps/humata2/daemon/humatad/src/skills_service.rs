//! SkillsService (SK-1 to SK-11): previewing a sync, syncing skills to
//! machines, picking a name clash, and syncing a machine by itself when its
//! peer link comes up.
//!
//! The engine (`humata2_machines`, skills.rs) scans the host's skill folders
//! and plans; `peer_skills` sends a set over a machine's link, where the
//! machine applies it whole (SK-9). This is the host's half of the wiring.
//!
//! What the host keeps, in the state folder (the SettingsService isn't
//! built yet, so settings live in a file of their own):
//! - `skills-settings.json`: `{"auto_sync": true, "auto_off": ["box-1"],
//!   "excludes": ["*.bak"]}` (SK-3, SK-6). All three are optional.
//! - `skills/<machine>.json`: the manifest of the last set the machine took,
//!   the clashes it reported for it, the user's clash picks, whether a sync
//!   is waiting for the link, and the last failure (SK-8).
//!
//! Nothing polls (TR-7): a sync runs when the user asks, and once each time
//! a machine's link comes up.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use humata2_access::{AccessRequest, Call, MachineAction, MachineActionKind, MachineId, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_machines::{
    Choice, ChangeKind, ClashUse, EntryState, SkillName, SkillSet, SkillsError, SkillsManifest, SkillsPlan,
    plan_skills_sync, scan_skills,
};
use pb::peer_service_client::PeerServiceClient;
use serde_json::{Value, json};
use tonic::transport::Channel;
use tonic::{Code, Request, Response, Status};

use crate::channel_service::{self, Gated, answer_gated};
use crate::guard::Guard;
use crate::identity::{self, Caller};
use crate::inbox::Inbox;
use crate::machine_service::{Work, lock, machine_id, machine_of, status_of};
use crate::operations::{Operations, Started};
use crate::peer_skills::{self, agent_sources};
use crate::status;

/// How long a plan token stays good.
const PLAN_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// Why the host's skills state couldn't be read, written or scanned.
#[derive(Debug, thiserror::Error)]
pub(crate) enum HostError {
    #[error("couldn't read {}: {source}. Check that the file is yours and readable, then try again", .path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't write {}: {source}. Check that the state folder is writable and the disk isn't full, then try again", .path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{} isn't valid JSON. Fix it, or delete it to start from defaults (a deleted machine file means its next sync is a first sync)", .path.display())]
    Syntax {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("{} doesn't have the expected shape ({what}). Fix it, or delete it to start from defaults", .path.display())]
    Shape { path: PathBuf, what: &'static str },
    #[error("couldn't read the host's skills: {source}")]
    Scan {
        #[source]
        source: SkillsError,
    },
    #[error("{name:?} can't be used as a machine's skills file name")]
    BadName { name: String },
    #[error("reading the host's skills stopped before it finished: {source}")]
    Task {
        #[source]
        source: tokio::task::JoinError,
    },
}

impl HostError {
    fn fix(&self) -> &'static str {
        match self {
            Self::Scan { .. } => "fix the folder the message names (the agent skill folders under your home) or exclude the file in the skills settings, then try again",
            Self::BadName { .. } => "use a machine name of letters, digits and dashes",
            Self::Task { .. } => "try again",
            _ => "fix the file the message names, then try again",
        }
    }

    fn error(&self) -> pb::Error {
        pb::Error {
            message: status::chain(self),
            fix: self.fix().to_owned(),
            retry: None,
            kind: None,
        }
    }

    fn status(&self) -> Status {
        status::failure(Code::Internal, self.error())
    }
}

/// The settings file's contents.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Settings {
    auto_sync: bool,
    auto_off: BTreeSet<String>,
    excludes: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_sync: true,
            auto_off: BTreeSet::new(),
            excludes: Vec::new(),
        }
    }
}

impl Settings {
    fn wants_auto(&self, machine: &str) -> bool {
        self.auto_sync && !self.auto_off.contains(machine)
    }

    fn parse(path: &Path, text: &str) -> Result<Self, HostError> {
        let value: Value = serde_json::from_str(text).map_err(|source| HostError::Syntax {
            path: path.to_owned(),
            source,
        })?;
        let shape = |what| HostError::Shape {
            path: path.to_owned(),
            what,
        };
        let strings = |key: &str, what| -> Result<Vec<String>, HostError> {
            match value.get(key) {
                None | Some(Value::Null) => Ok(Vec::new()),
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|item| item.as_str().map(str::to_owned).ok_or_else(|| shape(what)))
                    .collect(),
                Some(_) => Err(shape(what)),
            }
        };
        let auto_sync = match value.get("auto_sync") {
            None | Some(Value::Null) => true,
            Some(Value::Bool(on)) => *on,
            Some(_) => return Err(shape("auto_sync must be true or false")),
        };
        Ok(Self {
            auto_sync,
            auto_off: strings("auto_off", "auto_off must be a list of machine names")?
                .into_iter()
                .collect(),
            excludes: strings("excludes", "excludes must be a list of patterns")?,
        })
    }
}

/// What the host keeps about one machine's skills.
#[derive(Debug, Clone, Default)]
struct Record {
    /// The set the machine last took (none: never synced, SK-5).
    manifest: Option<SkillsManifest>,
    /// `agent/skill` names the machine said it also has of its own.
    clashes: Vec<String>,
    picks: BTreeMap<SkillName, Choice>,
    /// A sync the user asked for while the machine was offline (SK-7): the
    /// id of the set they approved.
    pending: Option<String>,
    /// The last sync's failure (SK-8): message and fix.
    problem: Option<(String, String)>,
}

impl Record {
    fn local_names(&self) -> BTreeMap<String, BTreeSet<String>> {
        let mut local: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for clash in &self.clashes {
            if let Some((agent, skill)) = clash.split_once('/') {
                local.entry(agent.to_owned()).or_default().insert(skill.to_owned());
            }
        }
        local
    }

    fn to_json(&self) -> Result<String, HostError> {
        let manifest = match &self.manifest {
            Some(manifest) => json!(manifest.to_json().map_err(|source| HostError::Scan { source })?),
            None => Value::Null,
        };
        let picks: Vec<Value> = self
            .picks
            .iter()
            .map(|(name, choice)| {
                json!({
                    "agent": name.agent,
                    "skill": name.skill,
                    "use": match choice {
                        Choice::Synced => "synced",
                        Choice::Local => "local",
                    },
                })
            })
            .collect();
        let problem = match &self.problem {
            Some((message, fix)) => json!({"message": message, "fix": fix}),
            None => Value::Null,
        };
        Ok(json!({
            "manifest": manifest,
            "clashes": self.clashes,
            "picks": picks,
            "pending": self.pending,
            "problem": problem,
        })
        .to_string())
    }

    fn parse(path: &Path, text: &str) -> Result<Self, HostError> {
        let value: Value = serde_json::from_str(text).map_err(|source| HostError::Syntax {
            path: path.to_owned(),
            source,
        })?;
        let shape = |what| HostError::Shape {
            path: path.to_owned(),
            what,
        };
        let manifest = match value.get("manifest") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(
                SkillsManifest::from_json(text).map_err(|_| shape("manifest isn't a skills manifest"))?,
            ),
            Some(_) => return Err(shape("manifest must be text")),
        };
        let clashes = match value.get("clashes") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| item.as_str().map(str::to_owned).ok_or_else(|| shape("clashes must be text")))
                .collect::<Result<_, _>>()?,
            Some(_) => return Err(shape("clashes must be a list")),
        };
        let mut picks = BTreeMap::new();
        match value.get("picks") {
            None | Some(Value::Null) => {}
            Some(Value::Array(items)) => {
                for item in items {
                    let text = |key: &str| item.get(key).and_then(Value::as_str);
                    let (Some(agent), Some(skill), Some(choice)) = (text("agent"), text("skill"), text("use"))
                    else {
                        return Err(shape("a pick needs agent, skill and use"));
                    };
                    let choice = match choice {
                        "synced" => Choice::Synced,
                        "local" => Choice::Local,
                        _ => return Err(shape("a pick's use must be synced or local")),
                    };
                    picks.insert(
                        SkillName {
                            agent: agent.to_owned(),
                            skill: skill.to_owned(),
                        },
                        choice,
                    );
                }
            }
            Some(_) => return Err(shape("picks must be a list")),
        }
        let problem = value.get("problem").and_then(|problem| {
            Some((
                problem.get("message")?.as_str()?.to_owned(),
                problem.get("fix")?.as_str()?.to_owned(),
            ))
        });
        Ok(Self {
            manifest,
            clashes,
            picks,
            pending: value.get("pending").and_then(Value::as_str).map(str::to_owned),
            problem,
        })
    }
}

/// How a machine took (or didn't take) a sync.
#[derive(Debug)]
pub(crate) enum Outcome {
    Applied,
    /// No link is up to the machine.
    Offline,
    Failed(pb::Error),
}

/// Everything a plan needs, read once.
struct Prepared {
    set: SkillSet,
    settings: Settings,
    machines: Vec<(String, Record, SkillsPlan)>,
}

/// The host's skills state, the machines' links, and the plan tokens.
pub(crate) struct SkillsHost {
    /// `<state>/skills`, one file per machine.
    dir: PathBuf,
    settings_file: PathBuf,
    home: PathBuf,
    links: Mutex<HashMap<String, Channel>>,
    /// Plan tokens: token to (machine, set id, made).
    plans: Mutex<HashMap<String, (String, String, Instant)>>,
    /// One sync to a machine at a time.
    gates: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl SkillsHost {
    /// The skills state in `state`; the host's skill folders are the agents'
    /// under `home`.
    pub(crate) fn new(state: &Path, home: PathBuf) -> Self {
        Self {
            dir: state.join("skills"),
            settings_file: state.join("skills-settings.json"),
            home,
            links: Mutex::new(HashMap::new()),
            plans: Mutex::new(HashMap::new()),
            gates: Mutex::new(HashMap::new()),
        }
    }

    fn settings(&self) -> Result<Settings, HostError> {
        match std::fs::read_to_string(&self.settings_file) {
            Ok(text) => Settings::parse(&self.settings_file, &text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(source) => Err(HostError::Read {
                path: self.settings_file.clone(),
                source,
            }),
        }
    }

    fn record_path(&self, machine: &str) -> Result<PathBuf, HostError> {
        if machine.is_empty() || machine.starts_with('.') || machine.contains(['/', '\\', '\0']) {
            return Err(HostError::BadName {
                name: machine.to_owned(),
            });
        }
        Ok(self.dir.join(format!("{machine}.json")))
    }

    fn record(&self, machine: &str) -> Result<Record, HostError> {
        let path = self.record_path(machine)?;
        match std::fs::read_to_string(&path) {
            Ok(text) => Record::parse(&path, &text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Record::default()),
            Err(source) => Err(HostError::Read { path, source }),
        }
    }

    /// Writes the record whole or not at all.
    fn save(&self, machine: &str, record: &Record) -> Result<(), HostError> {
        let path = self.record_path(machine)?;
        let text = record.to_json()?;
        let write = |source| HostError::Write {
            path: path.clone(),
            source,
        };
        std::fs::create_dir_all(&self.dir).map_err(write)?;
        let temp = path.with_extension("json.new");
        std::fs::write(&temp, text).map_err(write)?;
        std::fs::rename(&temp, &path).map_err(write)
    }

    /// Scans the host's skills once and plans each of `machines` (blocking).
    fn prepare_blocking(&self, machines: &[String]) -> Result<Prepared, HostError> {
        let settings = self.settings()?;
        let set = scan_skills(&agent_sources(&self.home), &settings.excludes)
            .map_err(|source| HostError::Scan { source })?;
        let mut planned = Vec::new();
        for machine in machines {
            let record = self.record(machine)?;
            let plan = plan_skills_sync(&set, record.manifest.as_ref(), &record.local_names(), &record.picks);
            planned.push((machine.clone(), record, plan));
        }
        Ok(Prepared {
            set,
            settings,
            machines: planned,
        })
    }

    async fn prepare(self: &Arc<Self>, machines: Vec<String>) -> Result<Prepared, HostError> {
        let host = self.clone();
        tokio::task::spawn_blocking(move || host.prepare_blocking(&machines))
            .await
            .map_err(|source| HostError::Task { source })?
    }

    /// The machine's page: whether it syncs by itself, and why its last
    /// sync failed, if it did (SK-3, SK-8).
    pub(crate) fn health(&self, machine: &str) -> (bool, Option<pb::Error>) {
        let auto = self.settings().map(|settings| settings.wants_auto(machine)).unwrap_or(true);
        let problem = self.record(machine).ok().and_then(|record| record.problem).map(|(message, fix)| pb::Error {
            message,
            fix,
            retry: None,
            kind: None,
        });
        (auto, problem)
    }

    fn gate(&self, machine: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.gates).entry(machine.to_owned()).or_default().clone()
    }

    fn issue(&self, machine: &str, set_id: &str) -> String {
        let token = format!("pl_{}", uuid::Uuid::new_v4().simple());
        let mut plans = lock(&self.plans);
        plans.retain(|_, (_, _, made)| made.elapsed() < PLAN_LIFETIME);
        plans.insert(token.clone(), (machine.to_owned(), set_id.to_owned(), Instant::now()));
        token
    }

    /// The one of `tokens` that is a good plan for `machine` and this set.
    fn good_plan(&self, tokens: &[String], machine: &str, set_id: &str) -> Option<String> {
        let plans = lock(&self.plans);
        tokens
            .iter()
            .find(|token| {
                plans.get(*token).is_some_and(|(planned, planned_set, made)| {
                    planned == machine && planned_set == set_id && made.elapsed() < PLAN_LIFETIME
                })
            })
            .cloned()
    }

    fn spend(&self, token: &str) {
        lock(&self.plans).remove(token);
    }

    /// A machine's link came up (SK-3, SK-7): remembers it for syncs, and
    /// syncs the machine in the background when it should, once. A first
    /// sync or a new clash is left for the user (SK-5, SK-11). Never blocks
    /// the link and never polls.
    pub(crate) fn link_up(self: &Arc<Self>, machine: &str, channel: Channel) {
        lock(&self.links).insert(machine.to_owned(), channel);
        let host = self.clone();
        let machine = machine.to_owned();
        tokio::spawn(async move { host.on_link_up(&machine).await });
    }

    /// The machine's link went down.
    pub(crate) fn link_down(&self, machine: &str) {
        lock(&self.links).remove(machine);
    }

    async fn on_link_up(self: &Arc<Self>, machine: &str) {
        let gate = self.gate(machine);
        let _one_at_a_time = gate.lock().await;
        let prepared = match self.prepare(vec![machine.to_owned()]).await {
            Ok(prepared) => prepared,
            Err(error) => {
                crate::log::say!("humatad2: couldn't plan the skills sync for {machine}: {}", status::chain(&error));
                return;
            }
        };
        let Some((_, record, plan)) = prepared.machines.into_iter().next() else {
            return;
        };
        if !(prepared.settings.wants_auto(machine) || record.pending.is_some()) {
            return;
        }
        if plan.changes.is_empty() {
            if record.pending.is_some() {
                let mut record = record;
                record.pending = None;
                if let Err(error) = self.save(machine, &record) {
                    crate::log::say!("humatad2: {}", status::chain(&error));
                }
            }
            return;
        }
        // A sync the user asked for and approved while the machine was away
        // goes ahead even when it is a first sync or has a clash (SK-5).
        let approved = record.pending.as_deref() == Some(prepared.set.set_id());
        if plan.needs_asking() && !approved {
            return;
        }
        if let Outcome::Failed(error) = self.apply_locked(machine, &prepared.set).await {
            crate::log::say!("humatad2: skills sync to {machine} failed: {}", error.message);
        }
    }

    /// Sends `set` to `machine` over its link. The caller holds the
    /// machine's gate. A failure is kept as the machine's problem (SK-8),
    /// and the next success clears it.
    async fn apply_locked(&self, machine: &str, set: &SkillSet) -> Outcome {
        let Some(channel) = lock(&self.links).get(machine).cloned() else {
            return Outcome::Offline;
        };
        let mut record = match self.record(machine) {
            Ok(record) => record,
            Err(error) => return Outcome::Failed(error.error()),
        };
        let mut peer = PeerServiceClient::new(channel);
        match peer_skills::send(&mut peer, set).await {
            Ok(answer) => {
                record.manifest = Some(set.manifest());
                record.clashes = answer.clashes;
                record.pending = None;
                record.problem = None;
                match self.save(machine, &record) {
                    Ok(()) => Outcome::Applied,
                    Err(error) => Outcome::Failed(error.error()),
                }
            }
            Err(error) => {
                let failure = pb::Error {
                    message: error.to_string(),
                    fix: error.fix().to_owned(),
                    retry: None,
                    kind: None,
                };
                record.problem = Some((failure.message.clone(), failure.fix.clone()));
                if let Err(error) = self.save(machine, &record) {
                    crate::log::say!("humatad2: {}", status::chain(&error));
                }
                Outcome::Failed(failure)
            }
        }
    }

    /// Marks `machine` to sync when its link returns (SK-7, SK-10).
    fn mark_pending(&self, machine: &str, set_id: &str) -> Result<(), HostError> {
        let mut record = self.record(machine)?;
        record.pending = Some(set_id.to_owned());
        self.save(machine, &record)
    }
}

#[derive(Clone)]
pub(crate) struct SkillsSvc {
    guard: Arc<Guard>,
    work: Arc<Work>,
    operations: Arc<Operations>,
    host: Arc<SkillsHost>,
    /// Where a call a rule sends to Ask me waits for the user (D38.6).
    inbox: Arc<Inbox>,
}

impl SkillsSvc {
    pub(crate) fn new(
        guard: Arc<Guard>,
        work: Arc<Work>,
        operations: Arc<Operations>,
        host: Arc<SkillsHost>,
        inbox: Arc<Inbox>,
    ) -> Self {
        Self {
            guard,
            work,
            operations,
            host,
            inbox,
        }
    }

    /// Previewing a sync and picking a clash are the user's review of a plan
    /// (SK-2): the peer is refused as MachineService refuses it, and agents
    /// and scripts are told to ask the user, before the guard is consulted.
    /// `fix` is what the refused agent can do.
    fn require(&self, caller: &Caller, machine: &str, what: &str, fix: &str) -> Result<(), Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        let action = MachineActionKind::SyncSkills;
        if !matches!(caller, Caller::User(_)) {
            return Err(status::user_only_call(action.capability(), what, fix));
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

    /// Checks the sync for one machine as [`SkillsSvc::require`] would for
    /// the user, but a rule of Ask me answers `Some(request)` for the caller
    /// to put in the Inbox rather than refusing (D38.6). An `approved` call
    /// goes on.
    fn gate(&self, caller: &Caller, machine: &str, approved: bool) -> Result<Option<Box<AccessRequest>>, Status> {
        let action = MachineActionKind::SyncSkills;
        channel_service::gate(
            &self.guard,
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

    /// The machine's name, after checking it is one this host has and
    /// that it is ready.
    fn ready(&self, id: Option<pb::MachineId>) -> Result<String, Status> {
        let name = machine_of(id)?;
        let entry = self.work.machines.get(&name).map_err(|error| status_of(&error))?;
        if entry.state != EntryState::Ready {
            return Err(status::failure(
                Code::FailedPrecondition,
                pb::Error {
                    message: format!("{name} is still being added or removed, so its skills can't be synced yet"),
                    fix: "wait for the add or remove to finish, then try again".to_owned(),
                    retry: None,
                    kind: None,
                },
            ));
        }
        Ok(name)
    }

    /// The body of SyncSkills; `approved` once the user said yes to it in
    /// the Inbox (D38.6, TP-13). Syncs to the named machines, or every ready
    /// one. A first sync or a new clash needs the machine's plan token (an
    /// agent has none: it can't preview, so a machine that needs asking
    /// refuses the whole call naming it, as for the user); the call is
    /// refused naming the machine without one. Offline machines are skipped
    /// and named in their steps; they sync when their link returns.
    async fn sync(
        &self,
        caller: &Caller,
        request: pb::SyncSkillsRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        if let Some(operation) = self.operations.existing(caller, request.idempotency_key.as_ref())? {
            return Ok(Gated::Done(operation));
        }
        let mut names: Vec<String> = Vec::new();
        if request.machines.is_empty() {
            let entries = self.work.machines.list().map_err(|error| status_of(&error))?;
            names.extend(entries.iter().filter(|entry| entry.state == EntryState::Ready).map(|entry| entry.name.clone()));
        } else {
            for id in request.machines {
                let name = machine_of(Some(id))?;
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        // Each machine is checked on its own, as the user's call is. The
        // strictest rule decides: a Never refuses the call naming that
        // machine, else any Ask defers the whole call as one Inbox request.
        let mut asked = None;
        for name in &names {
            if let Some(request) = self.gate(caller, name, approved)? {
                asked.get_or_insert(request);
            }
        }
        if let Some(request) = asked {
            return Ok(Gated::Ask(request));
        }
        // Whether a machine exists is looked at only once the rules let the
        // call through: an agent sees no machines (D61), so a call it must
        // ask for answers the same for a machine that is there and one that
        // isn't.
        for name in &names {
            self.ready(Some(machine_id(name)))?;
        }
        let tokens: Vec<String> = request.plans.into_iter().map(|plan| plan.value).collect();
        let prepared = self.host.prepare(names.clone()).await.map_err(|error| error.status())?;
        let mut spent = Vec::new();
        for (name, _, plan) in &prepared.machines {
            if plan.needs_asking() {
                let token = self
                    .host
                    .good_plan(&tokens, name, prepared.set.set_id())
                    .ok_or_else(|| plan_needed(name))?;
                spent.push(token);
            }
        }
        let target = pb::EntityRef {
            entity: match names.as_slice() {
                [only] => Some(pb::entity_ref::Entity::Machine(machine_id(only))),
                _ => None,
            },
        };
        let steps: Vec<(String, String)> = names
            .iter()
            .map(|name| (format!("sync:{name}"), format!("Syncing skills to {name}")))
            .collect();
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::SkillsSync,
            target,
            &steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        for token in &spent {
            self.host.spend(token);
        }
        let operation = progress.operation();
        let host = self.host.clone();
        tokio::spawn(async move {
            let Prepared { set, machines, .. } = prepared;
            let mut failed: Vec<(String, pb::Error)> = Vec::new();
            for (name, _, plan) in machines {
                let key = format!("sync:{name}");
                progress.set_step(&key, pb::StepState::Running, None);
                if plan.changes.is_empty() && !plan.first_sync {
                    progress.set_step(&key, pb::StepState::Done, None);
                    continue;
                }
                let gate = host.gate(&name);
                let _one_at_a_time = gate.lock().await;
                match host.apply_locked(&name, &set).await {
                    Outcome::Applied => progress.set_step(&key, pb::StepState::Done, None),
                    Outcome::Offline => {
                        let note = match host.mark_pending(&name, set.set_id()) {
                            Ok(()) => format!("{name} is offline, so it was skipped; it syncs when it is back"),
                            Err(error) => format!(
                                "{name} is offline, so it was skipped, and Humata couldn't note that it should sync when it is back: {}",
                                status::chain(&error)
                            ),
                        };
                        progress.set_step(&key, pb::StepState::Skipped, Some(note));
                    }
                    Outcome::Failed(error) => {
                        progress.set_step(&key, pb::StepState::Failed, Some(error.message.clone()));
                        failed.push((name, error));
                    }
                }
            }
            if failed.is_empty() {
                progress.succeed(None);
            } else {
                let names: Vec<&str> = failed.iter().map(|(name, _)| name.as_str()).collect();
                let reasons: Vec<String> = failed
                    .iter()
                    .map(|(name, error)| format!("{name}: {}", error.message))
                    .collect();
                progress.fail(pb::Error {
                    message: format!("couldn't sync skills to {}; {}", names.join(", "), reasons.join("; ")),
                    fix: "the steps say what failed on each machine; the others were synced. Fix the cause and sync again".to_owned(),
                    retry: None,
                    kind: None,
                });
            }
        });
        Ok(Gated::Done(operation))
    }
}

fn plan_needed(machine: &str) -> Status {
    status::failure(
        Code::FailedPrecondition,
        status::error(
            format!(
                "{machine}'s skills sync needs a plan you have looked at (its first sync, or a clash with a skill made on the machine), and none was given, or it is for other skills, used, or older than ten minutes"
            ),
            "preview the sync (PreviewSkillsSync), check what it says, then sync with its token",
            pb::error::Kind::PlanStale(pb::PlanStale { changes: Vec::new() }),
        ),
    )
}

fn shown_plan(machine: &str, plan: &SkillsPlan, token: String) -> pb::SkillsSyncPlan {
    pb::SkillsSyncPlan {
        machine: Some(machine_id(machine)),
        first_sync: plan.first_sync,
        changes: plan
            .changes
            .iter()
            .map(|change| pb::SkillChange {
                agent: change.name.agent.clone(),
                skill: change.name.skill.clone(),
                kind: match change.kind {
                    ChangeKind::Add => pb::skill_change::Kind::Add,
                    ChangeKind::Update => pb::skill_change::Kind::Update,
                    ChangeKind::Remove => pb::skill_change::Kind::Remove,
                } as i32,
            })
            .collect(),
        clashes: plan
            .clashes
            .iter()
            .map(|clash| pb::SkillClash {
                agent: clash.name.agent.clone(),
                skill: clash.name.skill.clone(),
                current: match clash.current {
                    ClashUse::Unpicked => pb::skill_clash::Use::Unspecified,
                    ClashUse::Synced => pb::skill_clash::Use::Synced,
                    ClashUse::Local => pb::skill_clash::Use::Local,
                } as i32,
            })
            .collect(),
        excluded_files: plan.excluded_files,
        token: Some(pb::PlanToken { value: token }),
    }
}

fn plain(field: &str, value: &str) -> Result<(), Status> {
    if value.is_empty() || value == "." || value == ".." || value.contains(['/', '\\', '\0']) {
        return Err(status::invalid_field(
            field,
            pb::invalid_field::Problem::BadFormat,
            format!("{field} {value:?} isn't a plain skill or agent name"),
            format!("use the {field} as the preview shows it"),
        ));
    }
    Ok(())
}

#[tonic::async_trait]
impl pb::skills_service_server::SkillsService for SkillsSvc {
    /// What a sync to the machine would do, and the token a sync takes. An
    /// offline machine is previewed from what the host last sent it.
    async fn preview_skills_sync(
        &self,
        request: Request<pb::PreviewSkillsSyncRequest>,
    ) -> Result<Response<pb::SkillsSyncPlan>, Status> {
        let caller = identity::caller(&request)?;
        let machine = machine_of(request.into_inner().machine)?;
        self.require(&caller, &machine, "previewing a skills sync", "ask the user with inbox_ask to preview and sync the machine's skills in the app; asking again is refused the same way")?;
        let machine = self.ready(Some(machine_id(&machine)))?;
        // A sync to this machine that is under way finishes first: its files
        // reach the machine before the host has recorded the set, and a
        // plan made in between would list changes that are already applied.
        let gate = self.host.gate(&machine);
        let _after_any_sync = gate.lock().await;
        let prepared = self.host.prepare(vec![machine.clone()]).await.map_err(|error| error.status())?;
        let Some((_, _, plan)) = prepared.machines.first() else {
            return Err(status::internal("the skills plan came back empty", ""));
        };
        let token = self.host.issue(&machine, prepared.set.set_id());
        Ok(Response::new(shown_plan(&machine, plan, token)))
    }

    /// Syncs to the named machines, or every ready one. An agent's sync waits
    /// in the Inbox when its rule says Ask me, and runs once the user
    /// approves it (D38.6); a first sync or a new clash still needs the
    /// plan tokens the rule requires.
    async fn sync_skills(&self, request: Request<pb::SyncSkillsRequest>) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request.idempotency_key.as_ref().map(|key| key.value.clone());
        let first = self.sync(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Operation",
            first,
            move || async move { me.sync(&who, request, true).await },
        )
        .await
    }

    /// Saves the user's pick for a clash on this machine (SK-5). Picks stay
    /// on the host; a sync's plan uses them.
    async fn resolve_skill_clash(
        &self,
        request: Request<pb::ResolveSkillClashRequest>,
    ) -> Result<Response<pb::ResolveSkillClashResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let machine = machine_of(request.machine)?;
        self.require(&caller, &machine, "picking which skill wins a clash", "ask the user with inbox_ask to pick in the app, then sync with SyncSkills; asking again is refused the same way")?;
        let machine = self.ready(Some(machine_id(&machine)))?;
        plain("agent", &request.agent)?;
        plain("skill", &request.skill)?;
        let choice = match pb::skill_clash::Use::try_from(request.r#use) {
            Ok(pb::skill_clash::Use::Synced) => Choice::Synced,
            Ok(pb::skill_clash::Use::Local) => Choice::Local,
            _ => {
                return Err(status::invalid_field(
                    "use",
                    pb::invalid_field::Problem::Missing,
                    "the pick says neither the synced skill nor the machine's own",
                    "pick USE_SYNCED or USE_LOCAL",
                ));
            }
        };
        let gate = self.host.gate(&machine);
        let _one_at_a_time = gate.lock().await;
        let mut record = self.host.record(&machine).map_err(|error| error.status())?;
        record.picks.insert(
            SkillName {
                agent: request.agent,
                skill: request.skill,
            },
            choice,
        );
        self.host.save(&machine, &record).map_err(|error| error.status())?;
        Ok(Response::new(pb::ResolveSkillClashResponse {}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, relative: &str, bytes: &[u8]) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn host() -> (tempfile::TempDir, SkillsHost) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        write(&home.join(".claude/skills"), "alpha/SKILL.md", b"alpha");
        write(&home.join(".codex/skills"), "beta/SKILL.md", b"beta");
        let skills = SkillsHost::new(&dir.path().join("state"), home);
        (dir, skills)
    }

    fn names(plan: &SkillsPlan) -> Vec<(String, String, ChangeKind)> {
        plan.changes
            .iter()
            .map(|change| (change.name.agent.clone(), change.name.skill.clone(), change.kind))
            .collect()
    }

    #[test]
    fn a_machine_never_synced_gets_a_first_sync_plan_adding_every_skill_of_an_installed_agent() {
        let (_dir, host) = host();
        let prepared = host.prepare_blocking(&["box-1".to_owned()]).unwrap();
        let plan = &prepared.machines[0].2;
        assert!(plan.first_sync);
        assert!(plan.needs_asking());
        assert_eq!(
            names(plan),
            vec![
                ("claude".to_owned(), "alpha".to_owned(), ChangeKind::Add),
                ("codex".to_owned(), "beta".to_owned(), ChangeKind::Add),
            ]
        );
    }

    #[test]
    fn a_record_survives_a_save_and_the_plan_after_it_is_empty_until_a_skill_changes() {
        let (dir, host) = host();
        let prepared = host.prepare_blocking(&["box-1".to_owned()]).unwrap();
        let mut record = Record::default();
        record.manifest = Some(prepared.set.manifest());
        record.clashes = vec!["claude/alpha".to_owned()];
        record.pending = Some("abc".to_owned());
        record.problem = Some(("it broke".to_owned(), "fix it".to_owned()));
        record.picks.insert(
            SkillName {
                agent: "claude".to_owned(),
                skill: "alpha".to_owned(),
            },
            Choice::Synced,
        );
        host.save("box-1", &record).unwrap();

        let again = host.record("box-1").unwrap();
        assert_eq!(again.manifest.as_ref().map(|m| m.set_id.clone()), Some(prepared.set.set_id().to_owned()));
        assert_eq!(again.clashes, record.clashes);
        assert_eq!(again.picks, record.picks);
        assert_eq!(again.pending.as_deref(), Some("abc"));
        assert_eq!(again.problem, record.problem);

        let plan = &host.prepare_blocking(&["box-1".to_owned()]).unwrap().machines[0].2;
        assert!(!plan.first_sync);
        assert!(plan.changes.is_empty());
        // The pick is the clash's current choice, and it isn't new.
        assert_eq!(plan.clashes.len(), 1);
        assert_eq!(plan.clashes[0].current, ClashUse::Synced);
        assert!(!plan.needs_asking());

        write(&dir.path().join("home/.claude/skills"), "alpha/SKILL.md", b"alpha, changed");
        write(&dir.path().join("home/.claude/skills"), "gamma/SKILL.md", b"new");
        let plan = &host.prepare_blocking(&["box-1".to_owned()]).unwrap().machines[0].2;
        assert_eq!(
            names(plan),
            vec![
                ("claude".to_owned(), "alpha".to_owned(), ChangeKind::Update),
                ("claude".to_owned(), "gamma".to_owned(), ChangeKind::Add),
            ]
        );
    }

    #[test]
    fn a_clash_reported_by_the_machine_is_new_until_it_is_picked() {
        let (_dir, host) = host();
        let prepared = host.prepare_blocking(&["box-1".to_owned()]).unwrap();
        let mut record = Record::default();
        record.manifest = Some(prepared.set.manifest());
        record.clashes = vec!["claude/alpha".to_owned()];
        host.save("box-1", &record).unwrap();
        let plan = &host.prepare_blocking(&["box-1".to_owned()]).unwrap().machines[0].2;
        assert!(plan.needs_asking());
        assert_eq!(plan.clashes[0].current, ClashUse::Unpicked);
    }

    #[test]
    fn settings_default_to_automatic_and_a_machine_can_opt_out() {
        let (dir, host) = host();
        assert!(host.settings().unwrap().wants_auto("box-1"));
        std::fs::create_dir_all(dir.path().join("state")).unwrap();
        std::fs::write(
            dir.path().join("state/skills-settings.json"),
            r#"{"auto_off": ["box-2"], "excludes": ["*.bak"]}"#,
        )
        .unwrap();
        let settings = host.settings().unwrap();
        assert!(settings.wants_auto("box-1"));
        assert!(!settings.wants_auto("box-2"));
        assert_eq!(settings.excludes, vec!["*.bak".to_owned()]);

        std::fs::write(dir.path().join("state/skills-settings.json"), r#"{"auto_sync": false}"#).unwrap();
        assert!(!host.settings().unwrap().wants_auto("box-1"));
    }

    #[test]
    fn excludes_in_the_settings_keep_files_out_of_the_set() {
        let (dir, host) = host();
        write(&dir.path().join("home/.claude/skills"), "alpha/draft.bak", b"x");
        std::fs::create_dir_all(dir.path().join("state")).unwrap();
        std::fs::write(dir.path().join("state/skills-settings.json"), r#"{"excludes": ["*.bak"]}"#).unwrap();
        let prepared = host.prepare_blocking(&["box-1".to_owned()]).unwrap();
        assert!(prepared.set.entries().iter().all(|entry| !entry.path.ends_with(".bak")));
        assert_eq!(prepared.set.excluded_files(), 1);
    }

    #[test]
    fn a_plan_token_is_for_one_machine_and_one_set_and_is_used_once() {
        let (_dir, host) = host();
        let token = host.issue("box-1", "set-a");
        let given = vec![token.clone()];
        assert_eq!(host.good_plan(&given, "box-2", "set-a"), None);
        assert_eq!(host.good_plan(&given, "box-1", "set-b"), None);
        assert_eq!(host.good_plan(&given, "box-1", "set-a"), Some(token.clone()));
        host.spend(&token);
        assert_eq!(host.good_plan(&given, "box-1", "set-a"), None);
    }

    #[test]
    fn a_machine_name_that_could_leave_the_skills_folder_is_refused() {
        let (_dir, host) = host();
        for name in ["", "../x", "a/b", ".hidden"] {
            assert!(host.record(name).is_err(), "{name:?}");
        }
    }
}
