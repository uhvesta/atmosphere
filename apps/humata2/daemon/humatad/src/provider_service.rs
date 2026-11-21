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
use humata2_machines::{
    Action, ConfigError, LoginError, LoginEvent, LoginSession, LoginTerminal, MachineName, MachineStatus,
    ManageError, Observed, OpStep, Plan, Provider, Reach, SetupError, StepEvent, SystemRunner, TakeoverError,
    detect_prompt, existing_link, parse_status, provider_link, register,
};
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
mod on_machine;
mod remove;

pub(crate) use on_machine::OnMachine;

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
pub struct ProviderSvc {
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
        let provider_id = request
            .provider
            .map(|provider| provider.value)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| status::missing("provider"))?;
        let resolved_name = self
            .work
            .resolve_provider_name(&provider_id, &request.name)
            .map_err(|error| status_of(&error))?;
        if let Some(asked) = self.gate(caller, resolved_name.trim(), MachineActionKind::Create, approved)? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(operation) = self.operations.existing(caller, request.idempotency_key.as_ref())? {
            return Ok(Gated::Done(operation));
        }
        let provider = self.provider(&provider_id)?;
        let plan = self
            .work
            .machines
            .plan_create(&provider, &resolved_name, request.params.into_iter().collect())
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
        } else if action == Action::Stop {
            // Pause the machine's peer supervisor immediately so `supervise`
            // never auto-wakes or spams an intentionally stopped VM.
            work.mark_user_stopped_vm(&name);
        }
        let job: Job = Arc::new(move |machines, drive| {
            if action == Action::Delete {
                machines.delete_machine(&provider, &name, drive)?;
                return Ok(None);
            }
            if action == Action::Stop {
                match machines.run_action(&provider, &name, action, drive) {
                    Ok(_) => {
                        work.saw(&name, pb::Reachability::Stopped);
                        return Ok(None);
                    }
                    Err(error) => {
                        work.clear_user_stopped_vm(&name);
                        work.link_up(&name, &Handle::current());
                        return Err(error);
                    }
                }
            }
            let output = machines.run_action(&provider, &name, action, drive)?;
            match action {
                Action::Start | Action::Wake => {
                    post_start_or_wake(&work, &provider, &name);
                }
                Action::Status => {
                    if parse_status(&output.stdout) == MachineStatus::Stopped {
                        work.saw(&name, pb::Reachability::Stopped);
                    }
                }
                _ => {}
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

    /// MP-23, D792, D906: any local/remote caller (owner, agent, or script —
    /// never a peer) asks the owner to import a provider file. A file with
    /// problems is refused at once; otherwise the ask waits in the Inbox under
    /// PROVIDER_IMPORT (Ask me or Never) showing the file whole, and only the
    /// owner's approval writes it while the providers file is unchanged.
    async fn request_provider_import(
        &self,
        request: Request<pb::RequestProviderImportRequest>,
    ) -> Result<Response<pb::SetProvidersConfigResponse>, Status> {
        let caller = identity::caller(&request)?;
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
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
        let asked = match &caller {
            Caller::Agent(_) => channel_service::gate(&self.guard, &caller, &call, false)?
                .ok_or_else(|| {
                    status::internal(
                        "an agent's provider import was let through without asking the user, so nothing was asked or added; the import rule offers only Ask me and Never",
                        "",
                    )
                })?,
            Caller::User(_) | Caller::Script(_) => Box::new(AccessRequest {
                capability: Capability::ProviderImport,
                channel: humata2_access::ChannelId::new(humata2_sessions::DESK_CHANNEL),
                requester: humata2_access::SessionId::new(""),
                call,
                always_let: None,
                ahead: false,
                answer_at: humata2_access::AnswerAt::Host,
            }),
            Caller::Peer => unreachable!(),
        };
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

    /// Runs an interactive login or interactive provider `refresh`/`wake`
    /// command in a real PTY. For providers with `interactive = true` that
    /// declare `refresh` (or `wake` without `connect`), the PTY stays open
    /// until the command itself exits (supporting browser/URL auth flows).
    async fn interactive_login(
        &self,
        request: Request<tonic::Streaming<pb::LoginClientMessage>>,
    ) -> Result<Response<Self::InteractiveLoginStream>, Status> {
        self.run_interactive_login(request).await
    }

    /// Built by the interactive login task, here.
    async fn make_unattended(
        &self,
        request: Request<pb::MakeUnattendedRequest>,
    ) -> Result<Response<pb::Machine>, Status> {
        self.login.make_unattended(request).await
    }
}

/// Runs after `ProviderAction::Start` or `Wake` finishes: clears the
/// `UserStoppedVm` mark, refreshes dynamic reach metadata, repairs missing
/// binaries / missing enrollment keys (never overwriting conflicting keys),
/// and kicks `link_up` immediately so `PeerUplink` comes online without
/// waiting for backoff.
fn post_start_or_wake(work: &Arc<Work>, provider: &Provider, name: &str) {
    work.clear_user_stopped_vm(name);
    let runner = SystemRunner::new(&crate::host_vars::Installed);
    if !provider.interactive() {
        match work.machines.refresh_reach(provider, name, &runner) {
            Ok(_) => {}
            Err(ManageError::HostKeyMismatch { .. })
            | Err(ManageError::Step {
                source: SetupError::HostKeyChanged { .. },
                ..
            }) => {
                work.saw(name, pb::Reachability::HostKeyChanged);
                return;
            }
            Err(error) => {
                crate::log::say!(
                    "humatad2: refreshing reach for {name} after start/wake failed: {}",
                    status::chain(&error)
                );
            }
        }
        let mut ignore = |_: StepEvent| {};
        let mut drive = humata2_machines::Drive {
            runner: &runner,
            builds: &*work.builds,
            wait: &std::thread::sleep,
            events: &mut ignore,
            cancelled: &|| false,
        };
        match work.machines.reconnect(name, &mut drive) {
            Ok(_) => {
                work.saw(name, pb::Reachability::Online);
            }
            Err(ManageError::HostKeyMismatch { .. })
            | Err(ManageError::Step {
                source: SetupError::HostKeyChanged { .. },
                ..
            }) => {
                work.saw(name, pb::Reachability::HostKeyChanged);
                return;
            }
            Err(ManageError::Step {
                source: SetupError::EnrollmentKeyConflict { .. },
                ..
            }) => {
                work.saw(name, pb::Reachability::IdentityMismatch);
                return;
            }
            Err(error) => {
                crate::log::say!(
                    "humatad2: ensuring {name} ready after start/wake failed: {}",
                    status::chain(&error)
                );
            }
        }
    }
    work.link_up(name, &Handle::current());
}

/// Interactive PTY execution for provider `refresh` / `wake` commands and
/// shell-takeover interactive logins.
static PROVIDER_ONE_SHOTS: std::sync::LazyLock<Mutex<BTreeMap<String, crate::peer_client::Pipe>>> =
    std::sync::LazyLock::new(|| Mutex::new(BTreeMap::new()));

/// Hands out a one-shot peer pipe created by [`ProviderSvc::interactive_login`].
pub(crate) fn one_shot(machine: &str) -> Option<Box<dyn crate::peer_client::Dial>> {
    let pipe = lock(&PROVIDER_ONE_SHOTS).remove(machine)?;
    Some(Box::new(ProviderOneShot {
        machine: machine.to_owned(),
        pipe: Mutex::new(Some(pipe)),
    }))
}

struct ProviderOneShot {
    machine: String,
    pipe: Mutex<Option<crate::peer_client::Pipe>>,
}

impl crate::peer_client::Dial for ProviderOneShot {
    fn dial(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<crate::peer_client::Pipe, crate::peer_client::LinkError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            lock(&self.pipe).take().ok_or_else(|| crate::peer_client::LinkError::Ended {
                machine: self.machine.clone(),
            })
        })
    }
}

fn pty_size(value: u32, default: u16) -> u16 {
    u16::try_from(value).ok().filter(|size| *size > 0).unwrap_or(default)
}

fn prompt_kind_pb(kind: humata2_machines::PromptKind) -> pb::LoginPromptKind {
    use humata2_machines::PromptKind;
    match kind {
        PromptKind::Password => pb::LoginPromptKind::Password,
        PromptKind::Passphrase => pb::LoginPromptKind::Passphrase,
        PromptKind::Code => pb::LoginPromptKind::Code,
        PromptKind::Confirm => pb::LoginPromptKind::Confirm,
        PromptKind::Device => pb::LoginPromptKind::Device,
    }
}

/// Selects the interactive provider command (`refresh`, or `wake` when there
/// is no `connect` command) to execute in a PTY until it exits.
fn interactive_provider_action(provider: &Provider) -> Option<Action> {
    if !provider.interactive() {
        return None;
    }
    if provider.has(Action::Refresh) {
        Some(Action::Refresh)
    } else if !provider.has_connect() && provider.has(Action::Wake) {
        Some(Action::Wake)
    } else {
        None
    }
}

impl ProviderSvc {
    fn require_login(&self, caller: &Caller, machine: &str) -> Result<(), Status> {
        let action = MachineActionKind::Reconnect;
        if !matches!(caller, Caller::User(_)) {
            return Err(status::refused(&Refusal::UserOnly {
                capability: Some(action.capability()),
            }));
        }
        self.require_machine(caller, machine, action)?;
        self.require_present(caller, action.capability())
    }

    async fn run_interactive_login(
        &self,
        request: Request<tonic::Streaming<pb::LoginClientMessage>>,
    ) -> Result<Response<ReceiverStream<Result<pb::LoginServerMessage, Status>>>, Status> {
        use pb::login_client_message::Message;
        let caller = identity::caller(&request)?;
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::UserOnly {
                capability: Some(Capability::MachineProvision),
            }));
        }
        if !matches!(caller, Caller::User(_)) {
            return Err(status::user_only_call(
                Capability::MachineProvision,
                "logging in to a machine by hand",
                "ask the user with inbox_ask to log in from the app (Hosts › the machine › Log in), then retry what you wanted",
            ));
        }
        let mut inbound = request.into_inner();
        let open = match inbound.message().await?.and_then(|message| message.message) {
            Some(Message::Open(open)) => open,
            _ => return Err(status::missing("open")),
        };
        let cols = pty_size(open.cols, 80);
        let rows = pty_size(open.rows, 24);

        // If targeting an existing machine whose interactive provider declares
        // `refresh` (or `wake` without `connect`), run that command in a real
        // PTY and keep the terminal open until the command itself exits.
        if let Some(pb::login_open::Target::Machine(ref id)) = open.target {
            let name = machine_of(Some(id.clone()))?;
            self.require_login(&caller, &name)?;
            if let Ok(entry) = self.work.machines.get(&name)
                && let Some(record) = entry.provider
                && let Some(provider) = crate::wire_machines::find_provider(self.work.machines.hosts_file(), &record.id)
                && let Some(action) = interactive_provider_action(&provider)
            {
                let machine_name = MachineName::new(&name).map_err(|source| {
                    status::invalid_field(
                        "machine",
                        pb::invalid_field::Problem::BadFormat,
                        format!("{name:?} isn't a machine name: {source}"),
                        "list the machines to see their names",
                    )
                })?;
                let invocation = provider
                    .invocation(action, &machine_name, &record.params)
                    .map_err(|source| status_of(&ManageError::Params { source }))?
                    .ok_or_else(|| {
                        status_of(&ManageError::ActionUndefined {
                            provider: provider.id().to_owned(),
                            action,
                        })
                    })?;
                return self
                    .spawn_interactive_pty_command(name, provider, action, invocation, cols, rows, inbound)
                    .await;
            }
        }

        // Otherwise, run the interactive shell-takeover login flow.
        self.spawn_interactive_shell_login(&caller, open, cols, rows, inbound)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn spawn_interactive_pty_command(
        &self,
        name: String,
        provider: Provider,
        action: Action,
        invocation: humata2_machines::Invocation,
        cols: u16,
        rows: u16,
        mut inbound: tonic::Streaming<pb::LoginClientMessage>,
    ) -> Result<Response<ReceiverStream<Result<pb::LoginServerMessage, Status>>>, Status> {
        use pb::login_client_message::Message;
        let (events_tx, events_rx) = std::sync::mpsc::channel();
        let terminal = LoginTerminal::spawn(
            &invocation,
            &crate::host_vars::Installed,
            cols,
            rows,
            events_tx.clone(),
        )
        .map_err(|error| login_error_status(&error, Some(provider.id())))?;

        let (out, receiver) = tokio::sync::mpsc::channel(64);
        let runtime = Handle::current();

        // Forward client input, resize, and cancel to the PTY; keep the PTY
        // open until the command itself exits (LoginDone is ignored).
        tokio::spawn(async move {
            loop {
                let message = match inbound.message().await {
                    Ok(Some(message)) => message.message,
                    _ => {
                        let _ = events_tx.send(LoginEvent::Cancel);
                        return;
                    }
                };
                let alive = match message {
                    Some(Message::Input(bytes)) => events_tx.send(LoginEvent::Input(bytes.to_vec())).is_ok(),
                    Some(Message::Resize(resize)) => events_tx
                        .send(LoginEvent::Resize {
                            cols: pty_size(resize.cols, 80),
                            rows: pty_size(resize.rows, 24),
                        })
                        .is_ok(),
                    Some(Message::Cancel(_)) => {
                        let _ = events_tx.send(LoginEvent::Cancel);
                        return;
                    }
                    Some(Message::Done(_)) | Some(Message::Open(_)) | None => true,
                };
                if !alive {
                    return;
                }
            }
        });

        let work = self.work.clone();
        std::thread::spawn(move || {
            run_interactive_pty_command(terminal, events_rx, &name, &provider, action, &work, &out, &runtime);
        });
        Ok(Response::new(ReceiverStream::new(receiver)))
    }

    async fn spawn_interactive_shell_login(
        &self,
        caller: &Caller,
        open: pb::LoginOpen,
        cols: u16,
        rows: u16,
        mut inbound: tonic::Streaming<pb::LoginClientMessage>,
    ) -> Result<Response<ReceiverStream<Result<pb::LoginServerMessage, Status>>>, Status> {
        use pb::login_client_message::Message;
        let bad_name = |machine: &str, source: humata2_machines::NameError| {
            status::invalid_field(
                "machine",
                pb::invalid_field::Problem::BadFormat,
                format!("{machine:?} isn't a machine name: {source}"),
                "list the machines to see their names",
            )
        };
        let (machine, link, made, operation) = match &open.target {
            Some(pb::login_open::Target::Machine(id)) => {
                let name = machine_of(Some(id.clone()))?;
                self.require_login(caller, &name)?;
                let machine = MachineName::new(&name).map_err(|source| bad_name(&name, source))?;
                let link = existing_link(&self.work.machines, &name)
                    .map_err(|error| login_error_status(&error, None))?;
                (machine, link, None, None)
            }
            Some(pb::login_open::Target::Operation(id)) => {
                let operation = id.value.clone();
                let left_made = lock(&self.left)
                    .get(&operation)
                    .filter(|made| made.ended)
                    .map(|made| (made.provider.clone(), made.machine.clone(), made.values.clone()))
                    .ok_or_else(|| {
                        status::not_found(
                            pb::EntityKind::Operation,
                            &operation,
                            format!("operation {operation} left no machine waiting for a login"),
                            "start Create machine again; a login is offered when its connect step asks for one",
                        )
                    })?;
                self.require_login(caller, &left_made.1)?;
                let provider = self.provider(&left_made.0)?;
                let machine = MachineName::new(&left_made.1).map_err(|source| bad_name(&left_made.1, source))?;
                if self.work.machines.get(&left_made.1).is_ok() {
                    return Err(status::failure(
                        Code::FailedPrecondition,
                        status::error(
                            format!("{} is already listed, so the create it came from can't be finished", left_made.1),
                            "log in from the machine's own page instead",
                            pb::error::Kind::NameTaken(pb::NameTaken {
                                kind: pb::EntityKind::Machine as i32,
                                name: left_made.1.clone(),
                                suggestion: format!("{}-2", left_made.1),
                            }),
                        ),
                    ));
                }
                let link = provider_link(&provider, &machine, &left_made.2)
                    .map_err(|error| login_error_status(&error, Some(&left_made.0)))?;
                (machine, link, Some((provider, left_made.2)), Some(operation))
            }
            None => return Err(status::missing("target")),
        };
        let progress = match &operation {
            Some(id) => Some(self.operations.progress_of(caller, id)?),
            None => None,
        };
        let invocation = link
            .interactive()
            .map_err(|source| login_error_status(&LoginError::Quote(source), None))?;
        let provider_id = made.as_ref().map(|(provider, _)| provider.id().to_owned());
        let (session, handle) = LoginSession::open(
            machine.clone(),
            &invocation,
            &crate::host_vars::Installed,
            cols,
            rows,
        )
        .map_err(|error| login_error_status(&error, provider_id.as_deref()))?;

        let (out, receiver) = tokio::sync::mpsc::channel(64);
        let runtime = Handle::current();

        tokio::spawn(async move {
            loop {
                let message = match inbound.message().await {
                    Ok(Some(message)) => message.message,
                    _ => {
                        handle.cancel();
                        return;
                    }
                };
                let alive = match message {
                    Some(Message::Input(bytes)) => handle.input(bytes.to_vec()),
                    Some(Message::Resize(resize)) => {
                        handle.resize(pty_size(resize.cols, 80), pty_size(resize.rows, 24))
                    }
                    Some(Message::Done(_)) => handle.done(),
                    Some(Message::Cancel(_)) => {
                        handle.cancel();
                        return;
                    }
                    Some(Message::Open(_)) | None => true,
                };
                if !alive {
                    return;
                }
            }
        });

        let work = self.work.clone();
        let left = self.left.clone();
        std::thread::spawn(move || {
            run_shell_login(session, machine, link, made, progress, provider_id, &work, &left, &out, &runtime);
        });
        Ok(Response::new(ReceiverStream::new(receiver)))
    }
}

