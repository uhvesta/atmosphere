//! This Mac's own daemon, for the one thing the app does to it that a
//! daemon can't do for itself: migrating an old daemon that predates
//! UPGRADE and so can't hold a Migrate item or answer one (D374).

use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::watch;

use crate::daemon::{DaemonError, Reply};

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

#[derive(Default)]
struct Script {
    migrations: Vec<String>,
    fail_next: Option<DaemonError>,
}

/// A [`LocalDaemon`] that records what it was asked and fails when told to.
/// By default This Mac's daemon never comes back.
#[derive(Clone)]
pub struct FakeLocalDaemon {
    script: Arc<Mutex<Script>>,
    /// The bundle This Mac's daemon answers at now, once it has come back.
    answering: Arc<watch::Sender<Option<String>>>,
}

impl Default for FakeLocalDaemon {
    fn default() -> Self {
        Self {
            script: Arc::default(),
            answering: Arc::new(watch::channel(None).0),
        }
    }
}

impl FakeLocalDaemon {
    fn script(&self) -> MutexGuard<'_, Script> {
        match self.script.lock() {
            Ok(script) => script,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// The staged bundles it was asked to migrate to, in order.
    #[must_use]
    pub fn migrations(&self) -> Vec<String> {
        self.script().migrations.clone()
    }

    /// Records that This Mac's daemon now answers at `bundle`. It stays so:
    /// a pending or later `back_at(bundle)` resolves, any other doesn't.
    pub fn come_back(&self, bundle: &str) {
        self.answering.send_replace(Some(bundle.to_owned()));
    }

    /// Makes the next migrate fail with `error`.
    pub fn fail_next(&self, error: DaemonError) {
        self.script().fail_next = Some(error);
    }
}

impl LocalDaemon for FakeLocalDaemon {
    fn migrate_old_daemon(&self, staged_bundle: &str) -> Reply<()> {
        let fake = self.clone();
        let bundle = staged_bundle.to_owned();
        Box::pin(async move {
            let mut script = fake.script();
            script.migrations.push(bundle);
            match script.fail_next.take() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        })
    }

    fn back_at(&self, staged_bundle: &str) -> Reply<()> {
        let mut answering = self.answering.subscribe();
        let bundle = staged_bundle.to_owned();
        Box::pin(async move {
            // The sender lives as long as any clone of the fake; if every
            // clone is gone nobody is waiting, so the wait just never ends.
            let closed = answering
                .wait_for(|now| now.as_deref() == Some(bundle.as_str()))
                .await
                .is_err();
            if closed {
                std::future::pending::<()>().await;
            }
            Ok(())
        })
    }
}
