//! Machines, providers, skills, port bridges, settings, agents, marks and
//! bug reports (CLI.md §5.6 and §5.7).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use humata2_proto::humata::v1 as pb;
use prost::Message;
use serde_json::{Value as Json, json};

use super::{pages, parse, reply, secret, settings, streams, values};
use crate::args::{
    AgentFields, AgentsCommand, BridgeCommand, HostCommand, PinnedCommand, ProviderCommand,
    ProviderConfigCommand, ReportCommand, SettingsCommand, SkillsCommand, VarCommand,
};
use crate::call::{Ctx, Done, Outcome};
use crate::console::Console;
use crate::daemon::{Daemon, DaemonError, ReportRequest, Rpc, rpc};
use crate::error::CliError;

/// Where a machine keeps Humata's folders until it is changed (PL-1).
const DEFAULT_ROOT: &str = "~/Humata";

/// `listed` (a ListMachines reply) with this machine in it, first, unless
/// the daemon listed it already (D672 (Provisional)).
fn with_this_machine(mut listed: Json, this: &pb::MachineId) -> Json {
    let machine = crate::as_json::typed(
        "humata.v1.Machine",
        &pb::Machine {
            id: Some(this.clone()),
            name: "This Mac".to_owned(),
            is_this_mac: true,
            reachability: pb::Reachability::Online as i32,
            ..Default::default()
        },
    );
    if !listed.is_object() {
        listed = json!({});
    }
    let Some(fields) = listed.as_object_mut() else {
        return listed;
    };
    let machines = fields.entry("machines").or_insert_with(|| json!([]));
    if let Some(list) = machines.as_array_mut() {
        // An id prints as its bare value (as_json folds one-value messages).
        let id_of = |entry: &Json| entry.get("id").and_then(Json::as_str).map(str::to_owned);
        // A machine's agent gets the host's list, where the host is This
        // Mac already (D768); its own machine is listed under its name.
        let named = |entry: &Json| {
            id_of(entry).as_deref() == Some(this.value.as_str()) || entry.get("is_this_mac") == Some(&Json::Bool(true))
        };
        if !list.iter().any(named) {
            list.insert(0, machine);
        }
    }
    listed
}

