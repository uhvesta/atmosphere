//! The file manager page (FM-*): two panes that each show any machine's
//! files, a transfers tray, each machine's trash and its Storage page.

mod edit;
mod labels;
mod naming;
mod pane;
mod storage;
mod trash;
mod tray;
mod when;

use std::path::PathBuf;
use std::sync::Arc;

use humata2_clients::model::{MachineId, SessionId, TransferId};
use humata2_clients::{
    Clients, EmptyPlan, FileManagerError, Folder, Listing, StorageReport, StorageRow, TransferRow,
    TrashListing, Workspace,
};
use tca::{Effect, Reducer};

pub use naming::{Naming, NamingKind};
pub use pane::{Pane, Side};
pub use tray::human_bytes;
pub use when::{age_label, days_left, modified_label};

/// The trash's open view (FM-3).
#[derive(Clone, Debug, PartialEq)]
pub struct TrashState {
    /// Whose trash.
    pub machine: MachineId,
    /// What it holds, once loaded.
    pub listing: Option<TrashListing>,
}

/// A machine's Storage page (LD-13).
#[derive(Clone, Debug, PartialEq)]
pub struct StorageState {
    /// Whose storage.
    pub machine: MachineId,
    /// What was measured last, if anything was yet.
    pub report: Option<StorageReport>,
    /// Whether a measurement is running.
    pub measuring: bool,
}

/// A build cache clear waiting for the person to say yes (LD-15).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingClear {
    /// The machine.
    pub machine: MachineId,
    /// The session whose cache it is.
    pub session: SessionId,
    /// The row's name.
    pub what: String,
}

/// The dialog shown where a transfer or copy didn't fit (LD-19).
#[derive(Clone, Debug, PartialEq)]
pub struct LowDisk {
    /// The machine that is short.
    pub machine: MachineId,
    /// What the action needs.
    pub needs_bytes: u64,
    /// What is free now.
    pub free_bytes: u64,
    /// What Humata keeps there, largest first, each with its action.
    pub largest: Vec<StorageRow>,
}

/// What a finished change was, for the message that follows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// A folder was made.
    NewFolder,
    /// A name changed.
    Rename,
    /// Names went to the trash.
    Delete {
        /// How many.
        count: usize,
        /// On which machine.
        machine: MachineId,
    },
    /// A copy, move or upload started.
    Transfer,
    /// A transfer was cancelled.
    Cancel,
    /// A trashed thing was put back.
    Restore,
    /// A Storage action ran on the machine; it is measured again after.
    Storage(MachineId),
}

/// The files page's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilesState {
    /// Where machines and their online state come from.
    pub workspace: Arc<Workspace>,
    /// The left pane.
    pub left: Pane,
    /// The right pane.
    pub right: Pane,
    /// The pane actions apply to.
    pub active: Side,
    /// Whether the page is open.
    pub opened: bool,
    /// Whether the panes were set up (restored or defaulted).
    pub ready: bool,
    /// Every transfer, newest first (FM-7).
    pub transfers: Vec<TransferRow>,
    /// How many things the active pane's machine has in its trash.
    pub trash_count: u32,
    /// The trash, while it is open.
    pub trash: Option<TrashState>,
    /// What emptying a trash would do, until confirmed or cancelled (LD-7).
    pub empty_plan: Option<EmptyPlan>,
    /// How much the last emptying freed.
    pub last_freed: Option<u64>,
    /// A machine's Storage page, while it is open.
    pub storage: Option<StorageState>,
    /// A build cache clear waiting for a yes.
    pub pending_clear: Option<PendingClear>,
    /// The low-disk dialog, while it is open.
    pub low_disk: Option<LowDisk>,
    /// What to do again after the person made room.
    pub retry: Option<FilesAction>,
    /// The name prompt (New folder, Rename), while it is open.
    pub naming: Option<Naming>,
    /// Which pane's machine list is open.
    pub picker: Option<Side>,
    /// What went wrong last, as a message.
    pub error: Option<String>,
}

/// What the files page asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Say this in a toast.
    Toast(String),
    /// Archive this session, with the normal checks (LD-15).
    ArchiveSession(SessionId),
}

