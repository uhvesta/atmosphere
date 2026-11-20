//! After a host restarts (PER-15, PER-17, PER-29, frames 46 and 47): the
//! report is kept, a toast says what happened, and a session whose resume
//! failed can be tried again or started fresh with a summary.
use std::sync::Arc;

use humata2_clients::model::{MachineId, Restart, Session, SessionId, Status};
use humata2_clients::{DaemonError, Workspace};
use humata2_notify::NotifyEvent;
use tca::Effect;

use crate::{App, AppAction, AppState, Load, Route};

/// A change from the restart views.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestartAction {
    /// Start the agent of a session whose resume failed again.
    TryAgain(SessionId),
    /// Start it on a new conversation seeded with a summary of the old one.
    StartFresh(SessionId),
    /// Open the host's page, where its restart report is.
    OpenReport(MachineId),
    /// A try again or start fresh finished.
    Finished(Result<(), DaemonError>),
}

/// The sessions Home groups after a restart (frame 47).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RestartGroups<'a> {
    /// "Couldn't resume after the restart": the resume failed.
    pub couldnt_resume: Vec<&'a Session>,
    /// "Idle after the restart": started again, now idle.
    pub idle: Vec<&'a Session>,
}

/// The two Home groups for what the hosts found at their last restarts.
#[must_use]
pub fn restart_groups(workspace: &Workspace) -> RestartGroups<'_> {
    let couldnt_resume = workspace
        .sessions
        .iter()
        .filter(|session| !session.archived && session.resume_failed.is_some())
        .collect();
    let idle = workspace
        .sessions
        .iter()
        .filter(|session| {
            !session.archived
                && session.status == Status::Idle
                && workspace
                    .restarts
                    .iter()
                    .any(|restart| restart.resumed.contains(&session.id))
        })
        .collect();
    RestartGroups {
        couldnt_resume,
        idle,
    }
}

fn counted(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// The toast for a restart: what came back and what didn't.
fn told(state: &AppState, restart: &Restart) -> String {
    let host = state
        .workspace
        .machine(&restart.machine)
        .map_or_else(|| restart.machine.as_str().to_owned(), |m| m.name.clone());
    let back = restart.resumed.len();
    let lost = restart.not_brought_back.len();
    let mut text = format!(
        "{host} restarted: {} started again, {lost} couldn't.",
        counted(back, "session")
    );
    if back > 0 {
        text.push_str(" Mid-turn agents are idle.");
    }
    text
}

impl App {
    pub(super) fn heard_restart(
        &self,
        state: &mut AppState,
        restart: Restart,
    ) -> Effect<AppAction> {
        let clock = self.clients.clock.clone();
        let text = told(state, &restart);
        let notice = if state.load == Load::Ready && !restart.not_brought_back.is_empty() {
            let frontmost = Self::frontmost_context(state);
            let now_secs = self.clients.clock.now().as_secs();
            let machine_name = state
                .workspace
                .machine(&restart.machine)
                .map_or_else(|| restart.machine.as_str().to_owned(), |m| m.name.clone());
            let sessions = restart
                .not_brought_back
                .iter()
                .filter_map(|item| item.session.clone())
                .collect();
            humata2_notify::evaluate(
                &state.settings.preferences.notify,
                &frontmost,
                &mut state.notify_burst,
                NotifyEvent::RestartFailed {
                    machine_id: restart.machine.clone(),
                    machine_name,
                    sessions,
                    not_brought_back: restart.not_brought_back.len(),
                },
                now_secs,
            )
        } else {
            None
        };
        let workspace = Arc::make_mut(&mut state.workspace);
        workspace.restarts.retain(|old| old.machine != restart.machine);
        workspace.restarts.insert(0, restart);
        let toast = state.toast(text, &clock);
        match notice {
            Some(notice) => Effect::merge([toast, self.post_notice(notice)]),
            None => toast,
        }
    }

    /// Tells each restart a load brought that the app hadn't held before:
    /// the daemon brought its sessions back before this app connected, so
    /// no restart event came (D390). `before` is what the app held before
    /// the load replaced its workspace, so a reload tells nothing again.
    pub(super) fn loaded_restarts(
        &self,
        state: &mut AppState,
        before: &[Restart],
    ) -> Effect<AppAction> {
        let clock = self.clients.clock.clone();
        let fresh: Vec<String> = state
            .workspace
            .restarts
            .iter()
            .filter(|restart| {
                !before.iter().any(|old| {
                    old.machine == restart.machine && old.at_unix_ms == restart.at_unix_ms
                })
            })
            .map(|restart| told(state, restart))
            .collect();
        Effect::merge(
            fresh
                .into_iter()
                .map(|text| state.toast(text, &clock))
                .collect::<Vec<_>>(),
        )
    }

    pub(super) fn change_restart(
        &self,
        state: &mut AppState,
        change: RestartAction,
    ) -> Effect<AppAction> {
        match change {
            RestartAction::TryAgain(session) => {
                let work = self.clients.daemon.resume(&session);
                Effect::future(async move { AppAction::Restart(RestartAction::Finished(work.await)) })
            }
            RestartAction::StartFresh(session) => {
                let work = self.clients.daemon.start_fresh(&session);
                Effect::future(async move { AppAction::Restart(RestartAction::Finished(work.await)) })
            }
            RestartAction::OpenReport(_) => self.navigate(state, Route::Hosts),
            RestartAction::Finished(Ok(())) => Effect::none(),
            RestartAction::Finished(Err(error)) => {
                let clock = self.clients.clock.clone();
                state.toast(error.to_string(), &clock)
            }
        }
    }
}
