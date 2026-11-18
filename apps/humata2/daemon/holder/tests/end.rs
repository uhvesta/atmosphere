//! Ending a holder by its pid when it doesn't take its shutdown. The holders
//! are real `humata2_holder` processes. A stopped one is the wedged holder
//! that a shutdown can't reach; the end is raced against a guard.

use std::path::{Path, PathBuf};
use std::time::Duration;

use humata2_holder::process::Snapshot;
use humata2_holder::{
    Event, HolderError, HolderSpec, Stopped, attach, end_by_pid, find_holder, read_saved,
    saved_scrollback_path, shutdown_or_end, spawn,
};
use rustix::process::{Pid, Signal, kill_process, test_kill_process};
use tempfile::TempDir;

/// Thirty seconds. A stopped holder is ended within a few seconds (the
/// shutdown's three, the grace's two and a kill's one), so this only
/// catches a hang.
const GUARD: Duration = Duration::from_secs(30);

fn holder_binary() -> PathBuf {
    let path = std::env::var_os("HUMATA2_HOLDER")
        .map(PathBuf::from)
        .or_else(|| option_env!("CARGO_BIN_EXE_humata2_holder").map(PathBuf::from))
        .expect("set HUMATA2_HOLDER to the humata2_holder binary");
    std::fs::canonicalize(path).unwrap()
}

fn pid_of(raw: u32) -> Pid {
    i32::try_from(raw)
        .ok()
        .and_then(Pid::from_raw)
        .expect("a positive pid")
}

/// A holder running `sh`, ended when the test ends.
struct Holder {
    socket: PathBuf,
    pid: u32,
    _dir: TempDir,
}

impl Holder {
    fn start() -> Self {
        Self::start_from(|_| holder_binary())
    }

    /// A holder running `sh` with `args`, in the test's folder.
    fn start_running(args: &[&str]) -> Self {
        Self::start_with(|_| holder_binary(), args)
    }

    /// A holder started from the binary `holder` gives, which it may put in
    /// the test's folder.
    fn start_from(holder: impl FnOnce(&Path) -> PathBuf) -> Self {
        Self::start_with(holder, &[])
    }

    fn start_with(holder: impl FnOnce(&Path) -> PathBuf, args: &[&str]) -> Self {
        // Socket paths must be short, so not under the test's own temp.
        let dir = tempfile::Builder::new()
            .prefix("h2e")
            .tempdir_in("/tmp")
            .unwrap();
        let spec = HolderSpec {
            holder: holder(dir.path()),
            socket: dir.path().join("t.sock"),
            program: PathBuf::from("/bin/sh"),
            args: args.iter().map(Into::into).collect(),
            cwd: dir.path().to_owned(),
            env: vec![
                ("PS1".into(), "$ ".into()),
                ("ENV".into(), "/dev/null".into()),
            ],
            cols: 80,
            rows: 24,
            scrollback_bytes: 1 << 20,
            log: dir.path().join("holder.log"),
        };
        let spawned = spawn(&spec).unwrap();
        Self {
            socket: spec.socket,
            pid: spawned.pid,
            _dir: dir,
        }
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        if still_holder(self.pid, &self.socket) {
            let _ = kill_process(pid_of(self.pid), Signal::TERM);
        }
    }
}

/// Resumes a stopped holder when the test ends, so none is left stopped.
struct Resumed(u32);

impl Drop for Resumed {
    fn drop(&mut self) {
        let _ = kill_process(pid_of(self.0), Signal::CONT);
    }
}

#[tokio::test]
async fn a_holder_that_takes_its_shutdown_is_shut_and_not_ended() {
    let holder = Holder::start();

    let stopped = tokio::time::timeout(GUARD, shutdown_or_end(&holder.socket, holder.pid))
        .await
        .expect("a responsive holder is shut down within the guard")
        .expect("a responsive holder is shut down without error");
    assert!(matches!(stopped, Stopped::Shut), "{stopped:?}");
}

