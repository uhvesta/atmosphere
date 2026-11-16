//! The file manager client (FM-*, LD-13..17): files and folders on any machine,
//! transfers between them, each machine's trash, and Storage.
//! Declared from lib.rs as `mod file_manager;`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use std::collections::BTreeSet;
use std::sync::PoisonError;

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
        };
        join(&base, &self.path)
    }
}

fn join(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_owned()
    } else if name.is_empty() {
        base.to_owned()
    } else {
        format!("{base}/{name}")
    }
}

fn parent_of(path: &str) -> &str {
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
    #[error("{machine} is offline, so this can't be done there. Wait for it to come back or use another machine")]
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
        "{machine} doesn't have room: this needs {needs_bytes} bytes and only {free_bytes} are free. Free some space on {machine}, then try again"
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

#[derive(Clone, Debug)]
struct Node {
    kind: EntryKind,
    size: u64,
    modified: i64,
}

#[derive(Clone, Debug)]
struct TrashItem {
    id: String,
    description: String,
    bytes: u64,
    deleted: i64,
    nodes: Vec<(FileRoot, String, Node)>,
}

#[derive(Debug)]
struct MachineData {
    online: bool,
    free_bytes: u64,
    total_bytes: u64,
    files: BTreeMap<(FileRoot, String), Node>,
    trash: Vec<TrashItem>,
    rows: Vec<StorageRow>,
    skips: Vec<RetentionSkip>,
}

impl Default for MachineData {
    fn default() -> Self {
        Self {
            online: true,
            free_bytes: 500_000_000_000,
            total_bytes: 1_000_000_000_000,
            files: BTreeMap::new(),
            trash: Vec::new(),
            rows: Vec::new(),
            skips: Vec::new(),
        }
    }
}

#[derive(Debug)]
struct Transfer {
    row: TransferRow,
    from: Folder,
    into: Folder,
    mode: TransferMode,
    names: Vec<String>,
    uploads: Vec<(String, u64)>,
}

struct ManagerInner {
    host: MachineId,
    now: i64,
    utc_offset_s: i32,
    machines: BTreeMap<MachineId, MachineData>,
    transfers: Vec<Transfer>,
    subscribers: Vec<mpsc::Sender<TransferRow>>,
    next: u32,
    failure: Option<FileManagerError>,
}

const TRASH_DAYS_S: i64 = 30 * 24 * 3600;

impl ManagerInner {
    fn data(&mut self, machine: &MachineId) -> &mut MachineData {
        self.machines.entry(machine.clone()).or_default()
    }

    /// The machine's data if it answers.
    fn online(&mut self, machine: &MachineId) -> Result<&mut MachineData, FileManagerError> {
        let data = self.data(machine);
        if data.online {
            Ok(data)
        } else {
            Err(FileManagerError::Offline {
                machine: machine.clone(),
            })
        }
    }

    fn guard(&mut self, machine: &MachineId) -> Result<&mut MachineData, FileManagerError> {
        if let Some(failure) = self.failure.take() {
            return Err(failure);
        }
        self.online(machine)
    }

    fn emit(&mut self, index: usize) {
        let Some(transfer) = self.transfers.get(index) else {
            return;
        };
        let row = transfer.row.clone();
        self.subscribers.retain(|subscriber| match subscriber.try_send(row.clone()) {
            Err(mpsc::error::TrySendError::Closed(_)) => false,
            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => true,
        });
    }

    fn position(&self, id: &TransferId) -> Option<usize> {
        self.transfers.iter().position(|transfer| &transfer.row.id == id)
    }

    #[allow(clippy::too_many_arguments)]
    fn new_transfer(
        &mut self,
        from: &Folder,
        into: &Folder,
        mode: TransferMode,
        names: Vec<String>,
        uploads: Vec<(String, u64)>,
        total: u64,
        what: String,
    ) -> TransferId {
        self.next += 1;
        let id = TransferId::new(format!("fm-transfer-{}", self.next));
        let via_host = from.machine != into.machine && from.machine != self.host && into.machine != self.host;
        self.transfers.push(Transfer {
            row: TransferRow {
                id: id.clone(),
                what,
                from: from.machine.clone(),
                to: into.machine.clone(),
                via_host,
                done_bytes: 0,
                total_bytes: total,
                phase: TransferPhase::Copying,
            },
            from: from.clone(),
            into: into.clone(),
            mode,
            names,
            uploads,
        });
        let index = self.transfers.len() - 1;
        self.emit(index);
        id
    }

    fn check_room(&mut self, machine: &MachineId, needs: u64) -> Result<(), FileManagerError> {
        let free = self.data(machine).free_bytes;
        if needs > free {
            return Err(FileManagerError::DiskFull {
                machine: machine.clone(),
                needs_bytes: needs,
                free_bytes: free,
            });
        }
        Ok(())
    }

    fn finish(&mut self, index: usize) {
        let Some(transfer) = self.transfers.get(index) else {
            return;
        };
        if matches!(transfer.row.phase, TransferPhase::Paused { .. }) || transfer.row.phase.is_over() {
            return;
        }
        let (from, into, mode, names, uploads, total) = (
            transfer.from.clone(),
            transfer.into.clone(),
            transfer.mode,
            transfer.names.clone(),
            transfer.uploads.clone(),
            transfer.row.total_bytes,
        );
        let now = self.now;
        for (name, size) in uploads {
            self.data(&into.machine).files.insert(
                (into.root.clone(), join(&into.path, &name)),
                Node {
                    kind: EntryKind::File,
                    size,
                    modified: now,
                },
            );
        }
        for name in &names {
            let source = from.child(name);
            let nodes = subtree(self.data(&from.machine), &source.root, &source.path);
            let target = into.child(name);
            let destination = self.data(&into.machine);
            for (path, node) in nodes {
                let relative = path.strip_prefix(&source.path).unwrap_or("");
                destination
                    .files
                    .insert((target.root.clone(), format!("{}{relative}", target.path)), node);
            }
            if mode == TransferMode::Move {
                self.trash_names(&from, std::slice::from_ref(name));
            }
        }
        let data = self.data(&into.machine);
        data.free_bytes = data.free_bytes.saturating_sub(total);
        if let Some(transfer) = self.transfers.get_mut(index) {
            transfer.row.done_bytes = transfer.row.total_bytes;
            transfer.row.phase = TransferPhase::Done;
        }
        self.emit(index);
    }

    /// Puts `names` in `folder` into the machine's trash.
    fn trash_names(&mut self, folder: &Folder, names: &[String]) {
        let now = self.now;
        self.next += 1;
        let mut next = self.next;
        let data = self.data(&folder.machine);
        for name in names {
            let child = folder.child(name);
            let nodes = subtree(data, &child.root, &child.path);
            let bytes = nodes.iter().map(|(_, node)| node.size).sum();
            let is_folder = nodes.first().is_some_and(|(_, node)| node.kind == EntryKind::Folder);
            for (path, _) in &nodes {
                data.files.remove(&(child.root.clone(), path.clone()));
            }
            next += 1;
            data.trash.push(TrashItem {
                id: format!("trash-{next}"),
                description: if is_folder { format!("{name}/") } else { name.clone() },
                bytes,
                deleted: now,
                nodes: nodes
                    .into_iter()
                    .map(|(path, node)| (child.root.clone(), path, node))
                    .collect(),
            });
        }
        self.next = next;
    }
}

/// The node at `path` and everything under it, shallowest first.
fn subtree(data: &MachineData, root: &FileRoot, path: &str) -> Vec<(String, Node)> {
    let prefix = format!("{path}/");
    let mut nodes: Vec<(String, Node)> = data
        .files
        .iter()
        .filter(|((r, p), _)| r == root && (p == path || p.starts_with(&prefix)))
        .map(|((_, p), node)| (p.clone(), node.clone()))
        .collect();
    nodes.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)));
    nodes
}

