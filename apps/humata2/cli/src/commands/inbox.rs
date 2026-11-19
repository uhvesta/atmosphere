//! The Inbox, access rules, logins and script tokens (CLI.md §5.4).

use humata2_proto::humata::v1 as pb;
use serde_json::json;

use super::{pages, reply, secret, values};
use crate::args::{AccessCommand, InboxCommand, TokenCommand};
use crate::call::{Ctx, Done};
use crate::console::Console;
use crate::daemon::{Daemon, DaemonError, rpc};
use crate::error::CliError;
use crate::as_json;
use crate::resolve;

/// The gRPC status a failed answer's own error is reported with.
const UNKNOWN: i32 = 2;

fn request_id(value: String) -> Option<pb::RequestId> {
    Some(pb::RequestId { value })
}

/// `--session` (with `--with-forks`), `--channel`, or every channel.
async fn scope<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    session: Option<String>,
    with_forks: bool,
) -> Result<pb::InboxScope, CliError> {
    use pb::inbox_scope::Scope;
    let scope = match (session, ctx.channel_flag().await?) {
        (Some(session), _) => Scope::Session(pb::SessionInbox {
            session: Some(ctx.session(&session).await?),
            with_forks,
        }),
        (None, Some(channel)) => Scope::Channel(channel),
        (None, None) => Scope::Global(pb::GlobalInbox {}),
    };
    Ok(pb::InboxScope { scope: Some(scope) })
}

pub(super) async fn inbox<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    all: bool,
    session: Option<String>,
    with_forks: bool,
    command: Option<InboxCommand>,
) -> Result<Done, CliError> {
    use pb::request_answer::Answer;
    match command {
        None => {
            let scope = scope(ctx, session, with_forks).await?;
            pages(ctx, rpc::inbox::LIST_REQUESTS, "requests", |page| {
                pb::ListRequestsRequest {
                    scope: Some(scope.clone()),
                    include_answered: all,
                    agent_inbox: false,
                    page,
                }
            })
            .await
            .map(Done::reply)
        }
        Some(InboxCommand::Show { request }) => {
            let request = pb::GetRequestRequest {
                id: request_id(request),
            };
            reply(ctx, rpc::inbox::GET_REQUEST, &request).await
        }
        Some(InboxCommand::Approve {
            request,
            note,
            always,
        }) => {
            if let Some(note) = note {
                return Err(CliError::BadValue {
                    field: "--note",
                    value: note,
                    expected: "no note: an approval can't carry one; use `humata inbox reply` \
                               or `humata session note` to say something",
                });
            }
            let answer = if always {
                let offered: pb::InboxRequest = ctx
                    .call(
                        rpc::inbox::GET_REQUEST,
                        &pb::GetRequestRequest {
                            id: request_id(request.clone()),
                        },
                    )
                    .await?;
                let Some(target) = offered.always.and_then(|always| always.target) else {
                    return Err(CliError::BadValue {
                        field: "--always",
                        value: request,
                        expected: "a request that offers \"always\"; approve it without --always",
                    });
                };
                Answer::Always(pb::AlwaysLet {
                    target: Some(target),
                })
            } else {
                Answer::Approve(pb::Approve {})
            };
            answer_one(ctx, request, answer).await
        }
        Some(InboxCommand::Deny { request, note }) => {
            let answer = Answer::Deny(pb::Deny {
                note: note.unwrap_or_default(),
            });
            answer_one(ctx, request, answer).await
        }
        Some(InboxCommand::Reply { request, text }) => {
            answer_one(ctx, request, Answer::Reply(pb::Reply { text })).await
        }
        Some(InboxCommand::ApproveAll) => {
            answer_all(ctx, scope(ctx, session, with_forks).await?, true).await
        }
        Some(InboxCommand::DenyAll) => {
            answer_all(ctx, scope(ctx, session, with_forks).await?, false).await
        }
    }
}

async fn answer_one<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    request: String,
    answer: pb::request_answer::Answer,
) -> Result<Done, CliError> {
    let key = ctx.key();
    let id = request_id(request);
    let answer = pb::RequestAnswer {
        answer: Some(answer),
    };
    ctx.change_present(rpc::inbox::ANSWER_REQUEST, |presence| pb::AnswerRequestRequest {
        idempotency_key: key.clone(),
        id: id.clone(),
        answer: Some(answer.clone()),
        expected_revision: None,
        presence,
    }).await
}

