//! The file manager client (FM-*, LD-13..17): files and folders on any machine,
//! transfers between them, each machine's trash, and Storage.
//! Declared from lib.rs as `mod file_manager;`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tca::BoxFuture;
use tokio::sync::mpsc;

use crate::model::{ChannelId, MachineId, SessionId, TransferId};

/// Which of a machine's folders a pane is looking in (FM-10).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FileRoot {
    /// The user's home folder on the machine.
    Home,
    /// A channel's folder.
    ChannelFolder(ChannelId),
    /// A channel's `shared/` folder.
    ChannelShared(ChannelId),
    /// One of a session's own clones.
    SessionClone {
        /// The session.
        session: SessionId,
        /// The repository.
        repo: String,
    },
    /// What was pasted into a session's terminals; the user only (D335).
    SessionPastes(SessionId),
    /// A session's own folder, where a Desk session's documents sit,
    /// outside every clone (D382).
    SessionFolder {
        /// The session.
        session: SessionId,
    },
    /// What other machines sent here: `<root>/received/<machine>/`. People
    /// see it as "From <machine name>", never as a path (PL-4).
    Received {
        /// The machine the files came from.
        from: MachineId,
    },
}

/// A folder on a machine: where a pane is.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Folder {
    /// The machine it is on.
    pub machine: MachineId,
    /// Which of the machine's roots it is under.
    pub root: FileRoot,
    /// Where under the root, with `/` between names; empty is the root.
    pub path: String,
}

impl Folder {
    /// A root's top folder.
    #[must_use]
    pub fn root(machine: MachineId, root: FileRoot) -> Self {
        Self {
            machine,
            root,
            path: String::new(),
        }
    }

    /// The user's home folder on `machine`.
    #[must_use]
    pub fn home(machine: MachineId) -> Self {
        Self::root(machine, FileRoot::Home)
    }

    /// The folder called `name` inside this one.
    #[must_use]
    pub fn child(&self, name: &str) -> Self {
        Self {
            path: join(&self.path, name),
            ..self.clone()
        }
    }

    /// The folder this one is in; `None` at a root.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        if self.path.is_empty() {
            return None;
        }
        Some(Self {
            path: parent_of(&self.path).to_owned(),
            ..self.clone()
        })
    }

    /// The last name in the path, or `None` at a root.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        if self.path.is_empty() {
            None
        } else {
            Some(self.path.rsplit('/').next().unwrap_or(&self.path))
        }
    }

    /// The folder as people read it: `~/datasets`, `#channel/shared/out`,
    /// `repo (session)/src`.
    #[must_use]
    pub fn display_path(&self) -> String {
        let base = match &self.root {
            FileRoot::Home => "~".to_owned(),
            FileRoot::ChannelFolder(channel) => format!("#{channel}"),
            FileRoot::ChannelShared(channel) => format!("#{channel}/shared"),
            FileRoot::SessionClone { session, repo } => format!("{repo} ({session})"),
            FileRoot::SessionPastes(session) => format!("pastes ({session})"),
            FileRoot::SessionFolder { session } => format!("({session})"),
            FileRoot::Received { from } => format!("received/{from}"),
        };
        join(&base, &self.path)
    }
}

#[doc(hidden)]
pub fn join(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_owned()
    } else if name.is_empty() {
        base.to_owned()
    } else {
        format!("{base}/{name}")
    }
}

#[doc(hidden)]
pub fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

/// What an entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// A file.
    File,
    /// A folder.
    Folder,
    /// A symbolic link.
    Link,
}

/// One row of a folder listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Its name.
    pub name: String,
    /// What it is.
    pub kind: EntryKind,
    /// Its size; `None` for a folder.
    pub size: Option<u64>,
    /// When it last changed, in Unix seconds.
    pub modified_unix_s: i64,
}

/// What a folder holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    /// The folder.
    pub folder: Folder,
    /// Folders first, then files, each by name.
    pub entries: Vec<Entry>,
    /// When it was read, in Unix seconds; the views say "Today" and "5m"
    /// against this, never against the system clock.
    pub read_unix_s: i64,
    /// The viewer's local offset from UTC in seconds, as the client knows it.
    pub utc_offset_s: i32,
}

/// Whether a transfer leaves the source in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferMode {
    /// The source stays.
    Copy,
    /// The source goes to its machine's trash once everything arrived (FM-14).
    Move,
}

/// Where a transfer is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferPhase {
    /// Bytes are moving.
    Copying,
    /// Everything arrived and checksums are being compared.
    Verifying,
    /// An end is offline; it continues when that machine is back (FM-13).
    Paused {
        /// The machine it waits for.
        waiting_for: Option<MachineId>,
        /// Why, as a sentence.
        reason: String,
    },
    /// Finished.
    Done,
    /// Stopped by an error.
    Failed {
        /// What went wrong.
        reason: String,
    },
    /// Stopped by the user.
    Cancelled,
}

