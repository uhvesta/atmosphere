//! Clean up… (DK-11, NV-27, frame 55).

use std::collections::BTreeSet;

use humata2_clients::DaemonError;
use humata2_clients::desk::{CleanUp, CleanUpRow};
use humata2_clients::model::SessionId;
use tca::Effect;

use crate::{Delegate, Desk, DeskAction, DeskState};

/// The Clean up… dialog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CleanUpDialog {
    /// The list is being loaded.
    pub loading: bool,
    /// The closed sessions, least recently used first.
    pub rows: Vec<CleanUpRow>,
    /// The sessions ticked for deleting.
    pub ticked: BTreeSet<SessionId>,
    /// The delete is in flight.
    pub deleting: bool,
    /// The list changed under the dialog; it shows the new one and asks
    /// again.
    pub changed: bool,
    /// Why the list or the delete failed.
    pub error: Option<String>,
}

impl CleanUpDialog {
    /// How many bytes deleting the ticked sessions frees.
    #[must_use]
    pub fn freed_bytes(&self) -> u64 {
        self.rows
            .iter()
            .filter(|row| self.ticked.contains(&row.session))
            .filter_map(|row| row.size_bytes)
            .sum()
    }
}

fn size(bytes: u64) -> String {
    const MB: u64 = 1_000_000;
    const GB: u64 = 1_000_000_000;
    if bytes >= GB {
        format!("{}.{} GB", bytes / GB, bytes % GB / (GB / 10))
    } else {
        format!("{} MB", bytes / MB)
    }
}

impl Desk {
    pub(super) fn open_clean_up(&self, state: &mut DeskState) -> Effect<DeskAction> {
        if state.sheet_open() {
            return Effect::none();
        }
        state.menu = None;
        state.clean_up = Some(CleanUpDialog {
            loading: true,
            ..CleanUpDialog::default()
        });
        let client = self.client.clone();
        Effect::future(async move { DeskAction::CleanUpListed(client.clean_up_list().await) })
    }

    pub(super) fn listed(state: &mut DeskState, result: Result<Vec<CleanUpRow>, DaemonError>) {
        let Some(dialog) = state.clean_up.as_mut() else {
            return;
        };
        dialog.loading = false;
        match result {
            Ok(rows) => {
                dialog.ticked = rows
                    .iter()
                    .filter(|row| row.size_bytes.is_some() && row.unpushed.is_none())
                    .map(|row| row.session.clone())
                    .collect();
                dialog.rows = rows;
            }
            Err(error) => dialog.error = Some(error.to_string()),
        }
    }

    pub(super) fn toggle_clean_up(state: &mut DeskState, session: SessionId) {
        let Some(dialog) = state.clean_up.as_mut() else {
            return;
        };
        if dialog.deleting || !dialog.rows.iter().any(|row| row.session == session) {
            return;
        }
        if !dialog.ticked.remove(&session) {
            dialog.ticked.insert(session);
        }
    }

    pub(super) fn confirm_clean_up(&self, state: &mut DeskState) -> Effect<DeskAction> {
        let Some(dialog) = state.clean_up.as_mut() else {
            return Effect::none();
        };
        if dialog.deleting || dialog.loading || dialog.ticked.is_empty() {
            return Effect::none();
        }
        dialog.deleting = true;
        dialog.changed = false;
        dialog.error = None;
        let shown = dialog.rows.clone();
        let delete: Vec<SessionId> = shown
            .iter()
            .filter(|row| dialog.ticked.contains(&row.session))
            .map(|row| row.session.clone())
            .collect();
        let client = self.client.clone();
        Effect::future(async move { DeskAction::CleanedUp(client.clean_up(shown, delete).await) })
    }

    pub(super) fn cleaned_up(
        state: &mut DeskState,
        result: Result<CleanUp, DaemonError>,
    ) -> Effect<DeskAction> {
        match result {
            Ok(CleanUp::Deleted {
                sessions,
                freed_bytes,
            }) => {
                state.clean_up = None;
                let noun = if sessions == 1 { "session" } else { "sessions" };
                let freed = if freed_bytes == 0 {
                    String::new()
                } else {
                    format!(", freeing {}", size(freed_bytes))
                };
                Effect::send(DeskAction::Delegate(Delegate::Toast(format!(
                    "Deleted {sessions} {noun}{freed}. You can restore them from the trash for 30 days."
                ))))
            }
            Ok(CleanUp::Changed(rows)) => {
                if let Some(dialog) = state.clean_up.as_mut() {
                    // A tick stays only on a row that is listed and exactly
                    // as it was when the person ticked it (NV-27).
                    let before = std::mem::take(&mut dialog.rows);
                    dialog.ticked.retain(|session| {
                        let old = before.iter().find(|row| &row.session == session);
                        let new = rows.iter().find(|row| &row.session == session);
                        old.is_some() && old == new
                    });
                    dialog.rows = rows;
                    dialog.deleting = false;
                    dialog.changed = true;
                }
                Effect::none()
            }
            Err(error) => {
                if let Some(dialog) = state.clean_up.as_mut() {
                    dialog.deleting = false;
                    dialog.error = Some(error.to_string());
                }
                Effect::none()
            }
        }
    }
}