#[tokio::test]
async fn a_stopped_holder_is_ended_by_its_pid_within_the_guard() {
    let holder = Holder::start();
    kill_process(pid_of(holder.pid), Signal::STOP).expect("the test can stop its own holder");
    // Declared after `holder`, so it resumes the holder before the holder ends.
    let _resumed = Resumed(holder.pid);

    let stopped = tokio::time::timeout(GUARD, shutdown_or_end(&holder.socket, holder.pid))
        .await
        .expect("the stopped holder was ended within the guard, rather than waited on")
        .expect("the stopped holder was ended by its pid");
    assert!(
        matches!(
            stopped,
            Stopped::Ended {
                reason: HolderError::ShutdownTimeout { .. }
            }
        ),
        "{stopped:?}"
    );
    let gone = !Snapshot::take()
        .await
        .expect("the process list can be read")
        .is_alive(holder.pid);
    assert!(gone, "the stopped holder is no longer running");
}

#[tokio::test]
async fn a_holder_ended_by_its_pid_is_gone_and_keeps_what_it_printed() {
    let holder = Holder::start();
    let attached = attach(&holder.socket, 0).await.unwrap();
    let (mut control, mut events) = (attached.control, attached.events);
    control.input(1, b"echo kept-$((3+4))\n").await.unwrap();
    let mut printed = Vec::new();
    while !printed.windows(6).any(|window| window == b"kept-7") {
        match tokio::time::timeout(GUARD, events.next()).await {
            Ok(Ok(Some(Event::Output { data, .. }))) => printed.extend(data),
            Ok(Ok(Some(_))) => {}
            other => panic!("the holder stopped printing: {other:?}"),
        }
    }

    tokio::time::timeout(GUARD, end_by_pid(&holder.socket, holder.pid, Duration::from_secs(5)))
        .await
        .expect("the holder was ended within the guard")
        .expect("the holder was ended by its pid");
    let gone = !Snapshot::take()
        .await
        .expect("the process list can be read")
        .is_alive(holder.pid);
    assert!(gone, "the holder is no longer running");
    let saved = read_saved(&saved_scrollback_path(&holder.socket))
        .unwrap()
        .expect("what it printed is kept for the holder after it");
    assert!(saved.saved_through > 0);
}

/// A program that ignores a hangup, as an agent's CLI or something it
/// started may. Its pid is read from the file it writes first.
struct Program(u32);