impl TransferPhase {
    /// Whether nothing more will happen to the transfer.
    #[must_use]
    pub const fn is_over(&self) -> bool {
        matches!(self, Self::Done | Self::Failed { .. } | Self::Cancelled)
    }
}

/// Where one thing a transfer carried ended up (PL-13).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Landing {
    /// Under its own name.
    AsAsked,
    /// The name was taken there, so it was kept beside it under this name.
    SavedAs(String),
    /// The same bytes were already there, so nothing was copied.
    AlreadyThere,
}

/// What became of one thing a finished transfer carried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemLanding {
    /// What it is called where it came from.
    pub name: String,
    /// Where it ended up.
    pub landing: Landing,
}

/// One line of the transfers tray (FM-7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferRow {
    /// Its id.
    pub id: TransferId,
    /// What moves, such as `datasets/`.
    pub what: String,
    /// The machine it comes from.
    pub from: MachineId,
    /// The machine it goes to.
    pub to: MachineId,
    /// Whether it runs through the host because neither end is it (FM-9).
    pub via_host: bool,
    /// Bytes confirmed so far.
    pub done_bytes: u64,
    /// Bytes in all.
    pub total_bytes: u64,
    /// Where it is.
    pub phase: TransferPhase,
    /// What became of each thing it carried; empty until it is done.
    pub outcomes: Vec<ItemLanding>,
}

impl TransferRow {
    /// How much is done, from 0 to 1.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn fraction(&self) -> f64 {
        if self.total_bytes == 0 {
            return if self.phase == TransferPhase::Done { 1.0 } else { 0.0 };
        }
        (self.done_bytes as f64 / self.total_bytes as f64).clamp(0.0, 1.0)
    }
}

/// One thing in a machine's trash (LD-6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashEntry {
    /// Its id, for Restore.
    pub id: String,
    /// What it was.
    pub description: String,
    /// How much room it takes.
    pub bytes: u64,
    /// When it was deleted, in Unix seconds.
    pub deleted_unix_s: i64,
    /// When the trash empties it, in Unix seconds (LD-7).
    pub empties_unix_s: i64,
}

/// A machine's trash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashListing {
    /// The machine.
    pub machine: MachineId,
    /// What is in it, newest first.
    pub items: Vec<TrashEntry>,
    /// What it takes in all.
    pub total_bytes: u64,
    /// When it was read, in Unix seconds.
    pub read_unix_s: i64,
}

/// What emptying a trash would do, shown before it is done (LD-7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmptyPlan {
    /// The machine.
    pub machine: MachineId,
    /// How many things go.
    pub items: u32,
    /// How much room that frees.
    pub frees_bytes: u64,
    /// Hand it back to [`FileManager::empty_trash`] to empty exactly these.
    pub token: String,
}

/// What a Storage row is about (LD-13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageKind {
    /// A session's own clones.
    SessionClones,
    /// The main clones.
    MainClones,
    /// A build cache in a session folder.
    BuildCache,
    /// The trash.
    Trash,
    /// Logs.
    Logs,
}

/// The one action that frees what a Storage row holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageAction {
    /// Nothing frees it.
    None,
    /// Archive the session; the parent runs the archive checks (LD-15).
    Archive(SessionId),
    /// Clear the session's build cache.
    ClearBuildCache(SessionId),
    /// Empty the trash after saying how much it frees.
    EmptyTrash,
    /// Trim old logs.
    TrimLogs,
}

/// One row of a machine's Storage page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageRow {
    /// What it is about.
    pub kind: StorageKind,
    /// Its name.
    pub what: String,
    /// A small second line, if it has one.
    pub note: Option<String>,
    /// What it takes.
    pub bytes: u64,
    /// When it was last used, in Unix seconds.
    pub last_used_unix_s: Option<i64>,
    /// What frees it.
    pub action: StorageAction,
    /// Whether the action asks first: a running session's build cache (LD-15).
    pub asks_first: bool,
}

/// A session a retention rule left alone, and why (LD-17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetentionSkip {
    /// The session.
    pub session: SessionId,
    /// Its title.
    pub title: String,
    /// Why it was skipped.
    pub reason: String,
}

/// What Humata keeps on a machine, measured when asked (LD-14).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageReport {
    /// The machine.
    pub machine: MachineId,
    /// Free room on its disk.
    pub free_bytes: u64,
    /// Its disk's size.
    pub total_bytes: u64,
    /// When this was measured, in Unix seconds.
    pub measured_unix_s: i64,
    /// What Humata keeps.
    pub rows: Vec<StorageRow>,
    /// Sessions a retention rule skipped.
    pub retention_skips: Vec<RetentionSkip>,
}

