//! `humatad2 stop`: stops this machine's sessions from the machine itself
//! (D262). The host runs it over a one-off SSH when a box's link doesn't
//! answer, and a person runs it at the box's terminal.
//!
//! It asks the daemon on its socket first. A daemon that doesn't answer in
//! time, or refuses a caller with no login, is stopped the hard way: the
//! stop record is written, the daemon is killed, and the holders are swept
//! as a stopped boot sweeps them (D269). Its service manager starts the
//! daemon again, which reads the record, counts what was ended here, and
//! sweeps whatever is left (D261), so the machine stays stopped.
//!
//! A daemon that already took a stop and left its socket to start itself
//! again is not ended: its next run sweeps, and a late SSH stop that killed
//! it there would leave the machine with no daemon to finish the stop.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hyper_util::rt::TokioIo;
use rustix::io::Errno;
use rustix::process::{Pid, Signal, kill_process};
use tokio::net::UnixStream;
use tonic::transport::{Endpoint, Uri};
use tonic::{Code, Request};

use super::sweep::{GRACE, Sweep, SweepError, holder_names};
use super::{ByKind, Record, RecordError, StopFolder, StopState, StoppedBy, Via};
use crate::pb;

/// How long the daemon has to answer before it is stopped the hard way.
const ASK_DEADLINE: Duration = Duration::from_secs(3);

#[derive(clap::Args)]
pub struct StopArgs {
    /// Makes a repeated stop act once. Default: a new one.
    #[arg(long)]
    pub id: Option<String>,
    /// The machine that asked, when the host stops this machine over SSH.
    /// Without it, the stop is this machine's own, made at its terminal.
    #[arg(long)]
    pub by: Option<String>,
    /// Why, shown on the machine's page.
    #[arg(long)]
    pub reason: Option<String>,
    /// Where the daemon keeps its state. Default: ~/.humata2/state.
    #[arg(long)]
    pub state: Option<PathBuf>,
    /// The daemon's socket. Default: ~/.humata2/humatad.sock.
    #[arg(long)]
    pub socket: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum StopCliError {
    #[error("HOME is not set; pass --state and --socket explicitly")]
    NoHome,

    #[error("background task failed: {source}")]
    Runtime {
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't save the stop record, so nothing was stopped the hard way; check that the state folder belongs to you and has room, then run `humatad2 stop` again"
    )]
    Record {
        #[source]
        source: RecordError,
    },

    #[error(
        "couldn't read the daemon's lock file {}; check that it belongs to you. The stop is recorded, so the daemon stops its sessions at its next start",
        path.display()
    )]
    Lock {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't end the daemon (process {pid}); the stop is recorded, so end it by hand (`kill -9 {pid}`) and the next start stops its sessions"
    )]
    Kill {
        pid: u32,
        #[source]
        source: Errno,
    },

    #[error(
        "couldn't end this machine's holders; the stop is recorded, so the daemon ends them at its next start"
    )]
    Sweep {
        #[source]
        source: SweepError,
    },

    #[error(
        "couldn't note which holders were ended, so the daemon's next start counts fewer in what the stop ended; the stop itself holds"
    )]
    Note {
        #[source]
        source: RecordError,
    },
}

/// Stops this machine's sessions.
pub async fn run(args: StopArgs) -> Result<(), StopCliError> {
    let home = || {
        std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(|home| PathBuf::from(home).join(".humata2"))
            .ok_or(StopCliError::NoHome)
    };
    let state = match args.state {
        Some(state) => state,
        None => home()?.join("state"),
    };
    // Resolved as the daemon resolves it, so the holders' sockets this
    // command ends match the ones the daemon's tabs record. A folder that
    // can't be resolved is used as given: there is nothing in it to end.
    let state = std::fs::canonicalize(&state).unwrap_or(state);
    let socket = match args.socket {
        Some(socket) => socket,
        None => home()?.join("humatad.sock"),
    };
    let stop_id = args
        .id
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let reason = args.reason.unwrap_or_default();
    let by = args.by.filter(|by| !by.is_empty());
    if ask_daemon(&socket, &stop_id, &reason, by.as_deref()).await {
        println!("Stopped this machine's sessions.");
        return Ok(());
    }
    let hard = Hard {
        state: state.clone(),
        stop_id,
        reason,
        by,
        listening: listening(&socket).await,
    };
    let killed = match blocking(move || hard.stop()).await? {
        Next::Done(said) => {
            println!("{said}");
            return Ok(());
        }
        Next::Sweep { killed } => killed,
    };
    let swept = Sweep {
        holders: state.join("holders"),
        grace: GRACE,
        programs: holder_names(Path::new("")),
    }
    .run()
    .await
    .map_err(|source| StopCliError::Sweep { source })?;
    humata2_daemon_runtime::fault::point("stop.remote.cli.holders_signalled");
    let ended = swept.ended.len();
    let folder = StopFolder::new(&state);
    blocking(move || {
        folder
            .note_ended_early(&swept.ended)
            .map_err(|source| StopCliError::Note { source })
    })
    .await?;
    println!(
        "Stopped this machine's sessions{}; ended {ended} holder(s). The daemon's next start finishes the stop.",
        if killed { " and ended its daemon" } else { "" }
    );
    Ok(())
}

