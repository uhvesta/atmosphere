//! Cuts a holder's power while its program prints (PER-25): a holder
//! started after the cut must resume at or past every offset a client was
//! sent, and what it shows again must be what that client saw, as far as
//! that client had read: output is saved before it is sent, so the saved
//! output can run past a client that was behind.
//!
//! The holder is built with power cuts playable: under
//! `HUMATA2_POWER_CUT_ROOTS`, SIGUSR2 drops whatever it wrote and didn't
//! sync, then kills it, as pulling the plug would.

use std::path::{Path, PathBuf};
use std::time::Duration;

use humata2_holder::{Event, HolderSpec, attach, read_saved, saved_scrollback_path, spawn};
use rustix::process::{Pid, Signal};

const WAIT: Duration = Duration::from_secs(15);

fn holder_binary() -> PathBuf {
    let path = std::env::var_os("HUMATA2_HOLDER")
        .map(PathBuf::from)
        .expect("set HUMATA2_HOLDER to the humata2_holder_faults binary");
    std::fs::canonicalize(path).unwrap()
}

fn spec(dir: &Path, script: &str, env: Vec<(std::ffi::OsString, std::ffi::OsString)>) -> HolderSpec {
    HolderSpec {
        holder: holder_binary(),
        socket: dir.join("root/t.sock"),
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".into(), script.into()],
        cwd: dir.to_owned(),
        env,
        cols: 80,
        rows: 24,
        scrollback_bytes: 8 << 20,
        log: dir.join("holder.log"),
    }
}

fn pid(raw: u32) -> Pid {
    Pid::from_raw(i32::try_from(raw).unwrap()).unwrap()
}

/// When to cut, once the client has been sent some output.
#[derive(Clone, Copy, Debug)]
enum CutAt {
    /// As soon as the first output arrives: likely nothing synced past the
    /// first reservation.
    FirstOutput,
    /// Once this many bytes arrived, mid-burst.
    Bytes(usize),
    /// Once output has kept arriving for long enough that a sync has run.
    AfterASync,
}

/// Starts a printing holder under a simulated power cut, cuts it at `at`,
/// and returns what the client was sent, from offset 0.
async fn print_then_cut(dir: &Path, at: CutAt) -> Vec<u8> {
    std::fs::create_dir(dir.join("root")).unwrap();
    let env = vec![
        ("HUMATA2_POWER_CUT_ROOTS".into(), dir.join("root").into_os_string()),
        ("HUMATA2_POWER_CUT_SPARE".into(), dir.join("spare").into_os_string()),
    ];
    let script = "i=0; while :; do echo line$i; i=$((i+1)); done";
    let spawned = spawn(&spec(dir, script, env)).unwrap();
    let socket = dir.join("root/t.sock");
    let attached = attach(&socket, 0).await.unwrap();
    let _control = attached.control;
    let mut events = attached.events;
    let mut seen = Vec::new();
    let mut cut = false;
    tokio::time::timeout(WAIT, async {
        // Reads until the holder is gone, so every byte it sent is counted.
        loop {
            let event = match events.next().await {
                Ok(Some(event)) => event,
                Ok(None) | Err(_) => break,
            };
            if let Event::Output { offset, data } = event {
                assert_eq!(offset, seen.len() as u64, "output skipped or repeated bytes");
                seen.extend(data);
            }
            if !cut {
                let due = match at {
                    CutAt::FirstOutput => !seen.is_empty(),
                    CutAt::Bytes(bytes) => seen.len() >= bytes,
                    // Goes on reading while it waits, so what it was sent keeps
                    // up with what the holder saves and syncs.
                    CutAt::AfterASync => synced_bytes(&socket).is_some_and(|synced| synced > 0),
                };
                if due {
                    cut = true;
                    rustix::process::kill_process(pid(spawned.pid), Signal::USR2).unwrap();
                }
            }
        }
    })
    .await
    .expect("the holder didn't die at the cut");
    assert!(cut, "the holder ended before it was cut");
    // The client's connection can close before the cut holder is gone: it
    // still listens while it undoes what it didn't sync and its SIGKILL
    // lands. Power comes back only once it is off, so wait for it to exit
    // (it is this process's child) before a new holder takes the socket.
    let cut_pid = pid(spawned.pid);
    tokio::time::timeout(
        WAIT,
        tokio::task::spawn_blocking(move || {
            let _ = rustix::process::waitpid(Some(cut_pid), rustix::process::WaitOptions::empty());
        }),
    )
    .await
    .expect("the cut holder never exited")
    .unwrap();
    seen
}

