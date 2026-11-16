use std::path::PathBuf;

/// What reading or writing Humata's state on disk can fail at.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(
        "couldn't create Humata's state folder {}; check that its parent exists and belongs to you",
        path.display()
    )]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't save {}; check that the disk has free space and the folder is writable. The copy saved before, if any, is untouched",
        path.display()
    )]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't read {}; check that it belongs to you and that the disk is mounted",
        path.display()
    )]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't remove {}; check that the folder belongs to you. Nothing else was changed",
        path.display()
    )]
    Remove {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't open Humata's state database {}; check that it belongs to you, that the disk is mounted and that no other program holds it open for writing",
        path.display()
    )]
    Open {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    #[error(
        "couldn't update Humata's state database {} from schema {from} to {to}; it is unchanged, so the humatad2 that wrote it still reads it. Free some disk space and start humatad2 again",
        path.display()
    )]
    Migrate {
        path: PathBuf,
        from: i64,
        to: i64,
        #[source]
        source: rusqlite::Error,
    },

    #[error(
        "the state in {} was written by a newer Humata (schema {found}; this humatad2 reads up to {supported}). Update humatad2 on this machine; don't delete the database, which would lose every session in it",
        path.display()
    )]
    SchemaTooNew {
        path: PathBuf,
        found: i64,
        supported: i64,
    },

    #[error(
        "couldn't {doing} in Humata's state database {}; nothing was changed. If the disk is full, free some space and try again",
        path.display()
    )]
    Database {
        path: PathBuf,
        doing: &'static str,
        #[source]
        source: rusqlite::Error,
    },

    #[error(
        "{what} in {} is damaged ({reason}); restore the state folder from a backup if you have one. Deleting the damaged entry loses what it held",
        path.display()
    )]
    Damaged {
        path: PathBuf,
        what: String,
        reason: &'static str,
    },

    #[error(
        "{what} in {} was written by a newer Humata, in a format or codec this build doesn't read. Update Humata on this machine to read it; don't delete the file or the state folder, since nothing else holds what it does",
        path.display()
    )]
    NewerFormat {
        path: PathBuf,
        what: String,
        #[source]
        source: humata2_chunk::FrameError,
    },

    #[error(
        "{what} in {} is damaged: its saved bytes don't decode. Restore the state folder from a backup if you have one; deleting the damaged entry loses what it held",
        path.display()
    )]
    Undecodable {
        path: PathBuf,
        what: String,
        #[source]
        source: humata2_chunk::FrameError,
    },

    #[error(
        "couldn't compress {what} to save it; nothing was changed. This is a bug in Humata: report it, with the error below"
    )]
    Encode {
        what: &'static str,
        #[source]
        source: humata2_chunk::FrameError,
    },

    #[error(
        "couldn't save a last good copy of the state at {}: the new copy failed its check ({problem}), so the one before it is kept. The database itself may be damaged too: run `PRAGMA integrity_check` on it, and don't delete the copy kept",
        path.display()
    )]
    Snapshot { path: PathBuf, problem: String },

    #[error(
        "the state database at {} is damaged ({damage}) and there's no last good copy to restore it from. Restore the state folder from a backup if you have one; deleting the database loses every session, tab and agent record on this machine",
        path.display()
    )]
    DamagedWithoutCopy { path: PathBuf, damage: String },

    #[error(
        "{id:?} can't be used as an id: ids are 1 to 128 letters, digits, '-', '_' or '.', and don't start with '.'"
    )]
    InvalidId { id: String },

    #[error(
        "no session {id} is saved on this machine; it was removed, or belongs to another machine"
    )]
    NoSession { id: String },

    #[error("no channel {id} is saved on this machine; it was removed, or was never saved here")]
    NoChannel { id: String },

    #[error(
        "channel {id} is at revision {actual}, not {expected} as the change was made against; read the channel again and send the change again"
    )]
    StaleChannel {
        id: String,
        expected: u64,
        actual: u64,
    },

    #[error(
        "channel {channel} has no repository {owner}/{name} saved; add it to the channel first"
    )]
    NoChannelRepo {
        channel: String,
        owner: String,
        name: String,
    },

    #[error(
        "{} holds no deleted channel, so there is nothing to bring back from it. If it came from the trash, restore the item again; don't delete the file, which holds the channel's sessions and board",
        path.display()
    )]
    NoDeletedChannel { path: PathBuf },

    #[error("no board item {key} is saved on this machine; it was never made here, or the key is mistyped")]
    NoBoardItem { key: String },

    #[error("{owner} has no board prefix yet; choose and claim one before making its first item")]
    NoBoardPrefix { owner: String },

    #[error("{column} is not a board column; columns are numbered 1 (Backlog) to 5 (Done)")]
    InvalidBoardColumn { column: i32 },
}

impl StoreError {
    /// True when a write failed because the disk is full, so the caller
    /// can say so in its own terms.
    pub fn is_disk_full(&self) -> bool {
        match self {
            Self::Write { source, .. } => source.kind() == std::io::ErrorKind::StorageFull,
            Self::Database { source, .. }
            | Self::Open { source, .. }
            | Self::Migrate { source, .. } => {
                source.sqlite_error_code() == Some(rusqlite::ErrorCode::DiskFull)
            }
            _ => false,
        }
    }
}