impl Program {
    async fn started_by(dir: &Path) -> Self {
        let file = dir.join("program.pid");
        let deadline = tokio::time::Instant::now() + GUARD;
        loop {
            if let Some(pid) = std::fs::read_to_string(&file)
                .ok()
                .and_then(|text| text.trim().parse().ok())
            {
                return Self(pid);
            }
            assert!(tokio::time::Instant::now() < deadline, "the program never wrote its pid");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

impl Drop for Program {
    /// Ends the program when the test ends, however it ended: only while
    /// it is still this test's `sleep`, so a reused pid is left alone.
    fn drop(&mut self) {
        let named = std::process::Command::new("/bin/ps")
            .args(["-o", "command=", "-p", &self.0.to_string()])
            .output()
            .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains("sleep 1000"));
        if named {
            let _ = kill_process(pid_of(self.0), Signal::KILL);
        }
    }
}

/// A holder that has to be killed can't end its program itself, so ending
/// it by its pid ends its program's group too: no agent outlives its
/// holder, even one that ignores the hangup.
#[tokio::test]
async fn a_holder_ended_by_its_pid_takes_its_program_with_it() {
    let holder = Holder::start_running(&[
        "-c",
        "echo $$ > program.pid; trap '' HUP; exec sleep 1000",
    ]);
    let program = Program::started_by(holder._dir.path()).await;

    // Shorter than the holder's own hangup grace, so it is killed while it
    // still waits on its program.
    tokio::time::timeout(GUARD, end_by_pid(&holder.socket, holder.pid, Duration::from_millis(100)))
        .await
        .expect("the holder was ended within the guard")
        .expect("the holder was ended by its pid");
    let snapshot = Snapshot::take().await.expect("the process list can be read");
    assert!(!snapshot.is_alive(holder.pid), "the holder is no longer running");
    assert!(!snapshot.is_alive(program.0), "the holder's program outlived it");
}

/// An installed holder can have a file name of its own. It is still the
/// process serving the socket, so it is ended.
#[tokio::test]
async fn a_holder_under_a_name_of_its_own_is_ended_by_its_pid() {
    let holder = Holder::start_from(|dir| {
        let renamed = dir.join("pty_keeper");
        std::fs::copy(holder_binary(), &renamed).expect("the holder binary can be copied");
        renamed
    });

    tokio::time::timeout(GUARD, end_by_pid(&holder.socket, holder.pid, Duration::from_secs(5)))
        .await
        .expect("the holder was ended within the guard")
        .expect("the holder was ended by its pid");
    let gone = !Snapshot::take()
        .await
        .expect("the process list can be read")
        .is_alive(holder.pid);
    assert!(gone, "the renamed holder is no longer running");
}

/// A holder whose pid file is gone is found by the socket it serves; a
/// socket no holder serves has none.
#[tokio::test]
async fn a_holder_whose_pid_file_is_gone_is_found_by_its_socket() {
    let holder = Holder::start();
    std::fs::remove_file(holder.socket.with_extension("pid")).expect("the pid file is there");

    let found = find_holder(&holder.socket).await.expect("the process list can be read");
    assert_eq!(found, Some(holder.pid));
    let nobody = holder.socket.with_file_name("nobody.sock");
    let found = find_holder(&nobody).await.expect("the process list can be read");
    assert_eq!(found, None);
}

#[tokio::test]
async fn ending_a_pid_that_is_not_a_holder_leaves_it_running() {
    let dir = tempfile::tempdir().unwrap();
    let mut other = std::process::Command::new("sleep")
        .arg("1000")
        .spawn()
        .expect("sleep starts");
    let pid = other.id();

    tokio::time::timeout(GUARD, end_by_pid(&dir.path().join("gone.sock"), pid, Duration::from_secs(5)))
        .await
        .expect("ended within the guard")
        .expect("a pid that isn't a holder is not an error");
    let alive = Snapshot::take()
        .await
        .expect("the process list can be read")
        .is_alive(pid);
    let _ = other.kill();
    let _ = other.wait();
    assert!(alive, "a process that isn't a holder was signalled");
}

#[tokio::test]
async fn a_pid_that_is_not_a_holder_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let mut other = std::process::Command::new("sleep")
        .arg("1000")
        .spawn()
        .expect("sleep starts");
    let pid = other.id();

    let stopped = tokio::time::timeout(GUARD, shutdown_or_end(&dir.path().join("gone.sock"), pid))
        .await
        .expect("a socket that is gone is ended within the guard")
        .expect("a pid that isn't a holder is not an error");
    assert!(
        matches!(
            stopped,
            Stopped::Ended {
                reason: HolderError::Connect { .. }
            }
        ),
        "{stopped:?}"
    );
    assert!(
        test_kill_process(pid_of(pid)).is_ok(),
        "a process that is not a holder is untouched"
    );

    kill_process(pid_of(pid), Signal::KILL).expect("the test stops its own sleep");
    let _ = other.wait();
}

/// Whether `pid` is still the holder on `socket`. A holder runs in a
/// session of its own, so once it ends its pid is free for any process on
/// the machine: a signal sent to it then could end another test's daemon.
fn still_holder(pid: u32, socket: &std::path::Path) -> bool {
    std::process::Command::new("/bin/ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains(&*socket.to_string_lossy()))
}
