//! Add repo to this session (CH-21): a sheet over a running session, opened
//! from the session menu and from the Diff and Files panels. It searches the
//! repo library and GitHub as every repo picker does (RL-5); picking a repo,
//! or typing `owner/name` and pressing Return, asks the daemon to clone it in
//! the session and add it to the channel when the channel lacks it; the agent
//! is told nothing (D743). A Desk session has its own Add repo (DK-3) and is
//! not offered this.

use humata2_clients::DaemonError;
use humata2_clients::model::SessionId;
use humata2_clients::progress::{OpKind, OpProgress, OpUpdate};
use tca::Effect;

use crate::repo_search::{self, RepoPicker, RepoSearchAction, RepoSearchState};
use crate::{App, AppAction, AppState, Overlay};

/// Tells this sheet's search from another on screen.
const KEY: &str = "session-add-repo";

/// The Add repo to this session sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionAddRepo {
    /// The session the repo is added to.
    pub session: SessionId,
    /// The one repo search (RL-5).
    pub search: RepoSearchState,
    /// The repo the daemon is adding; the sheet waits.
    pub adding: Option<String>,
    /// The running step of the operation while `adding`.
    pub progress: Option<OpProgress>,
    /// The operation id `progress` follows; latched by the first update.
    pub op: Option<String>,
    /// Why the last add failed, and what to do.
    pub error: Option<String>,
}

/// A change to the sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionAddRepoAction {
    /// Open the sheet for this session.
    Open(SessionId),
    /// Add the picked repo; `None` adds what was typed.
    Add(Option<String>),
    /// The daemon cloned the repo in the session, or refused.
    Added {
        /// The repo, as `owner/name`.
        repo: String,
        /// How it went.
        result: Result<(), DaemonError>,
    },
}

impl App {
    pub(super) fn session_add_repo(
        &self,
        state: &mut AppState,
        action: SessionAddRepoAction,
    ) -> Effect<AppAction> {
        match action {
            SessionAddRepoAction::Open(session) => self.open_session_add_repo(state, session),
            SessionAddRepoAction::Add(repo) => self.add_repo_to_open_session(state, repo),
            SessionAddRepoAction::Added { repo, result } => self.repo_added_to_session(state, &repo, result),
        }
    }

    /// Applies `action` to the sheet's search; dropped when the sheet closed.
    pub(super) fn session_repo_search(
        &self,
        state: &mut AppState,
        action: RepoSearchAction,
    ) -> Effect<AppAction> {
        let Some(sheet) = sheet_of(state) else {
            return Effect::none();
        };
        repo_search::reduce(&self.clients, &mut sheet.search, action, KEY)
            .map(|action| AppAction::RepoSearch(RepoPicker::SessionAddRepo, action))
    }

    fn open_session_add_repo(&self, state: &mut AppState, session: SessionId) -> Effect<AppAction> {
        state.session.menu = false;
        if state.workspace.session(&session).is_none()
            || state.is_desk_session(&session)
            || sheet_of(state).is_some()
        {
            return Effect::none();
        }
        state.overlays.push(Overlay::SessionAddRepo(SessionAddRepo {
            session,
            search: RepoSearchState::default(),
            adding: None,
            progress: None,
            op: None,
            error: None,
        }));
        self.session_repo_search(state, RepoSearchAction::Open)
    }

    fn add_repo_to_open_session(&self, state: &mut AppState, repo: Option<String>) -> Effect<AppAction> {
        let Some(sheet) = sheet_of(state) else {
            return Effect::none();
        };
        if sheet.adding.is_some() {
            return Effect::none();
        }
        let repo = repo.unwrap_or_else(|| sheet.search.query.clone()).trim().to_owned();
        if repo.is_empty() {
            sheet.error = Some("Pick a repo, or type owner/name, such as acme/api.".to_owned());
            return Effect::none();
        }
        sheet.error = None;
        sheet.adding = Some(repo.clone());
        sheet.progress = None;
        sheet.op = None;
        let (daemon, session) = (self.clients.daemon.clone(), sheet.session.clone());
        Effect::future(async move {
            let result = daemon.add_repo_to_session(&session, &repo).await;
            AppAction::SessionAddRepo(SessionAddRepoAction::Added { repo, result })
        })
    }

    fn repo_added_to_session(
        &self,
        state: &mut AppState,
        repo: &str,
        result: Result<(), DaemonError>,
    ) -> Effect<AppAction> {
        match result {
            Ok(()) => {
                state
                    .overlays
                    .retain(|overlay| !matches!(overlay, Overlay::SessionAddRepo(_)));
                let clock = self.clients.clock.clone();
                state.toast(format!("Added {repo} to this session"), &clock)
            }
            Err(error) => {
                if let Some(sheet) = sheet_of(state) {
                    sheet.adding = None;
                    sheet.progress = None;
                    sheet.op = None;
                    sheet.error = Some(error.to_string());
                }
                Effect::none()
            }
        }
    }
}

impl App {
    /// A watched operation moved on: the waiting sheet keeps its progress.
    pub(super) fn add_repo_operation(&self, state: &mut AppState, update: OpUpdate) {
        if update.kind != OpKind::AddRepo {
            return;
        }
        if let Some(sheet) = sheet_of(state).filter(|sheet| sheet.adding.is_some()) {
            humata2_session::follow_operation(&mut sheet.op, &mut sheet.progress, update);
        }
    }
}

/// The sheet, when it is open.
fn sheet_of(state: &mut AppState) -> Option<&mut SessionAddRepo> {
    state
        .overlays
        .iter_mut()
        .rev()
        .find_map(|overlay| match overlay {
            Overlay::SessionAddRepo(sheet) => Some(sheet),
            _ => None,
        })
}