pub(super) async fn host<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: HostCommand,
) -> Result<Done, CliError> {
    let key = ctx.key();
    match command {
        HostCommand::Ls => {
            let listed = pages(ctx, rpc::machine::LIST_MACHINES, "machines", |page| {
                pb::ListMachinesRequest { page }
            })
            .await?;
            // D672 (Provisional): the daemon lists the machines it was
            // given, never the one it runs on, so a single machine saw an
            // empty list and no id to start a session on.
            let this = ctx.this_machine().await?;
            Ok(Done::reply(with_this_machine(listed, &this)))
        }
        HostCommand::Show { machine } => {
            let request = pb::GetMachineRequest {
                id: Some(ctx.machine(&machine).await?),
            };
            reply(ctx, rpc::machine::GET_MACHINE, &request).await
        }
        HostCommand::Add {
            ssh_host,
            name,
            root,
            trust_host_key,
        } => {
            let request = pb::AddSshMachineRequest {
                idempotency_key: key,
                ssh_host,
                name: name.unwrap_or_default(),
                root: root.unwrap_or_default(),
                // Without it a first add stops at the host's key and names it.
                trust_host_key_fingerprint: trust_host_key.unwrap_or_default(),
            };
            if ctx.flags.dry_run {
                return ctx.change(rpc::machine::ADD_SSH_MACHINE, &request).await;
            }
            let started = ctx.raw(rpc::machine::ADD_SSH_MACHINE, &request).await?;
            follow(ctx, rpc::machine::ADD_SSH_MACHINE, &started)
                .await
                .map(Done::reply)
        }
        HostCommand::Reconnect { machine } => {
            let request = pb::ReconnectMachineRequest {
                id: Some(ctx.machine(&machine).await?),
            };
            ctx.change(rpc::machine::RECONNECT_MACHINE, &request).await
        }
        HostCommand::Remove { machine } => {
            let id = Some(ctx.machine(&machine).await?);
            ctx.planned(
                rpc::machine::PLAN_REMOVE_MACHINE,
                &pb::PlanRemoveMachineRequest { id: id.clone() },
                |plan: &pb::RemoveMachinePlan| plan.token.clone(),
                move |plan| async move {
                    let started = ctx
                        .present(rpc::machine::REMOVE_MACHINE, |presence| {
                            pb::RemoveMachineRequest {
                                idempotency_key: key.clone(),
                                id: id.clone(),
                                dispositions: Vec::new(),
                                plan: plan.clone(),
                                presence,
                            }
                        })
                        .await?;
                    follow(ctx, rpc::machine::REMOVE_MACHINE, &started).await
                },
            )
            .await
        }
        HostCommand::Forget { machine } => {
            let id = Some(ctx.machine(&machine).await?);
            ctx.planned(
                rpc::machine::PLAN_FORGET_MACHINE,
                &pb::PlanForgetMachineRequest { id: id.clone() },
                |plan: &pb::ForgetMachinePlan| plan.token.clone(),
                move |plan| async move {
                    ctx.present_json(rpc::machine::FORGET_MACHINE, |presence| {
                        pb::ForgetMachineRequest {
                            idempotency_key: key.clone(),
                            id: id.clone(),
                            plan: plan.clone(),
                            presence,
                        }
                    })
                    .await
                },
            )
            .await
        }
        HostCommand::Root {
            machine,
            path,
            default_root,
            move_channels,
        } => {
            let id = Some(ctx.machine(&machine).await?);
            let path = match (path, default_root) {
                (Some(path), _) => path,
                (None, true) => DEFAULT_ROOT.to_owned(),
                (None, false) => {
                    let detail: pb::MachineDetail = ctx
                        .call(rpc::machine::GET_MACHINE, &pb::GetMachineRequest { id })
                        .await?;
                    let root = detail.machine.map(|machine| machine.root).unwrap_or_default();
                    return Ok(Done::reply(json!({ "root": root })));
                }
            };
            ctx.planned(
                rpc::machine::PLAN_CHANGE_ROOT,
                &pb::PlanChangeRootRequest {
                    id: id.clone(),
                    root: path.clone(),
                },
                |plan: &pb::ChangeRootPlan| plan.token.clone(),
                move |plan| async move {
                    ctx.present_json(rpc::machine::CHANGE_ROOT, |presence| pb::ChangeRootRequest {
                        idempotency_key: key.clone(),
                        id: id.clone(),
                        root: path.clone(),
                        presence,
                        plan: plan.clone(),
                        move_channels,
                    })
                    .await
                },
            )
            .await
        }
        HostCommand::VmTier { machine, state } => {
            let id = Some(ctx.machine(&machine).await?);
            let on = values::on(state);
            ctx.change_present(rpc::machine::SET_VM_TIER, |presence| pb::SetVmTierRequest {
                idempotency_key: key.clone(),
                id: id.clone(),
                on,
                presence,
            }).await
        }
        HostCommand::Storage { machine } => {
            let request = pb::GetStorageRequest {
                id: Some(ctx.machine(&machine).await?),
            };
            reply(ctx, rpc::machine::GET_STORAGE, &request).await
        }
        HostCommand::ClearBuildCache {
            session,
            path,
            confirm_running,
        } => {
            let request = pb::ClearBuildCacheRequest {
                idempotency_key: key,
                session: Some(ctx.session(&session).await?),
                path,
                confirm_running,
            };
            ctx.change(rpc::machine::CLEAR_BUILD_CACHE, &request).await
        }
        HostCommand::TrimLogs { machine } => {
            let request = pb::TrimLogsRequest {
                idempotency_key: key,
                id: Some(ctx.machine(&machine).await?),
            };
            ctx.change(rpc::machine::TRIM_LOGS, &request).await
        }
        HostCommand::Stop { machine, reason } => {
            let request = pb::StopMachineSessionsRequest {
                machine: Some(ctx.machine(&machine).await?),
                stop_id: String::new(),
                reason: reason.unwrap_or_default(),
                via: pb::StopVia::Cli as i32,
            };
            ctx.change(rpc::machine::STOP_MACHINE_SESSIONS, &request).await
        }
        HostCommand::Start { machine } => {
            let request = pb::StartMachineSessionsRequest {
                machine: Some(ctx.machine(&machine).await?),
            };
            ctx.change(rpc::machine::START_MACHINE_SESSIONS, &request).await
        }
        HostCommand::Vars { machine } => {
            let request = pb::ListHostVariablesRequest {
                id: Some(ctx.machine(&machine).await?),
            };
            reply(ctx, rpc::machine::LIST_HOST_VARIABLES, &request).await
        }
        HostCommand::Var(VarCommand::Set {
            machine,
            name,
            value_on_command_line,
        }) => {
            // Refused before anything else, and never repeated in what is
            // printed: the text may be the secret itself (HO-13).
            if !value_on_command_line.is_empty() {
                return Err(CliError::BadValue {
                    field: "value",
                    value: "<not shown>".into(),
                    expected: "no value on the command line, where it would stay in the shell's history:                                pipe it on standard input, or run the command in a terminal and type it when asked",
                });
            }
            let id = Some(ctx.machine(&machine).await?);
            let value = secret(ctx, &format!("Value of {name}: "))?;
            ctx.change_present(rpc::machine::SET_HOST_VARIABLE, |presence| {
                pb::SetHostVariableRequest {
                    idempotency_key: key.clone(),
                    id: id.clone(),
                    name: name.clone(),
                    value: value.clone(),
                    presence,
                }
            }).await
        }
        HostCommand::Var(VarCommand::Rm { machine, name }) => {
            let id = Some(ctx.machine(&machine).await?);
            ctx.change_present(rpc::machine::REMOVE_HOST_VARIABLE, |presence| {
                pb::RemoveHostVariableRequest {
                    idempotency_key: key.clone(),
                    id: id.clone(),
                    name: name.clone(),
                    presence,
                }
            }).await
        }
        HostCommand::TrustKey {
            machine,
            fingerprint,
        } => {
            let id = Some(ctx.machine(&machine).await?);
            ctx.change_present(rpc::machine::TRUST_NEW_HOST_KEY, |presence| {
                pb::TrustNewHostKeyRequest {
                    idempotency_key: key.clone(),
                    id: id.clone(),
                    new_fingerprint: fingerprint.clone(),
                    presence,
                }
            }).await
        }
        // SEAM (MACHINESCRIPTS, D687).
        HostCommand::Script(command) => super::machine_scripts::script(ctx, command).await,
    }
}

