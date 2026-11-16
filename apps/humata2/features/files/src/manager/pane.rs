//! Panes: which machine and folder each shows, its listing, selection and
//! whether it is stale (FM-1, FM-12), and saving them (FM-15).

use std::sync::Arc;

use humata2_clients::model::MachineId;
use humata2_clients::{Entry, EntryKind, FileManagerError, Folder, Listing, Workspace};
use serde::{Deserialize, Serialize};
use tca::Effect;

use super::{Files, FilesAction, FilesState};

const KEY: &str = "files-panes";

/// Which pane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    /// The left pane.
    #[default]
    Left,
    /// The right pane.
    Right,
}

impl Side {
    /// The other pane.
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// One pane of the file manager (FM-1).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pane {
    /// The machine it shows.
    pub machine: Option<MachineId>,
    /// The folder it shows, on that machine.
    pub folder: Option<Folder>,
    /// What the folder held when last read; kept while the machine is offline.
    pub entries: Vec<Entry>,
    /// Whether a listing was read at least once.
    pub loaded: bool,
    /// Whether a read is running.
    pub loading: bool,
    /// Why the last read failed.
    pub error: Option<String>,
    /// The selected names.
    pub selected: Vec<String>,
    /// Whether its machine is offline: the listing is the last one read and
    /// every action on the pane is refused (FM-12).
    pub stale: bool,
    /// When the listing was read, in Unix seconds (from the client).
    pub read_unix_s: i64,
    /// The viewer's offset from UTC when it was read, in seconds.
    pub utc_offset_s: i32,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    left: Option<Folder>,
    right: Option<Folder>,
    active: Side,
}

impl FilesState {
    /// A pane.
    #[must_use]
    pub const fn pane(&self, side: Side) -> &Pane {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    pub(super) const fn pane_mut(&mut self, side: Side) -> &mut Pane {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }

    /// The pane actions apply to.
    #[must_use]
    pub const fn active_pane(&self) -> &Pane {
        self.pane(self.active)
    }

    /// Whether `machine` is known and answers.
    #[must_use]
    pub fn is_online(&self, machine: &MachineId) -> bool {
        self.workspace
            .machines
            .iter()
            .any(|known| &known.id == machine && known.online)
    }

    /// The name people see for `machine`.
    #[must_use]
    pub fn machine_name(&self, machine: &MachineId) -> String {
        self.workspace
            .machines
            .iter()
            .find(|known| &known.id == machine)
            .map_or_else(|| machine.to_string(), |known| known.name.clone())
    }

    fn host(&self) -> Option<MachineId> {
        let machines = &self.workspace.machines;
        machines
            .iter()
            .find(|machine| matches!(machine.kind, humata2_clients::model::MachineKind::ThisMac))
            .or_else(|| machines.first())
            .map(|machine| machine.id.clone())
    }

    /// The folder and side of the active pane, if actions on it are allowed.
    pub(super) fn usable(&self, side: Side) -> Option<Folder> {
        let pane = self.pane(side);
        if pane.stale {
            return None;
        }
        pane.folder.clone()
    }

    pub(super) fn select(&mut self, side: Side, name: &str, extend: bool) {
        let pane = self.pane_mut(side);
        if pane.stale || !pane.entries.iter().any(|entry| entry.name == name) {
            return;
        }
        self.active = side;
        let pane = self.pane_mut(side);
        if extend {
            if let Some(at) = pane.selected.iter().position(|selected| selected == name) {
                pane.selected.remove(at);
            } else {
                pane.selected.push(name.to_owned());
            }
        } else {
            pane.selected = vec![name.to_owned()];
        }
    }

    pub(super) fn listed(&mut self, side: Side, folder: &Folder, result: Result<Listing, FileManagerError>) {
        let pane = self.pane_mut(side);
        if pane.folder.as_ref() != Some(folder) {
            return;
        }
        pane.loading = false;
        match result {
            Ok(listing) => {
                pane.entries = listing.entries;
                pane.read_unix_s = listing.read_unix_s;
                pane.utc_offset_s = listing.utc_offset_s;
                pane.loaded = true;
                pane.error = None;
                let names: Vec<&str> = pane.entries.iter().map(|entry| entry.name.as_str()).collect();
                pane.selected.retain(|name| names.contains(&name.as_str()));
            }
            Err(error) => pane.error = Some(error.to_string()),
        }
    }
}

impl Files {
    pub(super) fn open(&self, state: &mut FilesState) -> Effect<FilesAction> {
        state.opened = true;
        if !state.ready && !state.workspace.machines.is_empty() {
            self.restore_panes(state);
        }
        if state.ready {
            self.load_all(state)
        } else {
            Effect::none()
        }
    }

