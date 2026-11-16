//! Keyed input against a real `humata2_holder` running `sh`: typed once,
//! whole, and the holder can say whether a key arrived.

use std::path::{Path, PathBuf};
use std::time::Duration;

use humata2_holder::{
    Event, Events, HolderSpec, KeyState, KeyedInput, attach, key_arrived, keyed_input, spawn,
};
use rustix::process::{Pid, Signal};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

const WAIT: Duration = Duration::from_secs(15);

fn holder_binary() -> PathBuf {
    let path = std::env::var_os("HUMATA2_HOLDER")
        .map(PathBuf::from)
        .or_else(|| option_env!("CARGO_BIN_EXE_humata2_holder").map(PathBuf::from))
        .expect("set HUMATA2_HOLDER to the humata2_holder binary");
    std::fs::canonicalize(path).unwrap()
}

/// Socket paths must be short, so not under the test's own temp.
fn short_dir() -> TempDir {
    tempfile::Builder::new()
        .prefix("h2k")
        .tempdir_in("/tmp")
        .unwrap()
}

struct Holder {
    socket: PathBuf,
    pid: u32,
    _dir: TempDir,
}

impl Holder {
    fn start() -> Self {
        let dir = short_dir();
        let spec = HolderSpec {
            holder: holder_binary(),
            socket: dir.path().join("t.sock"),
            program: PathBuf::from("/bin/sh"),
            args: Vec::new(),
            cwd: dir.path().to_owned(),
            env: vec![("PS1".into(), "$ ".into()), ("ENV".into(), "/dev/null".into())],
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
        if let Some(pid) = i32::try_from(self.pid).ok().and_then(Pid::from_raw) {
            let _ = rustix::process::kill_process(pid, Signal::TERM);
        }
    }
}

/// Counts how often `needle` appears in what the terminal printed, until
/// it appears `times` times or the wait runs out.
async fn wait_for_count(events: &mut Events, needle: &str, times: usize) -> usize {
    let mut seen = Vec::new();
    let count = |seen: &[u8]| String::from_utf8_lossy(seen).matches(needle).count();
    let _ = tokio::time::timeout(WAIT, async {
        while count(&seen) < times {
            match events.next().await.unwrap() {
                Some(Event::Output { data, .. }) => seen.extend(data),
                Some(_) => {}
                None => break,
            }
        }
    })
    .await;
    count(&seen)
}

/// Reads output for a moment and returns what the terminal printed.
async fn output_for(events: &mut Events, wait: Duration) -> String {
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(wait, async {
        while let Ok(Some(event)) = events.next().await {
            if let Event::Output { data, .. } = event {
                seen.extend(data);
            }
        }
    })
    .await;
    String::from_utf8_lossy(&seen).into_owned()
}

/// A shell command that appends one line to `file` each time it runs, and
/// prints `mark-<n>` where `n` is the line count: output from the echo of
/// the typed command itself reads `mark-$`, so only runs print `mark-1`.
fn bump(file: &Path, name: &str) -> String {
    format!(
        "echo {name} >> {file}; echo mark-$(wc -l < {file} | tr -d ' ')-{name}\n",
        file = file.display()
    )
}

fn frame(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 1) as u32).to_be_bytes().to_vec();
    out.push(tag);
    out.extend_from_slice(body);
    out
}

fn hello(from: u64) -> Vec<u8> {
    let mut body = 1u32.to_be_bytes().to_vec();
    body.extend_from_slice(&from.to_be_bytes());
    frame(1, &body)
}

/// A keyed input frame as the wire has it (tag 5).
fn keyed_frame(key: &str, last: bool, data: &[u8]) -> Vec<u8> {
    let mut body = (key.len() as u16).to_be_bytes().to_vec();
    body.extend_from_slice(key.as_bytes());
    body.push(u8::from(last));
    body.extend_from_slice(data);
    frame(5, &body)
}

#[tokio::test]
async fn a_keyed_input_types_once_and_a_repeat_types_nothing() {
    let holder = Holder::start();
    let attached = attach(&holder.socket, 0).await.unwrap();
    let mut events = attached.events;
    let dir = short_dir();
    let file = dir.path().join("runs");

    let line = bump(&file, "a");
    assert!(matches!(
        key_arrived(&holder.socket, "s/d-1").await.unwrap(),
        KeyState::NotArrived { .. }
    ));
    assert_eq!(
        keyed_input(&holder.socket, "s/d-1", line.as_bytes()).await.unwrap(),
        KeyedInput::Written
    );
    assert_eq!(wait_for_count(&mut events, "mark-1-a", 1).await, 1);
    assert!(matches!(
        key_arrived(&holder.socket, "s/d-1").await.unwrap(),
        KeyState::Arrived { .. }
    ));

    // The daemon restarted and sends the same key again on a new connection.
    assert_eq!(
        keyed_input(&holder.socket, "s/d-1", line.as_bytes()).await.unwrap(),
        KeyedInput::Repeat
    );
    let later = output_for(&mut events, Duration::from_millis(700)).await;
    assert!(!later.contains("mark-2-a"), "typed again: {later:?}");
    assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 1);
}

#[tokio::test]
async fn unkeyed_input_still_works_beside_keyed_input() {
    let holder = Holder::start();
    let attached = attach(&holder.socket, 0).await.unwrap();
    let mut events = attached.events;
    let mut control = attached.control;
    control.input(1, b"echo plain-$((20+1))\n").await.unwrap();
    assert_eq!(wait_for_count(&mut events, "plain-21", 1).await, 1);
    keyed_input(&holder.socket, "k", b"echo keyed-$((20+2))\n")
        .await
        .unwrap();
    assert_eq!(wait_for_count(&mut events, "keyed-22", 1).await, 1);
}

