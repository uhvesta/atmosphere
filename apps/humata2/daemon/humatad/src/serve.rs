//! `humatad2 serve`: opens the state, binds the socket and serves until a
//! stop signal. Terminals live in their holders, so they outlive this.
//! With `--stdio` it serves the peer link instead (`peer_link`), and no
//! socket.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::service::InterceptorLayer;
use tonic::transport::Server;
use tower::util::MapRequestLayer;

use crate::access_service::Access;
use crate::cli_login::{CliAuth, CliLogins};
use crate::daemon::Daemon;
use crate::error::DaemonError;
use crate::guard::Guard;
use crate::identity::{self, Authenticate, Identities};
use crate::inbox::Inbox;
use crate::not_built;
use crate::paste::PASTE_MAX_BYTES;
use crate::peer_link;
use crate::sync;
use crate::terminal::{Terminals, TerminalsConfig};
use crate::terminal_session::Kernel;

/// What Unix sockets allow in a path, without its terminating NUL.
#[cfg(target_os = "linux")]
const SOCKET_PATH_MAX: usize = 107;
#[cfg(not(target_os = "linux"))]
const SOCKET_PATH_MAX: usize = 103;

/// Room for a pasted image and the message around it.
const DECODE_MAX_BYTES: usize = PASTE_MAX_BYTES + (1 << 20);

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Role {
    /// The Mac: the source of truth.
    Host,
    /// Any other machine.
    Machine,
}

#[derive(clap::Args)]
pub(crate) struct ServeArgs {
    /// The socket to serve on, made 0600 in a folder only you can open.
    /// Default: ~/.humata2/humatad.sock.
    #[arg(long)]
    socket: Option<PathBuf>,
    /// Serve the peer link on stdin and stdout instead of a socket: what
    /// the Mac runs over SSH or a provider's connect command (MP-7). It
    /// serves PeerService and what the Mac relays, takes every call for
    /// the Mac's, and ends when stdin closes. A peer link is always a
    /// machine's, whatever --role says.
    #[arg(long, conflicts_with = "socket")]
    stdio: bool,
    /// With --stdio, print `HUMATA_SYNC_<nonce>` on a line of its own
    /// before the stream, so a terminal Humata took over can tell where
    /// the stream starts (MP-15). Letters and digits only.
    #[arg(long, requires = "stdio", value_parser = sync::parse_nonce)]
    sync: Option<String>,
    /// Where the daemon keeps its state. Default: ~/.humata2/state.
    #[arg(long)]
    state: Option<PathBuf>,
    /// The folder a machine shell starts in. Default: your home folder.
    #[arg(long)]
    root: Option<PathBuf>,
    /// The humata2_holder binary. Default: the one beside humatad2.
    #[arg(long)]
    holder: Option<PathBuf>,
    /// The shell new shell tabs run. Default: $SHELL, else /bin/sh.
    #[arg(long)]
    shell: Option<PathBuf>,
    /// An argument for the shell; repeat for more. Default: -l.
    #[arg(long = "shell-arg", allow_hyphen_values = true)]
    shell_args: Vec<OsString>,
    #[arg(long, value_enum, default_value_t = Role::Host)]
    role: Role,
    /// How much of each tab's output its holder keeps.
    #[arg(long, default_value_t = 8 << 20)]
    scrollback_bytes: usize,
    /// Test daemons only: a call with no credential acts as the user.
    /// Builds without the `admin` feature, as releases are, lack this flag.
    /// Never with --stdio: the peer link is the Mac's, not the user's.
    #[cfg(feature = "admin")]
    #[arg(long, conflicts_with = "stdio")]
    admin: bool,
    /// Test daemons only: as --admin, but only for calls from this
    /// process, so a test can play the app while every other process is
    /// judged as a release daemon judges it.
    #[cfg(feature = "admin")]
    #[arg(long, conflicts_with_all = ["stdio", "admin"])]
    admin_pid: Option<i32>,
    /// Test daemons only: every CLI login lasts this many seconds, not the
    /// hours it asked for, so tests can see one expire.
    #[cfg(feature = "admin")]
    #[arg(long, conflicts_with = "stdio")]
    cli_login_seconds: Option<u64>,
}

/// `~/.humata2`, where the daemon lives unless told otherwise.
fn home_folder() -> Result<PathBuf, DaemonError> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join(".humata2"))
        .ok_or(DaemonError::NoHome)
}

pub(crate) fn default_socket() -> Result<PathBuf, DaemonError> {
    Ok(home_folder()?.join("humatad.sock"))
}

