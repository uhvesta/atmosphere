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
//! a stale editor is refused. Agents see no providers and set none (MP-23).
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

use crate::channel_service::{self, Gated, answer_gated};
use crate::guard::{self, Guard};
use crate::identity::{self, Caller};
use crate::inbox::Inbox;
use crate::machine_service::{
    Job, Observer, ProvidersFileError, Work, launch_observed, lock, machine_id, machine_of, providers_path,
    read_providers, status_of,
};
use crate::operations::{Operations, Started};
use crate::status;

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
        launch_observed(self.work.clone(), progress, job, Some(observer));
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
        launch_observed(self.work.clone(), progress, job, None);
        Ok(Gated::Done(operation))
    }

    /// The user only (agents and scripts are refused), for the providers
    /// file: it is a list of commands Humata will run.
    fn require_settings(&self, caller: &Caller) -> Result<(), Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
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
                    "add it in Settings › Machine providers, or pick one of the listed providers",
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
                "fix the providers in Settings › Machine providers; the file on disk was left as it is",
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
                    "change that field in Settings › Machine providers, then save again",
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
                "remove {machine} with the provider's own tool; or add `delete` to the provider in Settings › Machine providers and try again"
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
    /// The user sees every provider; an agent sees none (MP-23).
    async fn list_providers(
        &self,
        request: Request<pb::ListProvidersRequest>,
    ) -> Result<Response<pb::ListProvidersResponse>, Status> {
        let providers = match identity::caller(&request)? {
            Caller::Peer => return Err(status::refused(&Refusal::PeerCannotCall)),
            Caller::Agent(_) | Caller::Script(_) => Vec::new(),
            Caller::User(_) => self.providers()?.iter().map(provider_of).collect(),
        };
        Ok(Response::new(pb::ListProvidersResponse { providers }))
    }

    async fn get_providers_config(
        &self,
        request: Request<pb::GetProvidersConfigRequest>,
    ) -> Result<Response<pb::ProvidersConfig>, Status> {
        let caller = identity::caller(&request)?;
        self.require_settings(&caller)?;
        let (toml, value) = current_revision(&self.work)?;
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
        self.require_settings(&caller)?;
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

    async fn preview_provider_import(
        &self,
        _request: Request<pb::PreviewProviderImportRequest>,
    ) -> Result<Response<pb::ProviderImportPlan>, Status> {
        Err(status::not_built("humata.v1.ProviderService.PreviewProviderImport"))
    }

    async fn import_provider(
        &self,
        _request: Request<pb::ImportProviderRequest>,
    ) -> Result<Response<pb::SetProvidersConfigResponse>, Status> {
        Err(status::not_built("humata.v1.ProviderService.ImportProvider"))
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
        launch_observed(self.work.clone(), progress, job, None);
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
