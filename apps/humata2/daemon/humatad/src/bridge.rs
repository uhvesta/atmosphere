//! `humatad2 bridge`: the daemon's socket on stdin and stdout, for a link
//! that only carries a process's stdio.

use std::path::PathBuf;

use tokio::net::UnixStream;

use crate::error::DaemonError;
use crate::serve;

#[derive(clap::Args)]
pub(crate) struct BridgeArgs {
    /// The daemon's socket. Default: ~/.humata2/humatad.sock.
    #[arg(long)]
    socket: Option<PathBuf>,
}

pub(crate) async fn run(args: BridgeArgs) -> Result<(), DaemonError> {
    let socket = match args.socket {
        Some(socket) => socket,
        None => serve::default_socket()?,
    };
    let mut daemon = UnixStream::connect(&socket)
        .await
        .map_err(|source| DaemonError::BridgeConnect {
            socket: socket.clone(),
            source,
        })?;
    let mut stdio = tokio::io::join(tokio::io::stdin(), tokio::io::stdout());
    tokio::io::copy_bidirectional(&mut stdio, &mut daemon)
        .await
        .map_err(|source| DaemonError::BridgeCopy { source })?;
    Ok(())
}
