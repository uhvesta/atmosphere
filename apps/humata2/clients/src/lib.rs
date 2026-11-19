//! The clients Humata 2's features depend on, one trait per outside
//! service: the daemons ([`Daemon`], [`Terminals`]), the clock, the
//! clipboard, the file system, the keychain, and where the app's own state
//! is saved between launches ([`Storage`], PER-25).
//!
//! The traits and the model are `humata2_client_api`, re-exported here
//! whole; this crate adds the fakes, holding the dummy data in [`fixture`],
//! and [`Clients`], which every feature takes. The real daemon client
//! (`clients/live`) depends on the api crate only, so a change to a fake or
//! a fixture doesn't rebuild it (SPLIT). See README.md.

pub use humata2_client_api::*;

pub mod comments {
    //! A session's review comments.
    pub use humata2_client_api::comments::*;
    pub mod fake;
    pub use fake::{Call, FakeComments, Typing};
}
#[doc(hidden)]
pub mod daemon {
    pub use humata2_client_api::daemon::*;
    mod fake;
    pub use fake::{FakeDaemon, FakeViews};
}
mod fakes {
    pub mod clipboard;
    pub mod file_manager;
    pub mod files;
    pub mod keychain;
    pub mod local_daemon;
    pub mod reveal;
}
pub mod fixture;
pub mod history {
    //! Search over every session's conversation.
    pub use humata2_client_api::history::*;
    pub mod fake;
    pub use fake::{Call, FakeHistory, IndexedSession};
}
pub mod machines {
    //! Adding machines and a machine's page.
    pub use humata2_client_api::machines::*;
    pub mod fake;
    pub use fake::{Call, FakeMachines};
}
pub mod marks {
    //! Pins and VIP marks.
    pub use humata2_client_api::marks::*;
    pub mod fake;
    pub use fake::{Call, FakeMarks};
}
pub mod pull_requests {
    //! Pull request reviews.
    pub use humata2_client_api::pull_requests::*;
    pub mod fake;
    pub use fake::{Call, FakePullRequests};
}
pub mod review_chat {
    //! The review chat.
    pub use humata2_client_api::review_chat::*;
    pub mod fake;
    pub use fake::FakeReviewChat;
}
pub mod views {
    //! The views an agent offers.
    pub use humata2_client_api::views::*;
    pub use crate::daemon::FakeViews;
}
mod workspace {
    pub use humata2_client_api::Workspace;
}

pub use comments::FakeComments;
pub use daemon::FakeDaemon;
pub use fakes::clipboard::FakeClipboard;
pub use fakes::file_manager::FakeFileManager;
pub use fakes::files::FakeFileSystem;
pub use fakes::keychain::FakeKeychain;
pub use fakes::local_daemon::FakeLocalDaemon;
pub use fakes::reveal::FakeReveal;

use std::sync::Arc;
use std::time::Duration;

/// The fakes behind [`fake_clients`], for tests to steer and inspect.
#[derive(Clone)]
pub struct Fakes {
    /// The daemon.
    pub daemon: FakeDaemon,
    /// The review comments.
    pub comments: FakeComments,
    /// Conversation search.
    pub history: history::FakeHistory,
    /// Pins and VIP marks.
    pub marks: marks::FakeMarks,
    /// The clipboard.
    pub clipboard: FakeClipboard,
    /// Finder.
    pub reveal: FakeReveal,
    /// The file system.
    pub files: FakeFileSystem,
    /// The file manager.
    pub file_manager: FakeFileManager,
    /// The keychain.
    pub keychain: FakeKeychain,
    /// This Mac's own daemon.
    pub local_daemon: FakeLocalDaemon,
    /// The machines client.
    pub machines: machines::FakeMachines,
    /// The views agents offered.
    pub views: views::FakeViews,
    /// Pull request reviews.
    pub pull_requests: pull_requests::FakePullRequests,
    /// The review chat.
    pub review_chat: review_chat::FakeReviewChat,
    /// Saved app state. Hand the same one to the next launch to restart.
    pub storage: Arc<MemoryStorage>,
}

