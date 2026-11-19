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
pub mod model;
// PROGRESS: the one model of a running operation's progress.
pub mod progress;
// REPOS (RL-1): the repo library and the one repo search.
pub mod repos;
pub mod machines;
pub mod machine_names;
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
pub use reveal::{FinderReveal, Reveal, RevealError, is_this_mac};
pub use tca::{Clock, MemoryStorage, Storage};
pub use workspace::Workspace;
