//! khodrd: the daemon on your Mac (`khodrd daemon`), and the host role on
//! any machine (`khodrd host --stdio`). One binary, so a host is set up by
//! copying it.

mod convert;
mod daemon;
mod error;
mod host;
mod socket;
mod store;

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use error::StartError;

use daemon::{Daemon, Hosts};
use host::Host;
use khodr_wire::pb;
use store::Store;

/// Sent in the hello, for messages about updating.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where khodrd keeps its files unless told otherwise.
pub fn default_state() -> Result<PathBuf, StartError> {
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or(StartError::NoHome)?;
    Ok(if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Khodr")
    } else {
        home.join(".local/state/khodr")
    })
}

/// Serves the host role on stdin and stdout until the daemon hangs up.
pub async fn host_stdio(state: &Path) -> Result<(), StartError> {
    let host = Arc::new(Host::open(state).await?);
    khodr_wire::serve(
        tokio::io::stdin(),
        tokio::io::stdout(),
        pb::Role::Host,
        VERSION,
        host,
    )
    .await
    .map(drop)
    .map_err(|source| StartError::Stdio { source })
}

/// Serves the daemon on `socket` until stopped. Projects are kept in
/// `<state>/khodr.sqlite3` and sessions on this Mac under `<state>/host`;
/// other hosts are read from `hosts`.
pub async fn daemon(state: &Path, socket: &Path, hosts: &Path) -> Result<(), StartError> {
    let store = Store::open(&state.join("khodr.sqlite3"))?;
    let local = Host::open(&state.join("host")).await?;
    let daemon = Arc::new(Daemon::new(
        local,
        store,
        Hosts::load(hosts)?,
        hosts.to_owned(),
    ));
    let listener = socket::listen(socket).await?;
    let serving = async {
        loop {
            let (stream, _) = listener
                .accept()
                .await
                .map_err(|source| StartError::Accept { source })?;
            let daemon = daemon.clone();
            tokio::spawn(async move {
                let (reader, writer) = stream.into_split();
                // A client that goes away or speaks another version only
                // ends its own connection.
                let _ = khodr_wire::serve(reader, writer, pb::Role::Daemon, VERSION, daemon).await;
            });
        }
    };
    let result = tokio::select! {
        result = serving => result,
        _ = tokio::signal::ctrl_c() => Ok(()),
        () = socket::terminated() => Ok(()),
    };
    let _ = std::fs::remove_file(socket);
    result
}

#[cfg(test)]
mod tests;
