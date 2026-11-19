//! Runtime `admin_mode` toggle for test daemons (SPLIT_PHASE3_DAEMON §2.1 Step 6a).
//!
//! Set once from `main.rs` when `cfg!(feature = "admin")` (or `--admin` /
//! `--admin-pid`) is active, so library crates read [`is_admin_mode`] at
//! runtime instead of needing `_admin` library twins.

use std::sync::atomic::{AtomicBool, Ordering};

static ADMIN_MODE: AtomicBool = AtomicBool::new(false);

/// Enables or disables runtime admin test-daemon mode.
pub fn set_admin_mode(enabled: bool) {
    ADMIN_MODE.store(enabled, Ordering::Relaxed);
}

/// Whether this daemon runs as an admin test build (`humatad2_admin` or
/// `humatad2_faults`). Always `false` in a release `humatad2` binary.
#[inline]
pub fn is_admin_mode() -> bool {
    ADMIN_MODE.load(Ordering::Relaxed)
}
