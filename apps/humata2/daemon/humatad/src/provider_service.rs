//! ProviderService (MP-1..MP-24): the providers file (Settings › Machine
//! providers), creating a machine through a provider, a provider machine's
//! start, stop, status and delete, and deleting what a failed create made.
//! The work is machines' (`humata2_machines::Machines`); this wires it to
//! the API the way `machine_service` does: the access checks, an
//! `Operation` whose steps change as the work does, and the errors in the
//! API's terms.
//!
//! The providers file is `providers.toml` beside `hosts.json`, replaced
//! whole. Its revision is a hash of its text, so it survives restarts and
//! a stale editor is refused. Agents may read the providers and the file
//! (which holds host variable names, never their values) but set none
//! (MP-23); an agent may only ask the user to import a file, in the Inbox
//! (D792).
//!
//! A machine's `create` that ran leaves what it made if a later step
//! fails; the failed operation remembers which provider and params made
//! it, so DeleteWhatWasMade can run the provider's `delete`. That memory is
//! in this daemon's memory, like operations: after a restart the half-made
//! machine is undone (machines' `recover`) but a VM `create` made is left
//! for the user to delete with the provider's own tool.
//!
//! InteractiveLogin and MakeUnattended are built in this file by a later
//! task, beside the other calls below.

use std::collections::{BTreeMap, HashMap};
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use humata2_access::{AccessRequest, Call, Capability, MachineAction, MachineActionKind, MachineId, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_machines::{Action, ConfigError, MachineStatus, OpStep, Provider, Reach, StepEvent, parse_status};
use sha2::{Digest, Sha256};
use tokio::runtime::Handle;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Request, Response, Status};

use crate::channel_service::{self, Gated, answer_gated, answer_gated_showing};
use crate::guard::{self, Guard};
use crate::identity::{self, Caller};
use crate::inbox::Inbox;
use crate::machine_service::{
    Job, Observer, ProvidersFileError, Work, launch_observed, lock, machine_id, machine_of, providers_path,
    read_providers, status_of,
};
use crate::operations::{Operations, Started};
use crate::status;

mod ask;
mod import;
mod remove;

/// What an agent refused a providers change can do instead.
const SETTINGS_FIX: &str = "ask the user to make the change in Settings › Machines › Providers; to have them add a provider file, use provider_import (humata provider import <file>), which asks them in the Inbox";

/// What an agent refused an import can do instead.
const IMPORT_FIX: &str = "use provider_import (humata provider import <file>), which asks the user in the Inbox, or ask the user to paste the file in Settings › Machines › Providers › Import provider…";

/// What a failed create left, for DeleteWhatWasMade.
struct Left {
    provider: String,
    machine: String,
    values: BTreeMap<String, String>,
    /// The operation ended without succeeding (it may still be running
    /// right after `create` ran).
    ended: bool,
}

#[derive(Clone)]
pub(crate) struct ProviderSvc {
    guard: Arc<Guard>,
    work: Arc<Work>,
    operations: Arc<Operations>,
    /// By the id of the CreateMachine operation.
    left: Arc<Mutex<HashMap<String, Left>>>,
    /// The interactive login and make-unattended (MP-12–MP-17).
    login: Arc<crate::login_service::LoginSvc>,
    /// Where a call a rule sends to Ask me waits for the user (D38.6).
    inbox: Arc<Inbox>,
    /// MP-23: import plans by token, each good once.
    imports: Arc<Mutex<HashMap<String, import::Planned>>>,
}

impl ProviderSvc {
    pub(crate) fn new(guard: Arc<Guard>, work: Arc<Work>, operations: Arc<Operations>, inbox: Arc<Inbox>) -> Self {
        let left: Arc<Mutex<HashMap<String, Left>>> = Arc::new(Mutex::new(HashMap::new()));
        // The login finds what a failed create made by its operation's id,
        // and forgets it once the login finished the add.
        let pending = crate::login_service::Pending {
            find: Box::new({
                let left = left.clone();
                move |id: &str| {
                    lock(&left).get(id).filter(|made| made.ended).map(|made| crate::login_service::Made {
                        provider: made.provider.clone(),
                        machine: made.machine.clone(),
                        values: made.values.clone(),
                    })
                }
            }),
            done: Arc::new({
                let left = left.clone();
                move |id: &str| {
                    lock(&left).remove(id);
                }
            }),
        };
        let login = crate::login_service::LoginSvc::new(guard.clone(), work.clone(), operations.clone(), pending);
        Self {
            guard,
            work,
            operations,
            left,
            login: Arc::new(login),
            inbox,
            imports: Arc::default(),
        }
    }

