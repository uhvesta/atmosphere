use std::path::PathBuf;

/// Why the mirror or its index couldn't do what was asked.
#[derive(Debug, thiserror::Error)]
pub enum MirrorError {
    /// A conversation file couldn't be read, or a mirror file couldn't be
    /// read or written.
    #[error(
        "couldn't read or write {}; check that it is readable by this user, that its folder is writable and that the disk has free space. What was mirrored before is kept, and the next sync goes on from where it stopped",
        path.display()
    )]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The search index failed in a way a rebuild at open didn't cover.
    #[error(
        "the search index {} couldn't be read or written; check that the disk has free space. The index only holds copies: if this keeps happening, stop the host, delete that file and start the host again to have it rebuilt from the mirrored conversations",
        path.display()
    )]
    Database {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    /// A session's mirror, which is user data, doesn't read back as it was
    /// written.
    #[error(
        "the mirrored conversation in {} is damaged ({reason}). It is the host's copy of that session's history and can't be rebuilt from the search index: restore the session's mirror folder from a backup. Deleting the folder makes the next sync copy the conversation again, but only what the agent's own file on the session's machine still holds comes back",
        path.display()
    )]
    Corrupt { path: PathBuf, reason: String },

    /// The search text is empty or only white space.
    #[error("the search has no words; give at least one word to look for")]
    EmptyQuery,

    /// The page token isn't one this mirror gave out for this request.
    #[error(
        "the page token doesn't belong to this search or session; ask again without a token to start from the first page"
    )]
    BadPageToken,
}
