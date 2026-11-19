//! Runtime admin/fault stop and hold points (SPLIT_PHASE3_DAEMON §2.1 Step 6a).
//!
//! Uses [`humata2_rpc_kit::admin::is_admin_mode`] (or `cfg!(feature = "faults")`)
//! at runtime so library crates do not need `_admin` compile twins, while a
//! release build ignores `HUMATA2_STOP_AT`, `HUMATA2_HOLD_AT`, and
//! `HUMATA2_HOLD_FIFO`.

use std::sync::atomic::{AtomicU64, Ordering};

#[inline]
fn test_points_enabled() -> bool {
    cfg!(feature = "faults") || humata2_rpc_kit::admin::is_admin_mode()
}

/// Kills the daemon here when `HUMATA2_STOP_AT` names this point, so a
/// test can show what a crash here leaves behind. Test daemons only: a
/// release build never reads the variable.
pub fn point(name: &str) {
    if !test_points_enabled() {
        return;
    }
    if std::env::var("HUMATA2_STOP_AT").is_ok_and(|wanted| wanted == name) {
        let _ = rustix::process::kill_process(
            rustix::process::getpid(),
            rustix::process::Signal::KILL,
        );
        std::process::abort();
    }
}

/// Holds the task here when `HUMATA2_HOLD_AT` names this point, until the
/// test opens `HUMATA2_HOLD_FIFO` for writing: opening a FIFO to read waits
/// for a writer, so a test sees the state mid-way with no timing. Test
/// daemons only.
pub async fn hold_point(name: &str) {
    if !test_points_enabled() {
        return;
    }
    if std::env::var("HUMATA2_HOLD_AT").is_ok_and(|wanted| wanted == name)
        && let Ok(fifo) = std::env::var("HUMATA2_HOLD_FIFO")
    {
        let _ = tokio::task::spawn_blocking(move || std::fs::File::open(fifo)).await;
    }
}

/// As [`hold_point`] for code already on a blocking thread, and for a point
/// reached many times: `HUMATA2_HOLD_AT=name@n` holds the nth time it is
/// reached (`name` alone, the first). Only the calling thread waits, so the
/// runtime's workers go on serving. Test daemons only.
pub fn hold_blocking(name: &str) {
    static REACHED: AtomicU64 = AtomicU64::new(0);
    if !test_points_enabled() {
        return;
    }
    let Ok(wanted) = std::env::var("HUMATA2_HOLD_AT") else {
        return;
    };
    let (point, nth) = match wanted.split_once('@') {
        Some((point, nth)) => (point, nth.parse().unwrap_or(1)),
        None => (wanted.as_str(), 1),
    };
    if point != name || REACHED.fetch_add(1, Ordering::SeqCst) + 1 != nth {
        return;
    }
    if let Ok(fifo) = std::env::var("HUMATA2_HOLD_FIFO") {
        let _ = std::fs::File::open(fifo);
    }
}