    fn require_machine(&self, caller: &Caller, machine: &str, action: MachineActionKind) -> Result<(), Status> {
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

    /// Checks the call as [`ProviderSvc::require_machine`] does, but a rule
    /// of Ask me answers `Some(request)` for the caller to put in the Inbox
    /// rather than refusing (D38.6). An `approved` call goes on.
    fn gate(
        &self,
        caller: &Caller,
        machine: &str,
        action: MachineActionKind,
        approved: bool,
    ) -> Result<Option<Box<AccessRequest>>, Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
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

    /// The body of CreateMachine; `approved` once the user said yes to it in
    /// the Inbox (D38.6, TP-13).
    async fn create(
        &self,
        caller: &Caller,
        request: pb::CreateMachineRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        if let Some(asked) = self.gate(caller, request.name.trim(), MachineActionKind::Create, approved)? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(operation) = self.operations.existing(caller, request.idempotency_key.as_ref())? {
            return Ok(Gated::Done(operation));
        }
        let provider_id = request
            .provider
            .map(|provider| provider.value)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| status::missing("provider"))?;
        let provider = self.provider(&provider_id)?;
        let plan = self
            .work
            .machines
            .plan_create(&provider, &request.name, request.params.into_iter().collect())
            .map_err(|error| status_of(&error))?;
        let name = plan.machine.as_str().to_owned();
        let progress = match self.start(
            caller,
            pb::OperationKind::ProviderCreate,
            &name,
            &OpStep::create_steps(provider.reach()),
            true,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        let id = operation.id.clone().map(|id| id.value).unwrap_or_default();

        // From the moment `create` runs, something may exist to delete.
        let observer: Observer = {
            let left = self.left.clone();
            let id = id.clone();
            let remember = Left {
                provider: provider_id.clone(),
                machine: name.clone(),
                values: plan.values.clone(),
                ended: false,
            };
            Arc::new(move |event: &StepEvent| {
                if matches!(event, StepEvent::Running(OpStep::Create)) {
                    lock(&left).insert(
                        id.clone(),
                        Left {
                            provider: remember.provider.clone(),
                            machine: remember.machine.clone(),
                            values: remember.values.clone(),
                            ended: false,
                        },
                    );
                }
            })
        };
        let work = self.work.clone();
        let left = self.left.clone();
        let job: Job = Arc::new(move |machines, drive| {
            let created = machines.create(plan.clone(), drive);
            match &created {
                Ok(entry) => {
                    lock(&left).remove(&id);
                    work.saw(&entry.name, pb::Reachability::Online);
                    work.link_up(&entry.name, &Handle::current());
                }
                Err(_) => {
                    if let Some(made) = lock(&left).get_mut(&id) {
                        made.ended = true;
                    }
                }
            }
            created.map(|entry| Some(pb::operation::Result::Machine(machine_id(&entry.name))))
        });
        launch_observed(
            self.work.clone(),
            Arc::new(crate::wire_machines::ProgressAdapter(std::sync::Mutex::new(Some(progress)))),
            job,
            Some(observer),
        );
        Ok(Gated::Done(operation))
    }

    /// The body of RunProviderAction; `approved` once the user said yes to
    /// it in the Inbox. Delete is never asked for (D38.6, TP-13).
    async fn run_action(
        &self,
        caller: &Caller,
        request: pb::RunProviderActionRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        let name = machine_of(request.machine)?;
        let (action, kind) = match pb::ProviderAction::try_from(request.action) {
            Ok(pb::ProviderAction::Start) => (Action::Start, MachineActionKind::Start),
            Ok(pb::ProviderAction::Stop) => (Action::Stop, MachineActionKind::Stop),
            // Reading a status needs no more than starting would.
            Ok(pb::ProviderAction::Status) => (Action::Status, MachineActionKind::Start),
            Ok(pb::ProviderAction::Delete) => (Action::Delete, MachineActionKind::Delete),
            _ => return Err(status::missing("action")),
        };
        if action == Action::Delete {
            // Delete is never asked for: the user only, present, with a plan.
            self.require_machine(caller, &name, kind)?;
            self.require_present(caller, MachineActionKind::Delete.capability())?;
        } else if let Some(asked) = self.gate(caller, &name, kind, approved)? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(operation) = self.operations.existing(caller, request.idempotency_key.as_ref())? {
            return Ok(Gated::Done(operation));
        }
        let entry = self.work.machines.get(&name).map_err(|error| status_of(&error))?;
        let Some(record) = entry.provider else {
            return Err(status_of(&humata2_machines::ManageError::NotProviderMachine {
                machine: name,
                provider: String::new(),
            }));
        };
        let provider = self.provider(&record.id)?;
        if !provider.has(action) {
            return Err(status_of(&humata2_machines::ManageError::ActionUndefined {
                provider: provider.id().to_owned(),
                action,
            }));
        }
        if action == Action::Delete {
            let token = request
                .remove
                .and_then(|remove| remove.plan)
                .map(|plan| plan.value)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| status::missing("remove.plan"))?;
            self.work.use_plan(&token, &name)?;
        }
        let steps: &[OpStep] = if action == Action::Delete {
            &OpStep::PROVIDER_DELETE
        } else {
            &OpStep::PROVIDER_ACTION
        };
        let progress = match self.start(
            caller,
            pb::OperationKind::ProviderAction,
            &name,
            steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        let work = self.work.clone();
        if action == Action::Delete {
            work.link_down(&name);
        }
        let job: Job = Arc::new(move |machines, drive| {
            if action == Action::Delete {
                machines.delete_machine(&provider, &name, drive)?;
                return Ok(None);
            }
            let output = machines.run_action(&provider, &name, action, drive)?;
            let stopped = match action {
                Action::Stop => true,
                Action::Status => parse_status(&output.stdout) == MachineStatus::Stopped,
                _ => false,
            };
            if stopped {
                work.saw(&name, pb::Reachability::Stopped);
            }
            Ok(None)
        });
        launch_observed(
            self.work.clone(),
            Arc::new(crate::wire_machines::ProgressAdapter(std::sync::Mutex::new(Some(progress)))),
            job,
            None,
        );
        Ok(Gated::Done(operation))
    }

    /// The user only (agents and scripts are refused), for the providers
    /// file: it is a list of commands Humata will run. An agent is told the
    /// user does `what` in Settings › Machines › Providers, and `fix`.
    fn require_settings(&self, caller: &Caller, what: &str, fix: &str) -> Result<(), Status> {
        match caller {
            Caller::Peer => return Err(status::refused(&Refusal::PeerCannotCall)),
            Caller::Agent(_) => return Err(status::user_only_call(Capability::SettingsChange, what, fix)),
            Caller::User(_) | Caller::Script(_) => {}
        }
        self.guard.require(
            caller,
            &Call::Use {
                capability: Capability::SettingsChange,
                target: Target::Global,
            },
        )
    }

    /// The user must be at the Mac, as for RemoveMachine.
    fn require_present(&self, caller: &Caller, capability: Capability) -> Result<(), Status> {
        let principal = guard::principal(caller);
        if self
            .guard
            .with_policy(|_, context| principal.present_user(context))
            .is_none()
        {
            return Err(status::refused(&Refusal::UserOnly {
                capability: Some(capability),
            }));
        }
        Ok(())
    }

    /// The configured providers, then the built-in Lima example unless the
    /// file defines a `lima` of its own.
    fn providers(&self) -> Result<Vec<Provider>, Status> {
        let (_, mut providers) = read_providers(self.work.machines.hosts_file()).map_err(file_status)?;
        if !providers.iter().any(|provider| provider.id() == "lima") {
            let lima = Provider::lima().map_err(|error| {
                status::internal(
                    format!("the built-in Lima provider is invalid: {}", status::chain(&error)),
                    "",
                )
            })?;
            providers.push(lima);
        }
        Ok(providers)
    }

    fn provider(&self, id: &str) -> Result<Provider, Status> {
        self.providers()?
            .into_iter()
            .find(|provider| provider.id() == id)
            .ok_or_else(|| {
                status::not_found(
                    pb::EntityKind::Provider,
                    id,
                    format!("there is no provider {id} in the providers file"),
                    "add it in Settings › Machines › Providers, or pick one of the listed providers",
                )
            })
    }

    /// Starts an operation of `kind` on `machine`.
    fn start(
        &self,
        caller: &Caller,
        kind: pb::OperationKind,
        machine: &str,
        steps: &[OpStep],
        cancellable: bool,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<Started, Status> {
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Machine(machine_id(machine))),
        };
        self.operations.start(caller, kind, target, steps, cancellable, key)
    }
}