/// Drives an interactive provider command (`refresh` or `wake`) in a PTY until
/// the command itself exits. Streams output and prompt detections to `out`,
/// and on exit 0 clears `user_stopped_vm`, kicks `link_up`, and emits `Ready`.
#[allow(clippy::too_many_arguments)]
fn run_interactive_pty_command(
    mut terminal: LoginTerminal,
    events: std::sync::mpsc::Receiver<LoginEvent>,
    name: &str,
    provider: &Provider,
    action: Action,
    work: &Arc<Work>,
    out: &tokio::sync::mpsc::Sender<Result<pb::LoginServerMessage, Status>>,
    runtime: &Handle,
) {
    use pb::login_server_message::Message;
    let send = |message: Message| {
        out.blocking_send(Ok(pb::LoginServerMessage {
            message: Some(message),
            ..Default::default()
        }))
        .is_ok()
    };
    let mut kept: Vec<u8> = Vec::new();
    let mut last_prompt: Option<String> = None;
    let mut exit_result: Option<humata2_machines::Exit> = None;
    while let Ok(event) = events.recv() {
        match event {
            LoginEvent::Output(bytes) => {
                kept.extend_from_slice(&bytes);
                if kept.len() > 4096 {
                    let drop_n = kept.len() - 4096;
                    kept.drain(..drop_n);
                }
                if !send(Message::Output(bytes.into())) {
                    return;
                }
                if let Some(prompt) = detect_prompt(&kept)
                    && last_prompt.as_deref() != Some(&prompt.line)
                {
                    last_prompt = Some(prompt.line.clone());
                    if out
                        .blocking_send(Ok(pb::LoginServerMessage {
                            message: Some(Message::PromptSeen(prompt.line)),
                            prompt_kind: prompt_kind_pb(prompt.kind) as i32,
                        }))
                        .is_err()
                    {
                        return;
                    }
                }
            }
            LoginEvent::Input(bytes) => {
                let _ = terminal.write(&bytes);
            }
            LoginEvent::Resize { cols, rows } => {
                let _ = terminal.resize(cols, rows);
            }
            LoginEvent::Done => {
                // Keep the terminal open until the command itself exits.
            }
            LoginEvent::Cancel => {
                drop(terminal);
                return;
            }
            LoginEvent::Exited(exit) => {
                exit_result = Some(exit);
                // Drain any final output that arrived just before exit.
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
                while let Some(rem) = deadline.checked_duration_since(std::time::Instant::now()) {
                    match events.recv_timeout(rem) {
                        Ok(LoginEvent::Output(bytes)) => {
                            kept.extend_from_slice(&bytes);
                            if kept.len() > 4096 {
                                let drop_n = kept.len() - 4096;
                                kept.drain(..drop_n);
                            }
                            let _ = send(Message::Output(bytes.into()));
                        }
                        Ok(LoginEvent::Eof) | Err(_) => break,
                        _ => {}
                    }
                }
                break;
            }
            LoginEvent::Eof => {}
        }
    }
    drop(terminal);
    match exit_result {
        Some(exit) if exit.success() => {
            work.clear_user_stopped_vm(name);
            // If `wake` ran interactively, refresh reach metadata before
            // kicking `link_up`; if `refresh` ran interactively, avoid
            // re-running `refresh` non-interactively and kick `link_up`.
            if action == Action::Wake && !provider.has(Action::Refresh) {
                let runner = SystemRunner::new(&crate::host_vars::Installed);
                let _ = work.machines.refresh_reach(provider, name, &runner);
            }
            work.link_up(name, runtime);
            work.publish_machines();
            let _ = send(Message::Ready(pb::LoginReady {
                machine: Some(machine_id(name)),
                can_make_unattended: false,
            }));
        }
        other => {
            let exit_code = match other {
                Some(humata2_machines::Exit::Code(code)) => code,
                _ => -1,
            };
            let output_tail = String::from_utf8_lossy(&kept).trim().to_owned();
            let _ = send(Message::Failed(pb::Error {
                message: format!(
                    "the provider's `{action}` command for {name} exited with status {exit_code}"
                ),
                fix: "check the terminal output above and try again".to_owned(),
                retry: None,
                kind: Some(pb::error::Kind::ProviderStepFailed(pb::ProviderStepFailed {
                    provider: Some(pb::ProviderId {
                        value: provider.id().to_owned(),
                    }),
                    step: pb::provider_step_failed::Step::Connect as i32,
                    exit_code,
                    output_tail,
                })),
            }));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_shell_login(
    session: LoginSession,
    machine: MachineName,
    link: humata2_machines::Link,
    made: Option<(Provider, BTreeMap<String, String>)>,
    progress: Option<crate::operations::Progress>,
    provider_id: Option<String>,
    work: &Arc<Work>,
    left: &Arc<Mutex<HashMap<String, Left>>>,
    out: &tokio::sync::mpsc::Sender<Result<pb::LoginServerMessage, Status>>,
    runtime: &Handle,
) {
    use pb::login_server_message::Message;
    let send = |message: Message| {
        out.blocking_send(Ok(pb::LoginServerMessage {
            message: Some(message),
            ..Default::default()
        }))
        .is_ok()
    };
    let name = machine.as_str().to_owned();
    let keys = work.machines.peer_key(&name);
    let enroll_into: Option<std::path::PathBuf> = (!keys.exists())
        .then(|| keys.parent().map(std::path::Path::to_path_buf))
        .flatten();
    let builds: &(dyn humata2_machines::Builds + Send + Sync) = &*work.builds;
    let plan = Plan {
        builds,
        enroll_into: enroll_into.as_deref(),
        shell_timeout: None,
        sync_timeout: None,
    };
    let result = session.run(&plan, &mut |observed| match observed {
        Observed::Output(bytes) => send(Message::Output(bytes.into())),
        Observed::Prompt(prompt) => out
            .blocking_send(Ok(pb::LoginServerMessage {
                message: Some(Message::PromptSeen(prompt.line)),
                prompt_kind: prompt_kind_pb(prompt.kind) as i32,
            }))
            .is_ok(),
    });
    let active_link = match result {
        Ok(active) => active,
        Err(LoginError::Cancelled { .. }) => return,
        Err(error) => {
            send(Message::Failed(login_error_pb(&error, provider_id.as_deref())));
            return;
        }
    };
    let platform = active_link.platform();
    let enrolled = active_link.enrolled();
    let made_ref = made.as_ref().map(|(provider, values)| (provider, values));
    if let Err(error) = register(&work.machines, &machine, made_ref, &link, platform) {
        drop(active_link);
        if made.is_some() {
            let _ = humata2_machines::unregister(&work.machines, &machine, enrolled);
        }
        send(Message::Failed(login_error_pb(&error, provider_id.as_deref())));
        return;
    }
    let (reader, writer) = active_link.split();
    let pipe = bridge_login_pipe(reader, writer, runtime);
    if enroll_into.is_some() {
        work.forget_journal_now(&name);
    }
    work.clear_user_stopped_vm(&name);
    lock(&PROVIDER_ONE_SHOTS).insert(name.clone(), pipe);
    work.link_up(&name, runtime);
    work.publish_machines();
    if let Some(progress) = progress {
        for step in [OpStep::Connect, OpStep::Check, OpStep::Install, OpStep::Start] {
            progress.step(&StepEvent::Done(step));
        }
        progress.step(&StepEvent::Skipped(OpStep::FindAgents));
        let operation = progress.operation().id.map(|id| id.value).unwrap_or_default();
        progress.succeed(Some(pb::operation::Result::Machine(machine_id(&name))));
        lock(left).remove(&operation);
    }
    send(Message::Ready(pb::LoginReady {
        machine: Some(machine_id(&name)),
        can_make_unattended: true,
    }));
}

fn bridge_login_pipe(
    mut reader: humata2_machines::LinkReader,
    mut writer: humata2_machines::LinkWriter,
    runtime: &Handle,
) -> crate::peer_client::Pipe {
    use std::io::Read as _;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let (near, far) = tokio::io::duplex(64 * 1024);
    let (mut from_host, mut to_host) = tokio::io::split(far);
    let machine_to_host = runtime.clone();
    std::thread::spawn(move || {
        let mut buffer = vec![0_u8; 16 * 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if machine_to_host.block_on(to_host.write_all(&buffer[..n])).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = machine_to_host.block_on(to_host.shutdown());
    });
    let host_to_machine = runtime.clone();
    std::thread::spawn(move || {
        let mut buffer = vec![0_u8; 16 * 1024];
        loop {
            match host_to_machine.block_on(from_host.read(&mut buffer)) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if writer.write_all(&buffer[..n]).and_then(|()| writer.flush()).is_err() {
                        break;
                    }
                }
            }
        }
        writer.close();
    });
    Box::new(near)
}

fn login_error_pb(error: &LoginError, provider: Option<&str>) -> pb::Error {
    let message = status::chain(error);
    let (fix, exit_code, output_tail) = match error {
        LoginError::Takeover(TakeoverError::NoShell { output_tail, .. }) => (
            "this login ends in a menu or a program, not a shell (MP-18): give the provider an `identity` command that prints SSH details instead, or point `connect` at a wrapper script that ends in a POSIX shell; then log in again",
            -1,
            output_tail.clone(),
        ),
        LoginError::Takeover(
            TakeoverError::SyncTimedOut { output_tail, .. }
            | TakeoverError::TerminalEnded { output_tail, .. },
        ) => (
            "check that humatad2 starts on the machine (the output says what it printed), then log in again",
            -1,
            output_tail.clone(),
        ),
        LoginError::Takeover(
            TakeoverError::Install { output_tail, .. } | TakeoverError::Bundle { output_tail, .. },
        ) => (
            "check that ~/.humata2 on the machine is writable and has room (the output says what the install printed), then log in again",
            -1,
            output_tail.clone(),
        ),
        LoginError::Takeover(_) => (
            "log in again; press Done only when you see the machine's shell prompt",
            -1,
            String::new(),
        ),
        LoginError::Ended { exit, output_tail, .. } => (
            "log in again and press Done only when you see a shell prompt",
            match exit {
                Some(humata2_machines::Exit::Code(code)) => *code,
                _ => -1,
            },
            output_tail.clone(),
        ),
        LoginError::Enrollment { output_tail, .. } => (
            "make sure the machine's home folder is writable, then log in again",
            -1,
            output_tail.clone(),
        ),
        LoginError::Stalled { output_tail, .. } => (
            "log in again and press Done only when you see a POSIX shell prompt (sh, bash, zsh); a shell that doesn't read typed commands can't be taken over",
            -1,
            output_tail.clone(),
        ),
        LoginError::StillNeedsLogin { output_tail, .. } | LoginError::Authorize { output_tail, .. } => (
            "log in again if the connect asks for one, then make the machine unattended again; the machine was left as it was",
            -1,
            output_tail.clone(),
        ),
        _ => (
            "fix what the message names, then try again; nothing was changed",
            -1,
            String::new(),
        ),
    };
    pb::Error {
        message,
        fix: fix.to_owned(),
        retry: None,
        kind: Some(pb::error::Kind::ProviderStepFailed(pb::ProviderStepFailed {
            provider: provider.map(|id| pb::ProviderId { value: id.to_owned() }),
            step: pb::provider_step_failed::Step::Connect as i32,
            exit_code,
            output_tail,
        })),
    }
}

fn login_error_status(error: &LoginError, provider: Option<&str>) -> Status {
    if let LoginError::Manage { source, .. } = error {
        return status_of(source);
    }
    let code = match error {
        LoginError::NotInteractive { .. }
        | LoginError::NoSshAddress { .. }
        | LoginError::HostKeyNotPinned { .. }
        | LoginError::IdentityProvider { .. }
        | LoginError::NoLink { .. } => Code::FailedPrecondition,
        _ => Code::Aborted,
    };
    status::failure(code, login_error_pb(error, provider))
}

#[cfg(test)]
mod interactive_pty_tests {
    use super::*;

    #[test]
    fn interactive_provider_action_prefers_refresh_then_wake_without_connect() {
        let with_refresh = Provider::parse(
            r#"
[providers.authbox]
label = "Auth Box"
interactive = true
identity = "echo '{}'"
refresh = "echo refresh"
wake = "echo wake"
"#,
        )
        .unwrap()
        .pop()
        .unwrap();
        assert_eq!(interactive_provider_action(&with_refresh), Some(Action::Refresh));

        let wake_only = Provider::parse(
            r#"
[providers.wakebox]
label = "Wake Box"
interactive = true
identity = "echo '{}'"
wake = "echo wake"
"#,
        )
        .unwrap()
        .pop()
        .unwrap();
        assert_eq!(interactive_provider_action(&wake_only), Some(Action::Wake));

        let non_interactive = Provider::parse(
            r#"
[providers.autobox]
label = "Auto Box"
interactive = false
identity = "echo '{}'"
refresh = "echo refresh"
"#,
        )
        .unwrap()
        .pop()
        .unwrap();
        assert_eq!(interactive_provider_action(&non_interactive), None);

        let connect_only = Provider::parse(
            r#"
[providers.shellbox]
label = "Shell Box"
interactive = true
connect = "echo {cmd}"
"#,
        )
        .unwrap()
        .pop()
        .unwrap();
        assert_eq!(interactive_provider_action(&connect_only), None);
    }

    #[tokio::test]
    async fn interactive_pty_command_stays_open_until_exit_and_clears_user_stopped_vm() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = dir.path().join("hosts.json");
        std::fs::write(
            &hosts,
            r#"{"version":1,"machines":[{"name":"vm1","ssh_host":"via authbox","root":"~/Humata","state":"ready","provider":{"id":"authbox","params":{}}}]}"#,
        )
        .unwrap();
        let machines = humata2_machines::Machines::open(humata2_machines::MachinesConfig {
            hosts_file: hosts,
            known_hosts: dir.path().join("known_hosts"),
            ssh_config: dir.path().join("ssh_config"),
            peer_keys: dir.path().join("peer-keys"),
            retry: humata2_machines::RetryPolicy::default(),
        });
        let builds = Box::new(humata2_machines::Bundle::beside(
            dir.path(),
            humata2_machines::Platform::host().unwrap(),
        ));
        let work = crate::wire_machines::new_work(
            machines,
            builds,
            Arc::new(crate::events::Feed::default()),
            crate::host_relay::Links::default(),
            crate::peer_reports::PeerReports::default(),
            Arc::new(std::time::SystemTime::now),
        );
        work.mark_user_stopped_vm("vm1");
        assert!(work.is_user_stopped_vm("vm1"));

        let provider = Provider::parse(
            r#"
[providers.authbox]
label = "Auth Box"
interactive = true
identity = "echo '{}'"
refresh = "echo 'Open https://example.invalid/auth'; sleep 0.05; echo 'auth ok'"
"#,
        )
        .unwrap()
        .pop()
        .unwrap();
        let machine = MachineName::new("vm1").unwrap();
        let invocation = provider
            .invocation(Action::Refresh, &machine, &BTreeMap::new())
            .unwrap()
            .unwrap();

        let (events_tx, events_rx) = std::sync::mpsc::channel();
        let terminal = LoginTerminal::spawn(
            &invocation,
            &crate::host_vars::Installed,
            80,
            24,
            events_tx.clone(),
        )
        .unwrap();
        // Sending LoginDone must NOT cut short the command; it stays open until exit.
        events_tx.send(LoginEvent::Done).unwrap();

        let (out_tx, mut out_rx) = tokio::sync::mpsc::channel(64);
        let runtime = Handle::current();
        let work_for_thread = work.clone();
        std::thread::spawn(move || {
            run_interactive_pty_command(
                terminal,
                events_rx,
                "vm1",
                &provider,
                Action::Refresh,
                &work_for_thread,
                &out_tx,
                &runtime,
            );
        });

        let mut saw_output = String::new();
        let mut saw_ready = false;
        while let Some(msg) = out_rx.recv().await {
            let msg = msg.unwrap();
            match msg.message {
                Some(pb::login_server_message::Message::Output(bytes)) => {
                    saw_output.push_str(&String::from_utf8_lossy(&bytes));
                }
                Some(pb::login_server_message::Message::Ready(ready)) => {
                    assert_eq!(ready.machine.map(|m| m.value), Some("vm1".to_owned()));
                    saw_ready = true;
                }
                Some(pb::login_server_message::Message::Failed(err)) => {
                    panic!("unexpected failure: {err:?}");
                }
                _ => {}
            }
        }
        assert!(saw_output.contains("auth ok"), "output was: {saw_output:?}");
        assert!(saw_ready, "expected Ready after exit 0");
        assert!(!work.is_user_stopped_vm("vm1"), "user_stopped_vm should be cleared");
    }
}
