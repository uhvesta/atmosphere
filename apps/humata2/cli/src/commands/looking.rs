//! Looking around (CLI.md §5.1): status, sessions, channels, activity,
//! events, audit and operations.

use humata2_proto::humata::v1 as pb;
use serde_json::{Value as Json, json};

use super::{pages, reply, settings, values};
use crate::args::{
    ChannelCommand, ChannelMachineCommand, ChannelRepoCommand, KeyCommand, OpsCommand, Topic,
};
use crate::call::{Ctx, Done, decode};
use crate::console::Console;
use crate::daemon::{Daemon, rpc};
use crate::error::CliError;
use crate::as_json;
use crate::principal::Principal;
use crate::resolve;

pub(super) async fn status<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>) -> Result<Done, CliError> {
    let server = ctx
        .json(rpc::daemon::GET_SERVER_INFO, &pb::GetServerInfoRequest {})
        .await?;
    let you = me(ctx).await?;
    Ok(Done::reply(json!({ "server": server, "you": you })))
}

pub(super) async fn whoami<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>) -> Result<Done, CliError> {
    me(ctx).await.map(Done::reply)
}

/// WhoAmI, with `tier`: `agent`, `script` or `user` (CLI.md §2). Whether a
/// user is logged in isn't in WhoAmIResponse yet (D28).
async fn me<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>) -> Result<Json, CliError> {
    let bytes = ctx
        .raw(rpc::daemon::WHO_AM_I, &pb::WhoAmIRequest {})
        .await?;
    let reply: pb::WhoAmIResponse = decode(rpc::daemon::WHO_AM_I, &bytes)?;
    let agent = matches!(
        reply.actor.as_ref().and_then(|actor| actor.actor.as_ref()),
        Some(pb::actor::Actor::Agent(_))
    );
    let tier = match ctx.principal {
        _ if agent => "agent",
        Principal::Script { .. } => "script",
        Principal::Agent { .. } | Principal::User => "user",
    };
    let mut result = as_json::encoded(rpc::daemon::WHO_AM_I.response, &bytes);
    if let Json::Object(fields) = &mut result {
        fields.insert("tier".into(), json!(tier));
    }
    Ok(result)
}

pub(super) async fn ls<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    all: bool,
) -> Result<Done, CliError> {
    let channel = ctx.channel_flag().await?;
    let machine = ctx.machine_flag().await?;
    pages(ctx, rpc::session::LIST_SESSIONS, "sessions", |page| {
        pb::ListSessionsRequest {
            channel: channel.clone(),
            machine: machine.clone(),
            include_archived: all,
            page,
        }
    })
    .await
    .map(Done::reply)
}

pub(super) async fn channels<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
) -> Result<Done, CliError> {
    pages(ctx, rpc::channel::LIST_CHANNELS, "channels", |page| {
        pb::ListChannelsRequest { page }
    })
    .await
    .map(Done::reply)
}

pub(super) async fn channel<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: ChannelCommand,
) -> Result<Done, CliError> {
    match command {
        ChannelCommand::Create { name, repo } => {
            let repos: Vec<pb::RepoRef> = repo
                .iter()
                .map(|repo| resolve::repo(repo))
                .collect::<Result<_, CliError>>()?;
            let request = pb::CreateChannelRequest {
                idempotency_key: ctx.key(),
                name,
                repos,
                machines: ctx.machines_flag().await?,
            };
            reply(ctx, rpc::channel::CREATE_CHANNEL, &request).await
        }
        ChannelCommand::Rename { channel, name } => {
            let request = pb::RenameChannelRequest {
                idempotency_key: ctx.key(),
                id: Some(ctx.channel(&channel).await?),
                name,
                expected_revision: None,
            };
            reply(ctx, rpc::channel::RENAME_CHANNEL, &request).await
        }
        ChannelCommand::Repo(ChannelRepoCommand::Add { channel, repo }) => {
            let request = pb::AddRepoRequest {
                idempotency_key: ctx.key(),
                channel: Some(ctx.channel(&channel).await?),
                repo: Some(resolve::repo(&repo)?),
            };
            reply(ctx, rpc::channel::ADD_REPO, &request).await
        }
        ChannelCommand::Repo(ChannelRepoCommand::Rm { channel, repo }) => {
            let channel = Some(ctx.channel(&channel).await?);
            let repo = Some(resolve::repo(&repo)?);
            let remove = pb::RemoveRepoRequest {
                idempotency_key: ctx.key(),
                channel: channel.clone(),
                repo: repo.clone(),
                plan: None,
            };
            ctx.planned(
                rpc::channel::PLAN_REMOVE_REPO,
                &pb::PlanRemoveRepoRequest { channel, repo },
                |plan: &pb::RemoveRepoPlan| plan.token.clone(),
                move |plan| async move {
                    ctx.json(rpc::channel::REMOVE_REPO, &pb::RemoveRepoRequest { plan, ..remove })
                        .await
                },
            )
            .await
        }
        ChannelCommand::Machine(ChannelMachineCommand::Add { channel, machine }) => {
            let request = pb::AddChannelMachineRequest {
                idempotency_key: ctx.key(),
                channel: Some(ctx.channel(&channel).await?),
                machine: Some(ctx.machine(&machine).await?),
            };
            reply(ctx, rpc::channel::ADD_MACHINE, &request).await
        }
        ChannelCommand::Machine(ChannelMachineCommand::Rm { channel, machine }) => {
            let channel = Some(ctx.channel(&channel).await?);
            let machine = Some(ctx.machine(&machine).await?);
            let remove = pb::RemoveChannelMachineRequest {
                idempotency_key: ctx.key(),
                channel: channel.clone(),
                machine: machine.clone(),
                dispositions: Vec::new(),
                plan: None,
            };
            ctx.planned(
                rpc::channel::PLAN_REMOVE_MACHINE,
                &pb::PlanRemoveChannelMachineRequest { channel, machine },
                |plan: &pb::RemoveChannelMachinePlan| plan.token.clone(),
                move |plan| async move {
                    ctx.json(
                        rpc::channel::REMOVE_MACHINE,
                        &pb::RemoveChannelMachineRequest { plan, ..remove },
                    )
                    .await
                },
            )
            .await
        }
        ChannelCommand::Grid { channel } => {
            let request = pb::GetGridRequest {
                channel: Some(ctx.channel(&channel).await?),
            };
            reply(ctx, rpc::channel::GET_GRID, &request).await
        }
        ChannelCommand::Settings { channel, command } => {
            let channel = ctx.channel(&channel).await?;
            match command {
                None => settings::channel_get(ctx, channel, None).await,
                Some(KeyCommand::Get { key }) => settings::channel_get(ctx, channel, key).await,
                Some(KeyCommand::Set { key, value }) => {
                    settings::channel_set(ctx, channel, &key, &value).await
                }
            }
        }
        ChannelCommand::Delete {
            channel,
            delete_work,
        } => {
            let channel = Some(ctx.channel(&channel).await?);
            let delete = pb::DeleteChannelRequest {
                idempotency_key: ctx.key(),
                channel: channel.clone(),
                plan: None,
                presence: None,
                delete_unpushed_work: delete_work,
            };
            ctx.planned(
                rpc::channel::PLAN_DELETE_CHANNEL,
                &pb::PlanDeleteChannelRequest { channel },
                |plan: &pb::DeleteChannelPlan| plan.token.clone(),
                move |plan| async move {
                    ctx.present_json(rpc::channel::DELETE_CHANNEL, |presence| {
                        pb::DeleteChannelRequest {
                            plan: plan.clone(),
                            presence,
                            ..delete.clone()
                        }
                    })
                    .await
                },
            )
            .await
        }
    }
}

