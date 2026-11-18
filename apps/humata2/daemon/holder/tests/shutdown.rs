//! A shutdown of a holder that doesn't answer gives up within its bound
//! rather than waiting on it. The holder is stopped here, as a wedged one
//! would be, and the shutdown is raced against a guard.

use std::path::PathBuf;
use std::time::Duration;

use humata2_holder::{HolderError, HolderSpec, shutdown, spawn};
use rustix::process::{Pid, Signal, kill_process};
use tempfile::TempDir;

/// Three times the shutdown's own bound of three seconds: a shutdown that
/// waited on the stopped holder would still be waiting when this runs out.
const GUARD: Duration = Duration::from_secs(9);

fn holder_binary() -> PathBuf {
    let path = std::env::var_os("HUMATA2_HOLDER")
        .map(PathBuf::from)
        .or_else(|| option_env!("CARGO_BIN_EXE_humata2_holder").map(PathBuf::from))
        .expect("set HUMATA2_HOLDER to the humata2_holder binary");
    std::fs::canonicalize(path).unwrap()
}

/// A holder running `sh`, ended when the test ends.
struct Holder {
    socket: PathBuf,
    pid: u32,
    _dir: TempDir,
}

impl Holder {
    fn start() -> Self {
        // Socket paths must be short, so not under the test's own temp.
        let dir = tempfile::Builder::new()
            .prefix("h2s")
            .tempdir_in("/tmp")
            .unwrap();
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
            let _ = kill_process(pid, Signal::TERM);
        }
    }
}

/// Resumes a stopped holder when the test ends, so none is left stopped.
struct Resumed(u32);

impl Drop for Resumed {
    fn drop(&mut self) {
        // A pid that no longer names a process is fine.
        if let Some(pid) = i32::try_from(self.0).ok().and_then(Pid::from_raw) {
            let _ = kill_process(pid, Signal::CONT);
        }
    }
}

#[tokio::test]
async fn a_shutdown_of_a_stopped_holder_gives_up_within_its_bound() {
    let holder = Holder::start();
    let pid = i32::try_from(holder.pid)
        .ok()
        .and_then(Pid::from_raw)
        .expect("a holder pid");
    kill_process(pid, Signal::STOP).expect("the test can stop its own holder");
    // Declared after `holder`, so it resumes the holder before the holder ends.
    let _resumed = Resumed(holder.pid);

    let result = tokio::time::timeout(GUARD, shutdown(&holder.socket))
        .await
        .expect("the shutdown returned within the guard, rather than waiting on the stopped holder");
    assert!(
        matches!(result, Err(HolderError::ShutdownTimeout { .. })),
        "{result:?}"
    );
}
