//! The clients Humata 2's features depend on, one trait per outside
//! service: the daemons ([`Daemon`], [`Terminals`]), the clock, the
//! clipboard, the file system, the keychain, and where the app's own state
//! is saved between launches ([`Storage`], PER-25).
//!
//! Only fakes exist so far, holding the dummy data in [`fixture`]. The real
//! daemon client will implement the same traits over `api/proto`, and the
//! desktop app will pick it in one place ([`Clients`]); features and views
//! don't change (see README.md).

mod clipboard;
mod daemon;
pub mod desk;
mod file_manager;
mod files;
pub mod fixture;
mod keychain;
pub mod model;
pub mod machines;
mod workspace;

use std::sync::Arc;
use std::time::Duration;

pub use clipboard::{Clipboard, FakeClipboard};
pub use daemon::{
    Daemon, DaemonError, EVENT_QUEUE, Event, Events, FakeDaemon, Operation, Reply, Terminals,
};
pub use desk::DeskClient;
pub use file_manager::{
    EmptyPlan, Entry, EntryKind, FakeFileManager, FileManager, FileManagerError, FileReply, FileRoot, Folder,
    Listing, RetentionSkip, StorageAction, StorageKind, StorageReport, StorageRow, TransferMode, TransferPhase,
    TransferRow, TrashEntry, TrashListing,
};
pub use files::{FakeFileSystem, FileError, FileSystem};
pub use keychain::{FakeKeychain, Keychain, KeychainError};
pub use tca::{Clock, MemoryStorage, Storage};
pub use workspace::Workspace;

/// Every client a feature may use. Cheap to clone.
#[derive(Clone)]
pub struct Clients {
    /// The daemons.
    pub daemon: Arc<dyn Daemon>,
    /// Sessions' terminal tabs.
    pub terminals: Arc<dyn Terminals>,
    /// The Desk: sessions outside any channel.
    pub desk: Arc<dyn DeskClient>,
    /// Time, for latency, debouncing and save intervals.
    pub clock: Arc<dyn Clock>,
    /// Where Copy puts text.
    pub clipboard: Arc<dyn Clipboard>,
    /// Files the app writes itself.
    pub files: Arc<dyn FileSystem>,
    /// Files on machines: the file manager, transfers, trash and Storage.
    pub file_manager: Arc<dyn FileManager>,
    /// Secrets.
    pub keychain: Arc<dyn Keychain>,
    /// Where the app's own state is saved between launches.
    pub storage: Arc<dyn Storage>,
}

/// The fakes behind [`Clients::fake`], for tests to steer and inspect.
#[derive(Clone)]
pub struct Fakes {
    /// The daemon.
    pub daemon: FakeDaemon,
    /// The clipboard.
    pub clipboard: FakeClipboard,
    /// The file system.
    pub files: FakeFileSystem,
    /// The file manager.
    pub file_manager: FakeFileManager,
    /// The keychain.
    pub keychain: FakeKeychain,
    /// Saved app state. Hand the same one to the next launch to restart.
    pub storage: Arc<MemoryStorage>,
}

impl Clients {
    /// Fakes holding `workspace`, on `clock`, with no latency.
    #[must_use]
    pub fn fake(workspace: Workspace, clock: Arc<dyn Clock>) -> (Self, Fakes) {
        let daemon = FakeDaemon::new(workspace, clock.clone());
        Self::with_fakes(Fakes {
            daemon,
            clipboard: FakeClipboard::default(),
            files: FakeFileSystem::default(),
            file_manager: FakeFileManager::default(),
            keychain: FakeKeychain::default(),
            storage: Arc::new(MemoryStorage::new()),
        }, clock)
    }

    /// Clients over existing fakes, as a relaunch keeps the daemon and the
    /// saved state.
    #[must_use]
    pub fn with_fakes(fakes: Fakes, clock: Arc<dyn Clock>) -> (Self, Fakes) {
        let clients = Self {
            daemon: Arc::new(fakes.daemon.clone()),
            terminals: Arc::new(fakes.daemon.clone()),
            desk: Arc::new(fakes.daemon.clone()),
            clock,
            clipboard: Arc::new(fakes.clipboard.clone()),
            files: Arc::new(fakes.files.clone()),
            file_manager: Arc::new(fakes.file_manager.clone()),
            keychain: Arc::new(fakes.keychain.clone()),
            storage: fakes.storage.clone(),
        };
        (clients, fakes)
    }

    /// Fakes as the desktop app runs them before the real daemon exists:
    /// the standard workspace, with a little latency and a streaming agent
    /// so loading states and streaming show, and state saved in `storage`.
    #[must_use]
    pub fn demo(clock: Arc<dyn Clock>, storage: Arc<dyn Storage>) -> (Self, FakeDaemon) {
        let daemon = FakeDaemon::new(fixture::design(), clock.clone())
            .with_latency(Duration::from_millis(120))
            .with_step(Duration::from_millis(350));
        let clients = Self {
            daemon: Arc::new(daemon.clone()),
            terminals: Arc::new(daemon.clone()),
            desk: Arc::new(daemon.clone()),
            clock,
            clipboard: Arc::new(FakeClipboard::default()),
            files: Arc::new(FakeFileSystem::default()),
            file_manager: Arc::new(FakeFileManager::default()),
            keychain: Arc::new(FakeKeychain::default()),
            storage,
        };
        (clients, daemon)
    }
}
