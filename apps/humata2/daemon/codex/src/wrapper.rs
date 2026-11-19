//! The `humata codex` wrapper (D795): runs Codex's app-server on a private
//! socket, attaches the TUI to it, links the two to Humata and ends the
//! app-server with the TUI. Without an app-server it runs plain Codex with
//! Humata's hooks.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;
use std::time::Duration;

use rustix::process::{Pid, Signal, kill_process};
use tokio::process::{Child, Command};

use crate::link::{Humata, run_link};

/// How long a process gets to end on SIGTERM before it is killed.
const GRACE: Duration = Duration::from_secs(3);
/// How often the socket's existence is checked.
const POLL: Duration = Duration::from_millis(20);

/// What the wrapper runs and where it keeps its files.
pub struct Wrapper {
    /// The `codex` program.
    pub program: PathBuf,
    /// The value of `-c hooks=`, given only to plain Codex in the fallback.
    pub hooks: Option<String>,
    /// The session's Codex arguments.
    pub args: Vec<OsString>,
    /// The folder both processes work in.
    pub folder: PathBuf,
    /// Where the private socket folder is made.
    pub socket_root: PathBuf,
    /// The wait before the link reconnects.
    pub retry: Duration,
    /// How long the app-server has to make its socket.
    pub wait_for_socket: Duration,
}

/// Why the wrapper could not run.
#[derive(Debug, thiserror::Error)]
pub enum WrapperError {
    /// The program is not there.
    #[error(
        "`{program}` was not found, so Codex is not on PATH. Install Codex with its own \
         installer or package manager, then run this again."
    )]
    NotInstalled { program: String },
    /// The program could not be started.
    #[error(
        "Could not start `{program}`. Check that it is Codex and that you may run it, then \
         try again."
    )]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    /// A file or folder could not be set up.
    #[error(
        "Could not set up {path}. Check that the folder exists and that you may write to it, \
         then try again."
    )]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Waiting for a process failed.
    #[error("Waiting for Codex failed. Close any Codex left running and try again.")]
    Wait {
        #[source]
        source: std::io::Error,
    },
    /// The app-server ended before the TUI.
    #[error(
        "Codex's app-server stopped ({status}), so its TUI was closed. Restart the session to \
         start both again."
    )]
    ServeEnded { status: String },
}

/// Runs the wrapper and returns the TUI's exit code.
pub async fn run_wrapper<H: Humata>(wrapper: Wrapper, humata: Arc<H>) -> Result<i32, WrapperError> {
    let program = wrapper.program.to_string_lossy().into_owned();
    let folder = private_folder(&wrapper.socket_root)?;
    let socket = folder.join("s");

    let mut serve = match Command::new(&wrapper.program)
        .arg("app-server")
        .arg("--listen")
        .arg(format!("unix://{}", socket.display()))
        .args(config_pairs(&wrapper.args))
        .current_dir(&wrapper.folder)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(serve) => serve,
        Err(source) => {
            remove(&folder);
            return Err(spawn_error(&program, source));
        }
    };

    if !socket_appears(&mut serve, &socket, wrapper.wait_for_socket).await {
        stop(&mut serve).await;
        remove(&folder);
        eprintln!(
            "Codex's app-server did not start, so Codex runs with Humata's hooks instead; Codex \
             asks you to trust them once in /hooks."
        );
        return plain_codex(&wrapper).await;
    }

    let mut tui = match Command::new(&wrapper.program)
        .arg("--remote")
        .arg(format!("unix://{}", socket.display()))
        .args(&wrapper.args)
        .current_dir(&wrapper.folder)
        .kill_on_drop(true)
        .spawn()
    {
        Ok(tui) => tui,
        Err(source) => {
            stop(&mut serve).await;
            remove(&folder);
            return Err(spawn_error(&program, source));
        }
    };
    let link = tokio::spawn(run_link(socket, humata, wrapper.retry));

    tokio::select! {
        status = tui.wait() => {
            link.abort();
            stop(&mut serve).await;
            remove(&folder);
            status
                .map(|status| exit_code(&status))
                .map_err(|source| WrapperError::Wait { source })
        }
        status = serve.wait() => {
            stop(&mut tui).await;
            link.abort();
            remove(&folder);
            let status = status.map_or_else(|error| error.to_string(), |s| s.to_string());
            Err(WrapperError::ServeEnded { status })
        }
    }
}

