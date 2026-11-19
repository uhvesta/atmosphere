//! Points a test can stop, hold or trace the daemon at, to show what a
//! crash or a power cut there leaves behind.
//!
//! A test daemon built with `faults` (Bazel's testonly `humatad2_faults`)
//! shares the points of the crates under it: [`point`] is
//! `humata2_store::fault::point`, so `HUMATA2_STOP_AT=name[@n]`,
//! `HUMATA2_FAULT_TRACE=<file>` and `HUMATA2_FAULT_ARM=<file>` mean the same
//! for a point in the daemon as for one in the sessions or revive crates, and
//! `HUMATA2_POWER_CUT_ROOTS` makes a stop a power cut. The plain admin test
//! daemon only stops at `HUMATA2_STOP_AT=name`. A release build reads none
//! of the variables, so it can't be stopped this way.

/// Marks a point a crash could happen at; kills the daemon here when
/// `HUMATA2_STOP_AT` names it.
#[cfg(feature = "faults")]
pub(crate) fn point(name: &str) {
    humata2_store::fault::point(name);
}

/// Kills the daemon here when `HUMATA2_STOP_AT` names this point, so a
/// test can show what a crash here leaves behind. Test daemons only: a
/// release build never reads the variable.
#[cfg(not(feature = "faults"))]
pub(crate) fn point(name: &str) {
    humata2_daemon_runtime::fault::point(name);
}

/// Holds the task here when `HUMATA2_HOLD_AT` names this point, until the
/// test opens `HUMATA2_HOLD_FIFO` for writing: opening a FIFO to read waits
/// for a writer, so a test sees the state mid-way with no timing. Test
/// daemons only.
pub(crate) async fn hold_point(name: &str) {
    humata2_daemon_runtime::fault::hold_point(name).await;
}

/// As [`hold_point`] for code already on a blocking thread, and for a point
/// reached many times: `HUMATA2_HOLD_AT=name@n` holds the nth time it is
/// reached (`name` alone, the first). Only the calling thread waits, so the
/// runtime's workers go on serving. Test daemons only.
pub(crate) fn hold_blocking(name: &str) {
    humata2_daemon_runtime::fault::hold_blocking(name);
}

/// Test builds only: SIGUSR2 stops the daemon as a power cut would, with
/// what it wrote and didn't sync undone (see humata2_durable). Needs a
/// running tokio runtime.
#[cfg(feature = "faults")]
pub(crate) fn power_cut_on_sigusr2() {
    let signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined2());
    if let Ok(mut signal) = signal {
        tokio::spawn(async move {
            if signal.recv().await.is_some() {
                humata2_durable::cut("SIGUSR2");
            }
        });
    }
}

#[cfg(not(feature = "faults"))]
pub(crate) fn power_cut_on_sigusr2() {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shipped daemon never stops or holds at a test point, whatever its
    /// environment says: the test runs itself again with the variables set
    /// and a FIFO no one writes to, and that run returns.
    #[cfg(not(feature = "faults"))]
    #[test]
    fn a_release_build_ignores_the_stop_and_hold_variables() {
        const CHILD: &str = "HUMATA2_TEST_POINTS_CHILD";
        if let Some(done) = std::env::var_os(CHILD) {
            point("ignored-point");
            tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("a runtime")
                .block_on(hold_point("ignored-point"));
            hold_blocking("ignored-point");
            std::fs::write(done, b"past").expect("the run says it got past");
            return;
        }
        let dir = tempfile::tempdir().expect("a temp dir");
        let (fifo, done) = (dir.path().join("hold"), dir.path().join("done"));
        let made = std::process::Command::new("mkfifo").arg(&fifo).status().expect("mkfifo runs");
        assert!(made.success());
        let mut child = std::process::Command::new(std::env::current_exe().expect("the test binary"))
            .args(["--exact", "fault::tests::a_release_build_ignores_the_stop_and_hold_variables"])
            .env(CHILD, &done)
            .env("HUMATA2_STOP_AT", "ignored-point")
            .env("HUMATA2_HOLD_AT", "ignored-point")
            .env("HUMATA2_HOLD_FIFO", &fifo)
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("the test runs itself");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(child.wait());
        });
        let ended = rx
            .recv_timeout(std::time::Duration::from_secs(60))
            .expect("the run returns instead of holding")
            .expect("the run is waited on");
        assert!(ended.success(), "the run was not stopped: {ended:?}");
        assert!(done.exists(), "the run got past both points");
    }
}
