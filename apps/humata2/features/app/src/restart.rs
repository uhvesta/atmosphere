//! After a host restarts (PER-15, PER-17, PER-29, frames 46 and 47): the
//! report is kept, a toast says what happened, and a session whose resume
//! failed can be tried again or started fresh with a summary.
use std::sync::Arc;

use humata2_clients::model::{MachineId, Restart, Session, SessionId, Status};
use humata2_clients::{DaemonError, Workspace};
use tca::Effect;

use crate::{App, AppAction, AppState, Route};

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

impl App {
    pub(super) fn heard_restart(
        &self,
        state: &mut AppState,
        restart: Restart,
    ) -> Effect<AppAction> {
        let clock = self.clients.clock.clone();
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
        let workspace = Arc::make_mut(&mut state.workspace);
        workspace.restarts.retain(|old| old.machine != restart.machine);
        workspace.restarts.insert(0, restart);
        state.toast(text, &clock)
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
