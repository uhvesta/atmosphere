//! The fake's Settings › Sync (FUSEFS): an in-memory store. A test makes the
//! next write fail with `fail_next(Operation::SetSyncSettings, ..)`.

use super::FakeDaemon;
use crate::daemon::{Operation, Reply, SyncChange, SyncSnapshot};

pub(super) fn sync_settings(daemon: &FakeDaemon) -> Reply<SyncSnapshot> {
    daemon.reply(None, "sync_settings".to_owned(), |inner| Ok(inner.sync.clone()))
}

pub(super) fn set_sync_settings(daemon: &FakeDaemon, change: SyncChange) -> Reply<SyncSnapshot> {
    daemon.reply(
        Some(Operation::SetSyncSettings),
        format!("set_sync_settings {change:?}"),
        move |inner| {
            if let Some(sync) = change.sync {
                inner.sync.sync = sync;
            }
            if let Some(on) = change.fusefs_enabled {
                inner.sync.fusefs_enabled = on;
            }
            Ok(inner.sync.clone())
        },
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tca::TestClock;

    use super::*;
    use crate::daemon::{Daemon, SyncSettings};
    use crate::fixture;

    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime.block_on(future),
            Err(error) => panic!("test runtime: {error}"),
        }
    }

    fn fake() -> FakeDaemon {
        FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()))
    }

    #[test]
    fn a_write_is_kept_and_read_back() {
        let daemon = fake();
        let sync = SyncSettings { mcp: true, ..SyncSettings::default() };
        let after = block_on(
            daemon.set_sync_settings(SyncChange { sync: Some(sync.clone()), fusefs_enabled: None }),
        );
        assert_eq!(after.map(|s| s.sync), Ok(sync.clone()));
        let read = block_on(daemon.sync_settings());
        assert_eq!(read.map(|s| (s.sync, s.fusefs_enabled)), Ok((sync, false)));
    }

    #[test]
    fn a_write_of_one_part_leaves_the_other() {
        let daemon = fake();
        let sync = SyncSettings { skills: false, ..SyncSettings::default() };
        let _ = block_on(
            daemon.set_sync_settings(SyncChange { sync: Some(sync.clone()), fusefs_enabled: None }),
        );
        let after =
            block_on(daemon.set_sync_settings(SyncChange { sync: None, fusefs_enabled: Some(true) }));
        assert_eq!(after.map(|s| (s.sync, s.fusefs_enabled)), Ok((sync, true)));
    }

    #[test]
    fn a_failed_write_changes_nothing() {
        let daemon = fake();
        daemon.fail_next(Operation::SetSyncSettings, "no");
        let change = SyncChange { sync: None, fusefs_enabled: Some(true) };
        assert!(block_on(daemon.set_sync_settings(change)).is_err());
        let read = block_on(daemon.sync_settings());
        assert_eq!(read.map(|s| s.fusefs_enabled), Ok(false));
    }
}
