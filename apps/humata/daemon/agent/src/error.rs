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
        "session {session} isn't waiting on permission request {request}; it was answered already, or its turn was stopped"
    )]
    UnknownPermissionRequest { session: SessionId, request: String },

    #[error(
        "session {session} has no queued message {queued}; its turn has started already, or it was taken out"
    )]
    UnknownQueuedMessage { session: SessionId, queued: String },

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