fn file_status(error: ProvidersFileError) -> Status {
    let message = status::chain(&error);
    match error {
        ProvidersFileError::Read { .. } => status::internal(message, ""),
        ProvidersFileError::Invalid { .. } => status::failure(
            Code::FailedPrecondition,
            status::error(
                message,
                "fix the providers in Settings › Machines › Providers; the file on disk was left as it is",
                pb::error::Kind::InvalidField(pb::InvalidField {
                    field: "toml".to_owned(),
                    problem: pb::invalid_field::Problem::BadFormat as i32,
                }),
            ),
        ),
    }
}

/// The file's revision: 0 for no file, else a hash of its text that is
/// never 0.
fn revision(text: &str, exists: bool) -> u64 {
    if !exists {
        return 0;
    }
    let digest = Sha256::digest(text.as_bytes());
    let mut first = [0_u8; 8];
    first.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(first).max(1)
}

fn current_revision(work: &Work) -> Result<(String, u64), Status> {
    let path = providers_path(work.machines.hosts_file());
    let exists = path.exists();
    let (text, _) = read_providers(work.machines.hosts_file()).or_else(|error| match error {
        // An invalid file on disk is still the file the revision names.
        ProvidersFileError::Invalid { .. } => std::fs::read_to_string(&path)
            .map(|text| (text, Vec::new()))
            .map_err(|source| file_status(ProvidersFileError::Read { path: path.clone(), source })),
        other => Err(file_status(other)),
    })?;
    let revision = revision(&text, exists);
    Ok((text, revision))
}

