use std::path::PathBuf;

/// What starting the daemon, or bridging to it, can fail at.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DaemonError {
    #[error("HOME isn't set, so humatad2 can't find its folder; set HOME, or pass --socket and --state")]
    NoHome,

    #[error("couldn't start humatad2's runtime; the system may be out of threads or memory")]
    Runtime {
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't create {}; check that its parent exists and belongs to you",
        path.display()
    )]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "{} can be opened by other users (mode {mode:o}), so the daemon's socket in it would be too; run `chmod 700` on it, or pass a --socket in a folder only you can open",
        path.display()
    )]
    FolderOpenToOthers { path: PathBuf, mode: u32 },

    #[error(
        "couldn't read this machine's id from {}; restore it from a backup if you have one. Deleting it makes the host see this machine as a new one",
        path.display()
    )]
    MachineId {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't open humatad2's state")]
    Store {
        #[source]
        source: humata2_store::StoreError,
    },

    #[error(
        "the holder {} isn't there; reinstall Humata on this machine, or pass --holder",
        path.display()
    )]
    NoHolder { path: PathBuf },

    #[error(
        "another humatad2 already answers on {}; use it, or stop it first. Stopping it leaves its terminals running",
        socket.display()
    )]
    AlreadyRunning { socket: PathBuf },

    #[error(
        "the socket path {} is longer than the {max} bytes Unix sockets allow; pass a shorter --socket",
        socket.display()
    )]
    SocketPathTooLong { socket: PathBuf, max: usize },

    #[error(
        "couldn't listen on {}; check that its folder belongs to you",
        socket.display()
    )]
    Bind {
        socket: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't wait for a stop signal")]
    Signals {
        #[source]
        source: std::io::Error,
    },

    #[error("the gRPC server stopped")]
    Serve {
        #[source]
        source: tonic::transport::Error,
    },

    #[error(
        "couldn't reach humatad2 at {}; start it with `humatad2 serve` on this machine",
        socket.display()
    )]
    BridgeConnect {
        socket: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("the bridge to humatad2 broke; reconnect")]
    BridgeCopy {
        #[source]
        source: std::io::Error,
    },
}
