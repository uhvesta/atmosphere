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
        "the terminal's holder at {} is from an older Humata build, which can't take typing the way this build sends it, so nothing was typed; the holder and the program in it keep running. Click Migrate on the \"Migrate the holders\" item in the Inbox: it restarts agent sessions onto the current holder at their next idle moment and keeps their conversations. A shell tab is never migrated; close it and open a new one",
        socket.display()
    )]
    Outdated { socket: PathBuf },

    #[error(
        "the terminal's holder at {} refused: {message}",
        socket.display()
    )]
    Refused { socket: PathBuf, message: String },

    #[error(
        "the terminal's holder at {} is from a build too old to move onto a new one in place; the holder and the program in it keep running. Move its session with the \"Migrate the holders\" item in the Inbox instead",
        socket.display()
    )]
    TooOldToMove { socket: PathBuf },

    #[error(
        "the terminal's holder at {} is from a build too old to pause its program for a move; the holder and the program in it keep running and nothing was carried. Click Migrate on the \"Migrate the holders\" item in the Inbox, then move the session again",
        socket.display()
    )]
    TooOldToFreeze { socket: PathBuf },

    #[error(
        "the terminal's holder at {} is paused for a move, so nothing was typed or moved; the program is held until the move finishes or is undone. Wait for the move, or cancel it to let the program run again",
        socket.display()
    )]
    Frozen { socket: PathBuf },

    #[error(
        "couldn't list the running processes with `ps` to pause the program under the holder at {}: {source}; the program was not paused. Check that `ps` runs for this user and try again",
        socket.display()
    )]
    Freeze {
        socket: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the terminal's holder at {} started moving onto the new build and didn't answer again within the time allowed, so the move is lost. The program in the tab may still be running, or may be gone with the holder; its output up to the move is saved beside the socket, so open the tab again to see what is left of it",
        socket.display()
    )]
    LostInMove { socket: PathBuf },

    #[error(
        "the handoff {} can't be used; the holder can't take over from it, and the handoff is kept for you to look at",
        path.display()
    )]
    Handoff {
        path: PathBuf,
        #[source]
        source: crate::handoff::HandoffError,
    },

    #[error("couldn't take over {what} from the holder that moved onto this build; the terminal is lost, so open a new tab")]
    Adopt {
        what: &'static str,
        #[source]
        source: std::io::Error,
    },

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
        "the holder answering at {} isn't the one its tab names: it was just started and the tab isn't saved with it yet. Try again in a moment; if it keeps happening, restart humatad2, which saves each tab with the holder that answers",
        socket.display()
    )]
    NotSaved { socket: PathBuf },

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
        "the holder {} didn't begin running within {seconds} seconds of being launched and was stopped. The machine is likely too busy to start programs; try again once it is less loaded. Its log {} may say more",
        holder.display(),
        log.display()
    )]
    LaunchTimeout {
        holder: PathBuf,
        log: PathBuf,
        seconds: u64,
    },

    #[error(
        "the holder {} was running but wasn't ready after {seconds} seconds and was stopped; its log {} may say why",
        holder.display(),
        log.display()
    )]
    StartTimeout {
        holder: PathBuf,
        log: PathBuf,
        seconds: u64,
    },

    #[error(
        "the terminal's holder at {} didn't answer within {seconds} seconds, so it may be stopped or wedged. End its process by the pid in the .pid file beside its socket; that also stops the program running in its tab",
        socket.display()
    )]
    ShutdownTimeout {
        socket: PathBuf,
        seconds: u64,
        #[source]
        source: tokio::time::error::Elapsed,
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
