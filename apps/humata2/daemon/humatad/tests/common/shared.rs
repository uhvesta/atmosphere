//! Per-binary shared daemon helper for integration tests that do not restart
//! the daemon, do not mutate global settings, and isolate their own resources.

use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::OnceCell;

use crate::common::{Daemon, World, start_world};

static SHARED: OnceCell<Daemon> = OnceCell::const_new();
static SHARED_WORLD: OnceCell<(Daemon, World)> = OnceCell::const_new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Returns a single shared admin `humatad2` daemon for the current test binary.
///
/// Only use this in tests that:
/// - do not restart or stop the daemon,
/// - do not mutate global daemon settings, and
/// - use unique channel/session/tab/folder names per test.
pub async fn shared_daemon() -> &'static Daemon {
    SHARED.get_or_init(Daemon::start).await
}

/// Returns a single shared `(Daemon, World)` for the current test binary.
///
/// Same isolation rules as [`shared_daemon`]: use [`unique_name`] for channel,
/// repo, and session names so concurrent tests never collide.
pub async fn shared_world() -> &'static (Daemon, World) {
    SHARED_WORLD.get_or_init(start_world).await
}

/// Returns a unique short name (`{prefix}-{n}`) for isolating channels or
/// folders across tests sharing one daemon.
pub fn unique_name(prefix: &str) -> String {
    let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{n}")
}
