//! Drives a real `humata2_holder` running `sh` over its socket.

use std::path::{Path, PathBuf};
use std::time::Duration;

use humata2_holder::{Event, Events, HolderError, HolderSpec, attach, shutdown, spawn};
use rustix::process::{Pid, Signal};
use tempfile::TempDir;

const WAIT: Duration = Duration::from_secs(15);

fn holder_binary() -> PathBuf {
    let path = std::env::var_os("HUMATA2_HOLDER")
        .map(PathBuf::from)
        .or_else(|| option_env!("CARGO_BIN_EXE_humata2_holder").map(PathBuf::from))
        .expect("set HUMATA2_HOLDER to the humata2_holder binary");
    std::fs::canonicalize(path).unwrap()
}

fn spec(dir: &Path, scrollback_bytes: usize) -> HolderSpec {
    HolderSpec {
        holder: holder_binary(),
        socket: dir.join("t.sock"),
        program: PathBuf::from("/bin/sh"),
        args: Vec::new(),
        cwd: dir.to_owned(),
        env: vec![("PS1".into(), "$ ".into()), ("ENV".into(), "/dev/null".into())],
        cols: 80,
        rows: 24,
        scrollback_bytes,
        log: dir.join("holder.log"),
    }
}

/// A holder that's stopped when the test ends, however it ends.
struct Holder {
    socket: PathBuf,
    pid: u32,
    _dir: TempDir,
}

impl Holder {
    fn start(scrollback_bytes: usize) -> Self {
        // Socket paths must be short, so not under the test's own temp.
        let dir = tempfile::Builder::new()
            .prefix("h2h")
            .tempdir_in("/tmp")
            .unwrap();
        let spec = spec(dir.path(), scrollback_bytes);
        let spawned = spawn(&spec).unwrap();
        Self {
            socket: spec.socket,
            pid: spawned.pid,
            _dir: dir,
        }
    }

    fn pid(&self) -> Pid {
        Pid::from_raw(i32::try_from(self.pid).unwrap()).unwrap()
    }

    fn is_running(&self) -> bool {
        rustix::process::test_kill_process(self.pid()).is_ok()
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        let _ = rustix::process::kill_process(self.pid(), Signal::TERM);
    }
}

/// What a test has read from one connection.
struct Transcript {
    events: Events,
    start: Option<u64>,
    cursor: u64,
    bytes: Vec<u8>,
    trimmed: Vec<u64>,
    written: Vec<u64>,
    exited: Option<i32>,
}

impl Transcript {
    fn new(events: Events) -> Self {
        Self {
            events,
            start: None,
            cursor: 0,
            bytes: Vec::new(),
            trimmed: Vec::new(),
            written: Vec::new(),
            exited: None,
        }
    }

    async fn next(&mut self) {
        match self.events.next().await.unwrap() {
            Some(Event::Output { offset, data }) => {
                match self.start {
                    None => self.start = Some(offset),
                    Some(_) => assert_eq!(offset, self.cursor, "output skipped or repeated bytes"),
                }
                self.cursor = offset + data.len() as u64;
                self.bytes.extend(data);
            }
            Some(Event::Trimmed { resume_at }) => self.trimmed.push(resume_at),
            Some(Event::InputWritten { seq }) => self.written.push(seq),
            Some(Event::Exited { code }) => self.exited = Some(code),
            Some(Event::Refused { message }) => panic!("refused: {message}"),
            None => panic!("the holder closed the connection"),
        }
    }

    async fn until(&mut self, done: impl Fn(&Self) -> bool) {
        tokio::time::timeout(WAIT, async {
            while !done(self) {
                self.next().await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "timed out; read so far: {:?}",
                String::from_utf8_lossy(&self.bytes)
            )
        });
    }