/// Runs blocking work of this command off the async threads.
async fn blocking<R: Send + 'static>(
    work: impl FnOnce() -> Result<R, StopCliError> + Send + 'static,
) -> Result<R, StopCliError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|source| StopCliError::Runtime {
            source: std::io::Error::other(source),
        })?
}

/// What the hard way's first part leaves to do.
enum Next {
    /// Nothing more: the line says why.
    Done(String),
    /// The holders are swept next; `killed` when a daemon was ended.
    Sweep { killed: bool },
}

/// Asks the daemon to stop. True when it took the stop, or already has it
/// (a stop it applied before, or sessions stopped already).
async fn ask_daemon(socket: &Path, stop_id: &str, reason: &str, by: Option<&str>) -> bool {
    let path = socket.to_owned();
    // The address is a placeholder: the connector dials the socket.
    let channel = Endpoint::from_static("http://[::]:50051").connect_with_connector_lazy(
        tower::service_fn(move |_: Uri| {
            let path = path.clone();
            async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?)) }
        }),
    );
    let request = pb::StopSessionsRequest {
        stop_id: stop_id.to_owned(),
        reason: reason.to_owned(),
        via: if by.is_some() {
            pb::StopVia::Ssh
        } else {
            pb::StopVia::Direct
        } as i32,
        by_machine: by.unwrap_or_default().to_owned(),
    };
    let mut client = pb::daemon_service_client::DaemonServiceClient::new(channel);
    match tokio::time::timeout(ASK_DEADLINE, client.stop_sessions(Request::new(request))).await {
        Ok(Ok(_)) => true,
        // Applied before, or stopped and unreadable: either way it stays so.
        Ok(Err(status)) => status.code() == Code::FailedPrecondition,
        Err(_) => false,
    }
}

/// Whether a daemon listens on `socket`. One that took a stop has removed
/// its socket and is starting itself again into the sweep; one that hangs
/// keeps it. A probe that can't tell counts as listening.
async fn listening(socket: &Path) -> bool {
    match tokio::time::timeout(ASK_DEADLINE, UnixStream::connect(socket)).await {
        Ok(Err(error)) => !matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
        ),
        Ok(Ok(_)) | Err(_) => true,
    }
}

/// The hard way: for a daemon that doesn't answer.
struct Hard {
    state: PathBuf,
    stop_id: String,
    reason: String,
    by: Option<String>,
    /// Whether a daemon listened on the socket once it didn't answer.
    listening: bool,
}

