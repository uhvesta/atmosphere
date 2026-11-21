//! The close confirmation (DV-4): closing a Desk session whose folder is not
//! kept, while it holds files, asks first.

use humata2_clients::DaemonError;
use humata2_clients::model::SessionId;
use tca::Effect;

use crate::{Change, Desk, DeskAction, DeskState};

/// The Close session question: Save version and close, or Close.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseDialog {
    /// The session being closed.
    pub session: SessionId,
    /// The version is being saved.
    pub saving: bool,
    /// Why the version wasn't saved, so the session was not closed.
    pub error: Option<String>,
}

impl DeskState {
    /// Whether closing `session` asks first (DV-4): its folder is one Humata
    /// made and nobody kept, it holds files outside its clones, and it can
    /// save a version. A folder made before versions has to be migrated
    /// first, so it closes at once.
    ///
    /// "Holds files" is the daemon's `holds_files` for the folder.
    #[must_use]
    pub fn close_asks(&self, session: &SessionId) -> bool {
        let Some(record) = self.record(session) else {
            return false;
        };
        record.closed.is_none()
            && record.folder.expires()
            && record.has_versions
            && record.holds_files
    }
}

impl Desk {
    /// Close session from the menu, the palette or a shortcut: asks when
    /// [`DeskState::close_asks`], closes at once otherwise.
    pub(super) fn ask_close(&self, state: &mut DeskState, id: SessionId) -> Effect<DeskAction> {
        // CD-4: a channel's Desk session always asks first.
        if state.workspace.desk.channel_sessions.contains_key(&id) {
            Self::ask_close_channel(state, id);
            return Effect::none();
        }
        if !state.close_asks(&id) {
            return self.close_now(state, id);
        }
        if state.sheet_open() || state.pending.contains(&id) {
            return Effect::none();
        }
        state.menu = None;
        state.close = Some(CloseDialog {
            session: id,
            saving: false,
            error: None,
        });
        Effect::none()
    }

    /// Closes without asking: Close in the dialog, or a session that needs
    /// no question.
    pub(super) fn close_now(&self, state: &mut DeskState, id: SessionId) -> Effect<DeskAction> {
        if state.close.as_ref().is_some_and(|dialog| dialog.session == id) {
            state.close = None;
        }
        self.change(state, id, Change::Close, |client, session| {
            client.close(session)
        })
    }

    /// Save version and close: saves with an empty message, then closes.
    pub(super) fn save_and_close(&self, state: &mut DeskState, id: SessionId) -> Effect<DeskAction> {
        let Some(dialog) = state.close.as_mut() else {
            return Effect::none();
        };
        if dialog.session != id || dialog.saving {
            return Effect::none();
        }
        dialog.saving = true;
        dialog.error = None;
        state.pending.insert(id.clone());
        let client = self.client.clone();
        Effect::future(async move {
            let result = client.save_version(&id, "").await.map(|_| ());
            DeskAction::SavedBeforeClose {
                session: id,
                result,
            }
        })
    }

    pub(super) fn saved_before_close(
        &self,
        state: &mut DeskState,
        id: SessionId,
        result: Result<(), DaemonError>,
    ) -> Effect<DeskAction> {
        state.pending.remove(&id);
        let asked = state
            .close
            .as_ref()
            .is_some_and(|dialog| dialog.session == id);
        match result {
            Err(error) => {
                if let Some(dialog) = state.close.as_mut().filter(|dialog| dialog.session == id) {
                    dialog.saving = false;
                    dialog.error = Some(error.to_string());
                } else {
                    // SMB-011: surface failure if dialog was dismissed while saving.
                    state.error = Some(error.to_string());
                }
                Effect::none()
            }
            // The question was cancelled while the version saved: the
            // version stays, the session does not close.
            Ok(()) if !asked => Effect::none(),
            Ok(()) => self.close_now(state, id),
        }
    }
}
