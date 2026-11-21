//! Boards, prepared views, diffs, comments and pull request reviews
//! (CLI.md §5.3).

use humata2_proto::humata::v1 as pb;

use super::{pages, parse, reply, values};
use crate::args::{
    BoardCommand, BoardWhich, Column, CommentCommand, DiffCommand, ReviewCommand, ViewAction,
};
use crate::call::{Ctx, Done, Outcome};
use crate::console::Console;
use crate::daemon::{Daemon, rpc};
use crate::error::CliError;
use crate::resolve;

fn item(value: String) -> Option<pb::BoardItemId> {
    Some(pb::BoardItemId { value })
}

/// `--global`, `--session`, or the channel's board.
async fn board_ref<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    which: &BoardWhich,
) -> Result<pb::BoardRef, CliError> {
    use pb::board_ref::Board;
    let board = if which.global {
        Board::Global(pb::GlobalBoard {})
    } else if let Some(session) = &which.session {
        Board::Session(ctx.session(session).await?)
    } else {
        Board::Channel(ctx.this_channel().await?)
    };
    Ok(pb::BoardRef { board: Some(board) })
}

/// The board a write names: `--global`, `--session` or `--channel`. With
/// none, no board is sent and the daemon picks the caller's own session
/// board, so every door agrees (VF-21).
async fn written_board<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    which: &BoardWhich,
) -> Result<Option<pb::BoardRef>, CliError> {
    let named = which.global || which.session.is_some() || ctx.channel_flag().await?.is_some();
    if named {
        Ok(Some(board_ref(ctx, which).await?))
    } else {
        Ok(None)
    }
}

/// `--revision`: the revision the caller read, checked by the daemon.
fn revision(read: Option<u64>) -> Option<pb::Revision> {
    read.map(|value| pb::Revision { value })
}

/// Session names or ids, resolved in order (BD-16).
async fn sessions<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    names: &[String],
) -> Result<Vec<pb::SessionId>, CliError> {
    let mut ids = Vec::with_capacity(names.len());
    for name in names {
        ids.push(ctx.session(name).await?);
    }
    Ok(ids)
}

pub(super) async fn board<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    which: BoardWhich,
    columns: Vec<Column>,
    command: Option<BoardCommand>,
) -> Result<Done, CliError> {
    match command {
        None => {
            let view = pb::BoardView {
                view: Some(pb::board_view::View::Board(board_ref(ctx, &which).await?)),
            };
            let columns: Vec<i32> = columns
                .into_iter()
                .map(|column| values::column(column) as i32)
                .collect();
            pages(ctx, rpc::board::LIST_ITEMS, "items", |page| pb::ListItemsRequest {
                view: Some(view.clone()),
                columns: columns.clone(),
                page,
            })
            .await
            .map(Done::reply)
        }
        Some(BoardCommand::Add {
            title,
            column,
            description,
            assign,
            depends_on,
        }) => {
            let request = pb::CreateItemRequest {
                idempotency_key: ctx.key(),
                board: written_board(ctx, &which).await?,
                title,
                column: column.map_or(pb::BoardColumn::Unspecified, values::column) as i32,
                description: description.unwrap_or_default(),
                reason: String::new(),
                sessions: sessions(ctx, &assign).await?,
                depends_on: depends_on.into_iter().filter_map(item).collect(),
                ..Default::default()
            };
            ctx.change(rpc::board::CREATE_ITEM, &request).await
        }
        Some(BoardCommand::Edit {
            item: id,
            title,
            description,
            assign,
            unassign,
            depends_on,
            drop_depends_on,
            revision: read,
        }) => {
            let request = pb::UpdateItemRequest {
                idempotency_key: ctx.key(),
                id: item(id),
                patch: Some(pb::ItemPatch {
                    title,
                    description,
                    sessions_add: sessions(ctx, &assign).await?,
                    sessions_remove: sessions(ctx, &unassign).await?,
                    depends_on_add: depends_on.into_iter().filter_map(item).collect(),
                    depends_on_remove: drop_depends_on.into_iter().filter_map(item).collect(),
                    ..Default::default()
                }),
                expected_revision: revision(read),
                reason: String::new(),
            };
            ctx.change(rpc::board::UPDATE_ITEM, &request).await
        }
        Some(BoardCommand::Move {
            item: id,
            column,
            after,
            revision: read,
        }) => {
            let request = pb::MoveItemRequest {
                idempotency_key: ctx.key(),
                id: item(id),
                column: values::column(column) as i32,
                after: after.and_then(item),
                expected_revision: revision(read),
                reason: String::new(),
            };
            ctx.change(rpc::board::MOVE_ITEM, &request).await
        }
        Some(BoardCommand::Rm { item: id, revision: read }) => {
            let request = pb::DeleteItemRequest {
                idempotency_key: ctx.key(),
                id: item(id),
                expected_revision: revision(read),
                reason: String::new(),
            };
            ctx.change(rpc::board::DELETE_ITEM, &request).await
        }
        Some(BoardCommand::History { item: id }) => {
            let id = item(id);
            pages(ctx, rpc::board::GET_ITEM_HISTORY, "entries", |page| {
                pb::GetItemHistoryRequest {
                    id: id.clone(),
                    page,
                }
            })
            .await
            .map(Done::reply)
        }
    }
}

