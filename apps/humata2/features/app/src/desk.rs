//! The Desk in the app (DK-1, DK-5, DK-11, DK-14): New desk session from
//! anywhere, what the Desk feature asks the app to do, leaving a Desk
//! session's page once it is closed, and when the sidebar lists the open
//! Desk sessions under its Desk row.

use humata2_clients::desk;
use humata2_clients::{FileRoot, Folder};
use humata2_clients::model::{ForkKind, SessionId};
use humata2_desk::{Change, Delegate, DeskAction, DeskState, Sort};
use humata2_files::FilesAction;
use humata2_session::SessionAction;
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
}

impl App {
    /// New desk session, from Home, the Desk page or a command: the form
    /// starts on the agent Settings picked.
    pub(super) fn start_new_desk_session(&self, state: &mut AppState) -> Effect<AppAction> {
        let agent = crate::new_session::default_agent(state, None);
        self.child(state, AppAction::Desk(DeskAction::StartNew { agent }))
    }

    /// New desk session for this session, from a session's menu (CD-6): the
    /// channel's dialog with that session as the target. A Desk session has
    /// no channel to look after, so it starts nothing there.
    pub(super) fn new_desk_session_for(
        &self,
        state: &mut AppState,
        id: &SessionId,
    ) -> Effect<AppAction> {
        if state.is_desk_session(id) {
            return Effect::none();
        }
        let Some(channel) = state
            .workspace
            .session(id)
            .map(|session| session.channel.clone())
        else {
            return Effect::none();
        };
        let agent = crate::new_session::default_agent(state, Some(&channel));
        let menu = self.child(state, AppAction::Session(SessionAction::Menu(false)));
        let dialog = self.child(
            state,
            AppAction::Desk(DeskAction::StartChannelNew {
                channel,
                target: Some(id.clone()),
                agent,
            }),
        );
        Effect::merge([menu, dialog])
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
            // D382: a Desk session has no repository, so its folder is
            // opened as the session's own, on the machine it runs on.
            DeskAction::Delegate(Delegate::OpenFolder(id)) => {
                let Some(machine) = state.workspace.session(id).map(|session| session.machine.clone())
                else {
                    return Effect::none();
                };
                let go = self.navigate(state, Route::Files);
                let side = state.files.active;
                let folder = Folder::root(machine, FileRoot::SessionFolder { session: id.clone() });
                let show = self.child(state, AppAction::Files(FilesAction::Navigate(side, folder)));
                Effect::merge([go, show])
            }
            DeskAction::Delegate(Delegate::Toast(text)) => state.toast(text.clone(), &clock),
            // CD-4: closing a channel's Desk session from its own page goes
            // back to that channel's Desk tab.
            DeskAction::Changed {
                session,
                change: Change::Close,
                result: Ok(()),
            } if state.route == Route::Session(session.clone())
                && state.workspace.desk.channel_sessions.contains_key(session) =>
            {
                let Some(channel) = state
                    .workspace
                    .desk
                    .channel_sessions
                    .get(session)
                    .map(|found| found.channel.clone())
                else {
                    return Effect::none();
                };
                state.channel_pages.entry(channel.clone()).or_default().tab =
                    crate::channel::Tab::Desk;
                let go = self.navigate(state, Route::Channel(channel));
                Effect::merge([go, self.load()])
            }
            DeskAction::Changed {
                session,
                change: Change::Close,
                result: Ok(()),
            } if state.route == Route::Session(session.clone()) => {
                self.navigate(state, Route::Desk)
            }
            // REPOS (RL-5, DK-14): New desk session's repo search opens on the library.
            DeskAction::New(humata2_desk::NewAction::AddingRepo(true)) => {
                self.open_new_desk_repo_search(state)
            }
            // REPOS (RL-3, RL-5): Add repo opens on the library.
            DeskAction::Menu(Some(humata2_desk::Menu::AddRepo(_))) => {
                self.open_desk_repo_search(state)
            }
            // The daemon's Desk events keep the Desk current; after the
            // app's own change it still reads again, which also brings the
            // session rows a start or a close changes.
            DeskAction::Started(Ok(_))
            | DeskAction::Changed { result: Ok(()), .. }
            | DeskAction::Copied(Ok(()))
            | DeskAction::CleanedUp(Ok(_)) => self.load(),
            // The Desk page shows its own error; anywhere else (a session's
            // folder bar) a failed change has only the toast to say so.
            DeskAction::Changed {
                result: Err(error), ..
            } if state.route != Route::Desk => state.toast(error.to_string(), &clock),
            _ => Effect::none(),
        }
    }
}

impl App {
    /// The open session, when it is a Desk session.
    fn open_desk_session(state: &AppState) -> Option<SessionId> {
        state
            .session
            .open
            .clone()
            .filter(|id| state.is_desk_session(id))
    }

    /// Archive, from the palette or a shortcut. On a Desk session it is
    /// Close session (DK-11): the folder stays and the session goes under
    /// Closed on the Desk.
    pub(super) fn archive_or_close(&self, state: &mut AppState) -> Effect<AppAction> {
        match Self::open_desk_session(state) {
            Some(id) => self.handle(state, AppAction::Desk(DeskAction::AskClose(id))),
            None => self.child(state, AppAction::Session(SessionAction::Archive)),
        }
    }

    /// Fork, from the palette or a shortcut. A Desk session has none
    /// (DK-12).
    pub(super) fn fork_unless_desk(
        &self,
        state: &mut AppState,
        kind: ForkKind,
    ) -> Effect<AppAction> {
        if Self::open_desk_session(state).is_some() {
            return Effect::none();
        }
        self.child(state, AppAction::Session(SessionAction::StartFork(kind)))
    }
}
