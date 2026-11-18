use std::path::{Path, PathBuf};

use humata2_store::StoreError;

use crate::engine::Problem;

/// What the Humata trash can fail at.
#[derive(Debug, thiserror::Error)]
pub enum TrashError {
    #[error(
        "couldn't {doing} {}; check that the folder belongs to you and the disk isn't full or read-only. Nothing else was changed",
        path.display()
    )]
    Io {
        doing: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't save a trash record in {}; check that the disk has free space. The file was not moved",
        path.display()
    )]
    Record {
        path: PathBuf,
        #[source]
        source: StoreError,
    },

    #[error(
        "couldn't read or write the trash record {}; it may be damaged. Remove that item's folder from the trash by hand if it's not needed",
        path.display()
    )]
    Format {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "{} is on another disk than Humata's folder, so it can't be moved to Humata's trash. Delete it with the system's own trash or copy it to this disk first",
        path.display()
    )]
    OtherDisk {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("there is nothing at {} to move to the trash", path.display())]
    Missing { path: PathBuf },

    #[error(
        "something already sits at {}, so the trash item wasn't restored over it. Rename or move what is there, then restore again; the item stays in the trash",
        path.display()
    )]
    Occupied { path: PathBuf },

    #[error("the trash has no item {id}; it may have been restored or emptied already")]
    NoSuchItem { id: String },
}

/// What one end of a transfer (a machine's files) can fail at.
#[derive(Debug, thiserror::Error)]
pub enum EndError {
    #[error(
        "couldn't {doing} {}; check that it exists, belongs to you and the disk isn't full. Nothing else was changed",
        path.display()
    )]
    Io {
        doing: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{} doesn't exist; check that it wasn't moved or deleted", path.display())]
    NotFound {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{} already exists; pick another name or choose to replace it", path.display())]
    Exists { path: PathBuf },

    #[error(
        "{} is a {what}, which can't be transferred; leave it out and transfer the rest",
        path.display()
    )]
    Unsupported { path: PathBuf, what: &'static str },

    #[error("couldn't move {} to the trash", path.display())]
    Trash {
        path: PathBuf,
        #[source]
        source: TrashError,
    },

    /// The machine holding this end can't be reached right now. Not a
    /// failure of the transfer: it continues once the link is back (FM-8,
    /// FM-13).
    #[error(
        "{machine} can't be reached right now; the transfer continues by itself when its link is back"
    )]
    Offline { machine: String },
}

impl EndError {
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Io { path, .. }
            | Self::NotFound { path, .. }
            | Self::Exists { path }
            | Self::Unsupported { path, .. }
            | Self::Trash { path, .. } => path,
            Self::Offline { .. } => Path::new(""),
        }
    }

    pub(crate) fn doing(&self) -> &'static str {
        match self {
            Self::Io { doing, .. } => doing,
            Self::NotFound { .. } => "find",
            Self::Exists { .. } => "create",
            Self::Unsupported { .. } => "transfer",
            Self::Trash { .. } => "move to the trash",
            Self::Offline { .. } => "reach the machine",
        }
    }
}

/// What starting or driving a transfer can fail at.
#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error(
        "couldn't save the transfer journal {}; check that the state folder has free space. The transfer was not changed on disk",
        path.display()
    )]
    Journal {
        path: PathBuf,
        #[source]
        source: StoreError,
    },

    #[error("couldn't encode the transfer journal {}", path.display())]
    JournalFormat {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "couldn't read the transfer journals in {}; check that the folder belongs to you",
        path.display()
    )]
    JournalRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't start a thread for transfer {id}; close other programs and resume it")]
    Spawn {
        id: String,
        #[source]
        source: std::io::Error,
    },

    #[error("there is no transfer {id}; it may have finished long ago and been forgotten")]
    Unknown { id: String },

    #[error("transfer {id} already exists; use another id")]
    Duplicate { id: String },

    #[error("{id} isn't a usable transfer id; use letters, digits, '-' and '_' only")]
    BadId { id: String },

    #[error("{name:?} isn't a usable file name; give a plain name without '/'")]
    BadName { name: String },

    #[error("{path} has no name to copy under; pick the item itself, not the root")]
    BadSource { path: String },

    #[error("a transfer needs at least one source; pick something to copy or move")]
    NoSources,

    #[error("a new name is for one source, and {count} were given; they can't all land under it")]
    NameNeedsOneSource { count: usize },

    #[error(
        "transfer {id} is too far along to change: it is finished or already putting files in place"
    )]
    TooLate { id: String },

    #[error("transfer {id} is an upload; resume it by starting the upload again with its id")]
    IsUpload { id: String },

    #[error("transfer {id} isn't an upload that can be resumed now")]
    NotResumable { id: String },

    #[error("transfer {id} is already running")]
    AlreadyRunning { id: String },

    #[error("upload {id} expected bytes from offset {expected}, not {got}; send from {expected}")]
    WrongOffset { id: String, expected: u64, got: u64 },

    #[error("upload {id} was sent more bytes than the {size} announced; start over with the right size")]
    Overrun { id: String, size: u64 },

    #[error("upload {id} has {offset} of {size} bytes; send the rest from {offset} before finishing")]
    Incomplete { id: String, offset: u64, size: u64 },

    #[error("upload {id} was paused or interrupted; resume it from its confirmed offset")]
    Interrupted { id: String },

    #[error("upload {id} was cancelled")]
    Cancelled { id: String },

    #[error("transfer {id} can't go ahead: {problem:?}")]
    Refused { id: String, problem: Problem },

    #[error("transfer {id} failed on a file")]
    End {
        id: String,
        #[source]
        source: EndError,
    },
}