pub(super) async fn views<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    session: Option<String>,
) -> Result<Done, CliError> {
    let session = match session {
        Some(session) => Some(ctx.session(&session).await?),
        None => None,
    };
    pages(ctx, rpc::view::LIST_VIEWS, "views", |page| pb::ListViewsRequest {
        session: session.clone(),
        include_forks: false,
        page,
    })
    .await
    .map(Done::reply)
}

pub(super) async fn view<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    action: ViewAction,
    view: String,
) -> Result<Done, CliError> {
    let request = pb::SetViewStateRequest {
        idempotency_key: ctx.key(),
        id: Some(pb::ViewId { value: view }),
        state: values::view_state(action) as i32,
    };
    ctx.change(rpc::view::SET_VIEW_STATE, &request).await
}

/// `--against`: what the session's changes are compared with.
async fn diff_base<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    against: &str,
) -> Result<pb::DiffBase, CliError> {
    use pb::diff_base::Base;
    let base = match against {
        "main" => Base::OriginMain(pb::OriginMain {}),
        "uncommitted" => Base::UncommittedOnly(pb::UncommittedOnly {}),
        "last-reviewed" => Base::SinceLastReviewed(pb::SinceLastReviewed {}),
        "fork" => Base::SinceFork(pb::SinceFork {}),
        "pr" => Base::PullRequestBase(pb::PullRequestBase {}),
        // CLI.md names these, but DiffBase can't say them yet.
        "last-turn" | "turn" => {
            return Err(CliError::BadValue {
                field: "--against",
                value: against.into(),
                expected: "main, uncommitted, last-reviewed, fork, pr or a session: this \
                           daemon can't compare against a turn yet",
            });
        }
        session => Base::OtherSession(ctx.session(session).await?),
    };
    Ok(pb::DiffBase { base: Some(base) })
}

pub(super) async fn diff<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    session: Option<String>,
    repos: Vec<String>,
    against: String,
    command: Option<DiffCommand>,
) -> Result<Done, CliError> {
    match command {
        Some(DiffCommand::MarkReviewed { session, paths }) => {
            if let Some(path) = paths.first() {
                return Err(CliError::BadValue {
                    field: "path",
                    value: path.clone(),
                    expected: "no paths: this daemon marks the whole diff reviewed; run it \
                               without paths",
                });
            }
            let session = Some(ctx.session(&session).await?);
            let key = ctx.key();
            ctx.change_present(rpc::diff::MARK_REVIEWED, |presence| pb::MarkReviewedRequest {
                idempotency_key: key.clone(),
                session: session.clone(),
                presence,
                // The daemon snapshots what its last GetDiff showed.
                shown_tree: Vec::new(),
            }).await
        }
        None => {
            let Some(session) = session else {
                return Err(CliError::Missing { flag: "<SESSION>" });
            };
            let repos: Vec<pb::RepoRef> = repos
                .iter()
                .map(|repo| resolve::repo(repo))
                .collect::<Result<_, CliError>>()?;
            let request = pb::GetDiffRequest {
                session: Some(ctx.session(&session).await?),
                base: Some(diff_base(ctx, &against).await?),
                repos,
            };
            reply(ctx, rpc::diff::GET_DIFF, &request).await
        }
    }
}

pub(super) async fn comment<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: CommentCommand,
) -> Result<Done, CliError> {
    match command {
        CommentCommand::Add { session, at, text } => {
            let anchor = parse::anchor(&at)?;
            let request = pb::CreateCommentRequest {
                idempotency_key: ctx.key(),
                session: Some(ctx.session(&session).await?),
                anchor: Some(anchor),
                text,
                destination: Some(pb::CommentDestination {
                    destination: Some(pb::comment_destination::Destination::Terminal(
                        pb::SessionTerminal {},
                    )),
                }),
            };
            ctx.change(rpc::comment::CREATE_COMMENT, &request).await
        }
        CommentCommand::Send { session, ids } => {
            let request = pb::SendCommentsRequest {
                idempotency_key: ctx.key(),
                session: Some(ctx.session(&session).await?),
                ids: ids.into_iter().map(|value| pb::CommentId { value }).collect(),
                overall_note: String::new(),
            };
            ctx.change(rpc::comment::SEND_COMMENTS, &request).await
        }
        CommentCommand::Ls { session } => {
            let session = Some(ctx.session(&session).await?);
            comments(ctx, session, Vec::new()).await
        }
        CommentCommand::Rm { comment } => {
            let request = pb::DropCommentRequest {
                idempotency_key: ctx.key(),
                id: Some(pb::CommentId { value: comment }),
                expected_revision: None,
            };
            ctx.change(rpc::comment::DROP_COMMENT, &request).await
        }
    }
}