pub(crate) async fn run(args: ServeArgs) -> Result<(), DaemonError> {
    let state = match args.state {
        Some(state) => state,
        None => home_folder()?.join("state"),
    };
    if args.stdio {
        private_folder(&state)?;
        private_folder(&state.join("logs"))?;
        let machine = machine_id(&state.join("machine-id"))?;
        return peer_link::run(&state, machine, args.sync.as_deref()).await;
    }
    let socket = match args.socket {
        Some(socket) => socket,
        None => default_socket()?,
    };
    let root = match args.root {
        Some(root) => root,
        None => std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .ok_or(DaemonError::NoHome)?,
    };
    let holder = match args.holder {
        Some(holder) => holder,
        None => beside_this_binary("humata2_holder"),
    };
    if !holder.is_file() {
        return Err(DaemonError::NoHolder { path: holder });
    }
    let shell = args
        .shell
        .or_else(|| std::env::var_os("SHELL").filter(|shell| !shell.is_empty()).map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/bin/sh"));
    let shell_args = if args.shell_args.is_empty() {
        vec![OsString::from("-l")]
    } else {
        args.shell_args
    };

    private_folder(&state)?;
    // Paths handed to clients and holders are absolute and resolved.
    let state = std::fs::canonicalize(&state).map_err(|source| DaemonError::CreateDir {
        path: state.clone(),
        source,
    })?;
    let holders = state.join("holders");
    let logs = state.join("logs");
    private_folder(&holders)?;
    private_folder(&logs)?;
    let store = Arc::new(Store::open(state.join("store")).map_err(|source| DaemonError::Store { source })?);
    let machine = machine_id(&state.join("machine-id"))?;
    let role = match args.role {
        Role::Host => pb::DaemonRole::Host,
        Role::Machine => pb::DaemonRole::Machine,
    };

    let identities = Arc::new(Identities::load(
        state.join("session-tokens"),
        &store,
        &machine,
    )?);
    #[cfg(feature = "admin")]
    let login_lifetime = args.cli_login_seconds.map(std::time::Duration::from_secs);
    #[cfg(not(feature = "admin"))]
    let login_lifetime = None;
    let logins = Arc::new(CliLogins::new(Box::new(Kernel), login_lifetime));
    #[cfg(feature = "admin")]
    let authenticate = match (args.admin, args.admin_pid) {
        (true, _) => {
            eprintln!(
                "humatad2: warning: admin mode: a call with no credential acts as the user. This is a test daemon; don't use it as your real one"
            );
            Authenticate::admin(identities.clone(), logins.clone(), identity::Admin::Anyone)
        }
        (false, Some(pid)) => {
            eprintln!(
                "humatad2: warning: admin mode: a call with no credential from process {pid} acts as the user. This is a test daemon; don't use it as your real one"
            );
            Authenticate::admin(identities.clone(), logins.clone(), identity::Admin::Process(pid))
        }
        (false, None) => Authenticate::new(identities.clone(), logins.clone()),
    };
    #[cfg(not(feature = "admin"))]
    let authenticate = Authenticate::new(identities.clone(), logins.clone());
    let guard = Arc::new(Guard::new(&machine, role));

    let listener = bind(&socket)?;
    let terminals = Terminals::new(TerminalsConfig {
        store: store.clone(),
        machine: machine.clone(),
        holder,
        shell,
        shell_args,
        root,
        holders,
        logs,
        scrollback_bytes: args.scrollback_bytes,
        guard: guard.clone(),
        identities: identities.clone(),
    });
    let inbox = Inbox::new(guard.clone(), store.clone(), machine.clone());
    let access = Access::new(guard);

    // Every call is matched to its caller before a service sees it. The
    // first layer is the outer one: it marks the methods open to anyone,
    // which the interceptor can't see the path of.
    let router = Server::builder()
        .layer(MapRequestLayer::new(identity::mark_open))
        .layer(InterceptorLayer::new(authenticate))
        .add_service(pb::daemon_service_server::DaemonServiceServer::new(Daemon::new(
            machine.clone(),
            role,
        )))
        .add_service(
            pb::terminal_service_server::TerminalServiceServer::new(terminals)
                .max_decoding_message_size(DECODE_MAX_BYTES),
        )
        .add_service(pb::inbox_service_server::InboxServiceServer::new(inbox))
        .add_service(pb::access_service_server::AccessServiceServer::new(access))
        .add_service(pb::cli_auth_service_server::CliAuthServiceServer::new(CliAuth {
            logins,
            store,
            machine,
            role,
        }));
    let router = not_built::register(router);

    let stop = stop_signal()?;
    let served = router
        .serve_with_incoming_shutdown(UnixListenerStream::new(listener), stop)
        .await;
    let _ = std::fs::remove_file(&socket);
    served.map_err(|source| DaemonError::Serve { source })
}

fn beside_this_binary(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|folder| folder.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Makes `path` if it's missing, readable only by this user, and refuses
/// one that others can open.
fn private_folder(path: &Path) -> Result<(), DaemonError> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|source| DaemonError::CreateDir {
            path: path.to_owned(),
            source,
        })?;
    let mode = std::fs::metadata(path)
        .map_err(|source| DaemonError::CreateDir {
            path: path.to_owned(),
            source,
        })?
        .mode()
        & 0o777;
    if mode & 0o077 != 0 {
        return Err(DaemonError::FolderOpenToOthers {
            path: path.to_owned(),
            mode,
        });
    }
    Ok(())
}

