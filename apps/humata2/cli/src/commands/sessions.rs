//! Sessions (CLI.md §5.2): their terminals, life cycle, repos and pushes,
//! and talking to their agents.

use humata2_proto::humata::v1 as pb;

use super::{pages, reply, streams, values};
use crate::args::{SessionCommand, SessionRepoCommand};
use crate::call::{Ctx, Done, Outcome};
use crate::console::Console;
use crate::daemon::{Daemon, rpc};
use crate::error::CliError;
use crate::resolve;

pub(super) async fn attach<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    session: &str,
    tab: Option<String>,
    read_only: bool,
) -> Result<Done, CliError> {
    streams::no_json(ctx)?;
    let open = pb::AttachOpen {
        session: Some(ctx.session(session).await?),
        tab: tab.map(|value| pb::TabId { value }),
        read_only,
        ..Default::default()
    };
    streams::attach(ctx, open).await
}

/// Opens a shell on a machine and attaches to it; with `--json`, prints
/// the new tab instead, for a script to attach to later.
pub(super) async fn shell<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    machine: &str,
) -> Result<Done, CliError> {
    let request = pb::OpenMachineShellRequest {
        idempotency_key: ctx.key(),
        machine: Some(ctx.machine(machine).await?),
    };
    if ctx.flags.json {
        return reply(ctx, rpc::terminal::OPEN_MACHINE_SHELL, &request).await;
    }
    let tab: pb::Tab = ctx.call(rpc::terminal::OPEN_MACHINE_SHELL, &request).await?;
    let open = pb::AttachOpen {
        session: tab.session,
        tab: tab.id,
        ..Default::default()
    };
    streams::attach(ctx, open).await
}

pub(super) async fn conversation<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    session: &str,
) -> Result<Done, CliError> {
    let id = ctx.session(session).await?;
    pages(ctx, rpc::session::READ_CONVERSATION, "entries", |page| {
        pb::ReadConversationRequest {
            id: Some(id.clone()),
            page,
        }
    })
    .await
    .map(Done::reply)
}