async fn comments<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    session: Option<pb::SessionId>,
    statuses: Vec<i32>,
) -> Result<Done, CliError> {
    pages(ctx, rpc::comment::LIST_COMMENTS, "comments", |page| {
        pb::ListCommentsRequest {
            session: session.clone(),
            statuses: statuses.clone(),
            file: None,
            page,
            archived: false,
        }
    })
    .await
    .map(Done::reply)
}

pub(super) async fn review<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: ReviewCommand,
) -> Result<Done, CliError> {
    match command {
        ReviewCommand::Start {
            prs,
            agent,
            message,
            allow_push,
        } => {
            let prs: Vec<pb::PullRequestRef> = prs
                .iter()
                .map(|pr| parse::pull_request(pr))
                .collect::<Result<_, CliError>>()?;
            let machine = Some(ctx.needs_machine().await?);
            let key = ctx.key();
            let agent = agent.map(|value| pb::AgentId { value });
            let first_message = message.unwrap_or_default();
            ctx.change_present(rpc::review::START_REVIEW, |presence| pb::StartReviewRequest {
                idempotency_key: key.clone(),
                prs: prs.clone(),
                machine: machine.clone(),
                agent: agent.clone(),
                model: String::new(),
                first_message: first_message.clone(),
                push_allowed: allow_push,
                presence,
            }).await
        }
        ReviewCommand::Drafts { session } => {
            let session = Some(ctx.session(&session).await?);
            comments(ctx, session, vec![pb::CommentStatus::Draft as i32]).await
        }
        ReviewCommand::Submit {
            session,
            verdict,
            summary,
        } => submit(ctx, &session, verdict, summary).await,
        ReviewCommand::AllowPush { session, state } => {
            let session = Some(ctx.session(&session).await?);
            let key = ctx.key();
            let allowed = values::on(state);
            ctx.change_present(rpc::review::SET_PUSH_ALLOWED, |presence| {
                pb::SetPushAllowedRequest {
                    idempotency_key: key.clone(),
                    session: session.clone(),
                    allowed,
                    presence,
                }
            }).await
        }
    }
}

/// Posts each pull request's drafts with one verdict. The plan lists the
/// drafts; with `--plan` its token is the one printed earlier.
async fn submit<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    session: &str,
    verdict: crate::args::Verdict,
    summary: Option<String>,
) -> Result<Done, CliError> {
    let session = Some(ctx.session(session).await?);
    let rpc_plan = rpc::review::PLAN_SUBMIT;
    let plan_request = pb::PlanSubmitRequest {
        session: session.clone(),
    };
    let bytes = ctx.raw(rpc_plan, &plan_request).await?;
    let plan: pb::SubmitPlan = crate::call::decode(rpc_plan, &bytes)?;
    let token = match &ctx.flags.plan {
        Some(value) => Some(pb::PlanToken {
            value: value.clone(),
        }),
        None => {
            let shown = crate::as_json::encoded(rpc_plan.response, &bytes);
            if ctx.flags.dry_run {
                return Ok(Done {
                    result: shown,
                    outcome: Outcome::Planned,
                });
            }
            ctx.confirm(&shown)?;
            plan.token.clone()
        }
    };
    let verdict = values::verdict(verdict) as i32;
    let reviews: Vec<pb::ReviewSubmission> = plan
        .pull_requests
        .into_iter()
        .map(|each| pb::ReviewSubmission {
            pr: each.pr,
            verdict,
            summary: summary.clone().unwrap_or(each.drafted_summary),
            drafts: each.drafts.into_iter().filter_map(|draft| draft.id).collect(),
        })
        .collect();
    let key = ctx.key();
    ctx.change_present(rpc::review::SUBMIT_REVIEW, |presence| pb::SubmitReviewRequest {
        idempotency_key: key.clone(),
        session: session.clone(),
        reviews: reviews.clone(),
        plan: token.clone(),
        presence,
    }).await
}