fn read(path: &std::path::Path) -> Result<Vec<u8>, CliError> {
    std::fs::read(path).map_err(|source| CliError::ReadFile {
        path: path.to_path_buf(),
        source,
    })
}

pub(super) async fn provider<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: ProviderCommand,
) -> Result<Done, CliError> {
    let key = ctx.key();
    match command {
        ProviderCommand::Ls => {
            reply(ctx, rpc::provider::LIST_PROVIDERS, &pb::ListProvidersRequest {}).await
        }
        ProviderCommand::Config(ProviderConfigCommand::Show) => {
            let request = pb::GetProvidersConfigRequest {};
            reply(ctx, rpc::provider::GET_PROVIDERS_CONFIG, &request).await
        }
        ProviderCommand::Config(ProviderConfigCommand::Edit { file }) => {
            let bytes = match &file {
                Some(path) => read(path)?,
                None => ctx
                    .console
                    .stdin()
                    .map_err(|source| CliError::Console { source })?,
            };
            let toml = String::from_utf8(bytes).map_err(|_| CliError::BadValue {
                field: "providers TOML",
                value: "<not UTF-8>".into(),
                expected: "UTF-8 TOML",
            })?;
            let current: pb::ProvidersConfig = ctx
                .call(
                    rpc::provider::GET_PROVIDERS_CONFIG,
                    &pb::GetProvidersConfigRequest {},
                )
                .await?;
            ctx.change_present(rpc::provider::SET_PROVIDERS_CONFIG, |presence| {
                pb::SetProvidersConfigRequest {
                    idempotency_key: key.clone(),
                    toml: toml.clone(),
                    expected_revision: current.revision,
                    presence,
                }
            }).await
        }
        ProviderCommand::Import { file } => {
            let preview = pb::PreviewProviderImportRequest {
                file: read(&file)?,
                file_name: file
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            };
            ctx.planned(
                rpc::provider::PREVIEW_PROVIDER_IMPORT,
                &preview,
                |plan: &pb::ProviderImportPlan| plan.token.clone(),
                move |plan| async move {
                    ctx.present_json(rpc::provider::IMPORT_PROVIDER, |presence| {
                        pb::ImportProviderRequest {
                            idempotency_key: key.clone(),
                            plan: plan.clone(),
                            presence,
                        }
                    })
                    .await
                },
            )
            .await
        }
        ProviderCommand::Create {
            provider,
            name,
            params,
        } => {
            let params: HashMap<String, String> = params
                .iter()
                .map(|text| parse::pair(text))
                .collect::<Result<_, CliError>>()?;
            let request = pb::CreateMachineRequest {
                idempotency_key: key,
                provider: Some(pb::ProviderId { value: provider }),
                name,
                params,
            };
            if ctx.flags.dry_run {
                return ctx.change(rpc::provider::CREATE_MACHINE, &request).await;
            }
            let started = ctx.raw(rpc::provider::CREATE_MACHINE, &request).await?;
            follow(ctx, rpc::provider::CREATE_MACHINE, &started)
                .await
                .map(Done::reply)
        }
        ProviderCommand::Start { machine } => action(ctx, &machine, pb::ProviderAction::Start).await,
        ProviderCommand::Stop { machine } => action(ctx, &machine, pb::ProviderAction::Stop).await,
        ProviderCommand::Status { machine } => {
            action(ctx, &machine, pb::ProviderAction::Status).await
        }
        ProviderCommand::Delete { machine } => {
            // Deleting a provider's machine removes it from Humata too, so
            // it is planned as a removal is.
            let id = Some(ctx.machine(&machine).await?);
            ctx.planned(
                rpc::machine::PLAN_REMOVE_MACHINE,
                &pb::PlanRemoveMachineRequest { id: id.clone() },
                |plan: &pb::RemoveMachinePlan| plan.token.clone(),
                move |plan| async move {
                    let remove = pb::RemoveMachineRequest {
                        idempotency_key: key.clone(),
                        id: id.clone(),
                        dispositions: Vec::new(),
                        plan,
                        presence: None,
                    };
                    let started = ctx
                        .present(rpc::provider::RUN_PROVIDER_ACTION, |presence| {
                            pb::RunProviderActionRequest {
                                idempotency_key: key.clone(),
                                machine: id.clone(),
                                action: pb::ProviderAction::Delete as i32,
                                remove: Some(remove.clone()),
                                presence,
                            }
                        })
                        .await?;
                    follow(ctx, rpc::provider::RUN_PROVIDER_ACTION, &started).await
                },
            )
            .await
        }
        ProviderCommand::Login { machine } => {
            let machine = ctx.machine(&machine).await?;
            streams::provider_login(ctx, machine).await
        }
    }
}

