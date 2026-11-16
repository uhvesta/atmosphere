//! A machine's trash (FM-3, LD-6, LD-7).

use humata2_clients::model::MachineId;
use humata2_clients::{FileManagerError, TrashListing};
use tca::Effect;

use super::{Delegate, Edit, Files, FilesAction, FilesState, TrashState, tray::human_bytes};

impl FilesState {
    pub(super) fn trash_loaded(&mut self, machine: &MachineId, result: Result<TrashListing, FileManagerError>) {
        match result {
            Ok(listing) => {
                if self.active_pane().machine.as_ref() == Some(machine) {
                    self.trash_count = u32::try_from(listing.items.len()).unwrap_or(u32::MAX);
                }
                if let Some(trash) = self.trash.as_mut().filter(|trash| &trash.machine == machine) {
                    trash.listing = Some(listing);
                }
            }
            Err(error) => {
                if self.trash.as_ref().is_some_and(|trash| &trash.machine == machine) {
                    self.error = Some(error.to_string());
                }
            }
        }
    }

    /// The machine whose trash the view is about: the open trash, the open
    /// Storage page, or the active pane's.
    fn trash_machine(&self) -> Option<MachineId> {
        self.trash
            .as_ref()
            .map(|trash| trash.machine.clone())
            .or_else(|| self.storage.as_ref().map(|storage| storage.machine.clone()))
            .or_else(|| self.active_pane().machine.clone())
    }
}

impl Files {
    pub(super) fn load_trash(&self, machine: MachineId) -> Effect<FilesAction> {
        let manager = self.clients.file_manager.clone();
        Effect::future(async move {
            let result = manager.trash(&machine).await;
            FilesAction::TrashLoaded(machine, result)
        })
    }

    /// Counts the active pane's trash, unless that pane is stale.
    pub(super) fn load_trash_count(&self, state: &FilesState) -> Effect<FilesAction> {
        let pane = state.active_pane();
        match (&pane.machine, pane.stale) {
            (Some(machine), false) => self.load_trash(machine.clone()),
            _ => Effect::none(),
        }
    }

    pub(super) fn open_trash(&self, state: &mut FilesState, machine: MachineId) -> Effect<FilesAction> {
        if !state.is_online(&machine) {
            return Effect::none();
        }
        state.trash = Some(TrashState {
            machine: machine.clone(),
            listing: None,
        });
        self.load_trash(machine)
    }

    pub(super) fn restore(&self, state: &mut FilesState, id: &str) -> Effect<FilesAction> {
        let Some(machine) = state.trash.as_ref().map(|trash| trash.machine.clone()) else {
            return Effect::none();
        };
        let id = id.to_owned();
        let manager = self.clients.file_manager.clone();
        Effect::future(async move { FilesAction::Finished(Edit::Restore, manager.restore(&machine, &id).await) })
    }

    pub(super) fn plan_empty_for_view(&self, state: &mut FilesState) -> Effect<FilesAction> {
        match state.trash_machine() {
            Some(machine) => self.plan_empty(machine),
            None => Effect::none(),
        }
    }

    pub(super) fn plan_empty(&self, machine: MachineId) -> Effect<FilesAction> {
        let manager = self.clients.file_manager.clone();
        Effect::future(async move { FilesAction::EmptyPlanned(manager.plan_empty_trash(&machine).await) })
    }

    pub(super) fn confirm_empty(&self, state: &mut FilesState) -> Effect<FilesAction> {
        let Some(plan) = state.empty_plan.take() else {
            return Effect::none();
        };
        let manager = self.clients.file_manager.clone();
        let machine = plan.machine.clone();
        Effect::future(async move {
            let result = manager.empty_trash(plan).await;
            FilesAction::TrashEmptied(machine, result)
        })
    }

    pub(super) fn trash_emptied(
        &self,
        state: &mut FilesState,
        machine: &MachineId,
        result: Result<u64, FileManagerError>,
    ) -> Effect<FilesAction> {
        match result {
            Ok(freed) => {
                state.last_freed = Some(freed);
                let toast = format!(
                    "Emptied the trash on {}, freeing {}",
                    state.machine_name(machine),
                    human_bytes(freed)
                );
                Effect::merge([
                    Effect::send(FilesAction::Delegate(Delegate::Toast(toast))),
                    self.after_change(state),
                ])
            }
            Err(error) => {
                state.error = Some(error.to_string());
                Effect::none()
            }
        }
    }
}