pub(super) async fn session<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: SessionCommand,
) -> Result<Done, CliError> {
    match command {
        SessionCommand::Queue { session } => {
            let request = pb::ListQueuedRequest {
                session: Some(ctx.session(&session).await?),
            };
            reply(ctx, rpc::terminal::LIST_QUEUED, &request).await
        }
        SessionCommand::SendNow { session, ids } => {
            let request = pb::SendQueuedNowRequest {
                idempotency_key: ctx.key(),
                session: Some(ctx.session(&session).await?),
                ids: ids
                    .into_iter()
                    .map(|value| pb::QueuedTypingId { value })
                    .collect(),
            };
            reply(ctx, rpc::terminal::SEND_QUEUED_NOW, &request).await
        }
        SessionCommand::Start {
            agent,
            model,
            repo,
            title,
            message,
        } => {
            let repos: Vec<pb::RepoChoice> = repo
                .iter()
                .map(|repo| {
                    resolve::repo(repo).map(|repo| pb::RepoChoice {
                        repo: Some(repo),
                        include: true,
                    })
                })
                .collect::<Result<_, CliError>>()?;
            let request = pb::StartSessionRequest {
                idempotency_key: ctx.key(),
                channel: Some(ctx.this_channel().await?),
                title: title.unwrap_or_default(),
                agent: agent.map(|value| pb::AgentId { value }),
                model: model.unwrap_or_default(),
                machine: Some(ctx.needs_machine().await?),
                repos,
                first_message: message.unwrap_or_default(),
                board_item: None,
                spec_files: Vec::new(),
            };
            reply(ctx, rpc::session::START_SESSION, &request).await
        }
        SessionCommand::Fork {
            session,
            kind,
            title,
        } => {
            let request = pb::ForkSessionRequest {
                idempotency_key: ctx.key(),
                parent: Some(ctx.session(&session).await?),
                kind: values::fork_kind(kind) as i32,
                before_turn: 0,
                title: title.unwrap_or_default(),
                agent: None,
                model: String::new(),
                machine: ctx.machine_flag().await?,
            };
            if ctx.flags.dry_run {
                let preview = ctx.json(rpc::session::PREVIEW_FORK, &request).await?;
                return Ok(Done {
                    result: preview,
                    outcome: Outcome::Planned,
                });
            }
            reply(ctx, rpc::session::FORK_SESSION, &request).await
        }
        SessionCommand::Rename { session, title } => {
            let request = pb::RenameSessionRequest {
                idempotency_key: ctx.key(),
                id: Some(ctx.session(&session).await?),
                title,
                expected_revision: None,
            };
            reply(ctx, rpc::session::RENAME_SESSION, &request).await
        }
        SessionCommand::Stop { session } => {
            let request = pb::StopSessionRequest {
                idempotency_key: ctx.key(),
                id: Some(ctx.session(&session).await?),
            };
            reply(ctx, rpc::session::STOP_SESSION, &request).await
        }
        SessionCommand::Resume { session, fresh } => {
            use pb::resume_session_request::Mode;
            let mode = if fresh {
                Mode::StartFreshWithSummary
            } else {
                Mode::Resume
            };
            let request = pb::ResumeSessionRequest {
                idempotency_key: ctx.key(),
                id: Some(ctx.session(&session).await?),
                mode: mode as i32,
            };
            reply(ctx, rpc::session::RESUME_SESSION, &request).await
        }
        SessionCommand::Archive {
            session,
            push_first,
            delete_work,
        } => {
            let choice = match (push_first, delete_work) {
                (true, _) => pb::ArchiveChoice::CommitPushPrThenArchive,
                (false, true) => pb::ArchiveChoice::ArchiveAndDeleteWork,
                (false, false) => pb::ArchiveChoice::Archive,
            };
            let id = Some(ctx.session(&session).await?);
            let archive = pb::ArchiveSessionRequest {
                idempotency_key: ctx.key(),
                id: id.clone(),
                choice: choice as i32,
                plan: None,
            };
            ctx.planned(
                rpc::session::PLAN_ARCHIVE,
                &pb::PlanArchiveRequest { id },
                |plan: &pb::ArchivePlan| plan.token.clone(),
                move |plan| async move {
                    ctx.json(
                        rpc::session::ARCHIVE_SESSION,
                        &pb::ArchiveSessionRequest { plan, ..archive },
                    )
                    .await
                },
            )
            .await
        }
        SessionCommand::Delete { session } => {
            let id = Some(ctx.session(&session).await?);
            let delete = pb::DeleteSessionRequest {
                idempotency_key: ctx.key(),
                id: id.clone(),
                plan: None,
            };
            ctx.planned(
                rpc::session::PLAN_DELETE,
                &pb::PlanDeleteRequest { id },
                |plan: &pb::DeletePlan| plan.token.clone(),
                move |plan| async move {
                    ctx.json(
                        rpc::session::DELETE_SESSION,
                        &pb::DeleteSessionRequest { plan, ..delete },
                    )
                    .await
                },
            )
            .await
        }
        SessionCommand::Restore { session } => {
            let request = pb::RestoreSessionRequest {
                idempotency_key: ctx.key(),
                id: Some(ctx.session(&session).await?),
                machine: ctx.machine_flag().await?,
            };
            reply(ctx, rpc::session::RESTORE_SESSION, &request).await
        }
        SessionCommand::Rebuild { session } => {
            let request = pb::RebuildSessionRequest {
                idempotency_key: ctx.key(),
                id: Some(ctx.session(&session).await?),
                machine: ctx.machine_flag().await?,
            };
            reply(ctx, rpc::session::REBUILD_SESSION, &request).await
        }
        SessionCommand::Move { session, to } => {
            let request = pb::MoveSessionRequest {
                idempotency_key: ctx.key(),
                id: Some(ctx.session(&session).await?),
                to: Some(ctx.machine(&to).await?),
            };
            reply(ctx, rpc::session::MOVE_SESSION, &request).await
        }
        SessionCommand::Repo(SessionRepoCommand::Add { session, repo }) => {
            let request = pb::AddRepoToSessionRequest {
                idempotency_key: ctx.key(),
                id: Some(ctx.session(&session).await?),
                repo: Some(resolve::repo(&repo)?),
            };
            reply(ctx, rpc::session::ADD_REPO_TO_SESSION, &request).await
        }
        SessionCommand::Repo(SessionRepoCommand::Drop { session, repo }) => {
            let id = Some(ctx.session(&session).await?);
            let repo = Some(resolve::repo(&repo)?);
            let drop = pb::DropRepoRequest {
                idempotency_key: ctx.key(),
                id: id.clone(),
                repo: repo.clone(),
                plan: None,
            };
            ctx.planned(
                rpc::session::PLAN_DROP_REPO,
                &pb::PlanDropRepoRequest { id, repo },
                |plan: &pb::DropRepoPlan| plan.token.clone(),
                move |plan| async move {
                    ctx.json(rpc::session::DROP_REPO, &pb::DropRepoRequest { plan, ..drop })
                        .await
                },
            )
            .await
        }
        SessionCommand::OwnCopy { session, repo } => {
            // GiveOwnCopyRequest names only the fork: the daemon copies
            // every repo it reads from its parent (see the report's gaps).
            resolve::repo(&repo)?;
            let request = pb::GiveOwnCopyRequest {
                idempotency_key: ctx.key(),
                fork: Some(ctx.session(&session).await?),
            };
            reply(ctx, rpc::session::GIVE_OWN_COPY, &request).await
        }
        SessionCommand::Spec { session, state } => {
            let request = pb::SetSpecModeRequest {
                idempotency_key: ctx.key(),
                id: Some(ctx.session(&session).await?),
                enabled: values::on(state),
                files: Vec::new(),
                expected_revision: None,
            };
            reply(ctx, rpc::session::SET_SPEC_MODE, &request).await
        }
        SessionCommand::Push {
            session,
            repo,
            pr,
            from_view,
            message,
        } => push(ctx, &session, &repo, pr, from_view, message).await,
        SessionCommand::Note { session, text } => {
            let request = pb::SendNoteRequest {
                idempotency_key: ctx.key(),
                to: Some(ctx.session(&session).await?),
                text,
                reason: String::new(),
            };
            reply(ctx, rpc::session::SEND_NOTE, &request).await
        }
        SessionCommand::Ask { session, text } => {
            let request = pb::AskSessionRequest {
                idempotency_key: ctx.key(),
                to: Some(ctx.session(&session).await?),
                question: text,
                reason: String::new(),
                timeout_seconds: ctx.wait_seconds(),
            };
            ctx.note(&format!("Waiting for {session} to answer…"));
            reply(ctx, rpc::session::ASK_SESSION, &request).await
        }
        SessionCommand::Type {
            session,
            text,
            at_turn_end,
            tab,
        } => {
            use pb::type_request::When;
            let when = if at_turn_end { When::TurnEnds } else { When::Now };
            let request = pb::TypeRequest {
                idempotency_key: ctx.key(),
                session: Some(ctx.session(&session).await?),
                tab: tab.map(|value| pb::TabId { value }),
                text,
                when: when as i32,
            };
            reply(ctx, rpc::terminal::TYPE, &request).await
        }
    }
}