async fn action<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    machine: &str,
    action: pb::ProviderAction,
) -> Result<Done, CliError> {
    let key = ctx.key();
    let machine = Some(ctx.machine(machine).await?);
    let build = |presence| pb::RunProviderActionRequest {
        idempotency_key: key.clone(),
        machine: machine.clone(),
        action: action as i32,
        remove: None,
        presence,
    };
    // Status only reads; starting and stopping change the machine.
    if ctx.flags.dry_run && action != pb::ProviderAction::Status {
        return ctx
            .change_present(rpc::provider::RUN_PROVIDER_ACTION, build)
            .await;
    }
    let started = ctx
        .present(rpc::provider::RUN_PROVIDER_ACTION, build)
        .await?;
    follow(ctx, rpc::provider::RUN_PROVIDER_ACTION, &started)
        .await
        .map(Done::reply)
}

/// What a call that starts an operation answers with: as it was started,
/// or, when this run waits (`--wait <seconds>`, or until done in a terminal),
/// as it ended. An operation that failed fails the run with its error, so
/// a script branches on the same `error.kind` and exit code as for any
/// refusal. One still running when the wait ends is printed as it is; ask
/// `humata ops <id>` for the rest.
pub(super) async fn follow<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    rpc: Rpc,
    started: &[u8],
) -> Result<Json, CliError> {
    let seconds = match ctx.flags.wait {
        Some(seconds) => seconds,
        None if ctx.interactive() => u32::MAX,
        None => 0,
    };
    let mut shown = crate::as_json::encoded(rpc.response, started);
    let Ok(mut operation) = pb::Operation::decode(started) else {
        return Ok(shown);
    };
    let running = pb::OperationState::Running as i32;
    if seconds == 0 || operation.state != running || operation.id.is_none() {
        return Ok(shown);
    }
    let until = Instant::now() + Duration::from_secs(u64::from(seconds.min(MAX_WAIT_SECONDS)));
    // The daemon pushes each change of the operation (TR-7): nothing is
    // asked again. The watch sends the operation as it is, then every
    // change, and ends when the operation does.
    let watch = rpc::operation::WATCH_OPERATION;
    let request = pb::WatchOperationRequest {
        id: operation.id.clone(),
        from_version: 0,
    };
    let mut changes = ctx.daemon.server_stream(watch, request.encode_to_vec()).await?;
    while operation.state == running {
        let left = until.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, changes.recv()).await {
            Ok(Some(change)) => {
                let bytes = change?;
                operation = crate::call::decode(watch, &bytes)?;
                shown = crate::as_json::encoded(watch.response, &bytes);
            }
            // The watch ended, or the wait ran out.
            Ok(None) | Err(_) => break,
        }
    }
    if operation.state == pb::OperationState::Failed as i32 {
        return Err(CliError::Daemon(DaemonError::Refused {
            rpc: rpc::operation::WATCH_OPERATION.path,
            code: FAILED_PRECONDITION,
            error: Box::new(operation.error.unwrap_or_default()),
        }));
    }
    Ok(shown)
}

