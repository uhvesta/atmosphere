use std::path::PathBuf;

use khodr_agent::AgentError;
use khodr_wire::{ClientError, WireError, pb};

/// What the host role can fail at, answered to the daemon as an error.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error(transparent)]
    Agent(#[from] AgentError),

    #[error(
        "this host doesn't run the {backend} backend; pick one it has, or update khodrd on this host"
    )]
    UnknownBackend { backend: &'static str },

    #[error(
        "a session needs a working directory; give the folder on the host the agent should work in"
    )]
    NoWorkingDirectory,

    #[error(
        "the watcher fell {missed} events behind and stopped; watch the session again to catch up from its history"
    )]
    FellBehind { missed: u64 },

    #[error(
        "only the daemon keeps projects and the board; send this to khodrd's daemon on your Mac, not a host"
    )]
    NotForHosts,
}

impl HostError {
    pub fn code(&self) -> pb::ErrorCode {
        match self {
            Self::Agent(AgentError::UnknownSession { .. }) => pb::ErrorCode::UnknownSession,
            Self::Agent(AgentError::UnknownEvent { .. }) => pb::ErrorCode::UnknownEvent,
            Self::Agent(AgentError::UnknownPermissionRequest { .. }) => {
                pb::ErrorCode::UnknownPermissionRequest
            }
            Self::Agent(AgentError::Archived { .. }) => pb::ErrorCode::Archived,
            Self::Agent(AgentError::ArchiveWhileWorking { .. }) => pb::ErrorCode::Busy,
            Self::Agent(AgentError::NoWorkspaces { .. }) => pb::ErrorCode::InvalidRequest,
            Self::Agent(AgentError::Workspace { .. }) => pb::ErrorCode::BackendFailed,
            Self::Agent(_) => pb::ErrorCode::StorageFailed,
            Self::UnknownBackend { .. } => pb::ErrorCode::UnknownBackend,
            Self::NoWorkingDirectory | Self::NotForHosts => pb::ErrorCode::InvalidRequest,
            Self::FellBehind { .. } => pb::ErrorCode::BackendFailed,
        }
    }
}

/// What the daemon's own store can fail at.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("couldn't create the folder {path} for khodrd's store; check that it is writable")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't open khodrd's store {path}; check that it is readable and writable and the disk isn't full"
    )]
    Open {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    #[error(
        "couldn't update khodrd's store {path} to this version; it is unchanged, so an older khodrd still reads it. Free some disk space and start khodrd again"
    )]
    Migrate {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    #[error(
        "khodrd's store {path} was written by a newer khodrd (schema {found}, this one knows up to {known}); update Khodr. Deleting the store loses your projects"
    )]
    NewerSchema {
        path: PathBuf,
        found: i64,
        known: i64,
    },

    #[error("khodrd's store {path} failed; if the disk is full, free some space and try again")]
    Query {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    #[error("there is no project {id:?}; list projects to find its id")]
    UnknownProject { id: String },

    #[error("there is no task or subtask {id:?}; list the project's items to find its id")]
    UnknownItem { id: String },

    #[error(
        "khodrd has never seen session {id:?}; list sessions to find its id, or check that its host is reachable"
    )]
    UnknownSession { id: String },

    #[error("there is no board change {id}; read the project's board log to find its number")]
    UnknownChange { id: u64 },

    #[error("a task or subtask needs a title; give it one")]
    NoTitle,

    #[error(
        "{title:?} is a subtask, and a subtask can't have its own; add this under its task instead"
    )]
    NestedSubtask { title: String },

    #[error("the task {title:?} is in another project; add the subtask in that project")]
    ParentInAnotherProject { title: String },

    #[error(
        "{title:?} is in another project than the session; pick an item of the session's own project"
    )]
    ItemInAnotherProject { title: String },

    #[error(
        "can't undo adding {title:?}: {subtasks} subtasks and {sessions} sessions are on it now. Move it to Won't do instead, or remove its subtasks and move its sessions first"
    )]
    ItemInUse {
        title: String,
        subtasks: i64,
        sessions: i64,
    },

    #[error(
        "can't bring back {title:?}: its task was removed since. Revert the change that removed the task first"
    )]
    ParentRemoved { title: String },
}