impl Hard {
    /// Records the stop and ends the daemon; the holders are swept after.
    /// Blocks.
    fn stop(&self) -> Result<Next, StopCliError> {
        let folder = StopFolder::new(&self.state);
        match folder.read() {
            // Already stopped and swept: nothing is left to end.
            Ok(StopState::Stopped(record)) if record.ended.is_some() => {
                return Ok(Next::Done(format!(
                    "This machine's sessions are stopped already (stop {}).",
                    record.stop_id
                )));
            }
            // Stopped, and the daemon holding the lock has left its socket:
            // it took this stop (or an earlier one) and is starting itself
            // again, and that next run sweeps. Ending it would leave the
            // machine with no daemon to finish the stop (D261).
            Ok(StopState::Stopped(record)) if !self.listening && self.lock_holder()?.is_some() => {
                return Ok(Next::Done(format!(
                    "This machine's sessions are being stopped already (stop {}); its daemon is starting itself again to end them.",
                    record.stop_id
                )));
            }
            // A record that can't be read keeps the machine stopped, and
            // is not written over.
            Ok(StopState::Stopped(_)) | Err(_) => {}
            Ok(StopState::Running | StopState::Starting(_)) => {
                match folder.refuse_applied(&self.stop_id) {
                    Ok(()) => {}
                    Err(RecordError::AlreadyApplied { stop_id }) => {
                        return Ok(Next::Done(format!(
                            "Stop {stop_id} was applied and started again before; nothing was stopped."
                        )));
                    }
                    Err(source) => return Err(StopCliError::Record { source }),
                }
                folder
                    .write_stopped(&self.record())
                    .map_err(|source| StopCliError::Record { source })?;
            }
        }
        humata2_daemon_runtime::fault::point("stop.remote.cli.record_written");
        let killed = self.end_daemon()?;
        humata2_daemon_runtime::fault::point("stop.remote.cli.daemon_killed");
        Ok(Next::Sweep { killed })
    }

    fn record(&self) -> Record {
        let (kind, via, machine) = match &self.by {
            Some(by) => (ByKind::User, Via::Ssh, by.clone()),
            None => (ByKind::Terminal, Via::Direct, self.own_machine()),
        };
        Record {
            stop_id: self.stop_id.clone(),
            stopped_at_unix_ms: super::service::unix_ms(),
            by: StoppedBy {
                kind,
                via,
                machine,
                session: None,
            },
            reason: Some(self.reason.clone()).filter(|reason| !reason.is_empty()),
            ended: None,
        }
    }