    pub(super) fn show(&self, state: &mut FilesState, workspace: Arc<Workspace>) -> Effect<FilesAction> {
        state.workspace = workspace;
        if !state.ready {
            if state.workspace.machines.is_empty() {
                return Effect::none();
            }
            self.restore_panes(state);
            return if state.opened { self.load_all(state) } else { Effect::none() };
        }
        let mut effects = Vec::new();
        for side in [Side::Left, Side::Right] {
            let Some(machine) = state.pane(side).machine.clone() else {
                continue;
            };
            let online = state.is_online(&machine);
            let pane = state.pane_mut(side);
            if !online && !pane.stale {
                pane.stale = true;
                pane.loading = false;
            } else if online && pane.stale {
                pane.stale = false;
                if state.opened {
                    effects.push(self.load_pane(state, side));
                }
            }
        }
        if state.opened {
            effects.push(self.load_trash_count(state));
        }
        Effect::merge(effects)
    }

    fn restore_panes(&self, state: &mut FilesState) {
        let saved = self.saved();
        let host = state.host();
        for side in [Side::Left, Side::Right] {
            let remembered = saved.as_ref().and_then(|saved| match side {
                Side::Left => saved.left.clone(),
                Side::Right => saved.right.clone(),
            });
            let folder = remembered
                .filter(|folder| state.workspace.machines.iter().any(|m| m.id == folder.machine))
                .or_else(|| host.clone().map(Folder::home));
            let stale = folder.as_ref().is_some_and(|folder| !state.is_online(&folder.machine));
            let pane = state.pane_mut(side);
            pane.machine = folder.as_ref().map(|folder| folder.machine.clone());
            pane.folder = folder;
            pane.stale = stale;
        }
        if let Some(saved) = saved {
            state.active = saved.active;
        }
        state.ready = true;
    }

    fn saved(&self) -> Option<Saved> {
        let bytes = self.clients.storage.load(KEY).ok().flatten()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub(super) fn persist(&self, state: &FilesState) -> Effect<FilesAction> {
        let saved = Saved {
            left: state.left.folder.clone(),
            right: state.right.folder.clone(),
            active: state.active,
        };
        let storage = self.clients.storage.clone();
        Effect::run(move |send| async move {
            let result = serde_json::to_vec(&saved)
                .map_err(|error| format!("couldn't write the file manager's panes as JSON: {error}. This is a bug; please report it."))
                .and_then(|bytes| {
                    storage.save(KEY, &bytes).map_err(|error| {
                        format!("couldn't remember where the file manager's panes were: {error}")
                    })
                });
            if let Err(message) = result {
                send.send(FilesAction::SaveFailed(message));
            }
        })
    }

    pub(super) fn load_all(&self, state: &mut FilesState) -> Effect<FilesAction> {
        let left = self.load_pane(state, Side::Left);
        let right = self.load_pane(state, Side::Right);
        Effect::merge([
            left,
            right,
            self.load_transfers(),
            self.load_trash_count(state),
            self.follow_transfers(),
        ])
    }

    pub(super) fn load_pane(&self, state: &mut FilesState, side: Side) -> Effect<FilesAction> {
        let pane = state.pane_mut(side);
        let Some(folder) = pane.folder.clone() else {
            return Effect::none();
        };
        if pane.stale {
            return Effect::none();
        }
        pane.loading = true;
        let manager = self.clients.file_manager.clone();
        Effect::future(async move {
            let result = manager.list(&folder).await;
            FilesAction::Listed(side, folder, result)
        })
    }

    pub(super) fn pick(&self, state: &mut FilesState, side: Side, machine: MachineId) -> Effect<FilesAction> {
        state.picker = None;
        self.go(state, side, Folder::home(machine))
    }

    /// Takes `side` to `folder` unless its machine is offline.
    pub(super) fn go(&self, state: &mut FilesState, side: Side, folder: Folder) -> Effect<FilesAction> {
        if !state.is_online(&folder.machine) {
            return Effect::none();
        }
        state.active = side;
        let pane = state.pane_mut(side);
        pane.machine = Some(folder.machine.clone());
        pane.folder = Some(folder);
        pane.entries.clear();
        pane.selected.clear();
        pane.loaded = false;
        pane.error = None;
        pane.stale = false;
        Effect::merge([
            self.load_pane(state, side),
            self.load_trash_count(state),
            self.persist(state),
        ])
    }

    pub(super) fn open_folder(&self, state: &mut FilesState, side: Side, name: &str) -> Effect<FilesAction> {
        let Some(folder) = state.usable(side) else {
            return Effect::none();
        };
        let is_folder = state
            .pane(side)
            .entries
            .iter()
            .any(|entry| entry.name == name && entry.kind == EntryKind::Folder);
        if !is_folder {
            return Effect::none();
        }
        self.go(state, side, folder.child(name))
    }

    pub(super) fn up(&self, state: &mut FilesState, side: Side) -> Effect<FilesAction> {
        match state.usable(side).and_then(|folder| folder.parent()) {
            Some(parent) => self.go(state, side, parent),
            None => Effect::none(),
        }
    }
}