fn valid_name(name: &str) -> Result<(), FileManagerError> {
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(FileManagerError::NotAllowed {
            what: format!("{name:?} can't be a name. Use a name without / that isn't empty"),
        });
    }
    Ok(())
}

fn ready<T: Send + 'static>(value: Result<T, FileManagerError>) -> FileReply<T> {
    Box::pin(async move { value })
}

/// A file manager in memory, for tests and the demo: a tree per machine,
/// transfers that tests step by hand, a trash and a Storage report each.
#[derive(Clone)]
pub struct FakeFileManager {
    shared: Arc<Mutex<ManagerInner>>,
}

impl Default for FakeFileManager {
    fn default() -> Self {
        Self::new(MachineId::new("this-mac"))
    }
}

impl FakeFileManager {
    /// A fake whose host (FM-9) is `host`; every machine starts online,
    /// empty, with 500 GB free of 1 TB.
    #[must_use]
    pub fn new(host: MachineId) -> Self {
        Self {
            shared: Arc::new(Mutex::new(ManagerInner {
                host,
                now: 1_700_000_000,
                utc_offset_s: 0,
                machines: BTreeMap::new(),
                transfers: Vec::new(),
                subscribers: Vec::new(),
                next: 0,
                failure: None,
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ManagerInner> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sets what the fake thinks the time is, in Unix seconds.
    pub fn set_now(&self, unix_s: i64) {
        self.lock().now = unix_s;
    }

    /// Sets the viewer's local offset from UTC the fake reports, in seconds.
    pub fn set_utc_offset(&self, seconds: i32) {
        self.lock().utc_offset_s = seconds;
    }

    /// Makes `folder`, and the folders above it, exist.
    pub fn put_folder(&self, folder: &Folder) {
        let mut inner = self.lock();
        let now = inner.now;
        let data = inner.data(&folder.machine);
        let mut path = String::new();
        for name in folder.path.split('/').filter(|name| !name.is_empty()) {
            path = join(&path, name);
            data.files.entry((folder.root.clone(), path.clone())).or_insert(Node {
                kind: EntryKind::Folder,
                size: 0,
                modified: now,
            });
        }
    }

    /// Puts a file of `size` bytes called `name` in `folder`.
    pub fn put_file(&self, folder: &Folder, name: &str, size: u64) {
        self.put_folder(folder);
        let mut inner = self.lock();
        let now = inner.now;
        inner.data(&folder.machine).files.insert(
            (folder.root.clone(), join(&folder.path, name)),
            Node {
                kind: EntryKind::File,
                size,
                modified: now,
            },
        );
    }

    /// Fills `folder` with entries: a size is a file, `None` a folder.
    #[must_use]
    pub fn with_tree(self, folder: &Folder, entries: &[(&str, Option<u64>)]) -> Self {
        for (name, size) in entries {
            match size {
                Some(size) => self.put_file(folder, name, *size),
                None => self.put_folder(&folder.child(name)),
            }
        }
        self
    }

    /// Sets what a machine's Storage page shows besides free room.
    pub fn set_storage(&self, machine: &MachineId, rows: Vec<StorageRow>, skips: Vec<RetentionSkip>) {
        let mut inner = self.lock();
        let data = inner.data(machine);
        data.rows = rows;
        data.skips = skips;
    }

    /// Takes a machine offline or brings it back. Transfers with an end
    /// there pause, and continue once it returns (FM-13).
    pub fn set_online(&self, machine: &MachineId, online: bool) {
        let mut inner = self.lock();
        inner.data(machine).online = online;
        for index in 0..inner.transfers.len() {
            let transfer = &mut inner.transfers[index];
            let ends = [transfer.row.from.clone(), transfer.row.to.clone()];
            if !ends.contains(machine) {
                continue;
            }
            if !online && matches!(transfer.row.phase, TransferPhase::Copying | TransferPhase::Verifying) {
                transfer.row.phase = TransferPhase::Paused {
                    waiting_for: Some(machine.clone()),
                    reason: format!("{machine} went offline"),
                };
            } else if online
                && matches!(&transfer.row.phase, TransferPhase::Paused { waiting_for: Some(waiting), .. } if waiting == machine)
            {
                transfer.row.phase = TransferPhase::Copying;
            } else {
                continue;
            }
            inner.emit(index);
        }
    }

    /// Sets a machine's free room.
    pub fn set_free_bytes(&self, machine: &MachineId, bytes: u64) {
        self.lock().data(machine).free_bytes = bytes;
    }

    /// Moves a running transfer on by `bytes`; at its total it starts
    /// verifying. A paused or finished one doesn't move.
    pub fn advance(&self, transfer: &TransferId, bytes: u64) {
        let mut inner = self.lock();
        let Some(index) = inner.position(transfer) else {
            return;
        };
        let row = &mut inner.transfers[index].row;
        if row.phase != TransferPhase::Copying {
            return;
        }
        row.done_bytes = (row.done_bytes + bytes).min(row.total_bytes);
        if row.done_bytes == row.total_bytes {
            row.phase = TransferPhase::Verifying;
        }
        inner.emit(index);
    }

    /// Completes a transfer: a copy's names appear in the destination, a
    /// move's sources also go to their machine's trash (FM-14).
    pub fn finish(&self, transfer: &TransferId) {
        let mut inner = self.lock();
        if let Some(index) = inner.position(transfer) {
            inner.finish(index);
        }
    }

    /// Makes the next call, of any kind, fail with `error`.
    pub fn fail_next(&self, error: FileManagerError) {
        self.lock().failure = Some(error);
    }

    /// The names in `folder`, as listed.
    #[must_use]
    pub fn names_in(&self, folder: &Folder) -> Vec<String> {
        let mut inner = self.lock();
        let data = inner.data(&folder.machine);
        listing_of(data, folder).into_iter().map(|entry| entry.name).collect()
    }

    /// What is in `machine`'s trash, by description.
    #[must_use]
    pub fn trash_names(&self, machine: &MachineId) -> Vec<String> {
        let mut inner = self.lock();
        inner.data(machine).trash.iter().map(|item| item.description.clone()).collect()
    }

    /// A machine's free room.
    #[must_use]
    pub fn free_bytes(&self, machine: &MachineId) -> u64 {
        self.lock().data(machine).free_bytes
    }
}

fn listing_of(data: &MachineData, folder: &Folder) -> Vec<Entry> {
    let mut entries: Vec<Entry> = data
        .files
        .iter()
        .filter(|((root, path), _)| root == &folder.root && parent_of(path) == folder.path && !path.is_empty())
        .map(|((_, path), node)| Entry {
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            kind: node.kind,
            size: (node.kind != EntryKind::Folder).then_some(node.size),
            modified_unix_s: node.modified,
        })
        .collect();
    entries.sort_by(|a, b| {
        (a.kind != EntryKind::Folder)
            .cmp(&(b.kind != EntryKind::Folder))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

fn folder_exists(data: &MachineData, folder: &Folder) -> bool {
    folder.path.is_empty()
        || data
            .files
            .get(&(folder.root.clone(), folder.path.clone()))
            .is_some_and(|node| node.kind == EntryKind::Folder)
}

fn not_found(folder: &Folder) -> FileManagerError {
    FileManagerError::NotFound {
        path: folder.display_path(),
    }
}

impl FileManager for FakeFileManager {
    fn list(&self, folder: &Folder) -> FileReply<Listing> {
        let mut inner = self.lock();
        let (read_unix_s, utc_offset_s) = (inner.now, inner.utc_offset_s);
        ready(inner.guard(&folder.machine).and_then(|data| {
            if folder_exists(data, folder) {
                Ok(Listing {
                    folder: folder.clone(),
                    entries: listing_of(data, folder),
                    read_unix_s,
                    utc_offset_s,
                })
            } else {
                Err(not_found(folder))
            }
        }))
    }

    fn create_folder(&self, folder: &Folder, name: &str) -> FileReply<()> {
        let mut inner = self.lock();
        let now = inner.now;
        let result = (|| {
            valid_name(name)?;
            let data = inner.guard(&folder.machine)?;
            let child = folder.child(name);
            if data.files.contains_key(&(child.root.clone(), child.path.clone())) {
                return Err(FileManagerError::NameTaken { name: name.to_owned() });
            }
            data.files.insert(
                (child.root, child.path),
                Node {
                    kind: EntryKind::Folder,
                    size: 0,
                    modified: now,
                },
            );
            Ok(())
        })();
        ready(result)
    }

    fn rename(&self, folder: &Folder, old_name: &str, new_name: &str) -> FileReply<()> {
        let mut inner = self.lock();
        let result = (|| {
            valid_name(new_name)?;
            let data = inner.guard(&folder.machine)?;
            let old = folder.child(old_name);
            let new = folder.child(new_name);
            if data.files.contains_key(&(new.root.clone(), new.path.clone())) {
                return Err(FileManagerError::NameTaken {
                    name: new_name.to_owned(),
                });
            }
            let nodes = subtree(data, &old.root, &old.path);
            if nodes.is_empty() {
                return Err(not_found(&old));
            }
            for (path, node) in nodes {
                data.files.remove(&(old.root.clone(), path.clone()));
                let relative = path.strip_prefix(&old.path).unwrap_or("");
                data.files.insert((new.root.clone(), format!("{}{relative}", new.path)), node);
            }
            Ok(())
        })();
        ready(result)
    }

    fn delete(&self, folder: &Folder, names: Vec<String>) -> FileReply<()> {
        let mut inner = self.lock();
        let result = (|| {
            let data = inner.guard(&folder.machine)?;
            for name in &names {
                let child = folder.child(name);
                if subtree(data, &child.root, &child.path).is_empty() {
                    return Err(not_found(&child));
                }
            }
            inner.trash_names(folder, &names);
            Ok(())
        })();
        ready(result)
    }

    fn start_transfer(
        &self,
        from: &Folder,
        names: Vec<String>,
        into: &Folder,
        mode: TransferMode,
    ) -> FileReply<TransferId> {
        let mut inner = self.lock();
        let result = (|| {
            inner.guard(&from.machine)?;
            inner.online(&into.machine)?;
            let mut total = 0;
            for name in &names {
                let child = from.child(name);
                let nodes = subtree(inner.data(&from.machine), &child.root, &child.path);
                if nodes.is_empty() {
                    return Err(not_found(&child));
                }
                total += nodes.iter().map(|(_, node)| node.size).sum::<u64>();
                let target = into.child(name);
                if inner.data(&into.machine).files.contains_key(&(target.root, target.path)) {
                    return Err(FileManagerError::NameTaken { name: name.clone() });
                }
            }
            inner.check_room(&into.machine, total)?;
            let what = names.join(", ");
            Ok(inner.new_transfer(from, into, mode, names, Vec::new(), total, what))
        })();
        ready(result)
    }

    fn upload(&self, local: Vec<PathBuf>, into: &Folder) -> FileReply<TransferId> {
        let mut inner = self.lock();
        let result = (|| {
            inner.guard(&into.machine)?;
            let uploads: Vec<(String, u64)> = local
                .iter()
                .filter_map(|path| {
                    let name = path.file_name()?.to_string_lossy().into_owned();
                    let size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
                    Some((name, size))
                })
                .collect();
            for (name, _) in &uploads {
                let target = into.child(name);
                if inner.data(&into.machine).files.contains_key(&(target.root, target.path)) {
                    return Err(FileManagerError::NameTaken { name: name.clone() });
                }
            }
            let total = uploads.iter().map(|(_, size)| size).sum();
            inner.check_room(&into.machine, total)?;
            let what = uploads.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(", ");
            let from = Folder::home(inner.host.clone());
            Ok(inner.new_transfer(&from, into, TransferMode::Copy, Vec::new(), uploads, total, what))
        })();
        ready(result)
    }

    fn cancel_transfer(&self, transfer: &TransferId) -> FileReply<()> {
        let mut inner = self.lock();
        let result = (|| {
            if let Some(failure) = inner.failure.take() {
                return Err(failure);
            }
            let Some(index) = inner.position(transfer) else {
                return Err(FileManagerError::NotFound {
                    path: transfer.to_string(),
                });
            };
            if !inner.transfers[index].row.phase.is_over() {
                inner.transfers[index].row.phase = TransferPhase::Cancelled;
                inner.emit(index);
            }
            Ok(())
        })();
        ready(result)
    }

    fn transfers(&self) -> FileReply<Vec<TransferRow>> {
        let mut inner = self.lock();
        let result = match inner.failure.take() {
            Some(failure) => Err(failure),
            None => Ok(inner.transfers.iter().rev().map(|transfer| transfer.row.clone()).collect()),
        };
        ready(result)
    }

    fn transfer_events(&self) -> mpsc::Receiver<TransferRow> {
        let (sender, receiver) = mpsc::channel(crate::EVENT_QUEUE);
        self.lock().subscribers.push(sender);
        receiver
    }

    fn trash(&self, machine: &MachineId) -> FileReply<TrashListing> {
        let mut inner = self.lock();
        let now = inner.now;
        let result = inner.guard(machine).map(|data| trash_listing(machine, data, now));
        ready(result)
    }

    fn restore(&self, machine: &MachineId, id: &str) -> FileReply<()> {
        let mut inner = self.lock();
        let result = (|| {
            let data = inner.guard(machine)?;
            let Some(position) = data.trash.iter().position(|item| item.id == id) else {
                return Err(FileManagerError::NotFound { path: id.to_owned() });
            };
            let item = &data.trash[position];
            if let Some((root, path, _)) = item.nodes.first() {
                if data.files.contains_key(&(root.clone(), path.clone())) {
                    return Err(FileManagerError::NameTaken {
                        name: path.rsplit('/').next().unwrap_or(path).to_owned(),
                    });
                }
            }
            let item = data.trash.remove(position);
            for (root, path, node) in item.nodes {
                data.files.insert((root, path), node);
            }
            Ok(())
        })();
        ready(result)
    }

    fn plan_empty_trash(&self, machine: &MachineId) -> FileReply<EmptyPlan> {
        let mut inner = self.lock();
        let result = inner.guard(machine).map(|data| EmptyPlan {
            machine: machine.clone(),
            items: u32::try_from(data.trash.len()).unwrap_or(u32::MAX),
            frees_bytes: data.trash.iter().map(|item| item.bytes).sum(),
            token: data.trash.iter().map(|item| item.id.as_str()).collect::<Vec<_>>().join(","),
        });
        ready(result)
    }

    fn empty_trash(&self, plan: EmptyPlan) -> FileReply<u64> {
        let mut inner = self.lock();
        let result = inner.guard(&plan.machine).map(|data| {
            let ids: BTreeSet<&str> = plan.token.split(',').filter(|id| !id.is_empty()).collect();
            let mut freed = 0;
            data.trash.retain(|item| {
                if ids.contains(item.id.as_str()) {
                    freed += item.bytes;
                    false
                } else {
                    true
                }
            });
            data.free_bytes += freed;
            freed
        });
        ready(result)
    }

    fn storage(&self, machine: &MachineId) -> FileReply<StorageReport> {
        let mut inner = self.lock();
        let now = inner.now;
        let result = inner.guard(machine).map(|data| {
            let trash = trash_listing(machine, data, now);
            let mut rows = data.rows.clone();
            for row in rows.iter_mut().filter(|row| row.kind == StorageKind::Trash) {
                row.bytes = trash.total_bytes;
                row.note = Some(format!("{} items", trash.items.len()));
            }
            StorageReport {
                machine: machine.clone(),
                free_bytes: data.free_bytes,
                total_bytes: data.total_bytes,
                measured_unix_s: now,
                rows,
                retention_skips: data.skips.clone(),
            }
        });
        ready(result)
    }

    fn clear_build_cache(&self, machine: &MachineId, session: &SessionId) -> FileReply<()> {
        let mut inner = self.lock();
        let result = (|| {
            let data = inner.guard(machine)?;
            let Some(row) = data
                .rows
                .iter_mut()
                .find(|row| row.action == StorageAction::ClearBuildCache(session.clone()))
            else {
                return Err(FileManagerError::NotFound {
                    path: format!("the build cache of {session}"),
                });
            };
            data.free_bytes += std::mem::take(&mut row.bytes);
            Ok(())
        })();
        ready(result)
    }

    fn trim_logs(&self, machine: &MachineId) -> FileReply<()> {
        let mut inner = self.lock();
        let result = inner.guard(machine).map(|data| {
            for row in data.rows.iter_mut().filter(|row| row.kind == StorageKind::Logs) {
                data.free_bytes += std::mem::take(&mut row.bytes);
            }
        });
        ready(result)
    }
}

fn trash_listing(machine: &MachineId, data: &MachineData, read_unix_s: i64) -> TrashListing {
    let items: Vec<TrashEntry> = data
        .trash
        .iter()
        .rev()
        .map(|item| TrashEntry {
            id: item.id.clone(),
            description: item.description.clone(),
            bytes: item.bytes,
            deleted_unix_s: item.deleted,
            empties_unix_s: item.deleted + TRASH_DAYS_S,
        })
        .collect();
    TrashListing {
        machine: machine.clone(),
        total_bytes: items.iter().map(|item| item.bytes).sum(),
        items,
        read_unix_s,
    }
}
