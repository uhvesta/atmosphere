//! What a Desk session looks like to the app: its folder, its switches, its
//! times and the work that holds its folder (DK-2, DK-11, DK-4, D182).

use crate::pb;
use humata2_sessions::doc_repo::{self, DocGit};
use humata2_sessions::{documents_under, repos_under};
use humata2_store::{DeskFolderKind, DeskRow, SessionRecord};

use crate::ports::timestamp_ms;

/// Seven days, the scratch folder's time after it closes (DK-11).
const SEVEN_DAYS_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// How long a closed scratch folder stays. A test daemon built with the
/// `admin` feature takes `HUMATA2_DESK_EXPIRE_MS` in milliseconds instead.
pub fn expiry_ms() -> i64 {
    if humata2_rpc_kit::admin::is_admin_mode()
        && let Some(ms) = std::env::var("HUMATA2_DESK_EXPIRE_MS")
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
    {
        return ms.max(0);
    }
    SEVEN_DAYS_MS
}

/// When a closed scratch folder comes due, whatever work it holds (D182.4).
/// None for a folder that stays: open, kept, picked or deleted.
pub fn expires_at(row: &DeskRow) -> Option<i64> {
    let closed = row.closed_unix_ms?;
    let scratch_to_go = matches!(row.kind, DeskFolderKind::Scratch { kept: false });
    (scratch_to_go && row.deleted_unix_ms.is_none()).then(|| closed.saturating_add(expiry_ms()))
}

/// When a closed scratch folder goes. Unset while the folder stays: open,
/// kept, picked, or holding work (DK-11, LD-17).
pub fn delete_at_ms(row: &DeskRow) -> Option<i64> {
    expires_at(row).filter(|_| row.held_work.is_empty())
}

/// Whether the folder is a versions repository and its newest version's id
/// (empty before the first). Runs one `git rev-parse` for a repository; a
/// failure is logged and shows as no version.
pub fn doc_state(folder: &std::path::Path) -> (bool, String) {
    if !doc_repo::is_doc_repo(folder) {
        return (false, String::new());
    }
    match doc_repo::head(&DocGit::inherit(), folder) {
        Ok(head) => (true, head.unwrap_or_default()),
        Err(error) => {
            humata2_rpc_kit::log::say!(
                "humatad2: couldn't read the newest version of {}: {error}",
                folder.display()
            );
            (true, String::new())
        }
    }
}

/// How many versions the folder holds (0 when it has no repository or no
/// version yet) and whether it holds a file outside its clones and `.git`.
/// A git failure is logged and shows as no versions.
pub fn folder_signals(folder: &std::path::Path) -> (u32, bool) {
    let count = if doc_repo::is_doc_repo(folder) {
        doc_repo::version_count(&DocGit::inherit(), folder).unwrap_or_else(|error| {
            humata2_rpc_kit::log::say!(
                "humatad2: couldn't count the versions of {}: {error}",
                folder.display()
            );
            0
        })
    } else {
        0
    };
    (count, holds_files(folder))
}

/// Whether `dir` holds a file anywhere under it, skipping `.git` and every
/// directory with a `.git` of its own (a clone). Stops at the first file;
/// an unreadable folder holds none.
fn holds_files(dir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_dir() {
            return true;
        }
        let path = entry.path();
        if entry.file_name() == ".git" || path.join(".git").exists() {
            continue;
        }
        if holds_files(&path) {
            return true;
        }
    }
    false
}

/// The `DeskSession` the app shows for `session` and its Desk row.
pub fn desk_pb(session: &SessionRecord, row: &DeskRow) -> pb::DeskSession {
    let kind = match row.kind {
        DeskFolderKind::Scratch { kept } => {
            pb::desk_folder::Kind::Scratch(pb::desk_folder::Scratch { kept })
        }
        DeskFolderKind::Picked => pb::desk_folder::Kind::Picked(pb::desk_folder::Picked {}),
    };
    let (has_versions, doc_head) = doc_state(&session.folder);
    let (version_count, holds_files) = folder_signals(&session.folder);
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
        channel_desk: None,
        doc_head,
        has_versions,
        version_count,
        holds_files,
        document_lines: std::collections::HashMap::new(),
    }
}