/// Starts a holder on the cut one's socket, not under a cut, and returns
/// where it resumes and the saved output it shows again, from offset 0.
async fn restart(dir: &Path) -> (u64, Vec<u8>) {
    let socket = dir.join("root/t.sock");
    let saved = read_saved(&saved_scrollback_path(&socket))
        .unwrap()
        .expect("the cut holder left nothing saved");
    let spawned = spawn(&spec(dir, "exec sleep 60", Vec::new())).unwrap_or_else(|error| {
        let log = std::fs::read_to_string(dir.join("holder.log")).unwrap_or_default();
        panic!("the holder after the cut didn't start: {error}; its log:\n{log}")
    });
    let attached = attach(&socket, 0).await.unwrap();
    let mut events = attached.events;
    let mut shown = Vec::new();
    tokio::time::timeout(WAIT, async {
        while (shown.len() as u64) < saved.saved_through {
            match events.next().await.unwrap() {
                Some(Event::Output { offset, data }) => {
                    assert_eq!(offset, shown.len() as u64, "the saved output has a gap");
                    shown.extend(data);
                }
                Some(Event::Trimmed { resume_at }) => {
                    panic!("the saved output was trimmed to {resume_at}, though it all fits")
                }
                Some(_) => {}
                None => panic!("the restarted holder closed the connection"),
            }
        }
    })
    .await
    .expect("the restarted holder didn't show its saved output");
    shown.truncate(usize::try_from(saved.saved_through).unwrap());
    let _ = rustix::process::kill_process(pid(spawned.pid), Signal::TERM);
    (saved.resumes_at, shown)
}

async fn check(at: CutAt) {
    // Socket paths must be short, so not under the test's own temp.
    let dir = tempfile::Builder::new().prefix("h2c").tempdir_in("/tmp").unwrap();
    let seen = print_then_cut(dir.path(), at).await;
    let (resumes_at, shown) = restart(dir.path()).await;
    assert!(
        resumes_at >= seen.len() as u64,
        "{at:?}: a client was sent up to {} but a new holder resumes at {resumes_at}",
        seen.len()
    );
    // The saved output may run past what the client had read by the cut.
    // Where both have bytes, they are the same bytes.
    let both = seen.len().min(shown.len());
    let differs_at = seen[..both]
        .iter()
        .zip(&shown[..both])
        .position(|(sent, saved)| sent != saved);
    assert!(
        differs_at.is_none(),
        "{at:?}: the saved output differs from what the client was sent, first at offset {differs_at:?} (sent {} bytes, saved {})",
        seen.len(),
        shown.len()
    );
    if matches!(at, CutAt::AfterASync) {
        assert!(!shown.is_empty(), "output synced before the cut was lost");
    }
}

#[tokio::test]
async fn a_cut_at_the_first_output_resumes_past_it() {
    check(CutAt::FirstOutput).await;
}

#[tokio::test]
async fn a_cut_mid_burst_resumes_past_what_was_sent() {
    check(CutAt::Bytes(64 << 10)).await;
}

#[tokio::test]
async fn a_cut_after_a_sync_keeps_what_was_synced() {
    check(CutAt::AfterASync).await;
}

/// How many bytes the holder on `socket` reports as synced, if it has synced yet.
fn synced_bytes(socket: &Path) -> Option<u64> {
    std::fs::read_to_string(socket.with_extension("synced"))
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
}

/// Waits until the holder on `socket` says its saved output is synced
/// through at least `end`.
async fn synced_to(socket: &Path, end: u64) {
    tokio::time::timeout(WAIT, async {
        loop {
            if synced_bytes(socket).is_some_and(|synced| synced >= end) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the holder never said its output was synced that far");
}

#[tokio::test]
async fn output_the_holder_says_is_synced_survives_a_cut() {
    let dir = tempfile::Builder::new().prefix("h2c").tempdir_in("/tmp").unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let env = vec![
        ("HUMATA2_POWER_CUT_ROOTS".into(), root.clone().into_os_string()),
        ("HUMATA2_POWER_CUT_SPARE".into(), dir.path().join("spare").into_os_string()),
    ];
    let script = "i=0; while [ $i -lt 200 ]; do echo line$i; i=$((i+1)); done; exec sleep 60";
    let spawned = spawn(&spec(dir.path(), script, env)).unwrap();
    let socket = root.join("t.sock");
    let attached = attach(&socket, 0).await.unwrap();
    let _control = attached.control;
    let mut events = attached.events;
    let mut seen = Vec::new();
    tokio::time::timeout(WAIT, async {
        while !String::from_utf8_lossy(&seen).contains("line199") {
            match events.next().await.unwrap() {
                Some(Event::Output { data, .. }) => seen.extend(data),
                Some(_) => {}
                None => panic!("the holder closed the connection before the program printed everything"),
            }
        }
    })
    .await
    .expect("the program's output never all arrived");

    synced_to(&socket, seen.len() as u64).await;
    rustix::process::kill_process(pid(spawned.pid), Signal::USR2).unwrap();
    tokio::time::timeout(WAIT, async { while let Ok(Some(_)) = events.next().await {} })
        .await
        .expect("the holder didn't die at the cut");
    let (_, shown) = restart(dir.path()).await;

    assert!(
        shown.len() >= seen.len(),
        "the holder said {} bytes were synced, but only {} came back after the cut",
        seen.len(),
        shown.len()
    );
    assert_eq!(&shown[..seen.len()], &seen[..], "what came back after the cut isn't what was printed");
}
