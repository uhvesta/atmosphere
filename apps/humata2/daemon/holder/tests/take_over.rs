//! A holder moved onto a newer build in place (LIVEHOLDER, D801): same
//! pid, the program keeps running and never sees a hangup, the screen and
//! scrollback carry over, and typing and the exit code still work.
//!
//! "The newer build" is this test's own copy of `humata2_holder` in another
//! temp folder, so the process's executable tells the two images apart.
//! Every holder here is the test's own, in its own temp folder.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use humata2_holder::{
    Event, Events, HolderError, HolderSpec, KeyState, KeyedInput, attach, key_arrived, keyed_input,
    move_to, spawn,
};
use rustix::process::{Pid, Signal};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

const WAIT: Duration = Duration::from_secs(15);

fn holder_binary() -> PathBuf {
    let path = std::env::var_os("HUMATA2_HOLDER")
        .map(PathBuf::from)
        .expect("set HUMATA2_HOLDER to the humata2_holder binary");
    std::fs::canonicalize(path).unwrap()
}

/// Socket paths must be short, so not under the test's own temp.
fn short_dir() -> TempDir {
    tempfile::Builder::new()
        .prefix("h2m")
        .tempdir_in("/tmp")
        .unwrap()
}

/// A copy of the holder binary in its own folder: the build to move onto.
fn newer_build(dir: &Path) -> PathBuf {
    let next = dir.join("next");
    std::fs::create_dir_all(&next).unwrap();
    let path = next.join("humata2_holder");
    std::fs::copy(holder_binary(), &path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A stand-in build: a shell script with `body` as its text.
fn stub_build(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("stub_holder");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

struct Holder {
    socket: PathBuf,
    pid: u32,
    dir: TempDir,
}

impl Holder {
    /// A holder running `sh` that notes a hangup in `hup.log` and writes its
    /// own pid to `program.pid`.
    fn start() -> Self {
        let dir = short_dir();
        let script = format!(
            "trap 'echo hup >> {d}/hup.log' HUP; echo $$ > {d}/program.pid; exec 2>&1; \
             while IFS= read -r line; do eval \"$line\"; done",
            d = dir.path().display()
        );
        let spec = HolderSpec {
            holder: holder_binary(),
            socket: dir.path().join("t.sock"),
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), script.into()],
            cwd: dir.path().to_owned(),
            env: vec![("PS1".into(), "$ ".into()), ("ENV".into(), "/dev/null".into())],
            cols: 80,
            rows: 24,
            scrollback_bytes: 1 << 20,
            log: dir.path().join("holder.log"),
        };
        let spawned = spawn(&spec).unwrap();
        let holder = Self {
            socket: spec.socket,
            pid: spawned.pid,
            dir,
        };
        holder.program_pid();
        holder
    }

    /// The program's pid, once it has written it.
    fn program_pid(&self) -> u32 {
        let file = self.dir.path().join("program.pid");
        let deadline = std::time::Instant::now() + WAIT;
        loop {
            if let Some(pid) = std::fs::read_to_string(&file)
                .ok()
                .and_then(|text| text.trim().parse().ok())
            {
                return pid;
            }
            assert!(std::time::Instant::now() < deadline, "the program never started");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn hung_up(&self) -> bool {
        self.dir.path().join("hup.log").exists()
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        if let Some(pid) = i32::try_from(self.pid).ok().and_then(Pid::from_raw) {
            let _ = rustix::process::kill_process(pid, Signal::TERM);
        }
    }
}

/// `ps` field `field` of process `pid`.
fn ps(pid: u32, field: &str) -> String {
    let out = std::process::Command::new("ps")
        .args(["-o", &format!("{field}="), "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn alive(pid: u32) -> bool {
    !ps(pid, "pid").is_empty()
}

/// Reads output until `needle` shows or the wait runs out.
async fn wait_for(events: &mut Events, needle: &str) -> bool {
    let mut seen = Vec::new();
    tokio::time::timeout(WAIT, async {
        loop {
            match events.next().await {
                Ok(Some(Event::Output { data, .. })) => {
                    seen.extend(data);
                    if String::from_utf8_lossy(&seen).contains(needle) {
                        return true;
                    }
                }
                Ok(Some(_)) => {}
                _ => return false,
            }
        }
    })
    .await
    .unwrap_or(false)
}

/// Waits for the exit code on `events`.
async fn exit_code(events: &mut Events) -> Option<i32> {
    tokio::time::timeout(WAIT, async {
        loop {
            match events.next().await {
                Ok(Some(Event::Exited { code })) => return Some(code),
                Ok(Some(_)) => {}
                _ => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
}

/// Everything the holder serves from offset 0, read until it goes quiet.
async fn everything(socket: &Path) -> (humata2_holder::Welcome, Vec<u8>) {
    let attached = attach(socket, 0).await.unwrap();
    let mut events = attached.events;
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(Duration::from_millis(700), async {
        while let Ok(Some(event)) = events.next().await {
            if let Event::Output { data, .. } = event {
                seen.extend(data);
            }
        }
    })
    .await;
    (attached.welcome, seen)
}

/// Types `line` and waits for `needle` in the output.
async fn run(socket: &Path, line: &str, needle: &str) -> bool {
    let attached = attach(socket, u64::MAX).await.unwrap();
    let mut events = attached.events;
    let mut control = attached.control;
    control.input(1, line.as_bytes()).await.unwrap();
    wait_for(&mut events, needle).await
}

#[tokio::test]
async fn a_shell_moved_onto_a_newer_build_keeps_its_pid_and_never_sees_a_hangup() {
    let holder = Holder::start();
    let program = holder.program_pid();
    let next = newer_build(holder.dir.path());

    move_to(&holder.socket, &next).await.unwrap();

    assert!(alive(holder.pid), "the holder process is gone");
    assert_eq!(humata2_holder::holder_pid(&holder.socket), Some(holder.pid));
    assert!(alive(program), "the program was ended");
    assert_eq!(ps(program, "ppid"), holder.pid.to_string(), "the program lost its parent");
    let exe = ps(holder.pid, "comm");
    assert!(
        exe.contains("/next/") || exe.ends_with(&next.display().to_string()),
        "still the old build: {exe}"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!holder.hung_up(), "the program saw a hangup");
}

#[tokio::test]
async fn the_screen_and_scrollback_are_identical_after_a_move() {
    let holder = Holder::start();
    assert!(run(&holder.socket, "printf 'before-%s\\n' $((40+2))\n", "before-42").await);
    let next = newer_build(holder.dir.path());
    let (welcome_before, bytes_before) = everything(&holder.socket).await;

    move_to(&holder.socket, &next).await.unwrap();

    let (welcome_after, bytes_after) = everything(&holder.socket).await;
    assert_eq!(welcome_after, welcome_before);
    assert_eq!(bytes_after, bytes_before);
}

#[tokio::test]
async fn typing_after_a_move_reaches_the_program() {
    let holder = Holder::start();
    let next = newer_build(holder.dir.path());
    move_to(&holder.socket, &next).await.unwrap();
    assert!(run(&holder.socket, "echo moved-$((6*7))\n", "moved-42").await);
}

#[tokio::test]
async fn a_viewer_attached_through_a_move_can_reattach_and_carry_on() {
    let holder = Holder::start();
    let viewer = attach(&holder.socket, 0).await.unwrap();
    let mut events = viewer.events;
    let next = newer_build(holder.dir.path());
    move_to(&holder.socket, &next).await.unwrap();

    // The old viewer's connection ends without an exit code.
    let closed = tokio::time::timeout(WAIT, async {
        loop {
            match events.next().await {
                Ok(Some(Event::Exited { .. })) => return false,
                Ok(Some(_)) => {}
                _ => return true,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(closed, "the viewer saw an exit, or its connection never ended");
    assert!(run(&holder.socket, "echo back-$((1+1))\n", "back-2").await);
}

#[tokio::test]
async fn the_exit_code_is_still_reported_after_a_move() {
    let holder = Holder::start();
    let next = newer_build(holder.dir.path());
    move_to(&holder.socket, &next).await.unwrap();

    let attached = attach(&holder.socket, u64::MAX).await.unwrap();
    let mut events = attached.events;
    let mut control = attached.control;
    control.input(1, b"exit 7\n").await.unwrap();
    assert_eq!(exit_code(&mut events).await, Some(7));
    // A later viewer is told too.
    assert_eq!(attach(&holder.socket, 0).await.unwrap().welcome.exited, Some(7));
}

#[tokio::test]
async fn a_program_that_exited_before_the_move_keeps_its_exit_code() {
    let holder = Holder::start();
    let attached = attach(&holder.socket, u64::MAX).await.unwrap();
    let mut events = attached.events;
    let mut control = attached.control;
    control.input(1, b"exit 3\n").await.unwrap();
    assert_eq!(exit_code(&mut events).await, Some(3));

    let next = newer_build(holder.dir.path());
    move_to(&holder.socket, &next).await.unwrap();
    assert_eq!(attach(&holder.socket, 0).await.unwrap().welcome.exited, Some(3));
}

#[tokio::test]
async fn a_keyed_input_across_a_move_is_typed_once() {
    let holder = Holder::start();
    let count = holder.dir.path().join("runs");
    let line = format!(
        "echo a >> {f}; echo mark-$(wc -l < {f} | tr -d ' ')\n",
        f = count.display()
    );
    assert_eq!(
        keyed_input(&holder.socket, "s/d-1", line.as_bytes()).await.unwrap(),
        KeyedInput::Written
    );
    let before = key_arrived(&holder.socket, "s/d-1").await.unwrap();
    let next = newer_build(holder.dir.path());
    move_to(&holder.socket, &next).await.unwrap();

    // The daemon sends it again after the move: the holder still knows it.
    assert_eq!(
        keyed_input(&holder.socket, "s/d-1", line.as_bytes()).await.unwrap(),
        KeyedInput::Repeat
    );
    let after = key_arrived(&holder.socket, "s/d-1").await.unwrap();
    assert!(matches!(after, KeyState::Arrived { .. }), "{after:?}");
    assert_eq!(after, before, "the holder's instance changed with the move");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(std::fs::read_to_string(&count).unwrap().lines().count(), 1);
}

#[tokio::test]
async fn a_holder_moved_twice_still_keeps_its_program() {
    let holder = Holder::start();
    let program = holder.program_pid();
    let first = newer_build(holder.dir.path());
    move_to(&holder.socket, &first).await.unwrap();
    let again = holder.dir.path().join("again");
    std::fs::create_dir_all(&again).unwrap();
    let second = newer_build(&again);
    move_to(&holder.socket, &second).await.unwrap();
    assert!(alive(program));
    assert_eq!(humata2_holder::holder_pid(&holder.socket), Some(holder.pid));
    assert!(run(&holder.socket, "echo twice-$((2*2))\n", "twice-4").await);
}

/// Asserts the holder still runs its old build, the same program, and
/// takes input: a refused move changed nothing.
async fn unchanged(holder: &Holder, program: u32, exe_before: &str) {
    assert!(alive(holder.pid));
    assert_eq!(ps(holder.pid, "comm"), exe_before);
    assert!(alive(program));
    assert!(!holder.hung_up());
    assert!(run(&holder.socket, "echo still-$((5+5))\n", "still-10").await);
}

#[tokio::test]
async fn a_move_onto_a_build_that_reads_no_common_version_is_refused_and_changes_nothing() {
    let holder = Holder::start();
    let program = holder.program_pid();
    let exe = ps(holder.pid, "comm");
    let stub = stub_build(
        holder.dir.path(),
        "case \"$1\" in --handoff-versions) echo 99; exit 0;; esac; exit 1",
    );
    let moved = move_to(&holder.socket, &stub).await;
    assert!(matches!(moved, Err(HolderError::Refused { .. })), "{moved:?}");
    unchanged(&holder, program, &exe).await;
}

#[tokio::test]
async fn a_target_that_fails_its_check_is_refused() {
    let holder = Holder::start();
    let program = holder.program_pid();
    let exe = ps(holder.pid, "comm");
    let real = holder_binary();
    let stub = stub_build(
        holder.dir.path(),
        &format!(
            "case \"$1\" in --handoff-versions) exec '{}' --handoff-versions;; esac; exit 1",
            real.display()
        ),
    );
    let moved = move_to(&holder.socket, &stub).await;
    assert!(matches!(moved, Err(HolderError::Refused { .. })), "{moved:?}");
    unchanged(&holder, program, &exe).await;
    assert!(
        !PathBuf::from(format!("{}.handoff", holder.socket.display())).exists(),
        "a refused move left its handoff behind"
    );
}

#[tokio::test]
async fn a_failed_exec_returns_to_the_old_holder() {
    let holder = Holder::start();
    let program = holder.program_pid();
    let exe = ps(holder.pid, "comm");
    let real = holder_binary();
    // Passes both checks with the real build, then removes itself, so the
    // exec that follows finds nothing.
    let stub = stub_build(
        holder.dir.path(),
        &format!(
            "case \"$1\" in --check-take-over) '{r}' \"$@\"; s=$?; rm -f \"$0\"; exit $s;; \
             *) exec '{r}' \"$@\";; esac",
            r = real.display()
        ),
    );
    let moved = move_to(&holder.socket, &stub).await;
    assert!(matches!(moved, Err(HolderError::Refused { .. })), "{moved:?}");
    unchanged(&holder, program, &exe).await;
}

#[tokio::test]
async fn a_move_onto_a_path_that_is_not_a_program_is_refused() {
    let holder = Holder::start();
    let program = holder.program_pid();
    let exe = ps(holder.pid, "comm");
    let moved = move_to(&holder.socket, &holder.dir.path().join("missing")).await;
    assert!(matches!(moved, Err(HolderError::Refused { .. })), "{moved:?}");
    unchanged(&holder, program, &exe).await;
}

fn frame(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 1) as u32).to_be_bytes().to_vec();
    out.push(tag);
    out.extend_from_slice(body);
    out
}

#[tokio::test]
async fn a_holder_too_old_to_move_says_so_and_keeps_running() {
    // Speaks the old protocol: welcomes, then hangs up on any frame it
    // doesn't know.
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
            let mut len = [0u8; 4];
            let _ = stream.read_exact(&mut len).await;
        }
    });
    let moved = move_to(&socket, &holder_binary()).await;
    assert!(matches!(moved, Err(HolderError::TooOldToMove { .. })), "{moved:?}");
    assert!(attach(&socket, 0).await.is_ok(), "the old holder stopped answering");
    assert!(!old.is_finished());
    old.abort();
}

#[tokio::test]
async fn moving_a_holder_that_is_gone_is_an_error() {
    let dir = short_dir();
    let moved = move_to(&dir.path().join("none.sock"), &holder_binary()).await;
    assert!(moved.is_err());
    assert!(!matches!(moved, Err(HolderError::TooOldToMove { .. })), "{moved:?}");
}
