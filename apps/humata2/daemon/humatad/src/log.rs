//! humatad2's own log (D77): each line it writes to stderr also goes to
//! `logs/humatad2.log` in its state folder, and so does a panic, so the
//! restart report of the next start can say what this run was doing when
//! it stopped (PER-29). The run before's log is kept as
//! `humatad2.previous.log` until the next start.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write as _};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

const CURRENT: &str = "humatad2.log";
const PREVIOUS: &str = "humatad2.previous.log";
/// Past this, the log starts over: what a restart report reads is its end.
const MOST_BYTES: u64 = 4 << 20;

struct Log {
    file: File,
    written: u64,
}

static LOG: OnceLock<Mutex<Log>> = OnceLock::new();

/// Writes `line` to stderr, and to the log once [`start`] has run.
pub(crate) fn line(line: &str) {
    eprintln!("{line}");
    let Some(log) = LOG.get() else { return };
    let mut log = log.lock().unwrap_or_else(PoisonError::into_inner);
    if log.written > MOST_BYTES && log.file.set_len(0).is_ok() {
        log.written = 0;
    }
    // The line is on stderr already; a log that can't be written loses it
    // only from the next restart report.
    if writeln!(log.file, "{line}").is_ok() {
        log.written += u64::try_from(line.len()).unwrap_or(u64::MAX) + 1;
    }
}

/// Starts this run's log in `logs`, keeping the run before's, and returns
/// where that one is, if there was one. Without a log the daemon still
/// runs; it says so, and restart reports lack its lines.
pub(crate) fn start(logs: &Path) -> Option<PathBuf> {
    let current = logs.join(CURRENT);
    let previous = logs.join(PREVIOUS);
    let kept = match fs::rename(&current, &previous) {
        Ok(()) => Some(previous),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            // A first start, or the log was removed: an older previous log
            // isn't the run before this one.
            let _ = fs::remove_file(&previous);
            None
        }
        Err(error) => {
            line(&format!(
                "humatad2: couldn't keep the last run's log {}, so this start's restart report won't have its lines; check the folder's permissions: {error}",
                current.display()
            ));
            None
        }
    };
    match OpenOptions::new().create(true).append(true).mode(0o600).open(&current) {
        Ok(file) => {
            if LOG.set(Mutex::new(Log { file, written: 0 })).is_ok() {
                let before = std::panic::take_hook();
                std::panic::set_hook(Box::new(move |info| {
                    if let Some(log) = LOG.get() {
                        let mut log = log.lock().unwrap_or_else(PoisonError::into_inner);
                        let _ = writeln!(log.file, "humatad2: panicked: {info}");
                    }
                    before(info);
                }));
            }
        }
        Err(error) => line(&format!(
            "humatad2: couldn't open its log {}, so the next restart report won't say what this run did; check the folder's permissions: {error}",
            current.display()
        )),
    }
    kept
}

/// As `eprintln!`, and into the daemon's log.
macro_rules! say {
    ($($arg:tt)*) => {
        $crate::log::line(&format!($($arg)*))
    };
}
pub(crate) use say;
