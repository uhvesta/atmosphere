//! humatad: the daemon on your Mac (`humatad daemon`), the host role on any
//! machine (`humatad host`, reached over SSH through `humatad relay`), and the MCP server agents on the Mac
//! use (`humatad mcp`). One binary, so a host is set up by copying it.

mod board;
mod convert;
mod daemon;
mod error;
mod files;
mod host;
mod hosts;
mod login_env;
mod machine;
mod mcp;
mod relay;
mod socket;
mod store;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub use error::StartError;
pub use login_env::{LoginEnvError, login_environment};
pub use mcp::{McpError, mcp_stdio};

use daemon::Daemon;
use host::Host;
use hosts::Hosts;
use humata_agent::McpServer;
use humata_wire::pb;
use sha2::{Digest, Sha256};
use store::Store;
use tokio::net::UnixStream;

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

/// How long a relay waits for the host it started to listen.
const HOST_START_TIMEOUT: Duration = Duration::from_secs(10);

/// Opens the host role's sessions under `state`, their copies of the code
/// under `<cache>/worktrees`, and serves its agents' humata tools.
async fn open_host(state: &Path, cache: &Path) -> Result<Arc<Host>, StartError> {
    // The board is the daemon's: this host's agents reach it through the
    // relay, which asks the daemon over the daemon's connection. Without the
    // relay's socket they work on without the humata tools.
    let socket = state.join("mcp.sock");
    let mut listening = socket::listen(&socket).await;
    if let Err(StartError::AlreadyRunning { .. }) = listening {
        // A host on its way out; this one takes over its agents' tools.
        let _ = std::fs::remove_file(&socket);
        listening = socket::listen(&socket).await;
    }
    let listener = match listening {
        Ok(listener) => Some(listener),
        Err(error) => {
            eprintln!(
                "humatad: agents here get no humata tools: {}",
                humata_wire::describe(&error)
            );
            None
        }
    };
    let mcp = listener.as_ref().and_then(|_| mcp_server(&socket));
    let host = Arc::new(Host::open(state, &cache.join("worktrees"), mcp).await?);
    if let Some(listener) = listener {
        let relay = host.relay();
        let build = build();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (relay, build) = (relay.clone(), build.clone());
                tokio::spawn(async move {
                    let (reader, writer) = stream.into_split();
                    let _ = humata_wire::serve(reader, writer, pb::Role::Host, &build, relay).await;
                });
            }
        });
    }
    Ok(host)
}

/// Serves the host role on stdin and stdout until the daemon hangs up.
/// Sessions' copies of their code go under `<cache>/worktrees`.
pub async fn host_stdio(state: &Path, cache: &Path) -> Result<(), StartError> {
    let build = build();
    let host = open_host(state, cache).await?;
    humata_wire::serve(
        tokio::io::stdin(),
        tokio::io::stdout(),
        pb::Role::Host,
        &build,
        host,
    )
    .await
    .map(drop)
    .map_err(|source| StartError::Stdio { source })
}

/// Serves the host role on `socket`, one daemon connection after another,
/// so sessions keep going while no daemon is connected. Stops when the
/// daemon asks, once a new humatad is installed, or on SIGTERM.
pub async fn host_socket(state: &Path, cache: &Path, socket: &Path) -> Result<(), StartError> {
    let listener = socket::listen(socket).await?;
    let build: Arc<str> = build().into();
    let host = open_host(state, cache).await?;
    host.serve_on(socket);
    let serving = async {
        loop {
            let (stream, _) = listener
                .accept()
                .await
                .map_err(|source| StartError::Accept { source })?;
            let (host, build) = (host.clone(), build.clone());
            tokio::spawn(async move {
                let (reader, writer) = stream.into_split();
                let _ = humata_wire::serve(reader, writer, pb::Role::Host, &build, host).await;
            });
        }
    };
    let result = tokio::select! {
        result = serving => result,
        () = host.stopped() => return Ok(()),
        () = socket::terminated() => Ok(()),
    };
    let _ = std::fs::remove_file(socket);
    result
}

