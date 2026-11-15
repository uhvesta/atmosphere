//! Where the daemon's socket is (GRPC.md §2, CLI.md §1).

use std::path::{Path, PathBuf};

use crate::daemon::DaemonError;
use crate::principal::Environment;

/// The sockets to try, in order: `HUMATA_SOCKET` alone when set; else the
/// Mac's host socket, this machine's socket under its root, and humatad2's
/// own default while it serves somewhere else (D15).
pub fn candidates(environment: &Environment) -> Result<Vec<PathBuf>, DaemonError> {
    if let Some(socket) = &environment.socket {
        return Ok(vec![socket.clone()]);
    }
    let home = environment.home.as_ref().ok_or(DaemonError::NoHome)?;
    let root = environment
        .root
        .clone()
        .unwrap_or_else(|| home.join("Humata"));
    Ok(vec![
        home.join("Library/Application Support/Humata/humatad.sock"),
        root.join(".humatad/humatad.sock"),
        home.join(".humata2/humatad.sock"),
    ])
}

/// The first candidate that exists, judged by `exists` so tests needn't
/// make sockets.
pub fn find(
    environment: &Environment,
    exists: impl Fn(&Path) -> bool,
) -> Result<PathBuf, DaemonError> {
    let tried = candidates(environment)?;
    match tried.iter().find(|socket| exists(socket)) {
        Some(socket) => Ok(socket.clone()),
        None => Err(DaemonError::NotRunning { tried }),
    }
}
