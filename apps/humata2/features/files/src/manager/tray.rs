//! The transfers tray (FM-7, FM-8, FM-13).

use humata2_clients::model::{MachineId, TransferId};
use humata2_clients::{TransferPhase, TransferRow};
use tca::Effect;

use super::{Edit, Files, FilesAction, FilesState, Side};

/// Bytes as people read them: `1.5 GB`.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

impl FilesState {
    /// How much of a transfer is done, from 0 to 1.
    #[must_use]
    pub fn fraction(&self, row: &TransferRow) -> f64 {
        row.fraction()
    }

    /// The transfer's state in a word or two, as the tray's status column
    /// reads: `Copying`, `Paused: gpu-box is offline`, `\u{2713} Done`.
    #[must_use]
    pub fn status_line(&self, row: &TransferRow) -> String {
        match &row.phase {
            TransferPhase::Copying => "Copying".to_owned(),
            TransferPhase::Verifying => "Verifying".to_owned(),
            TransferPhase::Paused { waiting_for: Some(machine), .. } => {
                format!("Paused: {} is offline", self.machine_name(machine))
            }
            TransferPhase::Paused { .. } => "Paused".to_owned(),
            TransferPhase::Done => "\u{2713} Done".to_owned(),
            TransferPhase::Failed { .. } => "Failed".to_owned(),
            TransferPhase::Cancelled => "Cancelled".to_owned(),
        }
    }

    /// The line under the transfer: what is guaranteed, or why it waits.
    #[must_use]
    pub fn detail_line(&self, row: &TransferRow) -> String {
        let through = if row.via_host { " \u{b7} through this Mac" } else { "" };
        match &row.phase {
            TransferPhase::Copying => format!("Resumable \u{b7} checksummed per chunk{through}"),
            TransferPhase::Verifying => format!("Comparing checksums{through}"),
            TransferPhase::Paused { waiting_for: Some(_), .. } => {
                "Resumes on reconnect, from the last confirmed offset \u{b7} checksummed per chunk".to_owned()
            }
            TransferPhase::Paused { reason, .. } | TransferPhase::Failed { reason } => reason.clone(),
            TransferPhase::Done => "Checksum ok".to_owned(),
            TransferPhase::Cancelled => "Stopped before it finished".to_owned(),
        }
    }

    /// Whether `machine` is an end of any transfer that isn't over.
    #[must_use]
    pub fn has_running_transfer(&self, machine: &MachineId) -> bool {
        self.transfers
            .iter()
            .any(|row| !row.phase.is_over() && (&row.from == machine || &row.to == machine))
    }
}

impl Files {
    pub(super) fn load_transfers(&self) -> Effect<FilesAction> {
        let manager = self.clients.file_manager.clone();
        Effect::future(async move { FilesAction::Transfers(manager.transfers().await) })
    }

    pub(super) fn follow_transfers(&self) -> Effect<FilesAction> {
        let manager = self.clients.file_manager.clone();
        Effect::run(move |send| async move {
            let mut events = manager.transfer_events();
            loop {
                send.ready().await;
                match events.recv().await {
                    Some(row) => send.send(FilesAction::TransferEvent(row)),
                    None => return,
                }
            }
        })
        .cancellable("files-transfer-events", true)
    }

    pub(super) fn transfer_event(&self, state: &mut FilesState, row: TransferRow) -> Effect<FilesAction> {
        let was_done = state
            .transfers
            .iter()
            .any(|known| known.id == row.id && known.phase == TransferPhase::Done);
        let done_now = row.phase == TransferPhase::Done;
        let (from, to) = (row.from.clone(), row.to.clone());
        match state.transfers.iter().position(|known| known.id == row.id) {
            Some(at) => state.transfers[at] = row,
            None => state.transfers.insert(0, row),
        }
        if !done_now || was_done {
            return Effect::none();
        }
        let mut effects = Vec::new();
        for side in [Side::Left, Side::Right] {
            let shows = state
                .pane(side)
                .machine
                .as_ref()
                .is_some_and(|machine| machine == &from || machine == &to);
            if shows {
                effects.push(self.load_pane(state, side));
            }
        }
        effects.push(self.load_trash_count(state));
        Effect::merge(effects)
    }

    pub(super) fn cancel(&self, id: TransferId) -> Effect<FilesAction> {
        let manager = self.clients.file_manager.clone();
        Effect::future(async move { FilesAction::Finished(Edit::Cancel, manager.cancel_transfer(&id).await) })
    }
}
