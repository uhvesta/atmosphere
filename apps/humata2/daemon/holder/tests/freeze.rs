//! A frozen holder (D801 J): the program is stopped (SIGSTOP) for a move to
//! another machine, keeps its pid, can be attached to while frozen, refuses
//! typing, and runs again once thawed. Every holder here is the test's own,
//! in its own temp folder.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use humata2_holder::{
    Event, Events, HolderError, HolderSpec, attach, freeze, is_frozen, spawn, thaw,
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
        .prefix("h2f")
        .tempdir_in("/tmp")
        .unwrap()
}

struct Holder {
    socket: PathBuf,
    pid: u32,
    dir: TempDir,
}

impl Holder {
    /// A holder running `sh` that writes its own pid to `program.pid`.
    fn start() -> Self {
        let dir = short_dir();
        let script = format!(
            "echo $$ > {d}/program.pid; exec 2>&1; \
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
            std::thread::sleep(Duration::from_millis(2));
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

/// `ps` field `field` of process `pid`.
fn ps(pid: u32, field: &str) -> String {
    let out = std::process::Command::new("ps")
        .args(["-o", &format!("{field}="), "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn stopped(pid: u32) -> bool {
    ps(pid, "stat").starts_with('T')
}

async fn wait_until(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + WAIT;
    while !check() {
        assert!(std::time::Instant::now() < deadline, "never happened: {what}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

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

#[tokio::test]
async fn freezing_stops_the_program_and_thawing_runs_it_again_with_the_same_pid() {
    let holder = Holder::start();
    let program = holder.program_pid();
    assert!(!is_frozen(&holder.socket).await.unwrap());

    freeze(&holder.socket).await.unwrap();

    assert!(is_frozen(&holder.socket).await.unwrap());
    wait_until("the program stops", || stopped(program)).await;
    assert_eq!(holder.program_pid(), program);

    thaw(&holder.socket).await.unwrap();

    assert!(!is_frozen(&holder.socket).await.unwrap());
    wait_until("the program runs", || {
        let state = ps(program, "stat");
        !state.is_empty() && !state.starts_with('T')
    })
    .await;
    assert_eq!(ps(program, "ppid"), holder.pid.to_string());
}

#[tokio::test]
async fn a_frozen_holder_can_be_attached_to_and_shows_its_screen() {
    let holder = Holder::start();
    let attached = attach(&holder.socket, u64::MAX).await.unwrap();
    let mut events = attached.events;
    let mut control = attached.control;
    control.input(1, b"echo before-$((6*7))\n").await.unwrap();
    assert!(wait_for(&mut events, "before-42").await);

    freeze(&holder.socket).await.unwrap();

    let again = attach(&holder.socket, 0).await.unwrap();
    let mut seen = Vec::new();
    let mut events = again.events;
    let _ = tokio::time::timeout(Duration::from_millis(700), async {
        while let Ok(Some(event)) = events.next().await {
            if let Event::Output { data, .. } = event {
                seen.extend(data);
                if String::from_utf8_lossy(&seen).contains("before-42") {
                    break;
                }
            }
        }
    })
    .await;
    assert!(String::from_utf8_lossy(&seen).contains("before-42"));
}

#[tokio::test]
async fn typing_into_a_frozen_program_is_refused_and_nothing_is_typed() {
    let holder = Holder::start();
    freeze(&holder.socket).await.unwrap();
    let attached = attach(&holder.socket, u64::MAX).await.unwrap();
    let mut events = attached.events;
    let mut control = attached.control;
    control.input(1, b"echo typed-while-frozen\n").await.unwrap();

    let refused = tokio::time::timeout(WAIT, async {
        loop {
            match events.next().await {
                Ok(Some(Event::Refused { .. })) => return true,
                Ok(Some(Event::InputWritten { .. })) => return false,
                Ok(Some(_)) => {}
                _ => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(refused, "typing into a frozen program was not refused");

    thaw(&holder.socket).await.unwrap();
    let attached = attach(&holder.socket, 0).await.unwrap();
    let mut events = attached.events;
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(Duration::from_millis(700), async {
        while let Ok(Some(event)) = events.next().await {
            if let Event::Output { data, .. } = event {
                seen.extend(data);
                if String::from_utf8_lossy(&seen).contains("typed-while-frozen") {
                    break;
                }
            }
        }
    })
    .await;
    assert!(!String::from_utf8_lossy(&seen).contains("typed-while-frozen"));
}

#[tokio::test]
async fn freezing_twice_and_thawing_twice_are_fine() {
    let holder = Holder::start();
    let program = holder.program_pid();
    freeze(&holder.socket).await.unwrap();
    freeze(&holder.socket).await.unwrap();
    wait_until("the program stops", || stopped(program)).await;
    thaw(&holder.socket).await.unwrap();
    thaw(&holder.socket).await.unwrap();
    wait_until("the program runs", || !stopped(program)).await;
}

#[tokio::test]
async fn shutting_down_a_frozen_holder_ends_its_program() {
    let holder = Holder::start();
    let program = holder.program_pid();
    freeze(&holder.socket).await.unwrap();
    humata2_holder::shutdown(&holder.socket).await.unwrap();
    wait_until("the program ends", || ps(program, "pid").is_empty()).await;
}

fn frame(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 1) as u32).to_be_bytes().to_vec();
    out.push(tag);
    out.extend_from_slice(body);
    out
}

#[tokio::test]
async fn a_holder_too_old_to_freeze_says_so() {
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
    let frozen = freeze(&socket).await;
    assert!(matches!(frozen, Err(HolderError::TooOldToFreeze { .. })), "{frozen:?}");
    old.abort();
}

#[tokio::test]
async fn thawing_during_a_freeze_leaves_the_program_running() {
    let dir = short_dir();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let real_ps = ["/bin/ps", "/usr/bin/ps"]
        .into_iter()
        .find(|path| std::path::Path::new(path).exists())
        .expect("ps binary on system");
    let entered = dir.path().join("ps.entered");
    let release = dir.path().join("ps.release");
    let fake_ps = bin.join("ps");
    std::fs::write(
        &fake_ps,
        format!(
            "#!/bin/sh\ntouch '{}'\nwhile [ ! -f '{}' ]; do sleep 0.01; done\nexec '{}' \"$@\"\n",
            entered.display(),
            release.display(),
            real_ps,
        ),
    )
    .unwrap();
    let mut perms = std::fs::metadata(&fake_ps).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_ps, perms).unwrap();

    let path_env = match std::env::var("PATH") {
        Ok(existing) => format!("{}:{existing}", bin.display()),
        Err(_) => bin.display().to_string(),
    };
    let script = format!(
        "echo $$ > {d}/program.pid; exec 2>&1; \
         while IFS= read -r line; do eval \"$line\"; done",
        d = dir.path().display()
    );
    let spec = HolderSpec {
        holder: holder_binary(),
        socket: dir.path().join("t.sock"),
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".into(), script.into()],
        cwd: dir.path().to_owned(),
        env: vec![
            ("PATH".into(), path_env.into()),
            ("PS1".into(), "$ ".into()),
            ("ENV".into(), "/dev/null".into()),
        ],
        cols: 80,
        rows: 24,
        scrollback_bytes: 1 << 20,
        log: dir.path().join("holder.log"),
    };
    let spawned = spawn(&spec).unwrap();
    let holder = Holder {
        socket: spec.socket,
        pid: spawned.pid,
        dir,
    };
    let program = holder.program_pid();

    let attached = attach(&holder.socket, u64::MAX).await.unwrap();
    let mut events = attached.events;
    let mut control = attached.control;

    let socket = holder.socket.clone();
    let freezing = tokio::spawn(async move { freeze(&socket).await });

    let deadline = std::time::Instant::now() + WAIT;
    while !entered.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "fake ps was never entered during freeze"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    thaw(&holder.socket).await.unwrap();
    std::fs::write(&release, b"go").unwrap();

    freezing.await.unwrap().unwrap();

    assert!(!is_frozen(&holder.socket).await.unwrap());
    assert!(!stopped(program));

    control.input(1, b"echo alive-after-thaw\n").await.unwrap();
    assert!(
        wait_for(&mut events, "alive-after-thaw").await,
        "program did not echo typed input after thaw raced freeze"
    );
}