/// Fakes holding `workspace`, on `clock`, with no latency.
#[must_use]
pub fn fake_clients(workspace: Workspace, clock: Arc<dyn Clock>) -> (Clients, Fakes) {
    let marks = marks::FakeMarks::seeded_from(&workspace);
    let views = views::FakeViews::seeded(&workspace);
    let daemon = FakeDaemon::new(workspace, clock.clone());
    // An add in the dialog makes the machine in the fake workspace.
    let fake_machines = machines::FakeMachines::new().through(daemon.clone());
    clients_with_fakes(Fakes {
        daemon,
        comments: FakeComments::new(),
        history: history::FakeHistory::new(),
        marks,
        clipboard: FakeClipboard::default(),
        reveal: FakeReveal::default(),
        files: FakeFileSystem::default(),
        file_manager: FakeFileManager::default(),
        keychain: FakeKeychain::default(),
        local_daemon: FakeLocalDaemon::default(),
        machines: fake_machines,
        views,
        pull_requests: pull_requests::FakePullRequests::new(),
        review_chat: review_chat::FakeReviewChat::new(),
        storage: Arc::new(MemoryStorage::new()),
    }, clock)
}

/// Clients over existing fakes, as a relaunch keeps the daemon and the
/// saved state.
#[must_use]
pub fn clients_with_fakes(fakes: Fakes, clock: Arc<dyn Clock>) -> (Clients, Fakes) {
    let clients = Clients {
        daemon: Arc::new(fakes.daemon.clone()),
        terminals: Arc::new(fakes.daemon.clone()),
        desk: Arc::new(fakes.daemon.clone()),
        comments: Arc::new(fakes.comments.clone()),
        history: Arc::new(fakes.history.clone()),
        marks: Arc::new(fakes.marks.clone()),
        clock,
        clipboard: Arc::new(fakes.clipboard.clone()),
        reveal: Arc::new(fakes.reveal.clone()),
        files: Arc::new(fakes.files.clone()),
        file_manager: Arc::new(fakes.file_manager.clone()),
        keychain: Arc::new(fakes.keychain.clone()),
        local_daemon: Arc::new(fakes.local_daemon.clone()),
        machines: Arc::new(fakes.machines.clone()),
        views: Arc::new(fakes.views.clone()),
        pull_requests: Arc::new(fakes.pull_requests.clone()),
        review_chat: Arc::new(fakes.review_chat.clone()),
        storage: fakes.storage.clone(),
    };
    (clients, fakes)
}

/// Fakes as the desktop app runs them before the real daemon exists:
/// the standard workspace, with a little latency and a streaming agent
/// so loading states and streaming show, and state saved in `storage`.
#[must_use]
pub fn demo_clients(clock: Arc<dyn Clock>, storage: Arc<dyn Storage>) -> (Clients, FakeDaemon) {
    // The demo has the Desk of frame 36, so the installed app shows it.
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let marks = marks::FakeMarks::seeded_from(&workspace);
    let views = views::FakeViews::seeded(&workspace);
    let daemon = FakeDaemon::new(workspace, clock.clone())
        .with_latency(Duration::from_millis(120))
        .with_step(Duration::from_millis(350));
    let clients = Clients {
        daemon: Arc::new(daemon.clone()),
        terminals: Arc::new(daemon.clone()),
        desk: Arc::new(daemon.clone()),
        comments: Arc::new(FakeComments::new()),
        history: Arc::new(history::FakeHistory::new()),
        marks: Arc::new(marks),
        clock,
        clipboard: Arc::new(FakeClipboard::default()),
        reveal: Arc::new(FakeReveal::default()),
        files: Arc::new(FakeFileSystem::default()),
        file_manager: Arc::new(FakeFileManager::default()),
        keychain: Arc::new(FakeKeychain::default()),
        local_daemon: Arc::new(FakeLocalDaemon::default()),
        machines: Arc::new(machines::FakeMachines::new().through(daemon.clone())),
        views: Arc::new(views),
        pull_requests: Arc::new(pull_requests::FakePullRequests::new()),
        review_chat: Arc::new(review_chat::FakeReviewChat::new()),
        storage,
    };
    (clients, daemon)
}
