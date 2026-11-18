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
pub mod docs;
mod file_manager;
mod files;
pub mod fixture;
pub mod github;
pub mod history;
mod keychain;
mod local_daemon;
pub mod marks;
pub mod model;
// REPOS (RL-1): the repo library and the one repo search.
pub mod repos;
pub mod machines;
pub mod machine_names;
mod reveal;
pub mod views;
pub mod pull_requests;
pub mod review_chat;
mod workspace;

use std::sync::Arc;
use std::time::Duration;

pub use clipboard::{Clipboard, FakeClipboard};
pub use daemon::{
    AgentSwitch, ArchivePlan, Daemon, DaemonError, EVENT_QUEUE, Event, Events, FakeDaemon, FileChange,
    ForkOptions, ImageKind, LinkSink, Operation, PastTurn, Reply, SESSION_FILES_CAP, SendError,
    SwitchChange, TerminalLink, TerminalNotice, Terminals, UnpushedWork,
};
pub use comments::{CommentsClient, FakeComments};
pub use desk::DeskClient;
pub use file_manager::{
    EmptyPlan, Entry, EntryKind, FakeFileManager, FileManager, FileManagerError, FileReply, FileRoot, Folder,
    ItemLanding, Landing, Listing, RetentionSkip, StorageAction, StorageKind, StorageReport, StorageRow, TransferMode,
    TransferPhase,
    TransferRow, TrashEntry, TrashListing,
};
pub use files::{FakeFileSystem, FileError, FileSystem};
pub use keychain::{FakeKeychain, Keychain, KeychainError};
pub use local_daemon::{FakeLocalDaemon, LocalDaemon};
pub use reveal::{FakeReveal, FinderReveal, Reveal, RevealError, is_this_mac};
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
    /// Search over every session's conversation, and reading a hit's turns.
    pub history: Arc<dyn history::History>,
    /// Pins and VIP marks, kept on the host.
    pub marks: Arc<dyn marks::MarksClient>,
    /// Time, for latency, debouncing and save intervals.
    pub clock: Arc<dyn Clock>,
    /// Where Copy puts text.
    pub clipboard: Arc<dyn Clipboard>,
    /// Reveal in Finder, for places on This Mac (PL-10).
    pub reveal: Arc<dyn Reveal>,
    /// Files the app writes itself.
    pub files: Arc<dyn FileSystem>,
    /// Files on machines: the file manager, transfers, trash and Storage.
    pub file_manager: Arc<dyn FileManager>,
    /// Secrets.
    pub keychain: Arc<dyn Keychain>,
    /// This Mac's own daemon, for migrating one too old to do it itself (D374).
    pub local_daemon: Arc<dyn LocalDaemon>,
    /// Adding machines and a machine's page (HO-1 to HO-7, MP-12 to MP-17, SK-2, SK-5).
    pub machines: Arc<dyn machines::Machines>,
    /// Views the agents prepared, and the person's answers to them.
    pub views: Arc<dyn views::ViewsClient>,
    /// Pull request reviews: a pasted pull request, its review and what it posts (PR-1 to PR-17).
    pub pull_requests: Arc<dyn pull_requests::PullRequestsClient>,
    /// The review chat: what the agent sees and changes in a review's view (PR-22 to PR-26).
    pub review_chat: Arc<dyn review_chat::ReviewChatClient>,
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

impl Clients {
    /// Fakes holding `workspace`, on `clock`, with no latency.
    #[must_use]
    pub fn fake(workspace: Workspace, clock: Arc<dyn Clock>) -> (Self, Fakes) {
        let marks = marks::FakeMarks::seeded_from(&workspace);
        let views = views::FakeViews::seeded(&workspace);
        let daemon = FakeDaemon::new(workspace, clock.clone());
        // An add in the dialog makes the machine in the fake workspace.
        let fake_machines = machines::FakeMachines::new().through(daemon.clone());
        Self::with_fakes(Fakes {
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
    pub fn with_fakes(fakes: Fakes, clock: Arc<dyn Clock>) -> (Self, Fakes) {
        let clients = Self {
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
    pub fn demo(clock: Arc<dyn Clock>, storage: Arc<dyn Storage>) -> (Self, FakeDaemon) {
        // The demo has the Desk of frame 36, so the installed app shows it.
        let mut workspace = fixture::design();
        fixture::desk(&mut workspace);
        let marks = marks::FakeMarks::seeded_from(&workspace);
        let views = views::FakeViews::seeded(&workspace);
        let daemon = FakeDaemon::new(workspace, clock.clone())
            .with_latency(Duration::from_millis(120))
            .with_step(Duration::from_millis(350));
        let clients = Self {
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
}