/// This machine's id, made once and kept. Daemons starting for the first
/// time at once agree on one: each writes an id of its own aside, synced,
/// and links it into place, which only the first can do; the others read
/// the first one's.
fn machine_id(path: &Path) -> Result<String, DaemonError> {
    let failed = |source| DaemonError::MachineId {
        path: path.to_owned(),
        source,
    };
    if let Some(id) = read_machine_id(path).map_err(failed)? {
        return Ok(id);
    }
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(failed(std::io::Error::new(
            ErrorKind::InvalidInput,
            "the path names no file in a folder",
        )));
    };
    let id = format!("ma_{}", uuid::Uuid::new_v4().simple());
    let aside = folder.join(format!(
        ".{}.{}.partial",
        name.to_string_lossy(),
        uuid::Uuid::new_v4().simple()
    ));
    let linked = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&aside)?;
        file.write_all(id.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::hard_link(&aside, path)
    })();
    // Linked or not, the id is in place or never will be from this copy.
    let _ = std::fs::remove_file(&aside);
    match linked {
        Ok(()) => {
            File::open(folder)
                .and_then(|folder| folder.sync_all())
                .map_err(failed)?;
            Ok(id)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => read_machine_id(path)
            .map_err(failed)?
            .ok_or_else(|| failed(ErrorKind::NotFound.into())),
        Err(error) => Err(failed(error)),
    }
}

/// The id kept at `path`, if there is one. An empty file is damage, not a
/// missing id: making a new one would make the host see a new machine.
fn read_machine_id(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(id) if !id.trim().is_empty() => Ok(Some(id.trim().to_owned())),
        Ok(_) => Err(std::io::Error::new(ErrorKind::InvalidData, "the file is empty")),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Listens on `socket`, 0600 in a folder only this user can open. A socket
/// left by a daemon that died is taken over; a live one is refused.
fn bind(socket: &Path) -> Result<UnixListener, DaemonError> {
    if socket.as_os_str().len() > SOCKET_PATH_MAX {
        return Err(DaemonError::SocketPathTooLong {
            socket: socket.to_owned(),
            max: SOCKET_PATH_MAX,
        });
    }
    if let Some(folder) = socket.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        private_folder(folder)?;
    }
    if socket.exists() {
        if std::os::unix::net::UnixStream::connect(socket).is_ok() {
            return Err(DaemonError::AlreadyRunning {
                socket: socket.to_owned(),
            });
        }
        std::fs::remove_file(socket).map_err(|source| DaemonError::Bind {
            socket: socket.to_owned(),
            source,
        })?;
    }
    let bind_failed = |source| DaemonError::Bind {
        socket: socket.to_owned(),
        source,
    };
    let listener = UnixListener::bind(socket).map_err(bind_failed)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).map_err(bind_failed)?;
    Ok(listener)
}

/// Resolves on SIGTERM or SIGINT.
pub(crate) fn stop_signal() -> Result<impl Future<Output = ()>, DaemonError> {
    let mut terminate =
        signal(SignalKind::terminate()).map_err(|source| DaemonError::Signals { source })?;
    let mut interrupt =
        signal(SignalKind::interrupt()).map_err(|source| DaemonError::Signals { source })?;
    Ok(async move {
        tokio::select! {
            _ = terminate.recv() => {}
            _ = interrupt.recv() => {}
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::*;

    #[test]
    fn first_starts_at_once_agree_on_one_machine_id() {
        const STARTS: usize = 16;
        for _ in 0..20 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("machine-id");
            let barrier = Arc::new(Barrier::new(STARTS));
            let starts: Vec<_> = (0..STARTS)
                .map(|_| {
                    let (path, barrier) = (path.clone(), barrier.clone());
                    std::thread::spawn(move || {
                        barrier.wait();
                        machine_id(&path).unwrap()
                    })
                })
                .collect();
            let ids: Vec<String> = starts.into_iter().map(|start| start.join().unwrap()).collect();
            assert!(ids.iter().all(|id| *id == ids[0]), "{ids:?}");
            assert_eq!(read_machine_id(&path).unwrap().as_deref(), Some(ids[0].as_str()));
            // Nothing is left aside.
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn a_kept_machine_id_is_used_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("machine-id");
        let first = machine_id(&path).unwrap();
        assert_eq!(machine_id(&path).unwrap(), first);
    }

    #[test]
    fn an_empty_machine_id_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("machine-id");
        std::fs::write(&path, "").unwrap();
        assert!(matches!(machine_id(&path), Err(DaemonError::MachineId { .. })));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
    }
}
