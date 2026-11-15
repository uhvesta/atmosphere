//! Machines, providers, skills, port bridges, settings, agents, marks and
//! bug reports (CLI.md §5.6 and §5.7).

use std::collections::HashMap;

use humata2_proto::humata::v1 as pb;
use serde_json::json;

use super::{pages, parse, reply, secret, settings, streams, values};
use crate::args::{
    AgentFields, AgentsCommand, BridgeCommand, HostCommand, PinnedCommand, ProviderCommand,
    ProviderConfigCommand, ReportCommand, SettingsCommand, SkillsCommand, VarCommand,
};
use crate::call::{Ctx, Done, Outcome};
use crate::console::Console;
use crate::daemon::{Daemon, ReportRequest, rpc};
use crate::error::CliError;

pub(super) async fn host<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: HostCommand,
) -> Result<Done, CliError> {
    let key = ctx.key();
    match command {
        HostCommand::Ls => pages(ctx, rpc::machine::LIST_MACHINES, "machines", |page| {
            pb::ListMachinesRequest { page }
        })
        .await
        .map(Done::reply),
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
        } => {
            let request = pb::AddSshMachineRequest {
                idempotency_key: key,
                ssh_host,
                name: name.unwrap_or_default(),
                root: root.unwrap_or_default(),
                // No flag for it yet: a first add fails naming the key.
                trust_host_key_fingerprint: String::new(),
            };
            reply(ctx, rpc::machine::ADD_SSH_MACHINE, &request).await
        }
        HostCommand::Reconnect { machine } => {
            let request = pb::ReconnectMachineRequest {
                id: Some(ctx.machine(&machine).await?),
            };
            reply(ctx, rpc::machine::RECONNECT_MACHINE, &request).await
        }
        HostCommand::Remove { machine } => {
            let id = Some(ctx.machine(&machine).await?);
            ctx.planned(
                rpc::machine::PLAN_REMOVE_MACHINE,
                &pb::PlanRemoveMachineRequest { id: id.clone() },
                |plan: &pb::RemoveMachinePlan| plan.token.clone(),
                move |plan| async move {
                    ctx.present_json(rpc::machine::REMOVE_MACHINE, |presence| {
                        pb::RemoveMachineRequest {
                            idempotency_key: key.clone(),
                            id: id.clone(),
                            dispositions: Vec::new(),
                            plan: plan.clone(),
                            presence,
                        }
                    })
                    .await
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
            move_channels,
        } => {
            let id = Some(ctx.machine(&machine).await?);
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
            ctx.present_json(rpc::machine::SET_VM_TIER, |presence| pb::SetVmTierRequest {
                idempotency_key: key.clone(),
                id: id.clone(),
                on,
                presence,
            })
            .await
            .map(Done::reply)
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
            reply(ctx, rpc::machine::CLEAR_BUILD_CACHE, &request).await
        }
        HostCommand::TrimLogs { machine } => {
            let request = pb::TrimLogsRequest {
                idempotency_key: key,
                id: Some(ctx.machine(&machine).await?),
            };
            reply(ctx, rpc::machine::TRIM_LOGS, &request).await
        }
        HostCommand::Vars { machine } => {
            let request = pb::ListHostVariablesRequest {
                id: Some(ctx.machine(&machine).await?),
            };
            reply(ctx, rpc::machine::LIST_HOST_VARIABLES, &request).await
        }
        HostCommand::Var(VarCommand::Set { machine, name }) => {
            let id = Some(ctx.machine(&machine).await?);
            let value = secret(ctx, &format!("Value of {name}: "))?;
            ctx.present_json(rpc::machine::SET_HOST_VARIABLE, |presence| {
                pb::SetHostVariableRequest {
                    idempotency_key: key.clone(),
                    id: id.clone(),
                    name: name.clone(),
                    value: value.clone(),
                    presence,
                }
            })
            .await
            .map(Done::reply)
        }
        HostCommand::Var(VarCommand::Rm { machine, name }) => {
            let id = Some(ctx.machine(&machine).await?);
            ctx.present_json(rpc::machine::REMOVE_HOST_VARIABLE, |presence| {
                pb::RemoveHostVariableRequest {
                    idempotency_key: key.clone(),
                    id: id.clone(),
                    name: name.clone(),
                    presence,
                }
            })
            .await
            .map(Done::reply)
        }
        HostCommand::TrustKey {
            machine,
            fingerprint,
        } => {
            let id = Some(ctx.machine(&machine).await?);
            ctx.present_json(rpc::machine::TRUST_NEW_HOST_KEY, |presence| {
                pb::TrustNewHostKeyRequest {
                    idempotency_key: key.clone(),
                    id: id.clone(),
                    new_fingerprint: fingerprint.clone(),
                    presence,
                }
            })
            .await
            .map(Done::reply)
        }
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
            ctx.present_json(rpc::provider::SET_PROVIDERS_CONFIG, |presence| {
                pb::SetProvidersConfigRequest {
                    idempotency_key: key.clone(),
                    toml: toml.clone(),
                    expected_revision: current.revision,
                    presence,
                }
            })
            .await
            .map(Done::reply)
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
            reply(ctx, rpc::provider::CREATE_MACHINE, &request).await
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
                    ctx.present_json(rpc::provider::RUN_PROVIDER_ACTION, |presence| {
                        pb::RunProviderActionRequest {
                            idempotency_key: key.clone(),
                            machine: id.clone(),
                            action: pb::ProviderAction::Delete as i32,
                            remove: Some(remove.clone()),
                            presence,
                        }
                    })
                    .await
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
    ctx.present_json(rpc::provider::RUN_PROVIDER_ACTION, |presence| {
        pb::RunProviderActionRequest {
            idempotency_key: key.clone(),
            machine: machine.clone(),
            action: action as i32,
            remove: None,
            presence,
        }
    })
    .await
    .map(Done::reply)
}

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
    reply(ctx, rpc::skills::SYNC_SKILLS, &request).await
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
            reply(ctx, rpc::bridge::CLOSE_PORT_FORWARD, &request).await
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
            reply(ctx, rpc::agents::LIST_AGENTS, &pb::ListAgentsRequest {}).await
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
            ctx.present_json(rpc::agents::ADD_AGENT, |presence| pb::AddAgentRequest {
                idempotency_key: key.clone(),
                agent: Some(agent.clone()),
                presence,
            })
            .await
            .map(Done::reply)
        }
        AgentsCommand::Edit { agent: name, fields } => {
            let listed: pb::ListAgentsResponse = ctx
                .call(rpc::agents::LIST_AGENTS, &pb::ListAgentsRequest {})
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
            ctx.present_json(rpc::agents::UPDATE_AGENT, |presence| pb::UpdateAgentRequest {
                idempotency_key: key.clone(),
                agent: Some(agent.clone()),
                expected_revision: revision,
                presence,
            })
            .await
            .map(Done::reply)
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
    reply(ctx, rpc::marks::SET_MARKS, &request).await
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
            reply(ctx, rpc::marks::REORDER_PINNED, &request).await
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
