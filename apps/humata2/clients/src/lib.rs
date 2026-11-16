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
pub mod comments;
mod daemon;
pub mod desk;
mod file_manager;
mod files;
pub mod fixture;
pub mod history;
mod keychain;
pub mod marks;
pub mod model;
pub mod machines;
mod workspace;

use std::sync::Arc;
use std::time::Duration;

pub use clipboard::{Clipboard, FakeClipboard};
pub use daemon::{
    Daemon, DaemonError, EVENT_QUEUE, Event, Events, FakeDaemon, Operation, Reply, Terminals,
};
pub use comments::{CommentsClient, FakeComments};
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
    /// A session's review comments.
    pub comments: Arc<dyn CommentsClient>,
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
    /// Adding machines and a machine's page (HO-1 to HO-7, MP-12 to MP-17, SK-2, SK-5).
    pub machines: Arc<dyn machines::Machines>,
    /// Where the app's own state is saved between launches.
    pub storage: Arc<dyn Storage>,
}

/// The fakes behind [`Clients::fake`], for tests to steer and inspect.
#[derive(Clone)]
pub struct Fakes {
    /// The daemon.
    pub daemon: FakeDaemon,
    /// The review comments.
    pub comments: FakeComments,
    /// The clipboard.
    pub clipboard: FakeClipboard,
    /// The file system.
    pub files: FakeFileSystem,
    /// The file manager.
    pub file_manager: FakeFileManager,
    /// The keychain.
    pub keychain: FakeKeychain,
    /// The machines client.
    pub machines: machines::FakeMachines,
    /// Saved app state. Hand the same one to the next launch to restart.
    pub storage: Arc<MemoryStorage>,
}

impl Clients {
    /// Fakes holding `workspace`, on `clock`, with no latency.
    #[must_use]
    pub fn fake(workspace: Workspace, clock: Arc<dyn Clock>) -> (Self, Fakes) {
        let daemon = FakeDaemon::new(workspace, clock.clone());
        // An add in the dialog makes the machine in the fake workspace.
        let fake_machines = machines::FakeMachines::new().adding_through(Arc::new(daemon.clone()));
        Self::with_fakes(Fakes {
            daemon,
            comments: FakeComments::new(),
            clipboard: FakeClipboard::default(),
            files: FakeFileSystem::default(),
            file_manager: FakeFileManager::default(),
            keychain: FakeKeychain::default(),
            machines: fake_machines,
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
            comments: Arc::new(fakes.comments.clone()),
            clock,
            clipboard: Arc::new(fakes.clipboard.clone()),
            files: Arc::new(fakes.files.clone()),
            file_manager: Arc::new(fakes.file_manager.clone()),
            keychain: Arc::new(fakes.keychain.clone()),
            machines: Arc::new(fakes.machines.clone()),
            storage: fakes.storage.clone(),
        };
        (clients, fakes)
    }

    /// Fakes as the desktop app runs them before the real daemon exists:
    /// the standard workspace, with a little latency and a streaming agent
    /// so loading states and streaming show, and state saved in `storage`.
    #[must_use]
    pub fn demo(clock: Arc<dyn Clock>, storage: Arc<dyn Storage>) -> (Self, FakeDaemon) {
        // The demo has the Desk of frame 36, so the installed app shows it.
        let mut workspace = fixture::design();
        fixture::desk(&mut workspace);
        let daemon = FakeDaemon::new(workspace, clock.clone())
            .with_latency(Duration::from_millis(120))
            .with_step(Duration::from_millis(350));
        let clients = Self {
            daemon: Arc::new(daemon.clone()),
            terminals: Arc::new(daemon.clone()),
            desk: Arc::new(daemon.clone()),
            comments: Arc::new(FakeComments::new()),
            clock,
            clipboard: Arc::new(FakeClipboard::default()),
            files: Arc::new(FakeFileSystem::default()),
            file_manager: Arc::new(FakeFileManager::default()),
            keychain: Arc::new(FakeKeychain::default()),
            machines: Arc::new(machines::FakeMachines::new().adding_through(Arc::new(daemon.clone()))),
            storage,
        };
        (clients, daemon)
    }
}
