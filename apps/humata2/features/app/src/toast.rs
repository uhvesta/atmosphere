use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::model::SessionId;
use humata2_review::comments::CommentsAction;
use tca::Effect;

use crate::{App, AppAction, AppState, ReportForm, Route, TOAST};

/// How long an archive toast stays: the undo window (MV-15, D110).
pub(super) const UNDO_TOAST: std::time::Duration = std::time::Duration::from_secs(10);

/// The action a toast offers.
#[derive(Clone, Debug, PartialEq)]
pub enum ToastAction {
    RestoreSession(SessionId),
    /// Undo a change to review comments by sending this.
    Comments(Box<CommentsAction>),
    /// Open the bug-report form for a saved report.
    OpenReport(Box<ReportForm>),
    /// The close dialog's toast button (D784): Open, Resume, or End for
    /// good's Undo.
    Close(humata2_session::CloseThen),
    /// Every agent is off: open Settings › Agents (AGENTOFF).
    OpenAgentSettings,
}

/// A message at the bottom of the window.
#[derive(Clone, Debug, PartialEq)]
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
        self.toast_for(text, clock, TOAST)
    }

    pub(super) fn toast_for(
        &mut self,
        text: String,
        clock: &Arc<dyn tca::Clock>,
        shown: std::time::Duration,
    ) -> Effect<AppAction> {
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
        let sleep = clock.sleep(shown);
        Effect::future(async move {
            sleep.await;
            AppAction::ToastExpired(id)
        })
        .cancellable(format!("toast-{id}"), true)
    }
}

impl AppState {
    /// Says every agent is off, with a button to Settings › Agents.
    pub(super) fn toast_no_agent(&mut self, clock: &Arc<dyn tca::Clock>) -> Effect<AppAction> {
        self.toast_agent_off(humata2_clients::NoAgentOn.to_string(), clock)
    }

    /// Says `text` about an agent that is off, with a button to
    /// Settings › Agents.
    pub(super) fn toast_agent_off(&mut self, text: String, clock: &Arc<dyn tca::Clock>) -> Effect<AppAction> {
        let effect = self.toast(text, clock);
        if let Some(toast) = self.toasts.last_mut() {
            toast.action = Some(ToastAction::OpenAgentSettings);
        }
        effect
    }
}

impl App {
    pub(super) fn undo_toast(&self, state: &mut AppState, id: u64) -> Effect<AppAction> {
        let Some(toast) = state.toasts.iter_mut().find(|t| t.id == id && !t.working) else {
            return Effect::none();
        };
        let session = match toast.action.clone() {
            Some(ToastAction::RestoreSession(session)) => session,
            Some(ToastAction::Comments(undo)) => {
                state.toasts.retain(|t| t.id != id);
                let undone = self.handle(state, AppAction::Comments(*undo));
                return Effect::merge([Effect::cancel(format!("toast-{id}")), undone]);
            }
            Some(ToastAction::Close(then)) => {
                state.toasts.retain(|t| t.id != id);
                let done = self.handle(
                    state,
                    AppAction::Session(humata2_session::SessionAction::Close(humata2_session::CloseAction::Then(then))),
                );
                return Effect::merge([Effect::cancel(format!("toast-{id}")), done]);
            }
            Some(ToastAction::OpenReport(form)) => {
                state.toasts.retain(|t| t.id != id);
                state.report.form = Some(*form);
                return Effect::cancel(format!("toast-{id}"));
            }
            Some(ToastAction::OpenAgentSettings) => {
                state.toasts.retain(|t| t.id != id);
                let go = self.navigate(state, Route::Settings);
                let section = self.child(
                    state,
                    AppAction::Settings(humata2_settings::SettingsAction::Section(
                        humata2_settings::Section::Agents,
                    )),
                );
                return Effect::merge([Effect::cancel(format!("toast-{id}")), go, section]);
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
