//! Files, copies and moves between machines, and the trash (CLI.md §5.5).

mod placement;

use humata2_proto::humata::v1 as pb;

use super::{pages, streams, values};
use crate::args::{FilesCommand, OnExisting, TransferArgs, TransferCommand, TrashCommand};
use crate::call::{Ctx, Done};
use crate::console::Console;
use crate::daemon::{Daemon, DaemonError, rpc};
use crate::error::CliError;
use crate::location::is_bare_shared;
use crate::principal::Principal;

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
            ctx.change(rpc::files::CREATE_FOLDER, &request).await
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
            ctx.change(rpc::files::RENAME, &request).await
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
            ctx.change(rpc::files::MOVE_TO_TRASH, &request).await
        }
    }
}

/// `humata cp` and `humata mv`: every location but the last is a source.
/// A last word that is only a machine places each source by PL-3 (PL-12),
/// one transfer per place they land in.
pub(super) async fn transfer_start<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    args: TransferArgs,
    is_move: bool,
) -> Result<Done, CliError> {
    let bare_shared = match ctx.principal {
        Principal::Agent { .. } => args.locations.iter().find(|text| is_bare_shared(text)).cloned(),
        _ => None,
    };
    match transfer_start_inner(ctx, args, is_move).await {
        // D777: a wrong guess about a bare `shared…` must be fixable from the error.
        Err(CliError::Daemon(source))
            if bare_shared.is_some() && matches!(&source, DaemonError::Refused { error, .. } if matches!(error.kind, Some(pb::error::Kind::NotFound(_)))) =>
        {
            Err(CliError::SharedNotFound {
                typed: bare_shared.unwrap_or_default(),
                source,
            })
        }
        other => other,
    }
}

async fn transfer_start_inner<D: Daemon, C: Console>(
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
    let start = |sources: Vec<pb::FileLocation>,
                 destination: pb::FileLocation,
                 on_existing: OnExisting,
                 key: Option<pb::IdempotencyKey>| pb::StartTransferRequest {
        idempotency_key: key,
        sources,
        destination: Some(destination),
        mode: mode as i32,
        on_existing: values::on_existing(on_existing) as i32,
        reason: String::new(),
        name: String::new(),
    };
    let Some(target) = machine_only(ctx, destination).await? else {
        let on_existing = args.on_existing.unwrap_or(OnExisting::Fail);
        let place = ctx.location(destination).await?;
        let (place, name) = match from.len() {
            1 => landing_name(ctx, destination, place).await?,
            _ => (place, String::new()),
        };
        let request = pb::StartTransferRequest {
            name,
            ..start(from, place, on_existing, ctx.key())
        };
        return ctx.change(rpc::transfer::START_TRANSFER, &request).await;
    };
    // PL-13: a name taken where a machine-only send lands keeps both.
    let on_existing = args.on_existing.unwrap_or(OnExisting::KeepBoth);
    let known = placement::Known::fetch(ctx).await?;
    let mut groups: Vec<(pb::FileLocation, Vec<pb::FileLocation>)> = Vec::new();
    for source in from {
        let lands = placement::landing(ctx, &known, &source, &target).await?;
        match groups.iter_mut().find(|(place, _)| *place == lands) {
            Some((_, sources)) => sources.push(source),
            None => groups.push((lands, vec![source])),
        }
    }
    let mut started = Vec::with_capacity(groups.len());
    for (n, (lands, sources)) in groups.into_iter().enumerate() {
        // One key per transfer, so a retry of the command repeats each one.
        let key = ctx.key().map(|key| match n {
            0 => key,
            n => pb::IdempotencyKey {
                value: format!("{}/{n}", key.value),
            },
        });
        let request = start(sources, lands, on_existing, key);
        if ctx.flags.dry_run {
            // The plan keeps each transfer it would start, and starts none.
            started.push(ctx.change(rpc::transfer::START_TRANSFER, &request).await?.result);
            continue;
        }
        started.push(ctx.json(rpc::transfer::START_TRANSFER, &request).await?);
    }
    let shown = match started.len() {
        1 => started.remove(0),
        _ => serde_json::Value::Array(started),
    };
    if ctx.flags.dry_run {
        return Ok(Done {
            result: shown,
            outcome: crate::call::Outcome::Planned,
        });
    }
    Ok(Done::reply(shown))
}

/// Where one source lands, as plain `cp a b` reads `b` (D769): an
/// existing folder takes it under its own name; anything else, a file there
/// or nothing at all, is the new name in its parent folder. A destination
/// written with a trailing `/`, a root, or one whose look fails for another
/// reason is sent as a folder, and the daemon says what is wrong with it.
async fn landing_name<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    text: &str,
    place: pb::FileLocation,
) -> Result<(pb::FileLocation, String), CliError> {
    if text.ends_with('/') || place.path.is_empty() {
        return Ok((place, String::new()));
    }
    let looked: Result<pb::FileEntry, CliError> = ctx
        .call(
            rpc::files::STAT,
            &pb::StatRequest {
                location: Some(place.clone()),
            },
        )
        .await;
    let new_name = match looked {
        Ok(entry) => entry.kind == pb::file_entry::Kind::File as i32,
        Err(CliError::Daemon(DaemonError::Refused { error, .. })) => {
            matches!(error.kind, Some(pb::error::Kind::NotFound(_)))
        }
        Err(_) => false,
    };
    if !new_name {
        return Ok((place, String::new()));
    }
    let (parent, name) = match place.path.rsplit_once('/') {
        Some((parent, name)) => (parent.to_owned(), name.to_owned()),
        None => (String::new(), place.path.clone()),
    };
    Ok((pb::FileLocation { path: parent, ..place }, name))
}

/// The machine when `text` names only a machine: no `:` and no `/`. For the
/// person such a word is always a machine. For an agent (D756) it is a
/// machine it can see when one has that name; else a word with a `.` is a
/// file of its own clone, the word `shared` is the channel's shared folder on
/// its machine (D777), and any other word is refused, since a typo or a
/// machine outside the channel must not quietly become a file. `./name` and
/// `name/` mean the clone, except `shared/`.
async fn machine_only<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    text: &str,
) -> Result<Option<pb::MachineId>, CliError> {
    if text.is_empty() || text.contains(':') || text.contains('/') {
        return Ok(None);
    }
    match ctx.machine(text).await {
        Ok(machine) => Ok(Some(machine)),
        Err(CliError::NoMatch { .. }) if matches!(ctx.principal, Principal::Agent { .. }) => {
            if text.contains('.') || text == "shared" {
                Ok(None)
            } else {
                Err(CliError::NotAMachineHere { word: text.into() })
            }
        }
        Err(error) => Err(error),
    }
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
    ctx.change(rpc, &request).await
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
            ctx.change(rpc::trash::RESTORE_FROM_TRASH, &request).await
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