/// Pushes the named repos, or every repo the push plan lists.
async fn push<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    session: &str,
    repos: &[String],
    pr: bool,
    from_view: Option<String>,
    message: Option<String>,
) -> Result<Done, CliError> {
    let id = Some(ctx.session(session).await?);
    let named: Vec<pb::RepoRef> = repos
        .iter()
        .map(|repo| resolve::repo(repo))
        .collect::<Result<_, CliError>>()?;
    let choice = move |repo: pb::RepoRef| pb::PushChoice {
        repo: Some(repo),
        commit_message: message.clone().unwrap_or_default(),
        open_pull_request: pr,
        pr_title: String::new(),
        pr_body: String::new(),
    };
    let plan_request = pb::PlanPushRequest { id: id.clone() };
    let again = plan_request.clone();
    let push = pb::PushAndOpenPullRequestsRequest {
        idempotency_key: ctx.key(),
        id,
        repos: Vec::new(),
        plan: None,
        from_view: from_view.map(|value| pb::ViewId { value }),
    };
    ctx.planned(
        rpc::session::PLAN_PUSH,
        &plan_request,
        |plan: &pb::PushPlan| plan.token.clone(),
        |plan| async move {
            let repos = if named.is_empty() {
                let listed: pb::PushPlan = ctx.call(rpc::session::PLAN_PUSH, &again).await?;
                listed.repos.into_iter().filter_map(|repo| repo.repo).collect()
            } else {
                named
            };
            let request = pb::PushAndOpenPullRequestsRequest {
                repos: repos.into_iter().map(choice).collect(),
                plan,
                ..push
            };
            ctx.json(rpc::session::PUSH_AND_OPEN_PULL_REQUESTS, &request)
                .await
        },
    )
    .await
}