    async fn until_text(&mut self, needle: &str) {
        self.until(|seen| contains(&seen.bytes, needle)).await;
    }
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[tokio::test]
async fn output_carries_contiguous_offsets_and_a_reattach_resumes_from_a_cursor() {
    let holder = Holder::start(1 << 20);
    let first = attach(&holder.socket, 0).await.unwrap();
    let mut control = first.control;
    let mut seen = Transcript::new(first.events);
    control.input(7, b"echo hello-$((1+1))\n").await.unwrap();
    seen.until_text("hello-2").await;
    seen.until(|seen| seen.written.contains(&7)).await;
    assert_eq!(seen.start, Some(0));
    let full = seen.bytes.clone();
    drop(seen);
    drop(control);

    let cursor = full.len() as u64 / 2;
    let again = attach(&holder.socket, cursor).await.unwrap();
    assert!(again.welcome.end >= full.len() as u64);
    let mut control = again.control;
    let mut resumed = Transcript::new(again.events);
    let wanted = full.len() - cursor as usize;
    resumed.until(|seen| seen.bytes.len() >= wanted).await;
    assert_eq!(resumed.start, Some(cursor));
    assert_eq!(&resumed.bytes[..wanted], &full[cursor as usize..]);
    assert!(resumed.trimmed.is_empty());

    control.input(8, b"echo again-$((2+2))\n").await.unwrap();
    resumed.until_text("again-4").await;
}

#[tokio::test]
async fn attaching_from_before_what_is_kept_says_it_was_trimmed() {
    let holder = Holder::start(256);
    let first = attach(&holder.socket, 0).await.unwrap();
    let mut control = first.control;
    let mut seen = Transcript::new(first.events);
    control.input(1, b"printf '%0600d\\n' 0\n").await.unwrap();
    seen.until_text(&"0".repeat(100)).await;
    seen.until(|seen| seen.cursor > 600).await;

    let late = attach(&holder.socket, 0).await.unwrap();
    assert!(late.welcome.earliest > 0);
    assert!(late.welcome.end - late.welcome.earliest <= 256);
    let mut replay = Transcript::new(late.events);
    replay.until(|seen| seen.start.is_some()).await;
    assert_eq!(replay.trimmed, vec![late.welcome.earliest]);
    assert_eq!(replay.start, Some(late.welcome.earliest));
}

#[tokio::test]
async fn a_resize_reaches_the_program() {
    let holder = Holder::start(1 << 20);
    let attached = attach(&holder.socket, 0).await.unwrap();
    assert_eq!((attached.welcome.cols, attached.welcome.rows), (80, 24));
    let mut control = attached.control;
    let mut seen = Transcript::new(attached.events);

    control.resize(101, 37).await.unwrap();
    control.input(1, b"stty size\n").await.unwrap();
    seen.until_text("37 101").await;

    let later = attach(&holder.socket, 0).await.unwrap();
    assert_eq!((later.welcome.cols, later.welcome.rows), (101, 37));
}

#[tokio::test]
async fn the_exit_code_is_sent_and_kept_for_later_clients() {
    let holder = Holder::start(1 << 20);
    let attached = attach(&holder.socket, 0).await.unwrap();
    assert_eq!(attached.welcome.exited, None);
    let mut control = attached.control;
    let mut seen = Transcript::new(attached.events);

    control.input(1, b"exit 3\n").await.unwrap();
    seen.until(|seen| seen.exited.is_some()).await;
    assert_eq!(seen.exited, Some(3));

    let later = attach(&holder.socket, 0).await.unwrap();
    assert_eq!(later.welcome.exited, Some(3));
    assert!(holder.is_running());
}

#[tokio::test]
async fn shutting_down_stops_the_holder() {
    let holder = Holder::start(1 << 20);
    shutdown(&holder.socket).await.unwrap();

    let deadline = tokio::time::Instant::now() + WAIT;
    while holder.is_running() {
        assert!(tokio::time::Instant::now() < deadline, "the holder is still running");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(matches!(
        attach(&holder.socket, 0).await,
        Err(HolderError::Connect { .. })
    ));
}

#[tokio::test]
async fn a_second_holder_on_a_live_socket_does_not_start() {
    let holder = Holder::start(1 << 20);
    let dir = holder.socket.parent().unwrap();

    let second = spawn(&spec(dir, 1 << 20));

    assert!(matches!(second, Err(HolderError::StartFailed { .. })));
    let still = attach(&holder.socket, 0).await.unwrap();
    assert_eq!(still.welcome.exited, None);
}

#[tokio::test]
async fn a_killed_holder_leaves_a_socket_the_next_one_takes_over() {
    let killed = Holder::start(1 << 20);
    rustix::process::kill_process(killed.pid(), Signal::KILL).unwrap();
    let deadline = tokio::time::Instant::now() + WAIT;
    while killed.is_running() {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let dir = killed.socket.parent().unwrap();
    assert!(killed.socket.exists());

    let spawned = spawn(&spec(dir, 1 << 20)).unwrap();
    let next = Holder {
        socket: killed.socket.clone(),
        pid: spawned.pid,
        _dir: tempfile::tempdir().unwrap(),
    };
    let attached = attach(&next.socket, 0).await.unwrap();
    assert_eq!(attached.welcome.exited, None);
}

#[test]
fn a_socket_path_too_long_for_unix_sockets_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut spec = spec(dir.path(), 1 << 20);
    spec.socket = PathBuf::from(format!("/tmp/{}.sock", "x".repeat(200)));
    assert!(matches!(
        spawn(&spec),
        Err(HolderError::SocketPathTooLong { .. })
    ));
}