/// Joins stdin and stdout to the host on `socket`, starting it first if
/// none is listening, until either side hangs up. SSH runs this, so a
/// dropped connection ends only the relay, never the host's sessions.
pub async fn relay(socket: &Path, state: &Path, cache: &Path) -> Result<(), StartError> {
    let stream = match UnixStream::connect(socket).await {
        Ok(stream) => stream,
        Err(_) => start_host(socket, state, cache).await?,
    };
    let failed = |source| StartError::Relay {
        socket: socket.to_owned(),
        source,
    };
    let (mut reader, mut writer) = stream.into_split();
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    tokio::select! {
        result = tokio::io::copy(&mut stdin, &mut writer) => result.map(drop).map_err(failed),
        result = tokio::io::copy(&mut reader, &mut stdout) => result.map(drop).map_err(failed),
    }
}

/// Starts `humatad host --socket` on its own, its output in
/// `<state>/host.log`, and connects once it listens.
async fn start_host(socket: &Path, state: &Path, cache: &Path) -> Result<UnixStream, StartError> {
    let program = std::env::current_exe().map_err(|source| StartError::OwnProgram { source })?;
    let log = state.join("host.log");
    let opened = std::fs::create_dir_all(state).and_then(|()| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
    });
    let output = opened.map_err(|source| StartError::OpenHostLog {
        path: log.clone(),
        source,
    })?;
    let errors = output
        .try_clone()
        .map_err(|source| StartError::OpenHostLog {
            path: log.clone(),
            source,
        })?;
    // Paths made absolute, since the host outlives this folder's meaning to
    // the shell that started it.
    let absolute = |path: &Path| std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    let mut child = tokio::process::Command::new(&program)
        .arg("host")
        .arg("--socket")
        .arg(absolute(socket))
        .arg("--state")
        .arg(absolute(state))
        .arg("--cache")
        .arg(absolute(cache))
        .stdin(std::process::Stdio::null())
        .stdout(output)
        .stderr(errors)
        // Its own group, so the SSH session ending doesn't signal it.
        .process_group(0)
        .spawn()
        .map_err(|source| StartError::SpawnHost {
            program: program.clone(),
            source,
        })?;
    let deadline = tokio::time::Instant::now() + HOST_START_TIMEOUT;
    loop {
        if let Ok(stream) = UnixStream::connect(socket).await {
            return Ok(stream);
        }
        if let Ok(Some(status)) = child.try_wait() {
            // Another relay's host may have won the race to listen.
            if let Ok(stream) = UnixStream::connect(socket).await {
                return Ok(stream);
            }
            let said = std::fs::read_to_string(&log).unwrap_or_default();
            let tail: Vec<&str> = said.lines().rev().take(20).collect();
            return Err(StartError::HostExited {
                status,
                log,
                said: tail.into_iter().rev().collect::<Vec<_>>().join("\n"),
            });
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(StartError::HostNotListening {
                socket: socket.to_owned(),
                seconds: HOST_START_TIMEOUT.as_secs(),
                log,
            });
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// `humatad mcp` talking to `socket`, for agents to start.
fn mcp_server(socket: &Path) -> Option<McpServer> {
    std::env::current_exe().ok().map(|program| McpServer {
        command: program,
        args: vec![
            "mcp".into(),
            "--socket".into(),
            socket.display().to_string(),
        ],
        env: Vec::new(),
    })
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
    let mcp = mcp_server(socket);
    let local = Host::open(&state.join("host"), &cache.join("worktrees"), mcp).await?;
    // Read before serving: an update may replace the file while it runs.
    let build: Arc<str> = build().into();
    let daemon = Daemon::new(
        local,
        store,
        Hosts::load(hosts)?,
        hosts.to_owned(),
        build.to_string(),
    );
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