/// What happens on the files page.
#[derive(Clone, Debug, PartialEq)]
pub enum FilesAction {
    /// The page opened: restore the panes, load everything and follow transfers.
    Opened,
    /// The workspace changed; machines and whether they are online.
    Show(Arc<Workspace>),
    /// A pane's machine was picked; offline machines are refused (FM-12).
    PickMachine(Side, MachineId),
    /// A pane goes to a folder, as from a machine's page (FM-11).
    Navigate(Side, Folder),
    /// Open the folder called this in the pane.
    OpenFolder(Side, String),
    /// Go to the folder above.
    Up(Side),
    /// Select a name; `extend` adds to or removes from the selection.
    Select(Side, String, bool),
    /// Make this pane the one actions apply to.
    Activate(Side),
    /// A folder was read.
    Listed(Side, Folder, Result<Listing, FileManagerError>),
    /// Open the name prompt for a new folder in the active pane.
    BeginNewFolder,
    /// Open the name prompt to rename the one selected name.
    BeginRename,
    /// The prompt's text was confirmed: check it, then make or rename.
    CommitName(String),
    /// Close the name prompt.
    CancelName,
    /// Open or close a pane's machine list; opening one closes the other.
    TogglePicker(Side),
    /// Close the machine list.
    ClosePicker,
    /// Make a folder in the active pane.
    NewFolder(String),
    /// Rename the one selected name in the active pane.
    Rename {
        /// The new name.
        new_name: String,
    },
    /// Move the active pane's selection to its machine's trash (FM-3).
    Delete,
    /// Copy the active pane's selection into the other pane's folder.
    Copy,
    /// Move the active pane's selection into the other pane's folder.
    Move,
    /// Files were dropped from Finder on a pane (FM-4).
    Dropped(Side, Vec<PathBuf>),
    /// A change finished.
    Finished(Edit, Result<(), FileManagerError>),
    /// A disk-full error's Storage was measured.
    LowDiskReady {
        /// The machine that is short.
        machine: MachineId,
        /// What the action needs.
        needs_bytes: u64,
        /// What was free.
        free_bytes: u64,
        /// What Humata keeps there.
        result: Result<StorageReport, FileManagerError>,
    },
    /// Do the failed action again after making room.
    RetryAfterLowDisk,
    /// Close the low-disk dialog.
    DismissLowDisk,
    /// Transfers were loaded.
    Transfers(Result<Vec<TransferRow>, FileManagerError>),
    /// A transfer changed.
    TransferEvent(TransferRow),
    /// Cancel a transfer (FM-7).
    CancelTransfer(TransferId),
    /// Open a machine's trash.
    OpenTrash(MachineId),
    /// Close the trash.
    CloseTrash,
    /// A trash was read.
    TrashLoaded(MachineId, Result<TrashListing, FileManagerError>),
    /// Put a trashed thing back.
    Restore(String),
    /// Work out what emptying the trash would free.
    PlanEmptyTrash,
    /// The plan is ready.
    EmptyPlanned(Result<EmptyPlan, FileManagerError>),
    /// Empty the trash as planned.
    ConfirmEmptyTrash,
    /// Don't empty the trash.
    CancelEmptyTrash,
    /// The trash was emptied, freeing this many bytes.
    TrashEmptied(MachineId, Result<u64, FileManagerError>),
    /// Open a machine's Storage page and measure it (LD-14).
    OpenStorage(MachineId),
    /// Measure again.
    MeasureAgain,
    /// Close the Storage page.
    CloseStorage,
    /// A machine was measured.
    Measured(MachineId, Result<StorageReport, FileManagerError>),
    /// Do a Storage row's action.
    StorageAct(usize),
    /// Do a low-disk row's action.
    LowDiskAct(usize),
    /// Clear the build cache that asked first.
    ConfirmClear,
    /// Don't clear it.
    CancelClear,
    /// The panes couldn't be saved.
    SaveFailed(String),
    /// Hide the error.
    DismissError,
    /// For the parent.
    Delegate(Delegate),
}

/// The files page's reducer.
#[derive(Clone)]
pub struct Files {
    clients: Clients,
}

impl Files {
    /// A files page on `clients`.
    #[must_use]
    pub const fn new(clients: Clients) -> Self {
        Self { clients }
    }
}

impl Reducer for Files {
    type State = FilesState;
    type Action = FilesAction;