/// Answers the pending requests listed now, and only those: one that
/// arrives meanwhile waits for its own answer.
async fn answer_all<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    scope: pb::InboxScope,
    approve: bool,
) -> Result<Done, CliError> {
    let listed = pages(ctx, rpc::inbox::LIST_REQUESTS, "requests", |page| {
        pb::ListRequestsRequest {
            scope: Some(scope.clone()),
            include_answered: false,
            agent_inbox: false,
            page,
        }
    })
    .await?;
    let ids: Vec<pb::RequestId> = listed
        .get("requests")
        .and_then(|requests| requests.as_array())
        .into_iter()
        .flatten()
        .filter(|request| request.get("state").and_then(|state| state.as_str()) == Some("pending"))
        .filter_map(|request| request.get("id").and_then(|id| id.as_str()))
        .map(|value| pb::RequestId {
            value: value.to_owned(),
        })
        .collect();
    if ids.is_empty() {
        ctx.note("Nothing is waiting.");
        return Ok(Done::reply(json!({ "answered": [], "skipped": [] })));
    }
    let verb = if approve { "Approving" } else { "Denying" };
    ctx.note(&format!("{verb} {} requests…", ids.len()));
    let key = ctx.key();
    ctx.change_present(rpc::inbox::ANSWER_ALL, |presence| pb::AnswerAllRequest {
        idempotency_key: key.clone(),
        scope: Some(scope.clone()),
        ids: ids.clone(),
        approve,
        presence,
    }).await
}

/// A grant target as text: `session:<s>` or `network:<host>:<port>` (on
/// `--machine` if given). Machine-wide and board-item targets aren't offered
/// (IN-22).
async fn target<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    text: &str,
) -> Result<pb::GrantTarget, CliError> {
    use pb::grant_target::Target;
    let bad = || CliError::BadValue {
        field: "target",
        value: text.into(),
        expected: "session:<session> or network:<host>:<port>",
    };
    let (kind, rest) = text.split_once(':').ok_or_else(bad)?;
    if rest.is_empty() {
        return Err(bad());
    }
    let target = match kind {
        "session" => Target::Session(ctx.session(rest).await?),
        "network" => {
            let (host, port) = rest.rsplit_once(':').ok_or_else(bad)?;
            let port = port.parse::<u16>().map_err(|_| bad())?;
            if host.is_empty() {
                return Err(bad());
            }
            Target::Network(pb::NetworkTarget {
                host: host.into(),
                port: u32::from(port),
                machine: ctx.machine_flag().await?,
            })
        }
        _ => return Err(bad()),
    };
    Ok(pb::GrantTarget {
        target: Some(target),
    })
}

