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
}

impl HostError {
    pub fn code(&self) -> pb::ErrorCode {
        match self {
            Self::Agent(AgentError::UnknownSession { .. }) => pb::ErrorCode::UnknownSession,
            Self::Agent(AgentError::UnknownEvent { .. }) => pb::ErrorCode::UnknownEvent,
            Self::Agent(AgentError::UnknownPermissionRequest { .. }) => {
                pb::ErrorCode::UnknownPermissionRequest
            }
            Self::Agent(_) => pb::ErrorCode::StorageFailed,
            Self::UnknownBackend { .. } => pb::ErrorCode::UnknownBackend,
            Self::NoWorkingDirectory => pb::ErrorCode::InvalidRequest,
            Self::FellBehind { .. } => pb::ErrorCode::BackendFailed,
        }
    }
}

/// What the daemon can fail at while reaching a host.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
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
            Self::UnknownHost { .. } | Self::NoCommand { .. } => pb::ErrorCode::InvalidRequest,
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

    #[error("there is no home folder for khodrd's files; set HOME, or pass --state and --socket")]
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
