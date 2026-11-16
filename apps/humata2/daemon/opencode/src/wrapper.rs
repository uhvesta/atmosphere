//! The `humata opencode` wrapper (D201, D201a): runs OpenCode's own server
//! with a fresh password, attaches the TUI to the conversation's session,
//! links the two to Humata and ends the server with the TUI.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustix::process::{Pid, Signal, kill_process, test_kill_process};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::http::Http;
use crate::link::{Humata, Server, run_link};
use crate::serve::{listening_url, session_id, supported};
use crate::state::{StateError, StateFile};

/// How long a leftover server gets to answer before it is taken as gone.
const PROBE: Duration = Duration::from_secs(2);
/// How long a process gets to end on SIGTERM before it is killed.
const GRACE: Duration = Duration::from_secs(3);
/// How long a leftover server gets to end on SIGTERM.
const LEFTOVER_GRACE: Duration = Duration::from_secs(2);
/// How long creating the session is tried.
const SESSION_TRIES: Duration = Duration::from_secs(10);
/// The most the server's log keeps.
const LOG_CAP: usize = 1 << 20;

/// What the wrapper runs and where it keeps its files.
pub struct Wrapper {
    /// The `opencode` program.
    pub program: PathBuf,
    /// The Humata conversation, which names the OpenCode session.
    pub conversation: String,
    /// The state file the daemon reads.
    pub state: PathBuf,
    /// The folder the session works in.
    pub folder: PathBuf,
    /// The wait between retries.
    pub retry: Duration,
    /// How long the server gets to print its URL.
    pub wait_for_url: Duration,
}

/// Why the wrapper could not run.
#[derive(Debug, thiserror::Error)]
pub enum WrapperError {
    /// The program is not there.
    #[error(
        "`{program}` was not found, so OpenCode is not on PATH. Install OpenCode with its own \
         installer or package manager, then run this again."
    )]
    NotInstalled { program: String },
    /// The program could not be started.
    #[error(
        "Could not start `{program}`. Check that it is OpenCode and that you may run it, then \
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
    #[error("Waiting for OpenCode failed. Close any OpenCode left running and try again.")]
    Wait {
        #[source]
        source: std::io::Error,
    },
    /// The state file could not be written.
    #[error("Could not write the state file for the Humata daemon. Fix the problem named below.")]
    State {
        #[source]
        source: StateError,
    },
    /// The server ended before the TUI.
    #[error(
        "OpenCode's server stopped ({status}), so its TUI was closed; its log is at {}. Restart \
         the session to start both again.",
        .log.display()
    )]
    ServeEnded { status: String, log: PathBuf },
}

/// Runs the wrapper and returns the TUI's exit code.
pub async fn run_wrapper<H: Humata>(wrapper: Wrapper, humata: Arc<H>) -> Result<i32, WrapperError> {
    let program = wrapper.program.to_string_lossy().into_owned();
    let output = version_output(&wrapper.program).await?;
    if let Err(error) = supported(&output) {
        eprintln!("{error}");
        return plain_tui(&wrapper.program).await;
    }
    end_leftover(&wrapper.state).await;

    let password = fresh_password()?;
    let log_path = log_path(&wrapper.state);
    prepare_state_folder(&wrapper.state)?;
    let log = open_log(&log_path).map(Arc::new);

    let mut serve = Command::new(&wrapper.program)
        .args(["serve", "--hostname", "127.0.0.1", "--port", "0"])
        .env("OPENCODE_SERVER_PASSWORD", &password)
        .current_dir(&wrapper.folder)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| spawn_error(&program, source))?;
    let mut stdout = serve.stdout.take().map(BufReader::new);
    if let Some(stderr) = serve.stderr.take() {
        drain(stderr, log.clone());
    }
    let url = match stdout.as_mut() {
        Some(reader) => {
            tokio::time::timeout(wrapper.wait_for_url, url_line(reader, log.as_deref()))
                .await
                .ok()
                .flatten()
        }
        None => None,
    };
    if let Some(reader) = stdout {
        drain(reader, log.clone());
    }
    let (Some(url), Some(serve_pid)) = (url, serve.id()) else {
        let _ = serve.kill().await;
        eprintln!(
            "OpenCode's server gave no loopback URL, so the TUI runs without Humata's turn and \
             Inbox link."
        );
        return plain_tui(&wrapper.program).await;
    };

    let session = session_id(&wrapper.conversation);
    StateFile {
        url: url.clone(),
        password: password.clone(),
        session: session.clone(),
        serve_pid,
    }
    .write(&wrapper.state)
    .map_err(|source| WrapperError::State { source })?;

    let Ok(http) = Http::new(&url, &password) else {
        let _ = serve.kill().await;
        eprintln!("OpenCode's server gave no loopback URL, so the TUI runs without Humata's link.");
        return plain_tui(&wrapper.program).await;
    };
    let http = Arc::new(http);
    create_session(&http, &session, &wrapper.folder, wrapper.retry).await;

    let mut tui = match Command::new(&wrapper.program)
        .args(["--server", &url, "-s", &session])
        .env("OPENCODE_PASSWORD", &password)
        .env_remove("OPENCODE_SERVER_PASSWORD")
        .current_dir(&wrapper.folder)
        .kill_on_drop(true)
        .spawn()
    {
        Ok(tui) => tui,
        Err(source) => {
            stop(&mut serve, GRACE).await;
            return Err(spawn_error(&program, source));
        }
    };
    let link = tokio::spawn(run_link(Arc::clone(&http), humata, wrapper.retry));

    tokio::select! {
        status = tui.wait() => {
            link.abort();
            stop(&mut serve, GRACE).await;
            status
                .map(|status| exit_code(&status))
                .map_err(|source| WrapperError::Wait { source })
        }
        status = serve.wait() => {
            stop(&mut tui, GRACE).await;
            link.abort();
            let status = status.map_or_else(|error| error.to_string(), |s| s.to_string());
            Err(WrapperError::ServeEnded { status, log: log_path })
        }
    }
}