/// Replaces `path` whole: a private temporary file, fsync, rename, then
/// the folder.
fn write_whole(path: &Path, text: &str) -> std::io::Result<()> {
    let folder = path.parent().filter(|folder| !folder.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(folder)?;
    let temporary = path.with_extension(format!("toml.{}.new", uuid::Uuid::new_v4().simple()));
    let result = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .and_then(|mut out| {
            out.write_all(text.as_bytes())?;
            out.sync_all()
        })
        .and_then(|()| std::fs::rename(&temporary, path))
        .and_then(|()| std::fs::File::open(folder)?.sync_all());
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Writes an approved ask's merged providers file, unless the file changed
/// since the agent asked (D792).
fn write_asked(work: &Work, merged: &str, base: u64) -> Result<pb::SetProvidersConfigResponse, Status> {
    let (_, current) = current_revision(work)?;
    if current != base {
        return Err(ask::stale());
    }
    let path = providers_path(work.machines.hosts_file());
    write_whole(&path, merged).map_err(|source| {
        status::internal(
            format!(
                "couldn't save the providers file {}: {source}. Free some disk space or fix the folder's permissions, then ask the agent to ask again",
                path.display()
            ),
            "",
        )
    })?;
    Ok(pb::SetProvidersConfigResponse {
        problems: Vec::new(),
        revision: Some(pb::Revision {
            value: revision(merged, true),
        }),
    })
}

fn problems_of(error: &ConfigError) -> Vec<pb::ProviderConfigProblem> {
    let invalid = |field: &str, message: String, fix: &str| pb::Error {
        message,
        fix: fix.to_owned(),
        retry: None,
        kind: Some(pb::error::Kind::InvalidField(pb::InvalidField {
            field: field.to_owned(),
            problem: pb::invalid_field::Problem::BadFormat as i32,
        })),
    };
    match error {
        ConfigError::Syntax { line, .. } => vec![pb::ProviderConfigProblem {
            provider: None,
            field: String::new(),
            line: u32::try_from(*line).unwrap_or(0),
            error: Some(invalid("toml", error.to_string(), "fix that line, then save again")),
        }],
        ConfigError::Problems(problems) => problems
            .0
            .iter()
            .map(|problem| pb::ProviderConfigProblem {
                provider: Some(pb::ProviderId {
                    value: problem.provider.clone(),
                }),
                field: problem.field.clone(),
                line: problem.line.and_then(|line| u32::try_from(line).ok()).unwrap_or(0),
                error: Some(invalid(
                    &problem.field,
                    problem.to_string(),
                    "change that field in Settings › Machines › Providers, then save again",
                )),
            })
            .collect(),
    }
}

fn provider_of(provider: &Provider) -> pb::Provider {
    let actions = [
        (Action::Start, pb::ProviderAction::Start),
        (Action::Stop, pb::ProviderAction::Stop),
        (Action::Status, pb::ProviderAction::Status),
        (Action::Delete, pb::ProviderAction::Delete),
    ]
    .into_iter()
    .filter(|(action, _)| provider.has(*action))
    .map(|(_, shown)| shown as i32)
    .collect();
    pb::Provider {
        id: Some(pb::ProviderId {
            value: provider.id().to_owned(),
        }),
        label: provider.label().to_owned(),
        params: provider
            .params()
            .iter()
            .map(|param| pb::ProviderParam {
                name: param.name.clone(),
                prompt: param.prompt.clone(),
                default_value: param.default.clone().unwrap_or_default(),
            })
            .collect(),
        env: provider.env().to_vec(),
        has_create: provider.has(Action::Create),
        reach: match provider.reach() {
            Reach::Connect => pb::provider::Reach::Connect,
            Reach::Identity => pb::provider::Reach::Identity,
        } as i32,
        actions,
        interactive: provider.interactive(),
        run_on: provider.run_on().map(machine_id),
        built_in: provider.built_in(),
        // ListProviders fills these; other replies leave them empty.
        ..Default::default()
    }
}

fn no_command_to_run_by_hand(provider: &str, machine: &str) -> Status {
    status::failure(
        Code::FailedPrecondition,
        status::error(
            format!(
                "the {provider} provider has no `delete` command, so Humata can't remove what its create made for {machine}"
            ),
            format!(
                "remove {machine} with the provider's own tool; or add `delete` to the provider in Settings › Machines › Providers and try again"
            ),
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: "operation".to_owned(),
                problem: pb::invalid_field::Problem::NotAllowedHere as i32,
            }),
        ),
    )
}

