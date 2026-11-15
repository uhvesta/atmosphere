//! Files, copies and moves between machines, and the trash (CLI.md §5.5).

use humata2_proto::humata::v1 as pb;

use super::{pages, reply, streams, values};
use crate::args::{FilesCommand, TransferArgs, TransferCommand, TrashCommand};
use crate::call::{Ctx, Done};
use crate::console::Console;
use crate::daemon::{Daemon, rpc};
use crate::error::CliError;

pub(super) async fn files<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: FilesCommand,
) -> Result<Done, CliError> {
    match command {
        FilesCommand::Ls { location } => {
            let location = ctx.location(&location).await?;
            pages(ctx, rpc::files::LIST_DIRECTORY, "entries", |page| {
                pb::ListDirectoryRequest {
                    location: Some(location.clone()),
                    page,
                }
            })
            .await
            .map(Done::reply)
        }
        FilesCommand::Cat { location } => {
            let location = ctx.location(&location).await?;
            streams::cat(ctx, location).await
        }
        FilesCommand::Find { session, query } => {
            let session = Some(ctx.session(&session).await?);
            pages(ctx, rpc::files::FIND_FILES, "entries", |page| pb::FindFilesRequest {
                session: session.clone(),
                query: query.clone(),
                page,
            })
            .await
            .map(Done::reply)
        }
        FilesCommand::Put {
            local,
            location,
            replace,
        } => {
            let location = ctx.location(&location).await?;
            streams::put(ctx, &local, location, replace).await
        }
        FilesCommand::Mkdir { location } => {
            let request = pb::CreateFolderRequest {
                idempotency_key: ctx.key(),
                location: Some(ctx.location(&location).await?),
            };
            reply(ctx, rpc::files::CREATE_FOLDER, &request).await
        }
        FilesCommand::Mv { location, name } => {
            if name.is_empty() || name.contains('/') {
                return Err(CliError::BadValue {
                    field: "name",
                    value: name,
                    expected: "a new name without '/': `humata mv` moves between folders",
                });
            }
            let request = pb::RenameRequest {
                idempotency_key: ctx.key(),
                location: Some(ctx.location(&location).await?),
                new_name: name,
            };
            reply(ctx, rpc::files::RENAME, &request).await
        }
        FilesCommand::Rm { locations } => {
            let mut found = Vec::with_capacity(locations.len());
            for location in &locations {
                found.push(ctx.location(location).await?);
            }
            let request = pb::MoveToTrashRequest {
                idempotency_key: ctx.key(),
                locations: found,
            };
            reply(ctx, rpc::files::MOVE_TO_TRASH, &request).await
        }
    }
}

/// `humata cp` and `humata mv`: every location but the last is a source.
pub(super) async fn transfer_start<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    args: TransferArgs,
    is_move: bool,
) -> Result<Done, CliError> {
    let Some((destination, sources)) = args.locations.split_last() else {
        return Err(CliError::Missing {
            flag: "a source and a destination",
        });
    };
    if sources.is_empty() {
        return Err(CliError::Missing {
            flag: "a source and a destination",
        });
    }
    let mut from = Vec::with_capacity(sources.len());
    for source in sources {
        from.push(ctx.location(source).await?);
    }
    let mode = if is_move {
        pb::TransferMode::Move
    } else {
        pb::TransferMode::Copy
    };
    let request = pb::StartTransferRequest {
        idempotency_key: ctx.key(),
        sources: from,
        destination: Some(ctx.location(destination).await?),
        mode: mode as i32,
        on_existing: values::on_existing(args.on_existing) as i32,
        reason: String::new(),
    };
    reply(ctx, rpc::transfer::START_TRANSFER, &request).await
}

pub(super) async fn transfers<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    active: bool,
) -> Result<Done, CliError> {
    pages(ctx, rpc::transfer::LIST_TRANSFERS, "transfers", |page| {
        pb::ListTransfersRequest {
            active_only: active,
            page,
        }
    })
    .await
    .map(Done::reply)
}

pub(super) async fn transfer<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: TransferCommand,
) -> Result<Done, CliError> {
    let (rpc, id) = match command {
        TransferCommand::Pause { id } => (rpc::transfer::PAUSE_TRANSFER, id),
        TransferCommand::Resume { id } => (rpc::transfer::RESUME_TRANSFER, id),
        TransferCommand::Cancel { id } => (rpc::transfer::CANCEL_TRANSFER, id),
    };
    let request = pb::TransferControlRequest {
        idempotency_key: ctx.key(),
        id: Some(pb::TransferId { value: id }),
    };
    reply(ctx, rpc, &request).await
}

pub(super) async fn trash<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: Option<TrashCommand>,
) -> Result<Done, CliError> {
    match command {
        None => {
            let machine = Some(ctx.needs_machine().await?);
            pages(ctx, rpc::trash::LIST_TRASH, "items", |page| pb::ListTrashRequest {
                machine: machine.clone(),
                page,
            })
            .await
            .map(Done::reply)
        }
        Some(TrashCommand::Restore { item }) => {
            let request = pb::RestoreFromTrashRequest {
                idempotency_key: ctx.key(),
                id: Some(pb::TrashItemId { value: item }),
            };
            reply(ctx, rpc::trash::RESTORE_FROM_TRASH, &request).await
        }
        Some(TrashCommand::Empty) => {
            let machine = Some(ctx.needs_machine().await?);
            let key = ctx.key();
            ctx.planned(
                rpc::trash::PLAN_EMPTY_TRASH,
                &pb::PlanEmptyTrashRequest {
                    machine: machine.clone(),
                },
                |plan: &pb::EmptyTrashPlan| plan.token.clone(),
                move |plan| async move {
                    ctx.present_json(rpc::trash::EMPTY_TRASH, |presence| pb::EmptyTrashRequest {
                        idempotency_key: key.clone(),
                        machine: machine.clone(),
                        plan: plan.clone(),
                        presence,
                    })
                    .await
                },
            )
            .await
        }
    }
}
