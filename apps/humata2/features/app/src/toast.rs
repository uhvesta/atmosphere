use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::model::SessionId;
use tca::Effect;

use crate::{App, AppAction, AppState, ReportForm, TOAST};

/// The action a toast offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToastAction {
    RestoreSession(SessionId),
    /// Open the bug-report form for a saved report.
    OpenReport(Box<ReportForm>),
}

/// A message at the bottom of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    /// Its id, to dismiss it.
    pub id: u64,
    /// What it says.
    pub text: String,
    pub action: Option<ToastAction>,
    pub working: bool,
}

impl AppState {
    pub(super) fn toast(&mut self, text: String, clock: &Arc<dyn tca::Clock>) -> Effect<AppAction> {
        self.next_toast += 1;
        let id = self.next_toast;
        self.toasts.push(Toast {
            id,
            text,
            action: None,
            working: false,
        });
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
        let sleep = clock.sleep(TOAST);
        Effect::future(async move {
            sleep.await;
            AppAction::ToastExpired(id)
        })
        .cancellable(format!("toast-{id}"), true)
    }
}

impl App {
    pub(super) fn undo_toast(&self, state: &mut AppState, id: u64) -> Effect<AppAction> {
        let Some(toast) = state.toasts.iter_mut().find(|t| t.id == id && !t.working) else {
            return Effect::none();
        };
        let session = match toast.action.clone() {
            Some(ToastAction::RestoreSession(session)) => session,
            Some(ToastAction::OpenReport(form)) => {
                state.toasts.retain(|t| t.id != id);
                state.report.form = Some(*form);
                return Effect::cancel(format!("toast-{id}"));
            }
            None => return Effect::none(),
        };
        toast.working = true;
        let daemon = self.clients.daemon.clone();
        Effect::future(async move {
            AppAction::Undone {
                toast: id,
                result: daemon.restore_session(&session).await,
            }
        })
    }

    pub(super) fn toast_undone(
        &self,
        state: &mut AppState,
        toast: u64,
        result: Result<(), DaemonError>,
    ) -> Effect<AppAction> {
        let clock = self.clients.clock.clone();
        match result {
            Ok(()) => {
                state.toasts.retain(|t| t.id != toast);
                Effect::none()
            }
            Err(error) => {
                if let Some(toast) = state.toasts.iter_mut().find(|t| t.id == toast) {
                    toast.working = false;
                }
                state.toast(error.to_string(), &clock)
            }
        }
    }

    pub(super) fn dismiss_toast(&self, state: &mut AppState, id: u64) -> Effect<AppAction> {
        state.toasts.retain(|toast| toast.id != id);
        Effect::cancel(format!("toast-{id}"))
    }

    pub(super) fn toast_expired(&self, state: &mut AppState, id: u64) -> Effect<AppAction> {
        state.toasts.retain(|toast| toast.id != id);
        Effect::none()
    }
}
