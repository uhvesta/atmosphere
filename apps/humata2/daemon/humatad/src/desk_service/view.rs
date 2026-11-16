//! What a Desk session looks like to the app: its folder, its switches, its
//! times and the work that holds its folder (DK-2, DK-11, DK-4, D182).

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{documents_under, repos_under};
use humata2_store::{DeskFolderKind, DeskRow, SessionRecord};

use crate::channel_service::timestamp_ms;

/// Seven days, the scratch folder's time after it closes (DK-11).
const SEVEN_DAYS_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// How long a closed scratch folder stays. A test daemon built with the
/// `admin` feature takes `HUMATA2_DESK_EXPIRE_MS` in milliseconds instead.
pub(crate) fn expiry_ms() -> i64 {
    #[cfg(feature = "admin")]
    if let Some(ms) = std::env::var("HUMATA2_DESK_EXPIRE_MS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
    {
        return ms.max(0);
    }
    SEVEN_DAYS_MS
}

/// When a closed scratch folder comes due, whatever work it holds (D182.4).
/// None for a folder that stays: open, kept, picked or deleted.
pub(crate) fn expires_at(row: &DeskRow) -> Option<i64> {
    let closed = row.closed_unix_ms?;
    let scratch_to_go = matches!(row.kind, DeskFolderKind::Scratch { kept: false });
    (scratch_to_go && row.deleted_unix_ms.is_none()).then(|| closed.saturating_add(expiry_ms()))
}

/// When a closed scratch folder goes. Unset while the folder stays: open,
/// kept, picked, or holding work (DK-11, LD-17).
pub(crate) fn delete_at_ms(row: &DeskRow) -> Option<i64> {
    expires_at(row).filter(|_| row.held_work.is_empty())
}

/// The `DeskSession` the app shows for `session` and its Desk row.
pub(crate) fn desk_pb(session: &SessionRecord, row: &DeskRow) -> pb::DeskSession {
    let kind = match row.kind {
        DeskFolderKind::Scratch { kept } => {
            pb::desk_folder::Kind::Scratch(pb::desk_folder::Scratch { kept })
        }
        DeskFolderKind::Picked => pb::desk_folder::Kind::Picked(pb::desk_folder::Picked {}),
    };
    pb::DeskSession {
        session: Some(pb::SessionId {
            value: session.id.clone(),
        }),
        folder: Some(pb::DeskFolder {
            path: session.folder.to_string_lossy().into_owned(),
            kind: Some(kind),
        }),
        access: Some(pb::DeskAccess {
            boards: row.boards,
            channels_and_machines: row.channels_and_machines,
        }),
        last_used_at: Some(timestamp_ms(row.last_used_unix_ms)),
        closed_at: row.closed_unix_ms.map(timestamp_ms),
        delete_at: delete_at_ms(row).map(timestamp_ms),
        held_work: row.held_work.clone(),
        repos: repos_under(&session.folder)
            .into_iter()
            .map(|(owner, name)| pb::RepoRef { owner, name })
            .collect(),
        documents: documents_under(&session.folder),
    }
}
