//! D398: a daemon says it listens, and only once it does. Its starter
//! hears READY=1 on the `--ready-fd` pipe, a service manager hears it on
//! `NOTIFY_SOCKET`, and the socket answers by then. A daemon that won't
//! listen closes the pipe without a word.

use std::io::Read;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::os::unix::net::{UnixDatagram, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use rustix::io::{FdFlags, fcntl_setfd};

/// A safety net, never a pace: each wait ends on the daemon's word.
const DEADLINE: Duration = Duration::from_secs(60);

fn binary(variable: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(variable).unwrap_or_else(|| panic!("set {variable}")))
}

fn folder(mode: u32) -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::Builder::new()
        .prefix("h2ry")
        .tempdir_in("/tmp")
        .expect("make a folder");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(mode))
        .expect("set the folder's mode");
    dir
}

fn daemon(dir: &Path) -> Command {
    let mut command = Command::new(binary("HUMATAD2"));
    command
        .arg("serve")
        .arg("--socket")
        .arg(dir.join("d.sock"))
        .arg("--state")
        .arg(dir.join("state"))
        .arg("--root")
        .arg(dir)
        .arg("--holder")
        .arg(binary("HUMATA2_HOLDER"))
        .arg("--shell")
        .arg("/bin/sh")
        .env_remove("NOTIFY_SOCKET")
        .stdin(Stdio::null());
    command
}

/// Starts `command` with a ready pipe, as `humatad2 bridge` does, and
/// returns what the daemon wrote before it closed the pipe.
fn start_with_pipe(mut command: Command) -> (Child, Vec<u8>) {
    let (mut reader, writer) = std::io::pipe().expect("make the ready pipe");
    let fd = writer.as_raw_fd();
    command.arg(format!("--ready-fd={fd}"));
    // SAFETY: only fcntl runs between fork and exec.
    unsafe {
        command.pre_exec(move || {
            fcntl_setfd(BorrowedFd::borrow_raw(fd), FdFlags::empty())?;
            Ok(())
        });
    }
    let child = command.spawn().expect("start humatad2");
    drop(writer);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut got = Vec::new();
        let mut byte = [0u8; 1];
        // Up to the line's end, or the pipe's: a daemon that listens says
        // READY=1 and a newline; one that won't closes the pipe.
        while let Ok(1) = reader.read(&mut byte) {
            got.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
        let _ = tx.send(got);
    });
    let got = rx.recv_timeout(DEADLINE).expect("the daemon says something or closes the pipe");
    (child, got)
}

fn stop(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn a_daemon_started_with_a_pipe_says_ready_and_then_answers() {
    let dir = folder(0o700);
    let (child, got) = start_with_pipe(daemon(dir.path()));
    assert!(got.starts_with(b"READY=1"), "the starter hears READY=1: {got:?}");
    let connected = UnixStream::connect(dir.path().join("d.sock"));
    stop(child);
    assert!(connected.is_ok(), "the socket answers once it said ready");
}

#[test]
fn a_daemon_under_a_service_manager_says_ready_and_then_answers() {
    let dir = folder(0o700);
    let notify = dir.path().join("notify");
    let manager = UnixDatagram::bind(&notify).expect("bind the manager's socket");
    manager
        .set_read_timeout(Some(DEADLINE))
        .expect("set the manager's deadline");
    let mut command = daemon(dir.path());
    command.env("NOTIFY_SOCKET", &notify);
    let child = command.spawn().expect("start humatad2");
    let mut got = [0u8; 256];
    let heard = manager.recv(&mut got);
    let connected = UnixStream::connect(dir.path().join("d.sock"));
    stop(child);
    let n = heard.expect("the service manager hears the daemon");
    assert_eq!(&got[..n], b"READY=1");
    assert!(connected.is_ok(), "the socket answers once it said ready");
}

#[test]
fn a_daemon_that_wont_listen_closes_the_pipe_without_ready() {
    // The daemon refuses a socket folder other users can open.
    let dir = folder(0o755);
    let (mut child, got) = start_with_pipe(daemon(dir.path()));
    let exited = child.wait().expect("wait for humatad2");
    assert!(got.is_empty(), "nothing is said: {got:?}");
    assert!(!exited.success(), "the daemon fails");
}
