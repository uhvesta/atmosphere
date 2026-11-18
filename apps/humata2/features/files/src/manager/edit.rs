//! Creating, renaming, deleting, copying, moving and dropping files (FM-2..4).

use std::path::PathBuf;

use humata2_clients::{FileManagerError, TransferMode, model::MachineId};
use tca::{Effect, Reducer};

use super::{Delegate, Edit, Files, FilesAction, FilesState, Side, storage};

impl Files {
    pub(super) fn new_folder(&self, state: &mut FilesState, name: String) -> Effect<FilesAction> {
        let Some(folder) = state.usable(state.active) else {
            return Effect::none();
        };
        let manager = self.clients.file_manager.clone();
        Effect::future(async move {
            FilesAction::Finished(Edit::NewFolder, manager.create_folder(&folder, &name).await)
        })
    }

    pub(super) fn rename(&self, state: &mut FilesState, new_name: String) -> Effect<FilesAction> {
        let Some(folder) = state.usable(state.active) else {
            return Effect::none();
        };
        let [old] = state.active_pane().selected.as_slice() else {
            return Effect::none();
        };
        let old = old.clone();
        let manager = self.clients.file_manager.clone();
        Effect::future(async move {
            FilesAction::Finished(Edit::Rename, manager.rename(&folder, &old, &new_name).await)
        })
    }

    pub(super) fn delete(&self, state: &mut FilesState) -> Effect<FilesAction> {
        let Some(folder) = state.usable(state.active) else {
            return Effect::none();
        };
        let names = state.active_pane().selected.clone();
        if names.is_empty() {
            return Effect::none();
        }
        let edit = Edit::Delete {
            count: names.len(),
            machine: folder.machine.clone(),
        };
        let manager = self.clients.file_manager.clone();
        Effect::future(async move { FilesAction::Finished(edit, manager.delete(&folder, names).await) })
    }

    pub(super) fn transfer(&self, state: &mut FilesState, mode: TransferMode) -> Effect<FilesAction> {
        let again = match mode {
            TransferMode::Copy => FilesAction::Copy,
            TransferMode::Move => FilesAction::Move,
        };
        let (Some(from), Some(into)) = (state.usable(state.active), state.usable(state.active.other())) else {
            return Effect::none();
        };
        let names = state.active_pane().selected.clone();
        if names.is_empty() {
            return Effect::none();
        }
        state.send_menu = None;
        state.retry = Some(again);
        let manager = self.clients.file_manager.clone();
        Effect::future(async move {
            let result = manager.start_transfer(&from, names, &into, mode).await;
            FilesAction::Finished(Edit::Transfer, result.map(|_| ()))
        })
    }

    /// Copies or moves the active pane's selection to a machine, where the
    /// placement rules put it (PL-3). The name is never asked: a name taken
    /// there keeps both (PL-13).
    pub(super) fn send_to(&self, state: &mut FilesState, machine: MachineId, mode: TransferMode) -> Effect<FilesAction> {
        state.send_menu = None;
        let again = match mode {
            TransferMode::Copy => FilesAction::CopyTo(machine.clone()),
            TransferMode::Move => FilesAction::MoveTo(machine.clone()),
        };
        let Some(from) = state.usable(state.active) else {
            return Effect::none();
        };
        let names = state.active_pane().selected.clone();
        if names.is_empty() || !state.is_online(&machine) {
            return Effect::none();
        }
        let into = super::placement::landing(&from, &machine, &state.workspace);
        state.retry = Some(again);
        let manager = self.clients.file_manager.clone();
        Effect::future(async move {
            let result = manager.start_transfer_keep_both(&from, names, &into, mode).await;
            FilesAction::Finished(Edit::Transfer, result.map(|_| ()))
        })
    }

    /// Uploads files dropped from Finder (FM-4).
    pub(super) fn dropped(&self, state: &mut FilesState, side: Side, paths: Vec<PathBuf>) -> Effect<FilesAction> {
        let Some(into) = state.usable(side) else {
            return Effect::none();
        };
        if paths.is_empty() {
            return Effect::none();
        }
        state.active = side;
        state.retry = Some(FilesAction::Dropped(side, paths.clone()));
        let manager = self.clients.file_manager.clone();
        Effect::future(async move {
            let result = manager.upload(paths, &into).await;
            FilesAction::Finished(Edit::Transfer, result.map(|_| ()))
        })
    }

    pub(super) fn finished(
        &self,
        state: &mut FilesState,
        edit: Edit,
        result: Result<(), FileManagerError>,
    ) -> Effect<FilesAction> {
        match result {
            Ok(()) => {
                state.error = None;
                // Freeing room mustn't forget what to retry.
                if !matches!(edit, Edit::Storage(_)) {
                    state.retry = None;
                }
                let mut effects = vec![self.after_change(state)];
                match &edit {
                    Edit::Delete { count, machine } => {
                        let noun = if *count == 1 { "item" } else { "items" };
                        let name = state.machine_name(machine);
                        effects.push(Effect::send(FilesAction::Delegate(Delegate::Toast(format!(
                            "Moved {count} {noun} to the trash on {name}"
                        )))));
                    }
                    _ => {}
                }
                Effect::merge(effects)
            }
            Err(FileManagerError::DiskFull {
                machine,
                needs_bytes,
                free_bytes,
            }) => {
                let manager = self.clients.file_manager.clone();
                Effect::future(async move {
                    let result = manager.storage(&machine).await;
                    FilesAction::LowDiskReady {
                        machine,
                        needs_bytes,
                        free_bytes,
                        result,
                    }
                })
            }
            Err(error) => {
                state.error = Some(error.to_string());
                state.retry = None;
                Effect::none()
            }
        }
    }

    /// Reads again everything a change may have touched.
    pub(super) fn after_change(&self, state: &mut FilesState) -> Effect<FilesAction> {
        let left = self.load_pane(state, Side::Left);
        let right = self.load_pane(state, Side::Right);
        let mut effects = vec![
            left,
            right,
            self.load_transfers(),
            self.load_trash_count(state),
        ];
        if let Some(trash) = &state.trash {
            effects.push(self.load_trash(trash.machine.clone()));
        }
        effects.extend(storage::machines_to_measure(state).into_iter().map(|machine| self.measure(machine)));
        Effect::merge(effects)
    }

    pub(super) fn retry_after_low_disk(&self, state: &mut FilesState) -> Effect<FilesAction> {
        state.low_disk = None;
        match state.retry.take() {
            Some(again) => self.reduce(state, again),
            None => Effect::none(),
        }
    }
}