    fn reduce(&self, state: &mut FilesState, action: FilesAction) -> Effect<FilesAction> {
        use FilesAction as A;
        match action {
            A::Opened => self.open(state),
            A::Show(workspace) => self.show(state, workspace),
            A::PickMachine(side, machine) => self.pick(state, side, machine),
            A::Navigate(side, folder) => self.go(state, side, folder),
            A::OpenFolder(side, name) => self.open_folder(state, side, &name),
            A::Up(side) => self.up(state, side),
            A::Select(side, name, extend) => {
                state.select(side, &name, extend);
                Effect::none()
            }
            A::Activate(side) => {
                state.active = side;
                Effect::none()
            }
            A::Listed(side, folder, result) => {
                state.listed(side, &folder, result);
                Effect::none()
            }
            A::BeginNewFolder => {
                state.begin_naming(NamingKind::NewFolder);
                Effect::none()
            }
            A::BeginRename => {
                state.begin_naming(NamingKind::Rename);
                Effect::none()
            }
            A::CommitName(text) => self.commit_name(state, text),
            A::CancelName => {
                state.naming = None;
                Effect::none()
            }
            A::TogglePicker(side) => {
                state.picker = if state.picker == Some(side) { None } else { Some(side) };
                Effect::none()
            }
            A::ClosePicker => {
                state.picker = None;
                Effect::none()
            }
            A::NewFolder(name) => self.new_folder(state, name),
            A::Rename { new_name } => self.rename(state, new_name),
            A::Delete => self.delete(state),
            A::Copy => self.transfer(state, humata2_clients::TransferMode::Copy),
            A::Move => self.transfer(state, humata2_clients::TransferMode::Move),
            A::Dropped(side, paths) => self.dropped(state, side, paths),
            A::Finished(edit, result) => self.finished(state, edit, result),
            A::LowDiskReady {
                machine,
                needs_bytes,
                free_bytes,
                result,
            } => {
                state.low_disk = Some(LowDisk {
                    machine,
                    needs_bytes,
                    free_bytes,
                    largest: result.map(|report| largest_first(report.rows)).unwrap_or_default(),
                });
                Effect::none()
            }
            A::RetryAfterLowDisk => self.retry_after_low_disk(state),
            A::DismissLowDisk => {
                state.low_disk = None;
                state.retry = None;
                Effect::none()
            }
            A::Transfers(result) => {
                match result {
                    Ok(rows) => state.transfers = rows,
                    Err(error) => state.error = Some(error.to_string()),
                }
                Effect::none()
            }
            A::TransferEvent(row) => self.transfer_event(state, row),
            A::CancelTransfer(id) => self.cancel(id),
            A::OpenTrash(machine) => self.open_trash(state, machine),
            A::CloseTrash => {
                state.trash = None;
                state.empty_plan = None;
                Effect::none()
            }
            A::TrashLoaded(machine, result) => {
                state.trash_loaded(&machine, result);
                Effect::none()
            }
            A::Restore(id) => self.restore(state, &id),
            A::PlanEmptyTrash => self.plan_empty_for_view(state),
            A::EmptyPlanned(result) => {
                match result {
                    Ok(plan) => state.empty_plan = Some(plan),
                    Err(error) => state.error = Some(error.to_string()),
                }
                Effect::none()
            }
            A::ConfirmEmptyTrash => self.confirm_empty(state),
            A::CancelEmptyTrash => {
                state.empty_plan = None;
                Effect::none()
            }
            A::TrashEmptied(machine, result) => self.trash_emptied(state, &machine, result),
            A::OpenStorage(machine) => self.open_storage(state, machine),
            A::MeasureAgain => self.measure_again(state),
            A::CloseStorage => {
                state.storage = None;
                state.pending_clear = None;
                Effect::none()
            }
            A::Measured(machine, result) => {
                state.measured(&machine, result);
                Effect::none()
            }
            A::StorageAct(index) => self.storage_act(state, index),
            A::LowDiskAct(index) => self.low_disk_act(state, index),
            A::ConfirmClear => self.confirm_clear(state),
            A::CancelClear => {
                state.pending_clear = None;
                Effect::none()
            }
            A::SaveFailed(message) => {
                state.error = Some(message);
                Effect::none()
            }
            A::DismissError => {
                state.error = None;
                Effect::none()
            }
            A::Delegate(_) => Effect::none(),
        }
    }
}

/// Rows with the largest first (LD-19).
pub(crate) fn largest_first(mut rows: Vec<StorageRow>) -> Vec<StorageRow> {
    rows.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    rows
}