/// What the daemon can fail at while reaching a host.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error(transparent)]
    Store(#[from] StoreError),

    #[error(
        "a project needs at least one repository; give the folder of each its sessions work in"
    )]
    NoRepositories,

    #[error(
        "the repository {repository} must be an absolute path to a folder on this Mac; check that it exists"
    )]
    NotAFolder {
        repository: PathBuf,
        #[source]
        source: Option<std::io::Error>,
    },

    #[error(
        "project {project:?} has its repository on this Mac, so a session on host {host:?} needs the folder to work in there; give it one"
    )]
    NoRemoteDirectory { project: String, host: String },

    #[error(
        "there is no host named {host:?}; add it to {hosts}, or leave the host empty for this Mac"
    )]
    UnknownHost { host: String, hosts: PathBuf },

    #[error("host {host:?} has no command to reach it; give it one in {hosts}")]
    NoCommand { host: String, hosts: PathBuf },

    #[error(
        "couldn't run {program:?} to reach host {host:?}; check that it is installed and on the PATH"
    )]
    Spawn {
        host: String,
        program: String,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't reach host {host:?}{}", said(.stderr))]
    Connect {
        host: String,
        stderr: String,
        #[source]
        source: WireError,
    },

    #[error("host {host:?} didn't answer")]
    Host {
        host: String,
        #[source]
        source: ClientError,
    },
}

fn said(stderr: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        String::new()
    } else {
        format!(" (it said: {stderr})")
    }
}

impl DaemonError {
    pub fn code(&self) -> pb::ErrorCode {
        match self {
            Self::Store(StoreError::UnknownProject { .. }) => pb::ErrorCode::UnknownProject,
            Self::Store(StoreError::UnknownItem { .. }) => pb::ErrorCode::UnknownItem,
            Self::Store(StoreError::UnknownSession { .. }) => pb::ErrorCode::UnknownSession,
            Self::Store(StoreError::UnknownChange { .. }) => pb::ErrorCode::UnknownBoardChange,
            Self::Store(
                StoreError::NoTitle
                | StoreError::NestedSubtask { .. }
                | StoreError::ParentInAnotherProject { .. }
                | StoreError::ItemInAnotherProject { .. }
                | StoreError::ItemInUse { .. }
                | StoreError::ParentRemoved { .. },
            ) => pb::ErrorCode::InvalidRequest,
            Self::Store(_) => pb::ErrorCode::StorageFailed,
            Self::UnknownHost { .. }
            | Self::NoCommand { .. }
            | Self::NotAFolder { .. }
            | Self::NoRepositories
            | Self::NoRemoteDirectory { .. } => pb::ErrorCode::InvalidRequest,
            Self::Spawn { .. } | Self::Connect { .. } => pb::ErrorCode::HostUnavailable,
            Self::Host {
                source: ClientError::Refused { code, .. },
                ..
            } => *code,
            Self::Host { .. } => pb::ErrorCode::HostUnavailable,
        }
    }
}

/// What stops khodrd from starting.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    Host(#[from] HostError),

    #[error(transparent)]
    Store(#[from] StoreError),

    #[error(
        "there is no home folder for khodrd's files; set HOME, or pass --state, --cache and --socket"
    )]
    NoHome,

    #[error("couldn't read the hosts file {path}; check that it is readable")]
    ReadHosts {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("the hosts file {path} isn't valid; fix its JSON, or remove it to use only this Mac")]
    BadHosts {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("couldn't create the folder {path} for khodrd's socket; check that it is writable")]
    CreateSocketDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "khodrd is already running on {path}; use that one, or stop it first. If it is a different user's, set KHODR_SOCKET to another path"
    )]
    AlreadyRunning { path: PathBuf },

    #[error("couldn't remove the stale socket {path}; delete it by hand once no khodrd is running")]
    RemoveStaleSocket {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't listen on {path}; check that its folder is writable and the path is short enough for a socket"
    )]
    Listen {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't make the socket {path} private to you; check that you own it")]
    ProtectSocket {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't accept a connection on the socket")]
    Accept {
        #[source]
        source: std::io::Error,
    },

    #[error("the connection on stdin and stdout failed")]
    Stdio {
        #[source]
        source: WireError,
    },
}
