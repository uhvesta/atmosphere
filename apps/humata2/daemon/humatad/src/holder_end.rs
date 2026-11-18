//! Shutting a tab's or an agent's holder down, and ending it when it won't
//! take that (D292). The shutdown and the ending by pid are the holder
//! crate's ([`humata2_holder::shutdown_or_end`]); this adds the owner's name
//! and the log line.

use std::path::{Path, PathBuf};

use humata2_holder::{EndError, HolderError, Stopped};

use crate::status;

/// A holder that neither took its shutdown nor could be ended by its pid.
#[derive(Debug, thiserror::Error)]
pub(crate) enum HolderEndError {
    #[error(
        "couldn't stop {owner}: its holder at {} didn't take its shutdown, and its process {pid} couldn't be ended by its pid either; stop again, and if the process stays, check with `ps -p {pid}` that it is a humata2 holder, then end it by hand",
        socket.display()
    )]
    EndByPid {
        owner: String,
        socket: PathBuf,
        pid: u32,
        #[source]
        source: EndError,
    },

    #[error(
        "couldn't stop {owner}: couldn't find out which process is its holder at {}; stop again, and if that fails too, check that `ps` runs for this user",
        socket.display()
    )]
    FindHolder {
        owner: String,
        socket: PathBuf,
        #[source]
        source: EndError,
    },
}

/// Ends the holder serving `socket`, whichever process it is: found by its
/// pid file, or by the socket it serves if that file is gone, then taken
/// through [`shut_or_end`], which signals a pid only while the process list
/// names it as a live holder. No holder found means none runs, which is Ok.
pub(crate) async fn end_holder_at(socket: &Path, owner: &str) -> Result<(), HolderEndError> {
    let found = humata2_holder::find_holder(socket)
        .await
        .map_err(|source| HolderEndError::FindHolder {
            owner: owner.to_owned(),
            socket: socket.to_owned(),
            source,
        })?;
    match found {
        Some(pid) => shut_or_end(socket, pid, owner).await,
        None => Ok(()),
    }
}

/// Shuts down the holder at `socket`, whose process is `pid`. When it
/// doesn't take the shutdown, or its socket is gone, its process is ended by
/// its pid instead. `owner` names whose holder it is, for the log and the
/// error. Returns an error only when the process couldn't be ended either.
pub(crate) async fn shut_or_end(
    socket: &Path,
    pid: u32,
    owner: &str,
) -> Result<(), HolderEndError> {
    let stopped = humata2_holder::shutdown_or_end(socket, pid)
        .await
        .map_err(|source| HolderEndError::EndByPid {
            owner: owner.to_owned(),
            socket: socket.to_owned(),
            pid,
            source,
        })?;
    if let Stopped::Ended { reason } = stopped
        // A socket that is gone is how a holder that already stopped looks,
        // so only the other failures are news.
        && !matches!(reason, HolderError::Connect { .. })
    {
        crate::log::say!(
            "humatad2: the holder of {owner} didn't take its shutdown: {}; its process {pid} was ended by its pid instead",
            status::chain(&reason)
        );
    }
    Ok(())
}
