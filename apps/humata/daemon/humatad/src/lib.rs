//! humatad: the daemon on your Mac (`humatad daemon`), the host role on any
//! machine (`humatad host --stdio`), and the MCP server agents on the Mac
//! use (`humatad mcp`). One binary, so a host is set up by copying it.

mod board;
mod convert;
mod daemon;
mod error;
mod files;
mod host;
mod hosts;
mod machine;
mod mcp;
mod socket;
mod store;

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use error::StartError;
pub use mcp::{McpError, mcp_stdio};

use daemon::Daemon;
use host::Host;
use hosts::Hosts;
use humata_agent::McpServer;
use humata_wire::pb;
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

/// Where humatad keeps its files unless told otherwise.
pub fn default_state() -> Result<PathBuf, StartError> {
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or(StartError::NoHome)?;
    Ok(if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Humata")
    } else {
        home.join(".local/state/humata")
    })
}

/// Where humatad keeps what it can make again, such as sessions' copies of
/// their code, unless told otherwise: `$HUMATA_CACHE`, or the system's cache
/// folder.
pub fn default_cache() -> Result<PathBuf, StartError> {
    if let Some(cache) = std::env::var_os("HUMATA_CACHE").filter(|cache| !cache.is_empty()) {
        return Ok(PathBuf::from(cache));
    }
    let home = || {
        std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .ok_or(StartError::NoHome)
    };
    if cfg!(target_os = "macos") {
        return Ok(home()?.join("Library/Caches/Humata"));
    }
    match std::env::var_os("XDG_CACHE_HOME").filter(|cache| !cache.is_empty()) {
        Some(cache) => Ok(PathBuf::from(cache).join("humata")),
        None => Ok(home()?.join(".cache/humata")),
    }
}

/// Serves the host role on stdin and stdout until the daemon hangs up.
/// Sessions' copies of their code go under `<cache>/worktrees`.
pub async fn host_stdio(state: &Path, cache: &Path) -> Result<(), StartError> {
    // The board is the daemon's, out of reach of a host's agents.
    let host = Arc::new(Host::open(state, &cache.join("worktrees"), None).await?);
    humata_wire::serve(
        tokio::io::stdin(),
        tokio::io::stdout(),
        pb::Role::Host,
        &build(),
        host,
    )
    .await
    .map(drop)
    .map_err(|source| StartError::Stdio { source })
}

/// Serves the daemon on `socket` until stopped. Projects are kept in
/// `<state>/humata.sqlite3` and sessions on this Mac under `<state>/host`,
/// their copies of the code under `<cache>/worktrees`; other hosts are read
/// from `hosts`.
pub async fn daemon(
    state: &Path,
    cache: &Path,
    socket: &Path,
    hosts: &Path,
) -> Result<(), StartError> {
    let store = Store::open(&state.join("humata.sqlite3"))?;
    // Agents here reach the board and review threads through `humatad mcp`.
    let mcp = std::env::current_exe().ok().map(|program| McpServer {
        command: program,
        args: vec![
            "mcp".into(),
            "--socket".into(),
            socket.display().to_string(),
        ],
        env: Vec::new(),
    });
    let local = Host::open(&state.join("host"), &cache.join("worktrees"), mcp).await?;
    // Read before serving: an update may replace the file while it runs.
    let build: Arc<str> = build().into();
    let daemon = Arc::new(Daemon::new(
        local,
        store,
        Hosts::load(hosts)?,
        hosts.to_owned(),
        build.to_string(),
    ));
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
                let _ = humata_wire::serve(reader, writer, pb::Role::Daemon, &build, daemon).await;
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