/// gRPC's FAILED_PRECONDITION: the status a failed operation's error rides
/// on, which doesn't matter to the CLI when the error has a kind.
const FAILED_PRECONDITION: i32 = 9;

/// A wait with no end in a terminal is a year, which an `Instant` can hold.
const MAX_WAIT_SECONDS: u32 = 31_536_000;

/// Previews a sync on each machine, then syncs them all. `--plan` takes
/// the plans' tokens, comma-separated in the order the machines are named.
pub(super) async fn skills<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: SkillsCommand,
) -> Result<Done, CliError> {
    let SkillsCommand::Sync { machines: named } = command;
    let mut machines = Vec::new();
    for machine in &named {
        machines.push(ctx.machine(machine).await?);
    }
    if machines.is_empty() {
        let listed: pb::ListMachinesResponse = ctx
            .call(
                rpc::machine::LIST_MACHINES,
                &pb::ListMachinesRequest { page: None },
            )
            .await?;
        machines = listed.machines.into_iter().filter_map(|machine| machine.id).collect();
    }
    let plans: Vec<pb::PlanToken> = match &ctx.flags.plan {
        Some(tokens) => {
            let tokens: Vec<pb::PlanToken> = tokens
                .split(',')
                .map(|value| pb::PlanToken {
                    value: value.trim().to_owned(),
                })
                .collect();
            if tokens.len() != machines.len() {
                return Err(CliError::BadValue {
                    field: "--plan",
                    value: tokens.len().to_string(),
                    expected: "one token per machine, comma-separated, as --dry-run printed them",
                });
            }
            tokens
        }
        None => {
            let mut shown = Vec::new();
            let mut tokens = Vec::new();
            for machine in &machines {
                let request = pb::PreviewSkillsSyncRequest {
                    machine: Some(machine.clone()),
                };
                let bytes = ctx.raw(rpc::skills::PREVIEW_SKILLS_SYNC, &request).await?;
                let plan: pb::SkillsSyncPlan =
                    crate::call::decode(rpc::skills::PREVIEW_SKILLS_SYNC, &bytes)?;
                tokens.push(plan.token.unwrap_or_default());
                shown.push(crate::as_json::encoded(
                    rpc::skills::PREVIEW_SKILLS_SYNC.response,
                    &bytes,
                ));
            }
            let token = tokens
                .iter()
                .map(|token| token.value.as_str())
                .collect::<Vec<_>>()
                .join(",");
            let shown = json!({ "plans": shown, "token": token });
            if ctx.flags.dry_run {
                return Ok(Done {
                    result: shown,
                    outcome: Outcome::Planned,
                });
            }
            ctx.confirm(&shown)?;
            tokens
        }
    };
    let request = pb::SyncSkillsRequest {
        idempotency_key: ctx.key(),
        machines,
        plans,
    };
    let started = ctx.raw(rpc::skills::SYNC_SKILLS, &request).await?;
    follow(ctx, rpc::skills::SYNC_SKILLS, &started)
        .await
        .map(Done::reply)
}

