use std::{net::SocketAddr, path::PathBuf};
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Could not {operation} terminal host state at {}. Check ownership, permissions and available disk space; preserve existing recovery files.",path.display())]
    Disk {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Terminal host state at {} could not be decoded. Preserve the file and restore a backup or use another private folder.",path.display())]
    Decode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "Terminal recovery checkpoint {id} is incompatible or damaged. Preserve it and start a new session, or restore a matching backup."
    )]
    Checkpoint { id: u64 },
    #[error("Recovery entry {} is not a regular file. Preserve it and restore its managed data from a backup, or choose another private state folder.",path.display())]
    RecoveryEntry { path: PathBuf },
    #[error("Recovery temporary entry {} must be a single-link regular file (regular={regular}, links={links}). Preserve any linked file and remove only this temporary entry, or choose another private state folder.",path.display())]
    RecoveryTemporary {
        path: PathBuf,
        regular: bool,
        links: u64,
    },
    #[error(
        "Terminal recovery checkpoint {id} could not be decompressed. Preserve it and restore a matching backup, or use another private host folder."
    )]
    CheckpointInflate {
        id: u64,
        #[source]
        source: flate2::DecompressError,
    },
    #[error(
        "Terminal {id}'s recovery checkpoint exceeds its {limit}-byte limit. Reduce retained history and retry; the previous checkpoint was preserved."
    )]
    CheckpointLimit { id: u64, limit: u64 },
    #[error(
        "Could not update terminal state. Retry the connection; the existing process was not replaced."
    )]
    Terminal {
        #[source]
        source: ghostty_vt::Error,
    },
    #[error(
        "Could not attach or control the existing terminal process. Check the host/SSH connection and retry; no replacement process was started."
    )]
    Session {
        #[source]
        source: terminal_session::Error,
    },
    #[error(
        "The selected checkpoint codec failed. Preserve existing recovery files; configure the codec recorded in the archive or correct the compression limits before retrying."
    )]
    Compression {
        #[source]
        source: terminal_codec::Error,
    },
    #[error(
        "Provider event for terminal {terminal} was rejected ({reason}). Use the current trusted binding and matching provider session ID; ordinary terminal output is not a provider hook."
    )]
    ProviderBinding { terminal: u64, reason: &'static str },
    #[error(
        "The structured provider event is invalid. Check the documented adapter schema and field limits before submitting it again."
    )]
    ProviderEvent {
        #[source]
        source: terminal_provider::Error,
    },
    #[error("Invalid terminal host configuration: {reason}.")]
    Invalid { reason: &'static str },
    #[error("Could not listen on {address}. Choose a free loopback port and try again.")]
    Listen {
        address: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "The terminal HTTP server stopped unexpectedly. Check the listener and restart the host; existing holders remain running."
    )]
    Serve {
        #[source]
        source: std::io::Error,
    },
    #[error(
        "Timed out settling terminal replies, resizes or input acknowledgements for sessions {sessions:?}. Their holders remain alive; a resumed host may preserve the last display read-only rather than retry uncertain control bytes. Use explicit provider recovery if that occurs."
    )]
    ShutdownPending {
        sessions: Vec<u64>,
        #[source]
        source: tokio::time::error::Elapsed,
    },
    #[error(
        "The terminal host is no longer available. Reconnect to the host before sending input."
    )]
    Closed,
}
impl From<terminal_session::Error> for Error {
    fn from(source: terminal_session::Error) -> Self {
        Self::Session { source }
    }
}
impl From<ghostty_vt::Error> for Error {
    fn from(source: ghostty_vt::Error) -> Self {
        Self::Terminal { source }
    }
}
pub(crate) fn disk(
    operation: &'static str,
    path: &std::path::Path,
    source: std::io::Error,
) -> Error {
    Error::Disk {
        operation,
        path: path.into(),
        source,
    }
}

impl From<terminal_codec::Error> for Error {
    fn from(source: terminal_codec::Error) -> Self {
        Self::Compression { source }
    }
}