#[tonic::async_trait]
impl pb::provider_service_server::ProviderService for ProviderSvc {
    /// Every provider, for the user and agents alike: ids, labels, params,
    /// actions, every command it runs (built-ins too), the host variables it
    /// reads with whether each is set on this Mac, and the machines it made.
    /// No variable's value is ever listed: an agent sees `‹NAME›` where one
    /// was pasted into a command, as GetProvidersConfig shows it.
    async fn list_providers(
        &self,
        request: Request<pb::ListProvidersRequest>,
    ) -> Result<Response<pb::ListProvidersResponse>, Status> {
        let caller = identity::caller(&request)?;
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        // No store means no variable is set and none can leak, so the list
        // still answers; a store that fails to read is an error.
        let variables = crate::host_vars::HostVars::installed().ok();
        let set: std::collections::BTreeSet<String> = match &variables {
            Some(variables) => variables.names().map_err(|error| error.status())?.into_iter().collect(),
            None => std::collections::BTreeSet::new(),
        };
        let scrubber = if matches!(caller, Caller::User(_)) {
            None
        } else if let Some(variables) = &variables {
            let values = variables.values().map_err(|error| {
                status::internal(
                    format!(
                        "couldn't read this Mac's host variables, so the providers' commands weren't shown to the agent (one could hold a value): {}",
                        status::chain(&error)
                    ),
                    "",
                )
            })?;
            Some(humata2_mirror::Scrubber::new(values))
        } else {
            None
        };
        let mut using = BTreeMap::<String, Vec<String>>::new();
        for entry in self.work.machines.list().map_err(|error| status_of(&error))? {
            if let Some(record) = entry.provider {
                using.entry(record.id).or_default().push(entry.name);
            }
        }
        let providers = self
            .providers()?
            .iter()
            .map(|provider| {
                let mut listed = provider_of(provider);
                listed.commands = provider
                    .commands()
                    .into_iter()
                    .map(|(action, command)| pb::ProviderCommand {
                        action: action.to_owned(),
                        command: scrubber
                            .as_ref()
                            .map_or_else(|| command.to_owned(), |scrubber| scrubber.scrub(command)),
                    })
                    .collect();
                let mut seen = std::collections::BTreeSet::new();
                listed.variables = provider
                    .env()
                    .iter()
                    .filter(|name| seen.insert(name.as_str()))
                    .map(|name| pb::ProviderVariable {
                        name: name.clone(),
                        set: set.contains(name),
                    })
                    .collect();
                listed.machines = using
                    .remove(provider.id())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|name| machine_id(&name))
                    .collect();
                listed
            })
            .collect();
        Ok(Response::new(pb::ListProvidersResponse { providers }))
    }

    /// The providers file as saved, for anyone but a peer: a read, so
    /// agents may make it too. The file names host variables but their
    /// values live only in the host variable store; should a value have
    /// been pasted into a command anyway, an agent sees `‹NAME›` in its
    /// place. Changing the file stays the user's (SetProvidersConfig,
    /// ImportProvider).
    async fn get_providers_config(
        &self,
        request: Request<pb::GetProvidersConfigRequest>,
    ) -> Result<Response<pb::ProvidersConfig>, Status> {
        let caller = identity::caller(&request)?;
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        let (mut toml, value) = current_revision(&self.work)?;
        if !matches!(caller, Caller::User(_)) {
            let values = crate::host_vars::HostVars::installed()
                .and_then(|variables| variables.values())
                .map_err(|error| {
                    status::internal(
                        format!(
                            "couldn't read this Mac's host variables, so the providers file wasn't shown to the agent (it could hold one of their values): {}",
                            status::chain(&error)
                        ),
                        "",
                    )
                })?;
            toml = humata2_mirror::Scrubber::new(values).scrub(&toml);
        }
        Ok(Response::new(pb::ProvidersConfig {
            toml,
            revision: Some(pb::Revision { value }),
        }))
    }

    /// Validates the text, checks it was written against the revision on
    /// disk, and replaces the file whole. Problems in the text come back
    /// in the response with nothing saved.
    async fn set_providers_config(
        &self,
        request: Request<pb::SetProvidersConfigRequest>,
    ) -> Result<Response<pb::SetProvidersConfigResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.require_settings(&caller, "changing the machine providers", SETTINGS_FIX)?;
        self.require_present(&caller, Capability::SettingsChange)?;
        let request = request.into_inner();
        let (_, current) = current_revision(&self.work)?;
        let expected = request.expected_revision.map_or(0, |revision| revision.value);
        if expected != current {
            return Err(status::failure(
                Code::FailedPrecondition,
                status::error(
                    "the providers changed since you opened them, so nothing was saved".to_owned(),
                    "read them again (GetProvidersConfig), merge your change, and save with the new revision",
                    pb::error::Kind::RevisionConflict(pb::RevisionConflict {
                        kind: pb::EntityKind::Provider as i32,
                        id: "providers".to_owned(),
                        expected: Some(pb::Revision { value: expected }),
                        actual: Some(pb::Revision { value: current }),
                    }),
                ),
            ));
        }
        if let Err(error) = Provider::parse(&request.toml) {
            return Ok(Response::new(pb::SetProvidersConfigResponse {
                problems: problems_of(&error),
                revision: Some(pb::Revision { value: current }),
            }));
        }
        let path = providers_path(self.work.machines.hosts_file());
        write_whole(&path, &request.toml).map_err(|source| {
            status::internal(
                format!(
                    "couldn't save the providers file {}: {source}. Free some disk space or fix the folder's permissions, then save again",
                    path.display()
                ),
                "",
            )
        })?;
        Ok(Response::new(pb::SetProvidersConfigResponse {
            problems: Vec::new(),
            revision: Some(pb::Revision {
                value: revision(&request.toml, true),
            }),
        }))
    }

    /// MP-23: the file's full text, its providers and every command they
    /// will run, and which ids it replaces. Nothing is saved; the token
    /// imports exactly this, once, while the providers file is unchanged.
    async fn preview_provider_import(
        &self,
        request: Request<pb::PreviewProviderImportRequest>,
    ) -> Result<Response<pb::ProviderImportPlan>, Status> {
        let caller = identity::caller(&request)?;
        self.require_settings(&caller, "reviewing a provider file to import", IMPORT_FIX)?;
        let request = request.into_inner();
        let text = String::from_utf8(request.file).map_err(|_| {
            status::invalid_field(
                "file",
                pb::invalid_field::Problem::BadFormat,
                format!("the provider file {} isn't UTF-8 text, so it can't be a providers TOML file", request.file_name),
                "pick the provider's .toml file, saved as UTF-8 text",
            )
        })?;
        let (current, revision) = current_revision(&self.work)?;
        let existing = self.providers()?;
        let plan = import::plan(&text, &current, &existing);
        let token = plan.merged.as_ref().map(|merged| {
            let token = format!("pl_{}", uuid::Uuid::new_v4().simple());
            let mut imports = lock(&self.imports);
            imports.retain(|_, planned| planned.made.elapsed() < import::PLAN_LIFETIME);
            imports.insert(
                token.clone(),
                import::Planned {
                    merged: merged.clone(),
                    base: revision,
                    made: std::time::Instant::now(),
                },
            );
            pb::PlanToken { value: token }
        });
        Ok(Response::new(pb::ProviderImportPlan {
            text,
            providers: plan.providers.iter().map(provider_of).collect(),
            commands: plan
                .providers
                .iter()
                .flat_map(|provider| {
                    provider.commands().into_iter().map(|(action, command)| pb::provider_import_plan::Command {
                        provider: Some(pb::ProviderId {
                            value: provider.id().to_owned(),
                        }),
                        action: action.to_owned(),
                        command: command.to_owned(),
                    })
                })
                .collect(),
            replaces: plan
                .replaces
                .into_iter()
                .map(|value| pb::ProviderId { value })
                .collect(),
            problems: plan.problems,
            token,
        }))
    }

    /// MP-23: adds what the plan showed, with the user at the Mac.
    async fn import_provider(
        &self,
        request: Request<pb::ImportProviderRequest>,
    ) -> Result<Response<pb::SetProvidersConfigResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.require_settings(&caller, "importing a machine provider", IMPORT_FIX)?;
        self.require_present(&caller, Capability::SettingsChange)?;
        let token = request
            .into_inner()
            .plan
            .map(|plan| plan.value)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| status::missing("plan"))?;
        let planned = lock(&self.imports)
            .remove(&token)
            .filter(|planned| planned.made.elapsed() < import::PLAN_LIFETIME)
            .ok_or_else(|| import::stale("the import plan is unknown, used or older than ten minutes"))?;
        let (_, current) = current_revision(&self.work)?;
        if current != planned.base {
            return Err(import::stale(
                "the providers changed since this import was previewed, so nothing was added",
            ));
        }
        let path = providers_path(self.work.machines.hosts_file());
        write_whole(&path, &planned.merged).map_err(|source| {
            status::internal(
                format!(
                    "couldn't save the providers file {}: {source}. Free some disk space or fix the folder's permissions, then import again",
                    path.display()
                ),
                "",
            )
        })?;
        Ok(Response::new(pb::SetProvidersConfigResponse {
            problems: Vec::new(),
            revision: Some(pb::Revision {
                value: revision(&planned.merged, true),
            }),
        }))
    }

    /// MP-23, D792: an agent asks the user to import a provider file. A
    /// file with problems is refused at once; otherwise the ask waits in
    /// the Inbox under PROVIDER_IMPORT (Ask me or Never) showing the file
    /// whole, and approving writes it while the providers file is as it
    /// was when the agent asked.
    async fn request_provider_import(
        &self,
        request: Request<pb::RequestProviderImportRequest>,
    ) -> Result<Response<pb::SetProvidersConfigResponse>, Status> {
        let caller = identity::caller(&request)?;
        match &caller {
            Caller::Peer => return Err(status::refused(&Refusal::PeerCannotCall)),
            Caller::User(_) | Caller::Script(_) => return Err(ask::not_for_the_user()),
            Caller::Agent(_) => {}
        }
        let request = request.into_inner();
        ask::check(&request).map_err(ask::AskError::into_status)?;
        let (current, base) = current_revision(&self.work)?;
        let plan = import::plan(&request.text, &current, &self.providers()?);
        let merged = ask::merged(&plan).map_err(ask::AskError::into_status)?;
        let call = Call::Use {
            capability: Capability::ProviderImport,
            target: Target::Global,
        };
        // Never allowed without asking: an approved ask is only the rerun.
        let asked = channel_service::gate(&self.guard, &caller, &call, false)?.ok_or_else(|| {
            status::internal(
                "an agent's provider import was let through without asking the user, so nothing was asked or added; the import rule offers only Ask me and Never",
                "",
            )
        })?;
        let card = crate::inbox::BoardAsk {
            preview: ask::preview(&request, &plan),
            reason: request.reason.clone(),
        };
        let key = request.idempotency_key.as_ref().map(|key| key.value.clone());
        let work = self.work.clone();
        answer_gated_showing(
            &self.inbox,
            &caller,
            key,
            "humata.v1.SetProvidersConfigResponse",
            Ok(Gated::Ask(asked)),
            Some(card),
            move || async move { write_asked(&work, &merged, base).map(Gated::Done) },
        )
        .await
    }

    /// PROVIDERS: takes an imported provider out of the providers file, with
    /// the user at the Mac. A built-in provider stays, and so does one that
    /// machines still use: those machines are named, to delete first.
    async fn remove_provider(
        &self,
        request: Request<pb::RemoveProviderRequest>,
    ) -> Result<Response<pb::SetProvidersConfigResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.require_settings(
            &caller,
            "removing a machine provider",
            "ask the user to remove it in Settings › Machines › Providers",
        )?;
        self.require_present(&caller, Capability::SettingsChange)?;
        let id = request
            .into_inner()
            .provider
            .map(|provider| provider.value)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| status::missing("provider"))?;
        let (_, configured) = read_providers(self.work.machines.hosts_file()).map_err(file_status)?;
        let Some(provider) = configured.iter().find(|provider| provider.id() == id) else {
            // Not in the file: a built-in one, or none at all.
            let provider = self.provider(&id)?;
            return Err(remove::built_in(provider.label()));
        };
        let mut using: Vec<String> = self
            .work
            .machines
            .list()
            .map_err(|error| status_of(&error))?
            .into_iter()
            .filter(|entry| entry.provider.as_ref().is_some_and(|record| record.id == id))
            .map(|entry| entry.name)
            .collect();
        using.sort();
        if !using.is_empty() {
            return Err(remove::in_use(provider.label(), &using));
        }
        let (text, _) = current_revision(&self.work)?;
        let removed = remove::without(&text, &id).map_err(|error| {
            status::internal(
                format!("couldn't take {id} out of the providers file, so nothing was removed: {error}"),
                "",
            )
        })?;
        let path = providers_path(self.work.machines.hosts_file());
        write_whole(&path, &removed).map_err(|source| {
            status::internal(
                format!(
                    "couldn't save the providers file {}: {source}. Free some disk space or fix the folder's permissions, then remove the provider again",
                    path.display()
                ),
                "",
            )
        })?;
        Ok(Response::new(pb::SetProvidersConfigResponse {
            problems: Vec::new(),
            revision: Some(pb::Revision {
                value: revision(&removed, true),
            }),
        }))
    }

    /// Checks the provider, the name and the params before anything runs,
    /// then answers with the running operation: create, identity (for an
    /// identity provider), connect, check, install, start, find_agents. An
    /// agent's create waits in the Inbox when its rule says Ask me (D38.6).
    async fn create_machine(
        &self,
        request: Request<pb::CreateMachineRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request.idempotency_key.as_ref().map(|key| key.value.clone());
        let first = self.create(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Operation",
            first,
            move || async move { me.create(&who, request, true).await },
        )
        .await
    }

    /// START, STOP, STATUS run the provider's command for a machine it
    /// made; DELETE removes the machine by its removal plan, then runs the
    /// provider's `delete` (MP-21). An agent's start, stop or status waits
    /// in the Inbox when its rule says Ask me (D38.6); delete never does.
    async fn run_provider_action(
        &self,
        request: Request<pb::RunProviderActionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request.idempotency_key.as_ref().map(|key| key.value.clone());
        let first = self.run_action(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Operation",
            first,
            move || async move { me.run_action(&who, request, true).await },
        )
        .await
    }

    /// Runs the provider's `delete` for what a failed CreateMachine made.
    async fn delete_what_was_made(
        &self,
        request: Request<pb::DeleteWhatWasMadeRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let id = request
            .operation
            .map(|operation| operation.value)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| status::missing("operation"))?;
        let (provider_id, name, values) = {
            let left = lock(&self.left);
            match left.get(&id).filter(|made| made.ended) {
                Some(made) => (made.provider.clone(), made.machine.clone(), made.values.clone()),
                None => {
                    return Err(status::not_found(
                        pb::EntityKind::Operation,
                        &id,
                        format!("operation {id} didn't leave anything made that Humata remembers"),
                        "only a Create machine that ran its create and then failed or was cancelled can be cleaned up this way, and only until humatad2 restarts; after a restart delete what the provider made with its own tool",
                    ));
                }
            }
        };
        self.require_machine(&caller, &name, MachineActionKind::Delete)?;
        self.require_present(&caller, MachineActionKind::Delete.capability())?;
        if let Some(operation) = self.operations.existing(&caller, request.idempotency_key.as_ref())? {
            return Ok(Response::new(operation));
        }
        let provider = self.provider(&provider_id)?;
        if !provider.has(Action::Delete) {
            return Err(no_command_to_run_by_hand(&provider_id, &name));
        }
        let progress = match self.start(
            &caller,
            pb::OperationKind::ProviderAction,
            &name,
            &OpStep::PROVIDER_ACTION,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Response::new(operation)),
        };
        let operation = progress.operation();
        let left = self.left.clone();
        let job: Job = Arc::new(move |machines, drive| {
            machines.delete_made(&provider, &name, &values, drive)?;
            lock(&left).remove(&id);
            Ok(None)
        });
        launch_observed(
            self.work.clone(),
            Arc::new(crate::wire_machines::ProgressAdapter(std::sync::Mutex::new(Some(progress)))),
            job,
            None,
        );
        Ok(Response::new(operation))
    }

    type InteractiveLoginStream = ReceiverStream<Result<pb::LoginServerMessage, Status>>;

    /// Built by the interactive login task, here.
    async fn interactive_login(
        &self,
        request: Request<tonic::Streaming<pb::LoginClientMessage>>,
    ) -> Result<Response<Self::InteractiveLoginStream>, Status> {
        self.login.interactive_login(request).await
    }

    /// Built by the interactive login task, here.
    async fn make_unattended(
        &self,
        request: Request<pb::MakeUnattendedRequest>,
    ) -> Result<Response<pb::Machine>, Status> {
        self.login.make_unattended(request).await
    }
}