pub(super) async fn bridge<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: BridgeCommand,
) -> Result<Done, CliError> {
    use pb::list_port_forwards_request::Scope;
    match command {
        BridgeCommand::Ls { session } => {
            let scope = match (session, ctx.machine_flag().await?) {
                (Some(session), _) => Some(Scope::Session(ctx.session(&session).await?)),
                (None, Some(machine)) => Some(Scope::Machine(machine)),
                (None, None) => None,
            };
            let request = pb::ListPortForwardsRequest { scope };
            reply(ctx, rpc::bridge::LIST_PORT_FORWARDS, &request).await
        }
        BridgeCommand::Close { forward } => {
            let request = pb::ClosePortForwardRequest {
                idempotency_key: ctx.key(),
                id: forward,
            };
            ctx.change(rpc::bridge::CLOSE_PORT_FORWARD, &request).await
        }
    }
}

/// Global settings, or the channel's with `--channel`.
pub(super) async fn settings<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: Option<SettingsCommand>,
) -> Result<Done, CliError> {
    let channel = ctx.channel_flag().await?;
    match (command, channel) {
        (None, None) => settings::get(ctx, None).await,
        (None, Some(channel)) => settings::channel_get(ctx, channel, None).await,
        (Some(SettingsCommand::Get { key }), None) => settings::get(ctx, key).await,
        (Some(SettingsCommand::Get { key }), Some(channel)) => {
            settings::channel_get(ctx, channel, key).await
        }
        (Some(SettingsCommand::Set { key, value }), None) => settings::set(ctx, &key, &value).await,
        (Some(SettingsCommand::Set { key, value }), Some(channel)) => {
            settings::channel_set(ctx, channel, &key, &value).await
        }
        (Some(SettingsCommand::Contain { state, inherit }), channel) => {
            settings::contain(ctx, channel, values::on(state), inherit).await
        }
    }
}

pub(super) async fn agents<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: AgentsCommand,
) -> Result<Done, CliError> {
    let key = ctx.key();
    match command {
        AgentsCommand::Ls => {
            reply(ctx, rpc::agents::LIST_AGENTS, &pb::ListAgentsRequest::default()).await
        }
        AgentsCommand::Add { name, fields } => {
            if fields.launch.is_none() {
                return Err(CliError::Missing { flag: "--launch" });
            }
            let mut agent = pb::AgentDefinition {
                name,
                ..Default::default()
            };
            apply(&mut agent, fields);
            ctx.change_present(rpc::agents::ADD_AGENT, |presence| pb::AddAgentRequest {
                idempotency_key: key.clone(),
                agent: Some(agent.clone()),
                presence,
            }).await
        }
        AgentsCommand::Edit { agent: name, fields } => {
            let listed: pb::ListAgentsResponse = ctx
                .call(
                    rpc::agents::LIST_AGENTS,
                    &pb::ListAgentsRequest {
                        include_disabled: true,
                    },
                )
                .await?;
            let id_of = |agent: &pb::AgentDefinition| {
                agent.id.as_ref().map(|id| id.value.clone()).unwrap_or_default()
            };
            let mut matches: Vec<pb::AgentDefinition> = listed
                .agents
                .into_iter()
                .filter(|agent| id_of(agent) == name || agent.name.eq_ignore_ascii_case(&name))
                .collect();
            let mut agent = match matches.len() {
                0 => {
                    return Err(CliError::NoMatch {
                        what: "agent",
                        name,
                    });
                }
                1 => matches.remove(0),
                _ => {
                    return Err(CliError::Ambiguous {
                        what: "agent",
                        name,
                        matches: matches.iter().map(id_of).collect(),
                    });
                }
            };
            apply(&mut agent, fields);
            let revision = agent.revision;
            ctx.change_present(rpc::agents::UPDATE_AGENT, |presence| pb::UpdateAgentRequest {
                idempotency_key: key.clone(),
                agent: Some(agent.clone()),
                expected_revision: revision,
                presence,
            }).await
        }
    }
}

/// The fields given on the command line, over what the agent had.
fn apply(agent: &mut pb::AgentDefinition, fields: AgentFields) {
    if let Some(launch) = fields.launch {
        agent.launch_command = launch;
    }
    if let Some(resume) = fields.resume_args {
        agent.resume_arguments = resume;
    }
    if let Some(fork) = fields.fork_args {
        agent.fork_arguments = fork;
    }
    if !fields.models.is_empty() {
        agent.models = fields.models;
    }
    if !fields.variables.is_empty() {
        agent.variables = fields.variables;
    }
}