/// The channel and its sessions, each with its forks (`forks` and
/// `fork_origin` on every session).
pub(super) async fn topology<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
) -> Result<Done, CliError> {
    let channel = ctx.this_channel().await?;
    let about = ctx
        .json(
            rpc::channel::GET_CHANNEL,
            &pb::GetChannelRequest {
                id: Some(channel.clone()),
            },
        )
        .await?;
    let sessions = pages(ctx, rpc::session::LIST_SESSIONS, "sessions", |page| {
        pb::ListSessionsRequest {
            channel: Some(channel.clone()),
            page,
            ..Default::default()
        }
    })
    .await?;
    Ok(Done::reply(json!({
        "channel": about,
        "sessions": sessions.get("sessions").cloned().unwrap_or(json!([])),
    })))
}

pub(super) async fn activity<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
) -> Result<Done, CliError> {
    let channel = ctx.channel_flag().await?;
    pages(ctx, rpc::activity::LIST_ACTIVITY, "events", |page| {
        pb::ListActivityRequest {
            channel: channel.clone(),
            page,
        }
    })
    .await
    .map(Done::reply)
}

/// Events as they come, one JSON object per line, until the stream ends
/// or the run is interrupted.
pub(super) async fn events<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    after: Option<u64>,
    topics: Vec<Topic>,
) -> Result<Done, CliError> {
    let request = pb::SubscribeRequest {
        after: after.map(|value| pb::EventSeq { value }),
        channels: ctx.channel_flag().await?.into_iter().collect(),
        sessions: Vec::new(),
        topics: topics
            .into_iter()
            .map(|topic| values::topic(topic) as i32)
            .collect(),
    };
    let mut replies = ctx
        .daemon
        .server_stream(rpc::events::SUBSCRIBE, prost::Message::encode_to_vec(&request))
        .await?;
    while let Some(event) = replies.recv().await {
        let event = event?;
        ctx.console
            .print(&as_json::encoded(rpc::events::SUBSCRIBE.response, &event).to_string());
    }
    Ok(Done::printed())
}

pub(super) async fn audit<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>) -> Result<Done, CliError> {
    let channel = ctx.channel_flag().await?;
    pages(ctx, rpc::audit::LIST_AUDIT, "entries", |page| {
        pb::ListAuditRequest {
            channel: channel.clone(),
            page,
            ..Default::default()
        }
    })
    .await
    .map(Done::reply)
}

pub(super) async fn ops<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    id: Option<String>,
    command: Option<OpsCommand>,
) -> Result<Done, CliError> {
    let operation = |value: String| Some(pb::OperationId { value });
    match (command, id) {
        (Some(OpsCommand::Cancel { id }), _) => {
            let request = pb::CancelOperationRequest {
                id: operation(id),
                idempotency_key: ctx.key(),
            };
            reply(ctx, rpc::operation::CANCEL_OPERATION, &request).await
        }
        (Some(OpsCommand::Retry { id }), _) => {
            let request = pb::RetryOperationRequest {
                id: operation(id),
                idempotency_key: ctx.key(),
            };
            reply(ctx, rpc::operation::RETRY_OPERATION, &request).await
        }
        (None, Some(id)) => {
            let request = pb::GetOperationRequest { id: operation(id) };
            reply(ctx, rpc::operation::GET_OPERATION, &request).await
        }
        (None, None) => pages(ctx, rpc::operation::LIST_OPERATIONS, "operations", |page| {
            pb::ListOperationsRequest {
                page,
                ..Default::default()
            }
        })
        .await
        .map(Done::reply),
    }
}
