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
use sha2::{Digest, Sha256};
use store::Store;

/// Sent in the hello, for messages about updating.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the daemon's hello says it is: [`VERSION`], `+`, and the start of a
/// SHA-256 of this executable, so the app can tell a daemon left running by
/// an older build of itself from its own. Just [`VERSION`] if the executable
/// can't be read.
fn build() -> String {
    let digest = std::env::current_exe()
        .and_then(std::fs::read)
        .map(|executable| Sha256::digest(executable));
    match digest {
        Ok(digest) => format!("{VERSION}+{}", hex(&digest[..8])),
        Err(_) => VERSION.to_owned(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

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

/// Where khodrd keeps what it can make again, such as sessions' copies of
/// their code, unless told otherwise: `$KHODR_CACHE`, or the system's cache
/// folder.
pub fn default_cache() -> Result<PathBuf, StartError> {
    if let Some(cache) = std::env::var_os("KHODR_CACHE").filter(|cache| !cache.is_empty()) {
        return Ok(PathBuf::from(cache));
    }
    let home = || {
        std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .ok_or(StartError::NoHome)
    };
    if cfg!(target_os = "macos") {
        return Ok(home()?.join("Library/Caches/Khodr"));
    }
    match std::env::var_os("XDG_CACHE_HOME").filter(|cache| !cache.is_empty()) {
        Some(cache) => Ok(PathBuf::from(cache).join("khodr")),
        None => Ok(home()?.join(".cache/khodr")),
    }
}

/// Serves the host role on stdin and stdout until the daemon hangs up.
/// Sessions' copies of their code go under `<cache>/worktrees`.
pub async fn host_stdio(state: &Path, cache: &Path) -> Result<(), StartError> {
    let host = Arc::new(Host::open(state, &cache.join("worktrees")).await?);
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
/// `<state>/khodr.sqlite3` and sessions on this Mac under `<state>/host`,
/// their copies of the code under `<cache>/worktrees`; other hosts are read
/// from `hosts`.
pub async fn daemon(
    state: &Path,
    cache: &Path,
    socket: &Path,
    hosts: &Path,
) -> Result<(), StartError> {
    let store = Store::open(&state.join("khodr.sqlite3"))?;
    let local = Host::open(&state.join("host"), &cache.join("worktrees")).await?;
    let daemon = Arc::new(Daemon::new(
        local,
        store,
        Hosts::load(hosts)?,
        hosts.to_owned(),
    ));
    // Read before serving: an update may replace the file while it runs.
    let build: Arc<str> = build().into();
    let listener = socket::listen(socket).await?;
    let serving = async {
        loop {
            let (stream, _) = listener
                .accept()
                .await
                .map_err(|source| StartError::Accept { source })?;
            let (daemon, build) = (daemon.clone(), build.clone());
            tokio::spawn(async move {
                let (reader, writer) = stream.into_split();
                // A client that goes away or speaks another version only
                // ends its own connection.
                let _ = khodr_wire::serve(reader, writer, pb::Role::Daemon, &build, daemon).await;
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
