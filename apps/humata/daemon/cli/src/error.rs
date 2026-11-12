use std::path::PathBuf;

use humata_wire::{ClientError, WireError};

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("say how to answer: --reply <text>, --approve, --move-to <status> or --dismiss")]
    NoAnswer,

    #[error(
        "{failed} sessions couldn't be restored, each printed with why; fix what it says and run the restore again, which leaves the restored ones alone"
    )]
    NotAllRestored { failed: usize },

    #[error(
        "there is no home folder to find humatad's socket in; set HOME, or pass --socket or set HUMATA_SOCKET"
    )]
    NoSocket,

    #[error(
        "couldn't reach humatad at {path}; start it with `humatad daemon`, or pass the socket it listens on with --socket"
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

    #[error("couldn't read the spec {path}; check the path, or pass - to read it from stdin")]
    ReadSpec {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("the project has no task or subtask {item:?}; list its items to find the id")]
    UnknownItem { item: String },

    #[error(
        "humatad stopped sending the session's events before it was idle; the message was sent, so read its history to see how far it got"
    )]
    WatchEnded,

    #[error("{given:?} isn't NAME=VALUE; give the variable's name, =, then its value")]
    NotAVariable { given: String },

    #[error("{name} isn't set in this shell; set it, or pass --set {name}=VALUE")]
    NoVariable { name: String },

    #[error(
        "terminal {terminal} didn't show {until:?} within {seconds} seconds; what it showed is above. Wait longer with --timeout, or check the command typed into it"
    )]
    TerminalTimedOut {
        terminal: String,
        until: String,
        seconds: u64,
    },

    #[error("host {host:?} can't be reached; see the problem above")]
    Unreachable { host: String },
}
