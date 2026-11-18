//! A holder build that crashes right after taking a holder over
//! (LIVEHOLDER, D801 B): the move is reported as lost, never silent, and
//! what the terminal printed before the move is kept on disk.
//!
//! The holder is built with faults: `HUMATA2_CRASH_AFTER_TAKE_OVER=1`
//! makes an image started with `--take-over` abort right after it reads
//! the handoff, before it serves.

use std::path::{Path, PathBuf};
use std::time::Duration;

use humata2_holder::{Event, HolderError, HolderSpec, attach, move_to, read_saved, saved_scrollback_path, spawn};
use rustix::process::{Pid, Signal};

const WAIT: Duration = Duration::from_secs(15);

fn holder_binary() -> PathBuf {
    let path = std::env::var_os("HUMATA2_HOLDER")
        .map(PathBuf::from)
        .expect("set HUMATA2_HOLDER to the humata2_holder_faults binary");
    std::fs::canonicalize(path).unwrap()
}

fn short_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("h2mf")
        .tempdir_in("/tmp")
        .unwrap()
}

struct Ended(u32);

impl Drop for Ended {
    fn drop(&mut self) {
        if let Some(pid) = i32::try_from(self.0).ok().and_then(Pid::from_raw) {
            let _ = rustix::process::kill_process(pid, Signal::KILL);
        }
    }
}

fn handoff_path(socket: &Path) -> PathBuf {
    PathBuf::from(format!("{}.handoff", socket.display()))
}

#[tokio::test]
async fn a_crash_right_after_the_take_over_is_reported_and_the_output_is_kept() {
    let dir = short_dir();
    let spec = HolderSpec {
        holder: holder_binary(),
        socket: dir.path().join("t.sock"),
        program: PathBuf::from("/bin/sh"),
        args: vec![
            "-c".into(),
            "echo printed-before-$((3*3)); while :; do sleep 1; done".into(),
        ],
        cwd: dir.path().to_owned(),
        env: vec![("HUMATA2_CRASH_AFTER_TAKE_OVER".into(), "1".into())],
        cols: 80,
        rows: 24,
        scrollback_bytes: 1 << 20,
        log: dir.path().join("holder.log"),
    };
    let spawned = spawn(&spec).unwrap();
    let _ended = Ended(spawned.pid);

    // Wait until the output is there.
    let attached = attach(&spec.socket, 0).await.unwrap();
    let mut events = attached.events;
    let printed = tokio::time::timeout(WAIT, async {
        let mut seen = Vec::new();
        while let Ok(Some(event)) = events.next().await {
            if let Event::Output { offset, data } = event {
                seen.extend_from_slice(&data);
                if String::from_utf8_lossy(&seen).contains("printed-before-9") {
                    return Some(offset + data.len() as u64);
                }
            }
        }
        None
    })
    .await
    .ok()
    .flatten();
    let printed_through = printed.expect("the program's output never arrived");
    drop(events);

    let moved = move_to(&spec.socket, &holder_binary()).await;
    assert!(matches!(moved, Err(HolderError::LostInMove { .. })), "{moved:?}");

    // The marker revive reads stays beside the dead holder, and the
    // output printed before the move is on disk.
    assert!(handoff_path(&spec.socket).exists(), "no marker left for revive");
    let saved = read_saved(&saved_scrollback_path(&spec.socket))
        .unwrap()
        .expect("nothing saved");
    assert!(
        saved.saved_through >= printed_through,
        "saved through {} but {printed_through} was printed before the move",
        saved.saved_through
    );
}
