//! The header's ⋯ session menu (SE-7) and its Rename dialog (FC-17).

use humata2_clients::model::Status;
use tca::{Effect, Reducer as _};

use crate::{Panel, SessionAction, SessionState, Sessions, open_session};

/// An item of the session menu (SE-7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItem {
    /// Rename the session, and its branch if asked.
    Rename,
    /// Move it to another machine.
    Move,
    /// Go on with its conversation as a new session in another channel
    /// (CV-8).
    ContinueInChannel,
    /// Stop the agent; the terminal and the clones stay.
    Stop,
    /// Push and open a pull request, from the Changes panel.
    PushAndPullRequest,
    /// Archive it, or close it on the Desk.
    Archive,
}

/// A rename being typed (FC-17).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenameDraft {
    /// The title as typed.
    pub title: String,
    /// Rename the branch from the new title too. Only an unpushed branch
    /// can be renamed; the daemon refuses a pushed one.
    pub rename_branch: bool,
    /// The rename is on its way to the daemon.
    pub sending: bool,
}

/// Whether the agent of the open session can be stopped: it runs, or its host
/// may still hold it.
fn can_stop(state: &SessionState) -> bool {
    open_session(state).is_some_and(|session| !matches!(session.status, Status::Exited { .. }))
}

impl Sessions {
    /// The menu's and the Rename dialog's actions.
    pub(super) fn menu(&self, state: &mut SessionState, action: SessionAction) -> Effect<SessionAction> {
        match action {
            SessionAction::Menu(open) => state.menu = open && open_session(state).is_some(),
            SessionAction::MenuPick(item) => {
                state.menu = false;
                if open_session(state).is_none() {
                    return Effect::none();
                }
                return match item {
                    MenuItem::Rename => self.reduce(state, SessionAction::StartRename),
                    MenuItem::Move => self.reduce(state, SessionAction::StartMove),
                    MenuItem::ContinueInChannel => {
                        self.reduce(state, SessionAction::StartContinue)
                    }
                    MenuItem::Stop => self.reduce(state, SessionAction::StopAgent),
                    // The app opens Review and comments with the panel, and
                    // closes a Desk session instead of archiving it.
                    MenuItem::PushAndPullRequest => {
                        Effect::send(SessionAction::SetPanel(Some(Panel::Changes)))
                    }
                    MenuItem::Archive => Effect::send(SessionAction::Archive),
                };
            }
            SessionAction::StartRename => {
                if let Some(session) = open_session(state) {
                    state.renaming = Some(RenameDraft {
                        title: session.title.clone(),
                        ..RenameDraft::default()
                    });
                    state.error = None;
                }
            }
            SessionAction::RenameTitle(title) => {
                if let Some(draft) = state.renaming.as_mut().filter(|draft| !draft.sending) {
                    draft.title = title;
                }
            }
            SessionAction::RenameBranch(on) => {
                if let Some(draft) = state.renaming.as_mut().filter(|draft| !draft.sending) {
                    draft.rename_branch = on;
                }
            }
            SessionAction::ConfirmRename => return self.confirm_rename(state),
            SessionAction::CancelRename => {
                if state.renaming.as_ref().is_some_and(|draft| !draft.sending) {
                    state.renaming = None;
                }
            }
            SessionAction::Renamed(result) => match result {
                Ok(()) => state.renaming = None,
                Err(error) => {
                    if let Some(draft) = &mut state.renaming {
                        draft.sending = false;
                    }
                    state.error = Some(error.to_string());
                }
            },
            SessionAction::StopAgent => {
                if can_stop(state)
                    && let Some(id) = state.open.clone()
                {
                    state.error = None;
                    return self.done(|daemon| daemon.stop_agent(&id));
                }
            }
            _ => {}
        }
        Effect::none()
    }

    /// Sends the rename. A blank title isn't sent; an unchanged one without
    /// the branch closes the dialog, since nothing would change.
    fn confirm_rename(&self, state: &mut SessionState) -> Effect<SessionAction> {
        let current = open_session(state).map(|session| session.title.clone());
        let (Some(id), Some(current), Some(draft)) =
            (state.open.clone(), current, state.renaming.as_mut())
        else {
            return Effect::none();
        };
        let title = draft.title.trim().to_owned();
        if draft.sending || title.is_empty() {
            return Effect::none();
        }
        if title == current && !draft.rename_branch {
            state.renaming = None;
            return Effect::none();
        }
        draft.sending = true;
        let rename_branch = draft.rename_branch;
        state.error = None;
        let daemon = self.clients.daemon.clone();
        Effect::future(async move {
            SessionAction::Renamed(daemon.rename_session(&id, &title, rename_branch).await)
        })
    }
}
