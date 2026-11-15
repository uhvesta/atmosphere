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
        "{} isn't a record Humata can read; it may have been edited by hand. Restore it from a backup if you have one: moving it aside makes Humata forget what it held",
        path.display()
    )]
    Corrupt {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "journal entry {} is damaged ({reason}); restore it from a backup if you have one. Deleting it loses that write, and the entries after it still replay",
        path.display()
    )]
    JournalEntryDamaged { path: PathBuf, reason: &'static str },

    #[error(
        "the state in {} was written by a newer Humata (schema {found}; this humatad2 reads up to {supported}). Update humatad2 on this machine; don't delete the folder, which would lose every session in it",
        path.display()
    )]
    SchemaTooNew {
        path: PathBuf,
        found: u32,
        supported: u32,
    },

    #[error(
        "{id:?} can't be used as an id: ids are 1 to 128 letters, digits, '-', '_' or '.', and don't start with '.'"
    )]
    InvalidId { id: String },

    #[error("no session {id} is saved on this machine; it was removed, or belongs to another machine")]
    NoSession { id: String },
}

impl StoreError {
    /// True when a write failed because the disk is full, so the caller
    /// can say so in its own terms.
    pub fn is_disk_full(&self) -> bool {
        matches!(
            self,
            Self::Write { source, .. } if source.kind() == std::io::ErrorKind::StorageFull
        )
    }
}
