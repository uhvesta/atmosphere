//! The Desk in the app (DK-1, DK-5, DK-11, DK-14): New desk session from
//! anywhere, what the Desk feature asks the app to do, leaving a Desk
//! session's page once it is closed, and when the sidebar lists the open
//! Desk sessions under its Desk row.

use humata2_clients::desk;
use humata2_clients::model::SessionId;
use humata2_desk::{Change, Delegate, DeskAction, DeskState, Sort};
use tca::Effect;

use crate::{App, AppAction, AppState, Route};

/// The Desk as the last run left it: only its sort is saved.
pub(super) fn restored(sort: Sort) -> DeskState {
    DeskState {
        sort,
        ..DeskState::default()
    }
}

impl AppState {
    /// Whether `id` is a Desk session.
    #[must_use]
    pub fn is_desk_session(&self, id: &SessionId) -> bool {
        self.workspace.session(id).is_some_and(desk::is_desk)
    }

    /// Whether the sidebar lists the open Desk sessions under its Desk row:
    /// on the Desk page and on a Desk session's page (D100).
    #[must_use]
    pub fn desk_in_sidebar(&self) -> bool {
        match &self.route {
            Route::Desk => true,
            Route::Session(id) => self.is_desk_session(id),
            _ => false,
        }
    }
}

impl App {
    /// New desk session, from Home, the Desk page or a command: the form
    /// starts on the agent Settings picked.
    pub(super) fn start_new_desk_session(&self, state: &mut AppState) -> Effect<AppAction> {
        let agent = state.settings.preferences.default_agent;
        self.child(state, AppAction::Desk(DeskAction::StartNew { agent }))
    }

    /// Does what the Desk asked for, and leaves a Desk session's page for
    /// the Desk once that session is closed.
    pub(super) fn desk_delegate(
        &self,
        state: &mut AppState,
        action: &DeskAction,
    ) -> Effect<AppAction> {
        let clock = self.clients.clock.clone();
        match action {
            DeskAction::Delegate(Delegate::OpenSession(id)) => {
                self.navigate(state, Route::Session(id.clone()))
            }
            DeskAction::Delegate(Delegate::Toast(text)) => state.toast(text.clone(), &clock),
            DeskAction::Changed {
                session,
                change: Change::Close,
                result: Ok(()),
            } if state.route == Route::Session(session.clone()) => {
                self.navigate(state, Route::Desk)
            }
            // The Desk page shows its own error; anywhere else (a session's
            // folder bar) a failed change has only the toast to say so.
            DeskAction::Changed {
                result: Err(error), ..
            } if state.route != Route::Desk => state.toast(error.to_string(), &clock),
            _ => Effect::none(),
        }
    }
}