    /// This machine's own id, as the daemon wrote it in the state folder.
    fn own_machine(&self) -> String {
        std::fs::read_to_string(self.state.join("machine-id"))
            .map(|id| id.trim().to_owned())
            .ok()
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| "this machine".to_owned())
    }

    /// Kills the daemon that holds the state lock, if one does. True when
    /// one was killed.
    fn end_daemon(&self) -> Result<bool, StopCliError> {
        let Some(pid) = self.lock_holder()? else {
            return Ok(false);
        };
        // The lock's holder wrote its own pid; SIGKILL can't be refused or
        // caught, so a daemon that hangs is ended too.
        match kill_process(pid, Signal::KILL) {
            Ok(()) => Ok(true),
            Err(Errno::SRCH) => Ok(false),
            Err(source) => Err(StopCliError::Kill {
                pid: pid.as_raw_nonzero().get().unsigned_abs(),
                source,
            }),
        }
    }

    /// The daemon that holds the state lock, as the pid it wrote there;
    /// none when nobody holds it.
    fn lock_holder(&self) -> Result<Option<Pid>, StopCliError> {
        let path = self.state.join("daemon.lock");
        let lock_failed = |source| StopCliError::Lock {
            path: path.clone(),
            source,
        };
        let file = match File::open(&path) {
            Ok(file) => file,
            // No daemon has ever run here.
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(lock_failed(source)),
        };
        match file.try_lock() {
            // Nobody holds it: no daemon runs, and the lock is let go here.
            Ok(()) => return Ok(None),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(source)) => return Err(lock_failed(source)),
        }
        let text = std::fs::read_to_string(&path).map_err(lock_failed)?;
        Ok(text
            .trim()
            .parse::<u32>()
            .ok()
            .and_then(|pid| i32::try_from(pid).ok())
            .and_then(Pid::from_raw))
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn hard(dir: &TempDir, by: Option<&str>) -> Hard {
        Hard {
            state: dir.path().to_owned(),
            stop_id: "s1".to_owned(),
            reason: "agents running wild".to_owned(),
            by: by.map(str::to_owned),
            listening: true,
        }
    }

    fn lock_holder(dir: &TempDir) -> (std::process::Child, File) {
        let child = std::process::Command::new("/bin/sleep")
            .arg("60")
            .spawn()
            .expect("sleep runs");
        let path = dir.path().join("daemon.lock");
        std::fs::write(&path, format!("{}\n", child.id())).expect("a lock file");
        let lock = File::open(&path).expect("the lock file opens");
        lock.try_lock().expect("the lock is free");
        (child, lock)
    }

    fn was_killed(child: &mut std::process::Child) -> bool {
        match child.try_wait().expect("the child can be asked") {
            Some(_) => true,
            None => {
                std::thread::sleep(Duration::from_millis(100));
                child.try_wait().expect("the child can be asked").is_some()
            }
        }
    }

    #[test]
    fn a_daemon_that_left_its_socket_after_taking_a_stop_is_left_to_sweep_it() {
        let dir = TempDir::new().expect("a temp dir");
        hard(&dir, None).stop().expect("recorded");
        let (mut daemon, _lock) = lock_holder(&dir);
        let late = Hard {
            stop_id: "s1".to_owned(),
            listening: false,
            ..hard(&dir, Some("host-mac"))
        };

        let next = late.stop().expect("answered");

        assert!(matches!(next, Next::Done(_)), "the stop went on the hard way");
        assert!(
            !was_killed(&mut daemon),
            "the daemon starting itself into the sweep was ended"
        );
        let _ = daemon.kill();
        let _ = daemon.wait();
    }

    #[test]
    fn a_stopped_daemon_that_listens_and_does_not_answer_is_ended() {
        let dir = TempDir::new().expect("a temp dir");
        hard(&dir, None).stop().expect("recorded");
        let (mut daemon, _lock) = lock_holder(&dir);

        let next = hard(&dir, None).stop().expect("taken");

        assert!(matches!(next, Next::Sweep { killed: true }));
        let _ = daemon.wait();
    }

    #[test]
    fn a_stop_made_at_the_terminal_is_recorded_as_the_terminals_own() {
        let dir = TempDir::new().expect("a temp dir");
        std::fs::write(dir.path().join("machine-id"), "ma_box\n").expect("an id");
        let next = hard(&dir, None).stop().expect("recorded");
        assert!(matches!(next, Next::Sweep { killed: false }));
        let StopState::Stopped(record) = StopFolder::new(dir.path()).read().expect("readable")
        else {
            panic!("the machine isn't recorded as stopped");
        };
        assert_eq!(record.stop_id, "s1");
        assert_eq!(record.by.kind, ByKind::Terminal);
        assert_eq!(record.by.via, Via::Direct);
        assert_eq!(record.by.machine, "ma_box");
        assert_eq!(record.reason.as_deref(), Some("agents running wild"));
    }

    #[test]
    fn a_stop_the_host_asked_for_is_recorded_as_the_users_over_ssh_from_that_machine() {
        let dir = TempDir::new().expect("a temp dir");
        hard(&dir, Some("host-mac")).stop().expect("recorded");
        let StopState::Stopped(record) = StopFolder::new(dir.path()).read().expect("readable")
        else {
            panic!("the machine isn't recorded as stopped");
        };
        assert_eq!(record.by.kind, ByKind::User);
        assert_eq!(record.by.via, Via::Ssh);
        assert_eq!(record.by.machine, "host-mac");
    }

    #[test]
    fn stopping_a_machine_already_stopped_keeps_the_first_stop() {
        let dir = TempDir::new().expect("a temp dir");
        hard(&dir, None).stop().expect("recorded");
        let second = Hard {
            stop_id: "s2".to_owned(),
            ..hard(&dir, None)
        };
        second.stop().expect("taken");
        let StopState::Stopped(record) = StopFolder::new(dir.path()).read().expect("readable")
        else {
            panic!("the machine isn't recorded as stopped");
        };
        assert_eq!(record.stop_id, "s1");
    }

    #[test]
    fn a_stop_already_applied_and_started_from_changes_nothing() {
        let dir = TempDir::new().expect("a temp dir");
        let folder = StopFolder::new(dir.path());
        hard(&dir, None).stop().expect("recorded");
        folder.begin_start().expect("started");
        folder.finish_start().expect("finished");
        hard(&dir, None).stop().expect("answered");
        assert_eq!(folder.read().expect("readable"), StopState::Running);
    }

    #[test]
    fn a_state_folder_with_no_daemon_has_none_to_end() {
        let dir = TempDir::new().expect("a temp dir");
        assert!(!hard(&dir, None).end_daemon().expect("read"));
        std::fs::write(dir.path().join("daemon.lock"), "999999\n").expect("a stale lock file");
        assert!(
            !hard(&dir, None).end_daemon().expect("read"),
            "nobody holds the lock"
        );
    }
}
