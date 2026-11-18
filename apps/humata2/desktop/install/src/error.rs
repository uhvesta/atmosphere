use std::path::PathBuf;

use humata2_machines::{BuildsError, StageError};

/// Why an install step failed.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error(
        "could not create the folder {path}. Check that you own it and its parent folder \
         and can write there, then quit Humata and open it again"
    )]
    CreateFolder { path: PathBuf, #[source] source: std::io::Error },

    #[error(
        "could not read Humata's own copy of {path}. The app looks damaged: \
         download Humata again and replace the installed app"
    )]
    ReadBundle { path: PathBuf, #[source] source: std::io::Error },

    #[error(
        "could not read the installed file {path} to compare it. Check its permissions \
         (or delete it so Humata can install a fresh copy), then quit Humata and open it again"
    )]
    Hash { path: PathBuf, #[source] source: std::io::Error },

    #[error(
        "could not copy {from} to {to}. Check that the disk is not full and that you can \
         write in that folder, then quit Humata and open it again"
    )]
    Copy { from: PathBuf, to: PathBuf, #[source] source: std::io::Error },

    #[error(
        "could not move {from} over {to}. Check you can write in that folder, \
         then quit Humata and open it again"
    )]
    Rename { from: PathBuf, to: PathBuf, #[source] source: std::io::Error },

    #[error(
        "could not write the release manifest {path}. Check you can write in that folder and that the \
         disk is not full, then quit Humata and open it again"
    )]
    WriteBundle { path: PathBuf, #[source] source: std::io::Error },

    #[error(
        "Humata's release could not be checked or staged: {source}. Quit Humata and open it again; \
         if it fails again, download Humata again and replace the installed app"
    )]
    Release { #[source] source: StageError },

    #[error(
        "could not read the staged file {path}. Check its permissions, then quit Humata and open it \
         again so it stages its release again; the running daemon was not changed"
    )]
    ReadStaged { path: PathBuf, #[source] source: std::io::Error },

    #[error(
        "the release staged in {bin} is not whole: its files do not match its .bundle.json.next, so \
         nothing was moved and the daemon was not restarted. Quit Humata and open it again so it \
         stages its release again; do not copy files into that folder by hand"
    )]
    StagedMismatch { bin: PathBuf, #[source] source: StageError },

    #[error(
        "Humata does not ship tools for this Mac's processor, so it cannot install them. Use a Mac \
         with Apple silicon or an Intel processor"
    )]
    UnsupportedHost,

    #[error(
        "could not write the launch agent file {path}. Check you can write in \
         ~/Library/LaunchAgents, then quit Humata and open it again"
    )]
    WritePlist { path: PathBuf, #[source] source: std::io::Error },

    #[error(
        "could not link {path} to {target}. Check you can write in that folder, \
         then quit Humata and open it again"
    )]
    Symlink { path: PathBuf, target: PathBuf, #[source] source: std::io::Error },

    #[error(
        "could not start the Humata background service from {plist}. \
         Run `launchctl bootstrap gui/$(id -u) {plist}` in a terminal to see the reason, \
         then quit Humata and open it again"
    )]
    Launchd { plist: PathBuf, #[source] source: LaunchdError },

    #[error(
        "the Linux tools inside the app ({path}) can't be read or don't match their manifest. \
         The app looks damaged: download Humata again and replace the installed app; adding a \
         Linux machine needs them"
    )]
    MachineTools { path: PathBuf, #[source] source: BuildsError },

    #[error(
        "could not write the Linux tools at {path}. Check you can write in ~/.humata2 and that the \
         disk is not full, then quit Humata and open it again; adding a Linux machine needs them"
    )]
    WriteMachineTools { path: PathBuf, #[source] source: std::io::Error },
}

/// Why a `launchctl` call failed.
#[derive(Debug, thiserror::Error)]
pub enum LaunchdError {
    #[error("could not run /bin/launchctl {args}. Check that macOS is intact and try again")]
    Spawn { args: String, #[source] source: std::io::Error },

    #[error(
        "`launchctl {args}` failed (exit status {status}): {stderr}. \
         Run it in a terminal to see more"
    )]
    Failed { args: String, status: i32, stderr: String },
}
