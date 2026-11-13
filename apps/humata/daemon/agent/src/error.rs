use std::path::PathBuf;

use crate::event::{EventId, SessionId};

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error(
        "there is no session {session} on this host; list the sessions to find its id. If it ran on another machine, ask that machine's host instead"
    )]
    UnknownSession { session: SessionId },

    #[error(
        "session {session} is on this host already, so it can't be restored here; it may have come back on its own. List the sessions to check, or restore onto another host"
    )]
    SessionExists { session: SessionId },

    #[error(
        "session {session} works in the repository {name}, which the restore wasn't given a copy of; clone the project's repositories on the host and restore again"
    )]
    MissingRepository { session: SessionId, name: String },

    #[error(
        "a backup names the backend file {name}, which would be written outside the backend's folder; the backup is damaged or wasn't made by Humata, so it wasn't restored"
    )]
    BadBackendFile { name: String },

    #[error(
        "session {session} has no event {event} to fork before; read the session's history for the ids it has"
    )]
    UnknownEvent { session: SessionId, event: EventId },

    #[error(
        "event {event} of session {session} isn't a message the person sent, so there is nothing to rewind to before it; rewind to one of their messages"
    )]
    NotAMessage { session: SessionId, event: EventId },

    #[error(
        "session {session} has no rewind to undo: a message was sent since, or it was undone already. What was cut is still kept, hidden, until the session is deleted"
    )]
    NothingToUndo { session: SessionId },

    #[error(
        "session {session} works in the code of the session it was forked from, so rewinding its files would change that session's too; rewind the chat only, or rewind the other session"
    )]
    SharedFiles { session: SessionId },

    #[error(
        "session {session} isn't waiting on permission request {request}; it was answered already, or its turn was stopped"
    )]
    UnknownPermissionRequest { session: SessionId, request: String },

    #[error(
        "session {session} has no queued message {queued}; its turn has started already, or it was taken out"
    )]
    UnknownQueuedMessage { session: SessionId, queued: String },

    #[error(
        "session {session} was sent an image of type {media_type}, which the agents can't read; send it as PNG, JPEG, GIF or WebP"
    )]
    UnsupportedImage {
        session: SessionId,
        media_type: String,
    },

    #[error(
        "session {session} was sent an image of {bytes} bytes, more than the {max} the agents take; send a smaller or more compressed copy"
    )]
    ImageTooLarge {
        session: SessionId,
        bytes: usize,
        max: usize,
    },

    #[error(
        "session {session} has no image {name}; it may have been removed with the session's files, or the message it came with was rewound"
    )]
    UnknownImage { session: SessionId, name: String },

    #[error(
        "couldn't save an image to {path}; check that the disk has room and the folder is writable, then send it again"
    )]
    SaveImage {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't read the image {path}; check that the file is readable")]
    ReadImage {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "{id} isn't a session this host skipped; list the skipped sessions again, as it may have been removed already"
    )]
    UnknownSkipped { id: String },

    #[error(
        "couldn't remove the skipped session's folder {path}; check that it is writable, or delete it by hand. Part of it may be gone already, so it can't be opened either way"
    )]
    RemoveSkipped {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't create the folder {path} for session state; check that it is writable and the disk has room"
    )]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't read session state in {path}; check that the folder exists and is readable")]
    ReadDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't read {path}; check that it is readable")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't write {path}; check that it is writable and the disk has room")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the session history {path} is damaged at line {line}, so the session can't be opened. Fixing the line keeps the history; removing it loses that event, and removing the line and everything after it loses the rest of the session"
    )]
    CorruptHistory {
        path: PathBuf,
        line: usize,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "the session record {path} is damaged, so the session can't be opened. Removing its folder loses the session"
    )]
    CorruptRecord {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "what the backend keeps for a session, {path}, is damaged. Removing the file keeps the session's history, but the agent starts the next turn without remembering it"
    )]
    CorruptBackendState {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "{path} in session {session}'s folder isn't in one of its repositories, so its turns didn't save versions of it; only files in a repository have versions"
    )]
    NotSnapshotted { session: SessionId, path: String },

    #[error("couldn't make or update session {session}'s copy of its code")]
    Workspace {
        session: SessionId,
        #[source]
        source: humata_workspace::WorkspaceError,
    },

    #[error(
        "session {session} works in repositories, but this host has no folder for sessions' copies of them; start humatad as the daemon or host role, which sets one"
    )]
    NoWorkspaces { session: SessionId },

    #[error(
        "session {session} is archived, so it takes no messages; unarchive it to work in it again, and its code comes back as it was"
    )]
    Archived { session: SessionId },

    #[error(
        "session {session} is working, so it can't be archived now; stop its turn or wait for it to finish, then archive it"
    )]
    ArchiveWhileWorking { session: SessionId },

    #[error(
        "session {session} isn't archived, so it can't be deleted; archive it first, which keeps its code to bring back, then delete it"
    )]
    DeleteUnarchived { session: SessionId },

    #[error(
        "session {session} can't be deleted while its fork {fork} exists, as the fork's history starts from its code; delete the fork first"
    )]
    DeleteForked { session: SessionId, fork: SessionId },

    #[error(
        "couldn't delete {path} of session {session}; check that it is writable, then delete the session again. Part of it may be gone already"
    )]
    DeleteSession {
        session: SessionId,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "session {session} is working, so its changes can't be committed now; stop its turn or wait for it to finish, then land it"
    )]
    LandWhileWorking { session: SessionId },

    #[error(
        "session {session} is working, so its base branch can't be merged in now; stop its turn or wait for it to finish, then update it"
    )]
    UpdateWhileWorking { session: SessionId },

    #[error(
        "session {session} has no copy of code of its own to commit or update: it works in no repository, is archived, or is a fork of the chat working in its parent's copy. Land the parent, or unarchive this one"
    )]
    NoCodeToLand { session: SessionId },

    #[error(
        "there is no Claude Code or Copilot session {id} on this host to import; list the importable sessions to find its id"
    )]
    UnknownImport { id: String },

    #[error(
        "session {id} has no message from you, or doesn't say which folder it worked in, so there is nothing to carry on from; start a new session instead"
    )]
    NothingToImport { id: String },

    #[error(
        "session {id} was already imported, as session {session}; open that one. Importing it again would have two sessions carry on the same conversation"
    )]
    AlreadyImported { id: String, session: SessionId },

    #[error(
        "session {id} worked in {path}, which isn't a folder on this host any more; restore the folder, or start a new session where the code is now"
    )]
    ImportedFolderMissing { id: String, path: PathBuf },

    #[error(
        "only Claude Code and Copilot sessions can be imported; this backend keeps none of its own"
    )]
    NotImportable,

    #[error(transparent)]
    Copilot(Box<crate::CopilotError>),

    #[error(transparent)]
    ModelServer(Box<crate::LocalModelError>),

    #[error("couldn't encode a session event as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

impl From<crate::CopilotError> for AgentError {
    fn from(source: crate::CopilotError) -> Self {
        Self::Copilot(Box::new(source))
    }
}

impl From<crate::LocalModelError> for AgentError {
    fn from(source: crate::LocalModelError) -> Self {
        Self::ModelServer(Box::new(source))
    }
}