#[tokio::test]
async fn a_keyed_input_cut_off_before_its_last_chunk_types_nothing() {
    let holder = Holder::start();
    let attached = attach(&holder.socket, 0).await.unwrap();
    let mut events = attached.events;

    // Half of it goes out, then the connection drops.
    let mut raw = UnixStream::connect(&holder.socket).await.unwrap();
    raw.write_all(&hello(u64::MAX)).await.unwrap();
    raw.write_all(&keyed_frame("cut", false, b"echo half-$((1+1))\n"))
        .await
        .unwrap();
    drop(raw);
    let seen = output_for(&mut events, Duration::from_millis(700)).await;
    assert!(!seen.contains("half-2"), "typed half: {seen:?}");
    assert!(matches!(
        key_arrived(&holder.socket, "cut").await.unwrap(),
        KeyState::NotArrived { .. }
    ));

    // The same key sent whole afterwards is typed once.
    assert_eq!(
        keyed_input(&holder.socket, "cut", b"echo whole-$((1+1))\n")
            .await
            .unwrap(),
        KeyedInput::Written
    );
    assert_eq!(wait_for_count(&mut events, "whole-2", 1).await, 1);
}

#[tokio::test]
async fn a_large_keyed_input_in_many_chunks_is_typed_whole() {
    let holder = Holder::start();
    let attached = attach(&holder.socket, 0).await.unwrap();
    let mut events = attached.events;
    // Over two chunks (64 KiB each), in lines short enough for the
    // terminal's line buffer: comment lines, then a command.
    let mut data = Vec::new();
    for _ in 0..3000 {
        data.extend_from_slice(b": ");
        data.extend(std::iter::repeat_n(b'x', 46));
        data.push(b'\n');
    }
    data.extend_from_slice(b"echo big-$((3+4))\n");
    assert_eq!(
        keyed_input(&holder.socket, "big", &data).await.unwrap(),
        KeyedInput::Written
    );
    assert_eq!(wait_for_count(&mut events, "big-7", 1).await, 1);
}

#[tokio::test]
async fn a_holder_that_closes_on_new_frames_reads_as_unsupported() {
    // Speaks the old protocol: welcomes, then hangs up on the next frame,
    // which it doesn't know.
    let dir = short_dir();
    let socket = dir.path().join("old.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let old = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut hello = [0u8; 4 + 1 + 4 + 8];
            if stream.read_exact(&mut hello).await.is_err() {
                continue;
            }
            let mut body = Vec::new();
            body.extend_from_slice(&0u64.to_be_bytes());
            body.extend_from_slice(&0u64.to_be_bytes());
            body.extend_from_slice(&80u16.to_be_bytes());
            body.extend_from_slice(&24u16.to_be_bytes());
            body.push(0);
            body.extend_from_slice(&0i32.to_be_bytes());
            let _ = stream.write_all(&frame(101, &body)).await;
            // Reads the next frame, doesn't know it, and hangs up.
            let mut len = [0u8; 4];
            let _ = stream.read_exact(&mut len).await;
        }
    });
    assert_eq!(
        keyed_input(&socket, "k", b"echo hi\n").await.unwrap(),
        KeyedInput::Unsupported
    );
    assert_eq!(
        key_arrived(&socket, "k").await.unwrap(),
        KeyState::Unsupported
    );
    old.abort();
}

#[tokio::test]
async fn a_holder_that_is_gone_is_an_error_not_unsupported() {
    let dir = short_dir();
    assert!(keyed_input(&dir.path().join("none.sock"), "k", b"x").await.is_err());
    assert!(key_arrived(&dir.path().join("none.sock"), "k").await.is_err());
}

fn instance_of(state: KeyState) -> u64 {
    match state {
        KeyState::Arrived { holder } | KeyState::NotArrived { holder } => holder,
        KeyState::Unsupported => panic!("the holder could not answer"),
    }
}

#[tokio::test]
async fn a_holder_gives_the_same_instance_every_time_it_is_asked() {
    let holder = Holder::start();
    let first = instance_of(key_arrived(&holder.socket, "a").await.unwrap());
    keyed_input(&holder.socket, "a", b"true\n").await.unwrap();
    let second = instance_of(key_arrived(&holder.socket, "a").await.unwrap());
    assert_ne!(first, 0);
    assert_eq!(first, second);
}

#[tokio::test]
async fn a_holder_started_again_on_the_same_socket_has_another_instance() {
    let first = Holder::start();
    let before = instance_of(key_arrived(&first.socket, "a").await.unwrap());
    attach(&first.socket, u64::MAX)
        .await
        .unwrap()
        .control
        .shutdown()
        .await
        .unwrap();

    // Starting fails until the first holder has stopped and let go of the
    // socket.
    let spec = HolderSpec {
        holder: holder_binary(),
        socket: first.socket.clone(),
        program: PathBuf::from("/bin/sh"),
        args: Vec::new(),
        cwd: first._dir.path().to_owned(),
        env: vec![("PS1".into(), "$ ".into()), ("ENV".into(), "/dev/null".into())],
        cols: 80,
        rows: 24,
        scrollback_bytes: 1 << 20,
        log: first._dir.path().join("again.log"),
    };
    let deadline = std::time::Instant::now() + WAIT;
    let again = loop {
        match spawn(&spec) {
            Ok(spawned) => break spawned,
            Err(error) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the first holder never stopped: {error}"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    };
    let after = instance_of(key_arrived(&spec.socket, "a").await.unwrap());
    if let Some(pid) = i32::try_from(again.pid).ok().and_then(Pid::from_raw) {
        let _ = rustix::process::kill_process(pid, Signal::TERM);
    }
    assert_ne!(after, 0);
    assert_ne!(before, after);
}
