//! This Mac's own daemon, for the one thing the app does to it that a
//! daemon can't do for itself: migrating an old daemon that predates
//! UPGRADE and so can't hold a Migrate item or answer one (D374).

use crate::daemon::Reply;

/// What the app asks of This Mac's daemon outside the daemon API.
pub trait LocalDaemon: Send + Sync + 'static {
    /// Migrates This Mac's old daemon to the release staged beside it,
    /// `staged_bundle`, and returns once the new daemon has started.
    fn migrate_old_daemon(&self, staged_bundle: &str) -> Reply<()>;

    /// Ends Ok once This Mac's daemon answers with ServerInfo
    /// `build.bundle == staged_bundle`. It has no bound of its own; the
    /// caller applies one.
    fn back_at(&self, staged_bundle: &str) -> Reply<()>;
}