pub(super) async fn access<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: Option<AccessCommand>,
) -> Result<Done, CliError> {
    let key = ctx.key();
    match command {
        None => {
            let request = pb::GetChannelRulesRequest {
                channel: Some(ctx.this_channel().await?),
            };
            reply(ctx, rpc::access::GET_CHANNEL_RULES, &request).await
        }
        Some(AccessCommand::Set {
            capability,
            decision,
        }) => {
            let capability = resolve::capability(&capability)? as i32;
            let channel = Some(ctx.this_channel().await?);
            let decision = values::decision(decision) as i32;
            ctx.change_present(rpc::access::SET_CHANNEL_RULE, |presence| {
                pb::SetChannelRuleRequest {
                    idempotency_key: key.clone(),
                    channel: channel.clone(),
                    capability,
                    decision,
                    expected_revision: None,
                    presence,
                }
            }).await
        }
        Some(AccessCommand::Away { mode, kinds }) => {
            let low_risk: Vec<i32> = kinds
                .iter()
                .map(|kind| resolve::capability(kind).map(|capability| capability as i32))
                .collect::<Result<_, CliError>>()?;
            let channel = Some(ctx.this_channel().await?);
            let away = pb::AwayRule {
                mode: values::away(mode) as i32,
                low_risk,
            };
            ctx.change_present(rpc::access::SET_AWAY_RULE, |presence| {
                pb::SetAwayRuleRequest {
                    idempotency_key: key.clone(),
                    channel: channel.clone(),
                    away: Some(away.clone()),
                    expected_revision: None,
                    presence,
                }
            }).await
        }
        Some(AccessCommand::Grants) => {
            let channel = ctx.channel_flag().await?;
            pages(ctx, rpc::access::LIST_GRANTS, "grants", |page| pb::ListGrantsRequest {
                channel: channel.clone(),
                requester: None,
                capability: pb::Capability::Unspecified as i32,
                page,
            })
            .await
            .map(Done::reply)
        }
        Some(AccessCommand::Revoke { grant }) => {
            let grant = Some(pb::GrantId { value: grant });
            ctx.change_present(rpc::access::REVOKE_GRANT, |presence| pb::RevokeGrantRequest {
                idempotency_key: key.clone(),
                grant: grant.clone(),
                presence,
            }).await
        }
        Some(AccessCommand::Why {
            session,
            capability,
            target: on,
        }) => {
            let capability = resolve::capability(&capability)? as i32;
            let target = match on {
                Some(text) => Some(target(ctx, &text).await?),
                None => None,
            };
            let request = pb::ExplainRequest {
                session: Some(ctx.session(&session).await?),
                capability,
                target,
            };
            reply(ctx, rpc::access::EXPLAIN, &request).await
        }
        Some(AccessCommand::Passphrase { generate }) => {
            let passphrase = if generate {
                String::new()
            } else {
                let typed = secret(ctx, "New approval passphrase: ")?;
                if typed.is_empty() {
                    return Err(CliError::BadValue {
                        field: "passphrase",
                        value: String::new(),
                        expected: "a passphrase, typed or on stdin, or --generate for one",
                    });
                }
                typed
            };
            ctx.change_present(rpc::access::SET_APPROVAL_PASSPHRASE, |presence| {
                pb::SetApprovalPassphraseRequest {
                    idempotency_key: key.clone(),
                    passphrase: passphrase.clone(),
                    presence,
                    generate,
                }
            }).await
        }
        Some(AccessCommand::AskAhead {
            capability,
            target: on,
            reason,
        }) => {
            let request = pb::RequestAccessRequest {
                idempotency_key: key,
                capability: resolve::capability(&capability)? as i32,
                target: Some(target(ctx, &on).await?),
                reason,
                wait_seconds: ctx.wait_seconds(),
            };
            if ctx.flags.dry_run {
                return ctx.change(rpc::inbox::REQUEST_ACCESS, &request).await;
            }
            let first: pb::WaitResponse = ctx.call(rpc::inbox::REQUEST_ACCESS, &request).await?;
            let answered = ctx.wait(first).await?;
            if let Some(error) = answered.error.clone() {
                return Err(DaemonError::Refused {
                    rpc: rpc::inbox::REQUEST_ACCESS.path,
                    code: UNKNOWN,
                    error: Box::new(error),
                }
                .into());
            }
            Ok(Done::reply(as_json::typed("humata.v1.WaitResponse", &answered)))
        }
    }
}

pub(super) async fn logins<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>) -> Result<Done, CliError> {
    reply(ctx, rpc::cli_auth::LIST_CLI_SESSIONS, &pb::ListCliSessionsRequest {}).await
}

pub(super) async fn token<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: TokenCommand,
) -> Result<Done, CliError> {
    let key = ctx.key();
    match command {
        TokenCommand::Create {
            name,
            capabilities,
            days,
        } => {
            let capabilities: Vec<i32> = capabilities
                .iter()
                .map(|text| resolve::capability(text).map(|capability| capability as i32))
                .collect::<Result<_, CliError>>()?;
            let channels: Vec<pb::ChannelId> = ctx.channel_flag().await?.into_iter().collect();
            ctx.change_present(rpc::cli_auth::CREATE_SCRIPT_TOKEN, |presence| {
                pb::CreateScriptTokenRequest {
                    idempotency_key: key.clone(),
                    name: name.clone(),
                    capabilities: capabilities.clone(),
                    channels: channels.clone(),
                    days,
                    presence,
                }
            }).await
        }
        TokenCommand::Ls => {
            reply(ctx, rpc::cli_auth::LIST_SCRIPT_TOKENS, &pb::ListScriptTokensRequest {}).await
        }
        TokenCommand::Revoke { token } => {
            ctx.change_present(rpc::cli_auth::REVOKE_SCRIPT_TOKEN, |presence| {
                pb::RevokeScriptTokenRequest {
                    token_id: token.clone(),
                    presence,
                }
            })
            .await
        }
    }
}