/// The text `<program> --version` prints.
async fn version_output(program: &Path) -> Result<String, WrapperError> {
    let name = program.to_string_lossy().into_owned();
    let output = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|source| spawn_error(&name, source))?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text)
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

/// Runs the TUI alone, with the terminal, and returns its exit code.
async fn plain_tui(program: &Path) -> Result<i32, WrapperError> {
    let name = program.to_string_lossy().into_owned();
    let status = Command::new(program)
        .status()
        .await
        .map_err(|source| spawn_error(&name, source))?;
    Ok(exit_code(&status))
}

/// The exit code, or 128 plus the signal, or 1.
fn exit_code(status: &ExitStatus) -> i32 {
    status
        .code()
        .or_else(|| status.signal().map(|signal| 128 + signal))
        .unwrap_or(1)
}

/// Ends the server a crashed wrapper left, if its state file names one that still answers.
async fn end_leftover(state: &Path) {
    let Ok(file) = StateFile::read(state) else {
        return;
    };
    let Ok(http) = Http::new(&file.url, &file.password) else {
        return;
    };
    let answered = tokio::time::timeout(PROBE, http.active()).await;
    if !matches!(answered, Ok(Ok(_))) {
        return;
    }
    let Some(pid) = pid_of(file.serve_pid) else {
        return;
    };
    let _ = kill_process(pid, Signal::TERM);
    let gone = async {
        while test_kill_process(pid).is_ok() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    if tokio::time::timeout(LEFTOVER_GRACE, gone).await.is_err() {
        let _ = kill_process(pid, Signal::KILL);
    }
}

fn pid_of(pid: u32) -> Option<Pid> {
    i32::try_from(pid).ok().and_then(Pid::from_raw)
}

/// 32 random bytes from the system, in hex.
fn fresh_password() -> Result<String, WrapperError> {
    let urandom = Path::new("/dev/urandom");
    let io = |source| WrapperError::Io {
        path: urandom.to_path_buf(),
        source,
    };
    let mut bytes = [0u8; 32];
    File::open(urandom)
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(io)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The log beside the state file: its path with `.log` added.
fn log_path(state: &Path) -> PathBuf {
    let mut name = state.as_os_str().to_owned();
    name.push(".log");
    PathBuf::from(name)
}

/// Makes the state file's folder, owner-only.
fn prepare_state_folder(state: &Path) -> Result<(), WrapperError> {
    let Some(folder) = state
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    else {
        return Ok(());
    };
    let io = |source| WrapperError::Io {
        path: folder.to_path_buf(),
        source,
    };
    fs::create_dir_all(folder).map_err(io)?;
    fs::set_permissions(folder, fs::Permissions::from_mode(0o700)).map_err(io)
}

/// The server's log, capped.
struct Log {
    inner: Mutex<(File, usize)>,
}

impl Log {
    /// Writes `bytes` unless the cap is reached.
    fn write(&self, bytes: &[u8]) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        let (file, written) = &mut *inner;
        if *written >= LOG_CAP {
            return;
        }
        let room = (LOG_CAP - *written).min(bytes.len());
        if file.write_all(&bytes[..room]).is_ok() {
            *written += room;
        }
    }
}

fn open_log(path: &Path) -> Option<Log> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .ok()?;
    Some(Log {
        inner: Mutex::new((file, 0)),
    })
}

/// Reads lines until the loopback URL `serve` prints, logging them.
async fn url_line<R: AsyncBufReadExt + Unpin>(reader: &mut R, log: Option<&Log>) -> Option<String> {
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => return None,
            Ok(_) => {}
        }
        if let Some(log) = log {
            log.write(&line);
        }
        if let Some(url) = listening_url(&String::from_utf8_lossy(&line)) {
            return Some(url);
        }
    }
}

/// Reads `reader` to its end in the background, keeping it in the log.
fn drain<R: AsyncRead + Unpin + Send + 'static>(mut reader: R, log: Option<Arc<Log>>) {
    tokio::spawn(async move {
        let mut buffer = [0u8; 4096];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    if let (Some(log), Some(chunk)) = (&log, buffer.get(..n)) {
                        log.write(chunk);
                    }
                }
            }
        }
    });
}

/// Creates the session, trying again until it works or the time is up.
async fn create_session(http: &Http, id: &str, folder: &Path, retry: Duration) {
    let tries = async {
        while http.create_session(id, folder).await.is_err() {
            tokio::time::sleep(retry).await;
        }
    };
    if tokio::time::timeout(SESSION_TRIES, tries).await.is_err() {
        eprintln!(
            "Could not create the OpenCode session; the TUI opens anyway, but it may not match \
             this conversation."
        );
    }
}

/// Ends `child` with SIGTERM, then kills it after `grace`, and reaps it.
async fn stop(child: &mut Child, grace: Duration) {
    if let Some(pid) = child.id().and_then(pid_of) {
        let _ = kill_process(pid, Signal::TERM);
    }
    if tokio::time::timeout(grace, child.wait()).await.is_err() {
        let _ = child.kill().await;
    }
}
