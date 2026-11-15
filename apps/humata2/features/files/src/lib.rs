//! Files across hosts (CH-18, ST-*): a filter over every host's files,
//! copies between hosts with Retry for a failed one, and Trash.

use std::sync::Arc;

use humata2_clients::model::{FileEntry, MachineId, TransferId};
use humata2_clients::{Clients, DaemonError, Workspace};
use tca::{Effect, Reducer};

/// The files page's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilesState {
    /// Where files come from.
    pub workspace: Arc<Workspace>,
    /// What the filter box holds.
    pub filter: String,
    /// One host's files only.
    pub machine: Option<MachineId>,
    /// What went wrong last.
    pub error: Option<String>,
}

impl FilesState {
    /// The files that match.
    #[must_use]
    pub fn shown(&self) -> Vec<&FileEntry> {
        let filter = self.filter.to_lowercase();
        self.workspace
            .files
            .iter()
            .filter(|file| self.machine.as_ref().is_none_or(|machine| &file.machine == machine))
            .filter(|file| file.path.to_lowercase().contains(&filter))
            .collect()
    }
}

/// What the files page asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Say this in a toast.
    Toast(String),
}

/// What happens on the files page.
#[derive(Clone, Debug, PartialEq)]
pub enum FilesAction {
    /// The data changed.
    Show(Arc<Workspace>),
    /// The filter changed.
    Filter(String),
    /// Show one host's files, or every host's.
    SetMachine(Option<MachineId>),
    /// Retry a failed copy.
    Retry(TransferId),
    /// Copy a path to the clipboard.
    CopyPath(String),
    /// A request finished.
    Done(Result<(), DaemonError>),
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
        match action {
            FilesAction::Show(workspace) => state.workspace = workspace,
            FilesAction::Filter(filter) => state.filter = filter,
            FilesAction::SetMachine(machine) => state.machine = machine,
            FilesAction::Retry(id) => {
                let daemon = self.clients.daemon.clone();
                return Effect::future(async move { FilesAction::Done(daemon.retry_transfer(&id).await) });
            }
            FilesAction::CopyPath(path) => {
                self.clients.clipboard.copy(&path);
                return Effect::send(FilesAction::Delegate(Delegate::Toast(format!("Copied {path}"))));
            }
            FilesAction::Done(result) => match result {
                Ok(()) => state.error = None,
                Err(error) => state.error = Some(error.to_string()),
            },
            FilesAction::Delegate(_) => {}
        }
        Effect::none()
    }
}
