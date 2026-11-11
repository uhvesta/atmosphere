use std::path::PathBuf;

use crate::event::{EventId, SessionId};

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error(
        "there is no session {session} on this host; list the sessions to find its id. If it ran on another machine, ask that machine's host instead"
    )]
    UnknownSession { session: SessionId },

    #[error(
        "session {session} has no event {event} to fork before; read the session's history for the ids it has"
    )]
    UnknownEvent { session: SessionId, event: EventId },

    #[error(
        "session {session} isn't waiting on permission request {request}; it was answered already, or its turn was stopped"
    )]
    UnknownPermissionRequest { session: SessionId, request: String },

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

    #[error("couldn't encode a session event as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}
