use std::path::PathBuf;

/// What starting, serving or talking to a holder can fail at.
#[derive(Debug, thiserror::Error)]
pub enum HolderError {
    #[error(
        "couldn't reach the terminal's holder at {}; the holder has stopped, so what ran in this tab is gone. Open a new tab",
        socket.display()
    )]
    Connect {
        socket: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the connection to the terminal's holder at {} broke; attach again from the last offset you have",
        socket.display()
    )]
    Lost {
        socket: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the terminal's holder at {} sent something this build can't read ({reason}); humatad2 and humata2_holder come from different builds. Reinstall Humata on this machine",
        socket.display()
    )]
    Protocol {
        socket: PathBuf,
        reason: &'static str,
    },

    #[error(
        "the terminal's holder at {} refused: {message}",
        socket.display()
    )]
    Refused { socket: PathBuf, message: String },

    #[error(
        "couldn't listen on {}; check that its folder belongs to you",
        socket.display()
    )]
    Bind {
        socket: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "another holder already answers on {}; attach to it instead. Stopping it ends the program in it",
        socket.display()
    )]
    AlreadyRunning { socket: PathBuf },

    #[error(
        "the socket path {} is longer than the {max} bytes Unix sockets allow; use a shorter state folder",
        socket.display()
    )]
    SocketPathTooLong { socket: PathBuf, max: usize },

    #[error("couldn't open a pseudo-terminal; the system may be out of them. Close some terminals and try again")]
    OpenPty {
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't start {} in {}; check that the program exists and the folder is still there",
        program.display(),
        cwd.display()
    )]
    StartProgram {
        program: PathBuf,
        cwd: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the key {key:?} is {len} bytes, more than the {max} a keyed input may have; use a shorter key"
    )]
    KeyTooLong { key: String, len: usize, max: usize },

    #[error(
        "{len} bytes is more than the {max} the holder types as one keyed input; send it in smaller parts"
    )]
    KeyedInputTooLarge { len: usize, max: usize },

    #[error("couldn't resize the terminal; the program in it may have ended")]
    Resize {
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't type into the terminal; the program in it may have ended")]
    Write {
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't run the holder {}; check that Humata is fully installed on this machine",
        holder.display()
    )]
    Launch {
        holder: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the holder {} stopped before it was ready; its log {} says why",
        holder.display(),
        log.display()
    )]
    StartFailed { holder: PathBuf, log: PathBuf },

    #[error(
        "the holder {} wasn't ready after {seconds} seconds and was stopped; its log {} may say why",
        holder.display(),
        log.display()
    )]
    StartTimeout {
        holder: PathBuf,
        log: PathBuf,
        seconds: u64,
    },

    #[error(
        "couldn't read or write {}, where the terminal's output is saved for after a restart; check that its folder belongs to you and the disk has room",
        path.display()
    )]
    Saved {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
