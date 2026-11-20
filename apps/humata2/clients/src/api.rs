//! The clients Humata 2's features depend on, one trait per outside
//! service: the daemons ([`Daemon`], [`Terminals`]), the clock, the
//! clipboard, the file system, the keychain, and where the app's own state
//! is saved between launches ([`Storage`], PER-25), with the domain model
//! they speak.
//!
//! No fakes and no dummy data: those are in `humata2_clients`, which
//! re-exports all of this crate, so a change to a fake or a fixture leaves
//! this crate and what depends only on it (the live clients) cached (SPLIT).

mod clipboard;
pub mod comments;
// The fakes in `humata2_clients` build on the daemon module's items.
#[doc(hidden)]
pub mod daemon;
pub mod desk;
pub mod docs;
// The file manager fake in `humata2_clients` builds on its helpers.
#[doc(hidden)]
pub mod file_manager;
mod files;
pub mod github;
pub mod history;
mod keychain;
mod local_daemon;
pub mod marks;
pub mod todo;
pub mod model;
pub mod mass_delete;
// PROGRESS: the one model of a running operation's progress.
pub mod progress;
// REPOS (RL-1): the repo library and the one repo search.
pub mod repos;
pub mod machines;
pub mod machine_names;
mod notifier;
mod reveal;
pub mod views;
pub mod pull_requests;
pub mod review_chat;
mod workspace;

pub use clipboard::Clipboard;
pub use daemon::{
    AgentSwitch, ArchivePlan, Daemon, DaemonError, EVENT_QUEUE, Event, Events, FileChange,
    ForkOptions, HolderTab, ImageKind, LinkSink, Operation, PastTurn, Reply, SESSION_FILES_CAP, SendError, fair_share,
    RunningHolder, SwitchChange, TerminalLink, agent_of_kind, TerminalNotice, Terminals, UnpushedWork,
};
pub use comments::CommentsClient;
pub use desk::DeskClient;
pub use file_manager::{
    EmptyPlan, Entry, EntryKind, FileManager, FileManagerError, FileReply, FileRoot, Folder,
    ItemLanding, Landing, Listing, RetentionSkip, StorageAction, StorageKind, StorageReport, StorageRow, TransferMode,
    TransferPhase,
    TransferRow, TrashEntry, TrashListing,
};
pub use files::{FileError, FileSystem};
pub use keychain::{Keychain, KeychainError};
pub use local_daemon::LocalDaemon;
pub use notifier::{DesktopNotice, NoopNotifier, Notifier, NotifyTarget};
pub use reveal::{FinderReveal, Reveal, RevealError, is_this_mac};
pub use tca::{Clock, MemoryStorage, Storage};
pub use workspace::Workspace;

use std::sync::Arc;

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
    /// The To-do list, kept on the host.
    pub todo: Arc<dyn todo::TodoClient>,
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
    /// Desktop notifications (NOTIFY, D889).
    pub notifier: Arc<dyn Notifier>,
    /// Where the app's own state is saved between launches.
    pub storage: Arc<dyn Storage>,
}