/// Every `-c <value>` pair in `args`, in order.
fn config_pairs(args: &[OsString]) -> Vec<&OsStr> {
    let mut pairs = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "-c"
            && let Some(value) = iter.next()
        {
            pairs.push(OsStr::new("-c"));
            pairs.push(value.as_os_str());
        }
    }
    pairs
}

/// Waits until `socket` exists as a socket; false if the app-server ends or
/// the time is up first. Codex makes `socket` a link to a socket of its own
/// elsewhere, so the link is followed.
async fn socket_appears(serve: &mut Child, socket: &Path, wait: Duration) -> bool {
    let appears = async {
        loop {
            if fs::metadata(socket).is_ok_and(|meta| meta.file_type().is_socket()) {
                return true;
            }
            if !matches!(serve.try_wait(), Ok(None)) {
                return false;
            }
            tokio::time::sleep(POLL).await;
        }
    };
    tokio::time::timeout(wait, appears).await.unwrap_or(false)
}

/// Runs plain Codex with the terminal and Humata's hooks.
async fn plain_codex(wrapper: &Wrapper) -> Result<i32, WrapperError> {
    let program = wrapper.program.to_string_lossy().into_owned();
    let mut command = Command::new(&wrapper.program);
    if let Some(hooks) = &wrapper.hooks {
        command.arg("-c").arg(format!("hooks={hooks}"));
    }
    let status = command
        .args(&wrapper.args)
        .current_dir(&wrapper.folder)
        .status()
        .await
        .map_err(|source| spawn_error(&program, source))?;
    Ok(exit_code(&status))
}

/// Makes a fresh owner-only folder under `root`.
fn private_folder(root: &Path) -> Result<PathBuf, WrapperError> {
    let io = |path: &Path, source| WrapperError::Io {
        path: path.to_path_buf(),
        source,
    };
    for _ in 0..16 {
        let mut bytes = [0u8; 8];
        File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut bytes))
            .map_err(|source| io(Path::new("/dev/urandom"), source))?;
        let name: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let folder = root.join(format!("h2cx-{name}"));
        match fs::DirBuilder::new().mode(0o700).create(&folder) {
            Ok(()) => return Ok(folder),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(source) => return Err(io(&folder, source)),
        }
    }
    Err(io(
        root,
        std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no unused folder name"),
    ))
}

fn remove(folder: &Path) {
    let _ = fs::remove_dir_all(folder);
}

fn spawn_error(program: &str, source: std::io::Error) -> WrapperError {
    if source.kind() == std::io::ErrorKind::NotFound {
        WrapperError::NotInstalled {
            program: program.to_owned(),
        }
    } else {
        WrapperError::Spawn {
            program: program.to_owned(),
            source,
        }
    }
}

/// The exit code, or 128 plus the signal, or 1.
fn exit_code(status: &ExitStatus) -> i32 {
    status
        .code()
        .or_else(|| status.signal().map(|signal| 128 + signal))
        .unwrap_or(1)
}

fn pid_of(pid: u32) -> Option<Pid> {
    i32::try_from(pid).ok().and_then(Pid::from_raw)
}

/// Ends `child` with SIGTERM, then kills it after the grace, and reaps it.
async fn stop(child: &mut Child) {
    if let Some(pid) = child.id().and_then(pid_of) {
        let _ = kill_process(pid, Signal::TERM);
    }
    if tokio::time::timeout(GRACE, child.wait()).await.is_err() {
        let _ = child.kill().await;
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;

    use super::*;

    fn running() -> Child {
        Command::new("sleep").arg("30").kill_on_drop(true).spawn().expect("sleep runs")
    }

    #[tokio::test]
    async fn a_link_to_a_socket_counts_as_the_socket() {
        let dir = tempfile::tempdir().expect("a folder");
        let real = dir.path().join("real.sock");
        let _listener = UnixListener::bind(&real).expect("a socket");
        let link = dir.path().join("s.sock");
        std::os::unix::fs::symlink(&real, &link).expect("a link");
        let mut serve = running();
        assert!(socket_appears(&mut serve, &link, Duration::from_secs(5)).await);
    }

    #[tokio::test]
    async fn a_link_to_nothing_is_not_a_socket() {
        let dir = tempfile::tempdir().expect("a folder");
        let link = dir.path().join("s.sock");
        std::os::unix::fs::symlink(dir.path().join("missing.sock"), &link).expect("a link");
        let mut serve = running();
        assert!(!socket_appears(&mut serve, &link, Duration::from_millis(50)).await);
    }
}