/// Something to pin or mark VIP: `channel:`, `session:`, `machine:`,
/// `item:`, `request:`, `comment:` or `view:` and a name or id; or a bare
/// id, whose prefix says what it is, and otherwise a board item.
async fn entity<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    text: &str,
) -> Result<pb::EntityRef, CliError> {
    use pb::entity_ref::Entity;
    let entity = match text.split_once(':') {
        Some(("channel", name)) => Entity::Channel(ctx.channel(name).await?),
        Some(("session", name)) => Entity::Session(ctx.session(name).await?),
        Some(("machine", name)) => Entity::Machine(ctx.machine(name).await?),
        Some(("item", id)) => Entity::BoardItem(pb::BoardItemId { value: id.into() }),
        Some(("request", id)) => Entity::Request(pb::RequestId { value: id.into() }),
        Some(("comment", id)) => Entity::Comment(pb::CommentId { value: id.into() }),
        Some(("view", id)) => Entity::View(pb::ViewId { value: id.into() }),
        Some(_) => {
            return Err(CliError::BadValue {
                field: "thing",
                value: text.into(),
                expected: "channel:, session:, machine:, item:, request:, comment: or view: \
                           and a name or id",
            });
        }
        None if text.starts_with("ch_") => Entity::Channel(ctx.channel(text).await?),
        None if text.starts_with("se_") => Entity::Session(ctx.session(text).await?),
        None if text.starts_with("ma_") => Entity::Machine(ctx.machine(text).await?),
        None if text.starts_with("rq_") => Entity::Request(pb::RequestId { value: text.into() }),
        None => Entity::BoardItem(pb::BoardItemId { value: text.into() }),
    };
    Ok(pb::EntityRef {
        entity: Some(entity),
    })
}

pub(super) async fn mark<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    thing: &str,
    pinned: Option<bool>,
    vip: Option<bool>,
) -> Result<Done, CliError> {
    let request = pb::SetMarksRequest {
        idempotency_key: ctx.key(),
        entity: Some(entity(ctx, thing).await?),
        pinned,
        vip,
    };
    ctx.change(rpc::marks::SET_MARKS, &request).await
}

pub(super) async fn pinned<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: Option<PinnedCommand>,
) -> Result<Done, CliError> {
    match command {
        None => reply(ctx, rpc::marks::LIST_PINNED, &pb::ListPinnedRequest {}).await,
        Some(PinnedCommand::Reorder { things }) => {
            let mut order = Vec::with_capacity(things.len());
            for thing in &things {
                order.push(entity(ctx, thing).await?);
            }
            let current: pb::ListPinnedResponse = ctx
                .call(rpc::marks::LIST_PINNED, &pb::ListPinnedRequest {})
                .await?;
            let request = pb::ReorderPinnedRequest {
                idempotency_key: ctx.key(),
                order,
                expected_revision: current.revision,
            };
            ctx.change(rpc::marks::REORDER_PINNED, &request).await
        }
    }
}

pub(super) async fn report<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: ReportCommand,
) -> Result<Done, CliError> {
    let request = match command {
        ReportCommand::Record { title } => ReportRequest::Record { title },
        ReportCommand::Stop => ReportRequest::Stop,
        ReportCommand::Snapshot { title } => ReportRequest::Snapshot { title },
    };
    let done = ctx.daemon.report(request).await?;
    if !done.folder.is_empty() {
        ctx.note(&format!("Saved the report in {}", done.folder));
    }
    Ok(Done::reply(json!({
        "folder": done.folder,
        "recording": done.recording,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_keeps_fields_not_given() {
        let mut agent = pb::AgentDefinition {
            launch_command: "claude".into(),
            models: vec!["opus".into()],
            ..Default::default()
        };
        apply(
            &mut agent,
            AgentFields {
                resume_args: Some("--resume".into()),
                ..Default::default()
            },
        );
        assert_eq!(agent.launch_command, "claude");
        assert_eq!(agent.models, vec!["opus".to_owned()]);
        assert_eq!(agent.resume_arguments, "--resume");
    }
}
