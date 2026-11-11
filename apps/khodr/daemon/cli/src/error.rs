use std::path::PathBuf;

use khodr_wire::{ClientError, WireError};

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error(
        "there is no home folder to find khodrd's socket in; set HOME, or pass --socket or set KHODR_SOCKET"
    )]
    NoSocket,

    #[error(
        "couldn't reach khodrd at {path}; start it with `khodrd daemon`, or pass the socket it listens on with --socket"
    )]
    Connect {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(transparent)]
    Handshake(#[from] WireError),

    #[error(transparent)]
    Wire(#[from] ClientError),

    #[error("couldn't read the backend config {path}; check the path")]
    ReadBackendConfig {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't resolve {path} from the current folder; give an absolute path")]
    Resolve {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "khodrd stopped sending the session's events before it was idle; the message was sent, so read its history to see how far it got"
    )]
    WatchEnded,
}
