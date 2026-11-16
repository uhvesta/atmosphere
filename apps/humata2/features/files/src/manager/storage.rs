//! A machine's Storage page (LD-13..LD-15, LD-17) and the low-disk rows
//! (LD-19). Sizes are measured only when asked (LD-14).

use humata2_clients::model::MachineId;
use humata2_clients::{FileManagerError, StorageAction, StorageReport, StorageRow};
use tca::Effect;

use super::{Delegate, Edit, Files, FilesAction, FilesState, PendingClear, StorageState, largest_first};

/// The machines whose open pages show measurements that a change made old.
pub(super) fn machines_to_measure(state: &FilesState) -> Vec<MachineId> {
    let mut machines: Vec<MachineId> = Vec::new();
    let open = state.storage.iter().map(|storage| &storage.machine);
    let short = state.low_disk.iter().map(|low| &low.machine);
    for machine in open.chain(short) {
        if !machines.contains(machine) {
            machines.push(machine.clone());
        }
    }
    machines
}

impl FilesState {
    pub(super) fn measured(&mut self, machine: &MachineId, result: Result<StorageReport, FileManagerError>) {
        match result {
            Ok(report) => {
                if let Some(low) = self.low_disk.as_mut().filter(|low| &low.machine == machine) {
                    low.largest = largest_first(report.rows.clone());
                    low.free_bytes = report.free_bytes;
                }
                if let Some(storage) = self.storage.as_mut().filter(|storage| &storage.machine == machine) {
                    storage.report = Some(report);
                    storage.measuring = false;
                }
            }
            Err(error) => {
                if let Some(storage) = self.storage.as_mut().filter(|storage| &storage.machine == machine) {
                    storage.measuring = false;
                    self.error = Some(error.to_string());
                }
            }
        }
    }
}

impl Files {
    pub(super) fn measure(&self, machine: MachineId) -> Effect<FilesAction> {
        let manager = self.clients.file_manager.clone();
        Effect::future(async move {
            let result = manager.storage(&machine).await;
            FilesAction::Measured(machine, result)
        })
    }

    pub(super) fn open_storage(&self, state: &mut FilesState, machine: MachineId) -> Effect<FilesAction> {
        if !state.is_online(&machine) {
            return Effect::none();
        }
        state.storage = Some(StorageState {
            machine: machine.clone(),
            report: None,
            measuring: true,
        });
        self.measure(machine)
    }

    pub(super) fn measure_again(&self, state: &mut FilesState) -> Effect<FilesAction> {
        let Some(storage) = state.storage.as_mut() else {
            return Effect::none();
        };
        storage.measuring = true;
        self.measure(storage.machine.clone())
    }

    pub(super) fn storage_act(&self, state: &mut FilesState, index: usize) -> Effect<FilesAction> {
        let Some(storage) = &state.storage else {
            return Effect::none();
        };
        let machine = storage.machine.clone();
        let Some(row) = storage.report.as_ref().and_then(|report| report.rows.get(index)).cloned() else {
            return Effect::none();
        };
        self.row_action(state, machine, row)
    }

    pub(super) fn low_disk_act(&self, state: &mut FilesState, index: usize) -> Effect<FilesAction> {
        let Some(low) = &state.low_disk else {
            return Effect::none();
        };
        let machine = low.machine.clone();
        let Some(row) = low.largest.get(index).cloned() else {
            return Effect::none();
        };
        self.row_action(state, machine, row)
    }

    fn row_action(&self, state: &mut FilesState, machine: MachineId, row: StorageRow) -> Effect<FilesAction> {
        match row.action {
            StorageAction::None => Effect::none(),
            // The parent runs the normal archive checks (LD-15, MV-9).
            StorageAction::Archive(session) => Effect::send(FilesAction::Delegate(Delegate::ArchiveSession(session))),
            StorageAction::ClearBuildCache(session) => {
                if row.asks_first {
                    state.pending_clear = Some(PendingClear {
                        machine,
                        session,
                        what: row.what,
                    });
                    Effect::none()
                } else {
                    self.clear(machine, session)
                }
            }
            StorageAction::EmptyTrash => self.plan_empty(machine),
            StorageAction::TrimLogs => {
                let manager = self.clients.file_manager.clone();
                Effect::future(async move {
                    let result = manager.trim_logs(&machine).await;
                    FilesAction::Finished(Edit::Storage(machine), result)
                })
            }
        }
    }

    fn clear(&self, machine: MachineId, session: humata2_clients::model::SessionId) -> Effect<FilesAction> {
        let manager = self.clients.file_manager.clone();
        Effect::future(async move {
            let result = manager.clear_build_cache(&machine, &session).await;
            FilesAction::Finished(Edit::Storage(machine), result)
        })
    }

    pub(super) fn confirm_clear(&self, state: &mut FilesState) -> Effect<FilesAction> {
        match state.pending_clear.take() {
            Some(pending) => self.clear(pending.machine, pending.session),
            None => Effect::none(),
        }
    }
}
