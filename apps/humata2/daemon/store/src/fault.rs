//! Named points a test can kill the process at, to show what a crash
//! there leaves behind.
//!
//! Built with the `faults` feature (Bazel's testonly `store_faults` and
//! the crates built on it), [`point`] reads `HUMATA2_STOP_AT`: `name`
//! kills the process with SIGKILL the first time it reaches `name`, and
//! `name@3` the third time. Nothing is flushed or unwound, as in a power
//! cut or `kill -9`; with `HUMATA2_POWER_CUT_ROOTS` set as well, writes
//! not yet synced are undone first, as a power cut would lose them.
//! `HUMATA2_FAULT_TRACE` and `HUMATA2_FAULT_ARM` add a trace of the
//! points reached and a gate on when they count (see [`point`]).
//! Without the feature, [`point`] does nothing and the
//! variables are never read, so a release build can't be stopped this way.
//!
//! [`fails`] is the same idea for an error instead of a kill:
//! `HUMATA2_FAIL_AT=name` makes the step named `name` return its own error
//! every time it runs, and `name@3` only the third time, so a test can show
//! what a failed step leaves behind while the process goes on. The points
//! are the names the code passes to [`fails`]: `revive.start_holder` (the
//! start of an agent's holder, for a launch, a resume or a relaunch) and
//! `restore_channel.finish_files` (moving the files of a restore a cut left
//! to finish).
//! `restore_channel.import_record` (bringing a deleted channel's records back
//! from its side file, for a restore a cut left to finish) is the third.

/// Marks the points either side of the store committing `op`:
/// `store.<op>.committing`, with the transaction written but not
/// committed, and `store.<op>.committed`, once it is.
pub(crate) fn commit_point(op: &str, stage: &str) {
    #[cfg(feature = "faults")]
    point(&format!("store.{op}.{stage}"));
    #[cfg(not(feature = "faults"))]
    let _ = (op, stage);
}

/// Whether the step `name` should fail now, as `HUMATA2_FAIL_AT` asks. The
/// caller returns its own error when this is true. Without the feature it is
/// always false, so a release build compiles it away and never reads the
/// variable.
#[cfg(not(feature = "faults"))]
#[inline(always)]
#[must_use]
pub fn fails(_name: &str) -> bool {
    false
}

/// Whether the step `name` should fail now: `HUMATA2_FAIL_AT=name` every
/// time, `HUMATA2_FAIL_AT=name@n` only the nth time this process reaches it.
#[cfg(feature = "faults")]
#[must_use]
pub fn fails(name: &str) -> bool {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU64, Ordering};

    static WANTED: OnceLock<Option<String>> = OnceLock::new();
    static HITS: AtomicU64 = AtomicU64::new(0);

    let Some(wanted) = WANTED.get_or_init(|| std::env::var("HUMATA2_FAIL_AT").ok()) else {
        return false;
    };
    let (point, nth) = split_fail(wanted);
    if point != name {
        return false;
    }
    fails_this_time(nth, HITS.fetch_add(1, Ordering::SeqCst) + 1)
}

/// `HUMATA2_FAIL_AT`'s value as the point it names and which time it fails.
#[cfg(feature = "faults")]
fn split_fail(wanted: &str) -> (&str, Option<u64>) {
    match wanted.split_once('@') {
        Some((point, nth)) => (point, Some(nth.parse().unwrap_or(1))),
        None => (wanted, None),
    }
}

#[cfg(feature = "faults")]
fn fails_this_time(nth: Option<u64>, reached: u64) -> bool {
    nth.is_none_or(|nth| nth == reached)
}

#[cfg(all(test, feature = "faults"))]
mod fail_tests {
    use super::{fails_this_time, split_fail};

    #[test]
    fn a_point_with_no_count_fails_every_time_it_is_reached() {
        let (point, nth) = split_fail("revive.start_holder");
        assert_eq!(point, "revive.start_holder");
        assert!((1..=3).all(|reached| fails_this_time(nth, reached)));
    }

    #[test]
    fn a_point_with_a_count_fails_only_that_time() {
        let (point, nth) = split_fail("revive.start_holder@2");
        assert_eq!(point, "revive.start_holder");
        let failed: Vec<u64> = (1..=4).filter(|reached| fails_this_time(nth, *reached)).collect();
        assert_eq!(failed, vec![2]);
    }
}

/// Marks a point a crash could happen at.
#[cfg(not(feature = "faults"))]
#[inline(always)]
pub fn point(_name: &str) {}

/// Marks a point a crash could happen at; kills the process here if
/// `HUMATA2_STOP_AT` names it.
///
/// `HUMATA2_FAULT_TRACE=<file>` also appends the name of every point
/// reached, one per line, with the line synced before this returns, so
/// it is there after a SIGKILL right after. `HUMATA2_FAULT_ARM=<file>`
/// holds all of this back until that file exists: a point reached
/// earlier is neither traced nor counted toward `name@n`, so a test
/// arms after its setup and sees only the operation it is about.
#[cfg(feature = "faults")]
pub fn point(name: &str) {
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    struct Config {
        target: Option<(String, u64)>,
        trace: Option<PathBuf>,
        arm: Option<PathBuf>,
    }

    static CONFIG: OnceLock<Config> = OnceLock::new();
    static HITS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

    let config = CONFIG.get_or_init(|| Config {
        target: std::env::var("HUMATA2_STOP_AT").ok().map(|wanted| match wanted.split_once('@') {
            Some((point, nth)) => (point.to_owned(), nth.parse().unwrap_or(1)),
            None => (wanted, 1),
        }),
        trace: std::env::var_os("HUMATA2_FAULT_TRACE").map(PathBuf::from),
        arm: std::env::var_os("HUMATA2_FAULT_ARM").map(PathBuf::from),
    });
    if config.target.is_none() && config.trace.is_none() {
        return;
    }
    if config.arm.as_ref().is_some_and(|arm| !arm.exists()) {
        return;
    }
    if let Some(trace) = &config.trace {
        // One write on an append-only file, then a sync: the line is whole
        // and on disk even if the next thing is a SIGKILL. A trace that
        // can't be written is not worth stopping the process for.
        let line = format!("{name}\n");
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(trace)
            && file.write_all(line.as_bytes()).is_ok()
        {
            let _ = file.sync_data();
        }
    }
    let Some((wanted, nth)) = &config.target else { return };
    if wanted != name {
        return;
    }
    let mut hits = HITS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let count = hits.get_or_insert_with(HashMap::new).entry(name.to_owned()).or_insert(0);
    *count += 1;
    if *count == *nth {
        stop(name);
    }
}

/// Kills this process with SIGKILL, as `kill -9` would; with a power cut
/// being simulated (`HUMATA2_POWER_CUT_ROOTS`, see humata2_durable),
/// what wasn't synced is put back first, as a power cut would leave it.
/// The process never goes on past here.
#[cfg(feature = "faults")]
pub(crate) fn stop(name: &str) -> ! {
    humata2_durable::cut(name)
}