/// Why a file manager call failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FileManagerError {
    /// The machine doesn't answer.
    #[error("{} is offline, so this can't be done there. Wait for it to come back or use another machine", crate::machine_names::name_of(.machine))]
    Offline {
        /// The machine.
        machine: MachineId,
    },
    /// There is no such file or folder.
    #[error("{path} isn't there any more. Reload the folder and try again")]
    NotFound {
        /// What was looked for.
        path: String,
    },
    /// The name is in use.
    #[error("{name} already exists there. Pick another name or move the old one away first")]
    NameTaken {
        /// The name.
        name: String,
    },
    /// It doesn't fit.
    #[error(
        "{name} doesn't have room: this needs {needs_bytes} bytes and only {free_bytes} are free. Free some space on {name}, then try again",
        name = crate::machine_names::name_of(.machine)
    )]
    DiskFull {
        /// The machine that is short.
        machine: MachineId,
        /// What the action needs.
        needs_bytes: u64,
        /// What is free.
        free_bytes: u64,
    },
    /// The action isn't allowed.
    #[error("{what}")]
    NotAllowed {
        /// What isn't allowed and what to do instead.
        what: String,
    },
    /// Anything else went wrong.
    #[error("{what} failed: {}. Try again; if it keeps failing, check the machine's link on the Hosts page", .message.trim_end_matches('.'))]
    Failed {
        /// What was being done.
        what: String,
        /// What the machine said.
        message: String,
    },
}

/// What a file manager call returns.
pub type FileReply<T> = BoxFuture<Result<T, FileManagerError>>;

/// Files on any machine, their transfers and trash.
pub trait FileManager: Send + Sync + 'static {
    /// What `folder` holds.
    fn list(&self, folder: &Folder) -> FileReply<Listing>;
    /// Makes a folder called `name` in `folder`.
    fn create_folder(&self, folder: &Folder, name: &str) -> FileReply<()>;
    /// Renames `old_name` in `folder`.
    fn rename(&self, folder: &Folder, old_name: &str, new_name: &str) -> FileReply<()>;
    /// Moves `names` in `folder` to that machine's trash (FM-3).
    fn delete(&self, folder: &Folder, names: Vec<String>) -> FileReply<()>;
    /// Starts copying or moving `names` from `from` into `into` (FM-2).
    fn start_transfer(
        &self,
        from: &Folder,
        names: Vec<String>,
        into: &Folder,
        mode: TransferMode,
    ) -> FileReply<TransferId>;
    /// Uploads files from this computer, as dropped from Finder (FM-4).
    fn upload(&self, local: Vec<PathBuf>, into: &Folder) -> FileReply<TransferId>;
    /// Like [`FileManager::start_transfer`], but a name taken in `into` is
    /// kept and the arriving one gets a number (`a-2.txt`) instead of the
    /// transfer failing (PL-13).
    fn start_transfer_keep_both(
        &self,
        from: &Folder,
        names: Vec<String>,
        into: &Folder,
        mode: TransferMode,
    ) -> FileReply<TransferId>;
    /// Uploads files from this computer into the session's pastes on
    /// `machine`, each paste in a folder of its own, and returns where each
    /// file is on that machine's disk, in order (D335).
    fn paste_files(
        &self,
        local: Vec<PathBuf>,
        machine: &MachineId,
        session: &SessionId,
    ) -> FileReply<Vec<String>>;
    /// Stops a transfer.
    fn cancel_transfer(&self, transfer: &TransferId) -> FileReply<()>;
    /// Every transfer, newest first.
    fn transfers(&self) -> FileReply<Vec<TransferRow>>;
    /// Every change to a transfer from now on.
    fn transfer_events(&self) -> mpsc::Receiver<TransferRow>;
    /// What is in `machine`'s trash.
    fn trash(&self, machine: &MachineId) -> FileReply<TrashListing>;
    /// Puts a trashed thing back where it was.
    fn restore(&self, machine: &MachineId, id: &str) -> FileReply<()>;
    /// What emptying `machine`'s trash would free.
    fn plan_empty_trash(&self, machine: &MachineId) -> FileReply<EmptyPlan>;
    /// Empties what `plan` said; returns the bytes freed.
    fn empty_trash(&self, plan: EmptyPlan) -> FileReply<u64>;
    /// Measures what Humata keeps on `machine`, now (LD-14).
    fn storage(&self, machine: &MachineId) -> FileReply<StorageReport>;
    /// Clears a session's build cache.
    fn clear_build_cache(&self, machine: &MachineId, session: &SessionId) -> FileReply<()>;
    /// Trims old logs.
    fn trim_logs(&self, machine: &MachineId) -> FileReply<()>;
}
