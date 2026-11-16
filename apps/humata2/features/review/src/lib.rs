//! Reviewing a session's changes (RV-1–27): files fold, and lines wrap or
//! scroll. Comments on lines belong to the daemon and live in [`comments`];
//! this page only hands over what older builds saved here (D151).
//!
//! The rows the view draws are prepared off the UI thread by an effect and
//! arrive ready to draw ([`ReviewAction::Prepared`]); a newer request
//! replaces an older one still in flight.

pub mod comments;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use humata2_clients::{Daemon, DaemonError, Workspace};
use humata2_clients::model::{Comment, DiffBase, DiffLine, PushChoice, RepoDiff, SessionId, Status};
use serde::{Deserialize, Serialize};
use tca::{Clock, Effect, Reducer};

use comments::HandOver;

/// A line on the review page, ready to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffRow {
    /// A file's header.
    File {
        /// Its path.
        path: String,
        /// Lines added.
        added: u32,
        /// Lines removed.
        removed: u32,
        /// Whether its lines show.
        open: bool,
    },
    /// A diff line.
    Line {
        /// The file.
        path: String,
        /// The line.
        line: DiffLine,
    },
}

/// Where a comment goes. Only older builds' saved drafts still name one.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Anchor {
    /// The file.
    pub path: String,
    /// The line in the new file.
    pub line: u32,
}

/// What survives a restart: the selected file and folded files. Comments
/// older builds kept here are read once and handed to the daemon.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewSaved {
    /// The selected repository and file, per session.
    #[serde(default)]
    pub selected: BTreeMap<SessionId, (String, String)>,
    /// Folded files per session.
    pub folded: BTreeMap<SessionId, BTreeSet<String>>,
    /// Comments older builds wrote but did not send, per session. Never
    /// written any more; removed as the daemon takes each.
    #[serde(default, rename = "queued", skip_serializing_if = "BTreeMap::is_empty")]
    pub legacy_queued: BTreeMap<SessionId, Vec<Comment>>,
    /// A comment older builds were writing, per session. Never written any
    /// more; removed as the daemon takes it.
    #[serde(default, rename = "drafts", skip_serializing_if = "BTreeMap::is_empty")]
    pub legacy_drafts: BTreeMap<SessionId, (Anchor, String)>,
}

/// What the page knows about its reads of one session's changes (D179).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fetch {
    /// A read is in flight.
    pub loading: bool,
    /// Something asked for another read while one was in flight.
    pub queued: bool,
    /// When the diff shown was read, on the clock.
    pub read_at: Option<Duration>,
    /// Why the last read or Mark reviewed failed, as the panel says it.
    pub error: Option<String>,
    /// The last diff read.
    pub diffs: Option<Vec<RepoDiff>>,
}

/// Where pushing one repository of a session's changes stands (RV-2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushState {
    /// The push is running.
    Pushing,
    /// Pushed; `pull_request` is the pull request open for its branch, if any.
    Pushed {
        /// The pull request's number.
        pull_request: Option<u32>,
    },
    /// The push didn't finish: why, and what to do.
    Failed(String),
}

/// The review page's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReviewState {
    /// Where the diffs come from.
    pub workspace: Arc<Workspace>,
    /// The session whose changes show.
    pub session: Option<SessionId>,
    /// The rows to draw, once prepared.
    pub rows: Option<Arc<Vec<DiffRow>>>,
    /// Which preparation the rows are from; older results are dropped.
    pub generation: u64,
    /// Long lines wrap instead of scrolling (RV-9).
    pub wrap: bool,
    /// What is saved.
    pub saved: ReviewSaved,
    /// What each session's changes are compared with; none means origin/main.
    pub base: HashMap<SessionId, DiffBase>,
    /// The reads of each session's changes.
    pub fetch: HashMap<SessionId, Fetch>,
    /// Whether the base menu under the Diff header is open (RV-1).
    pub base_menu: bool,
    /// Where each repository's push of a session's changes stands.
    pub pushes: HashMap<(SessionId, String), PushState>,
}

/// What the review page asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// These comments older builds saved are the daemon's to keep now.
    HandOver(HandOver),
    /// The daemon read these changes for the session: they replace what
    /// the app holds for it.
    Fetched(SessionId, Vec<RepoDiff>),
}

/// What happens on the review page.
#[derive(Clone, Debug, PartialEq)]
pub enum ReviewAction {
    /// Show a file from the open session's comparison.
    SelectFile { repo: String, path: String },
    /// The data changed.
    Show(Arc<Workspace>),
    /// Saved folds and older comments came back at launch.
    Restore(ReviewSaved),
    /// Show this session's changes.
    Open(SessionId),
    /// Rows were prepared.
    Prepared(u64, Arc<Vec<DiffRow>>),
    /// A file header was clicked.
    ToggleFile(String),
    /// Wrap long lines, or not.
    ToggleWrap,
    /// The daemon took this saved comment: forget it.
    HandedOver(SessionId, Comment),
    /// Compare the open session's changes with another base (RV-1).
    PickBase(DiffBase),
    /// Open the base menu, or close it if it is open (RV-1).
    ToggleBaseMenu,
    /// Close the base menu.
    CloseBaseMenu,
    /// Mark the open session's changes reviewed (RV-6).
    MarkReviewed,
    /// Read the open session's changes again.
    Retry,
    /// Commit, push and open a pull request for one repository of the open
    /// session's changes (RV-2).
    Push {
        /// The repository, as `owner/name`.
        repo: String,
    },
    /// A push of one repository ended.
    Pushed {
        /// The session pushed.
        session: SessionId,
        /// The repository pushed.
        repo: String,
        /// The pull request the push opened or found, or why it failed.
        result: Result<Option<u32>, String>,
    },
    /// A read of a session's changes ended.
    Fetched {
        session: SessionId,
        base: DiffBase,
        result: Result<Vec<RepoDiff>, String>,
    },
    /// Marking a session reviewed ended.
    Reviewed {
        session: SessionId,
        result: Result<(), String>,
    },
    /// For the parent.
    Delegate(Delegate),
}

/// The review page's reducer.
#[derive(Clone)]
pub struct Review {
    daemon: Arc<dyn Daemon>,
    clock: Arc<dyn Clock>,
}

impl Review {
    /// A reducer that reads changes from `daemon` and stamps reads on `clock`.
    #[must_use]
    pub fn new(daemon: Arc<dyn Daemon>, clock: Arc<dyn Clock>) -> Self {
        Self { daemon, clock }
    }

    /// Reads `session`'s changes, or queues a read behind the one in flight.
    fn fetch(&self, state: &mut ReviewState, session: &SessionId) -> Effect<ReviewAction> {
        let fetch = state.fetch.entry(session.clone()).or_default();
        if fetch.loading {
            fetch.queued = true;
            return Effect::none();
        }
        fetch.loading = true;
        let base = state.base.get(session).cloned().unwrap_or_default();
        let reply = self.daemon.diffs(session, &base);
        let session = session.clone();
        Effect::future(async move {
            ReviewAction::Fetched {
                session,
                base,
                result: reply.await.map_err(|error| error.to_string()),
            }
        })
    }

    /// Pushes one repository of `session`'s changes: plans the push, then
    /// commits and pushes it and opens its pull request if it has none.
    fn push(&self, state: &ReviewState, session: SessionId, repo: String) -> Effect<ReviewAction> {
        let daemon = self.daemon.clone();
        let title = state
            .workspace
            .sessions
            .iter()
            .find(|each| each.id == session)
            .map(|each| each.title.clone())
            .unwrap_or_default();
        Effect::future(async move {
            let result = push_repo(daemon.as_ref(), &session, &repo, &title)
                .await
                .map_err(|error| error.to_string());
            ReviewAction::Pushed {
                session,
                repo,
                result,
            }
        })
    }
}

/// Pushes `repo` as the session's current plan has it: commits its
/// uncommitted files when it has any, and opens a pull request when its
/// branch has none. Returns the pull request for the branch, if there is one.
async fn push_repo(
    daemon: &dyn Daemon,
    session: &SessionId,
    repo: &str,
    title: &str,
) -> Result<Option<u32>, DaemonError> {
    let plan = daemon.plan_push(session).await?;
    let Some(found) = plan.repos.into_iter().find(|each| each.repo == repo) else {
        return Err(DaemonError::Refused {
            what: format!("Pushing {repo}"),
            reason: format!(
                "{repo} has no changes in the session's plan now. Read the changes again, then push it from there"
            ),
        });
    };
    let choice = PushChoice {
        repo: found.repo,
        commit_message: if found.dirty_files > 0 {
            format!("Changes from {title}")
        } else {
            String::new()
        },
        open_pull_request: found.open_pull_request.is_none(),
        title: title.to_owned(),
        body: String::new(),
    };
    let number = daemon.push(session, &plan.token, vec![choice]).await?;
    Ok(number.or(found.open_pull_request))
}

fn working(workspace: &Workspace, session: &SessionId) -> bool {
    workspace
        .sessions
        .iter()
        .any(|each| &each.id == session && each.status == Status::Working)
}

/// Makes `diffs` the changes the workspace holds for `session`.
fn replace_diffs(workspace: &mut Arc<Workspace>, session: &SessionId, diffs: &[RepoDiff]) {
    let same = workspace
        .diffs
        .iter()
        .filter(|diff| &diff.session == session)
        .eq(diffs.iter());
    if same {
        return;
    }
    let workspace = Arc::make_mut(workspace);
    workspace.diffs.retain(|diff| &diff.session != session);
    workspace.diffs.extend(diffs.iter().cloned());
}

impl Reducer for Review {
    type State = ReviewState;
    type Action = ReviewAction;

    fn reduce(&self, state: &mut ReviewState, action: ReviewAction) -> Effect<ReviewAction> {
        match action {
            ReviewAction::SelectFile { repo, path } => {
                if let Some(session) = state.session.clone()
                    && state.workspace.diffs.iter().any(|diff| {
                        diff.session == session
                            && diff.repo == repo
                            && diff.files.iter().any(|file| file.path == path)
                    })
                {
                    state.saved.selected.insert(session, (repo, path));
                }
            }
            ReviewAction::Show(workspace) => {
                let turn_ended = state.session.clone().filter(|session| {
                    working(&state.workspace, session) && !working(&workspace, session)
                });
                state.workspace = workspace;
                let shown: Vec<(SessionId, Vec<RepoDiff>)> = state
                    .fetch
                    .iter()
                    .filter_map(|(session, fetch)| Some((session.clone(), fetch.diffs.clone()?)))
                    .collect();
                for (session, diffs) in &shown {
                    replace_diffs(&mut state.workspace, session, diffs);
                }
                let mut effects = vec![prepare(state)];
                if let Some(session) = turn_ended {
                    effects.push(self.fetch(state, &session));
                }
                return Effect::merge(effects);
            }
            ReviewAction::PickBase(base) => {
                state.base_menu = false;
                if let Some(session) = state.session.clone() {
                    state.base.insert(session.clone(), base);
                    return self.fetch(state, &session);
                }
            }
            ReviewAction::Retry => {
                if let Some(session) = state.session.clone() {
                    return self.fetch(state, &session);
                }
            }
            ReviewAction::ToggleBaseMenu => state.base_menu = !state.base_menu,
            ReviewAction::CloseBaseMenu => state.base_menu = false,
            ReviewAction::MarkReviewed => {
                state.base_menu = false;
                if let Some(session) = state.session.clone() {
                    let reply = self.daemon.mark_reviewed(&session);
                    return Effect::future(async move {
                        ReviewAction::Reviewed {
                            session,
                            result: reply.await.map_err(|error| error.to_string()),
                        }
                    });
                }
            }
            ReviewAction::Reviewed { session, result } => match result {
                Ok(()) => return self.fetch(state, &session),
                Err(error) => state.fetch.entry(session).or_default().error = Some(error),
            },
            ReviewAction::Push { repo } => {
                if let Some(session) = state.session.clone() {
                    let key = (session.clone(), repo.clone());
                    if state.pushes.get(&key) == Some(&PushState::Pushing) {
                        return Effect::none();
                    }
                    state.pushes.insert(key, PushState::Pushing);
                    return self.push(state, session, repo);
                }
            }
            ReviewAction::Pushed {
                session,
                repo,
                result,
            } => {
                let outcome = match result {
                    Ok(pull_request) => PushState::Pushed { pull_request },
                    Err(error) => PushState::Failed(error),
                };
                state.pushes.insert((session, repo), outcome);
            }
            ReviewAction::Fetched { session, base, result } => {
                let current = state.base.get(&session).cloned().unwrap_or_default();
                let fetch = state.fetch.entry(session.clone()).or_default();
                fetch.loading = false;
                let queued = std::mem::take(&mut fetch.queued);
                let mut effects = Vec::new();
                if base == current {
                    match result {
                        Ok(diffs) => {
                            fetch.error = None;
                            fetch.read_at = Some(self.clock.now());
                            fetch.diffs = Some(diffs.clone());
                            replace_diffs(&mut state.workspace, &session, &diffs);
                            effects.push(prepare(state));
                            effects.push(Effect::send(ReviewAction::Delegate(Delegate::Fetched(
                                session.clone(),
                                diffs,
                            ))));
                        }
                        Err(error) => fetch.error = Some(error),
                    }
                }
                if queued {
                    effects.push(self.fetch(state, &session));
                }
                return Effect::merge(effects);
            }
            ReviewAction::Restore(saved) => {
                state.saved = saved;
                let handovers = hand_overs(state);
                if !handovers.is_empty() {
                    return Effect::merge(
                        handovers
                            .into_iter()
                            .map(|hand| {
                                Effect::send(ReviewAction::Delegate(Delegate::HandOver(hand)))
                            })
                            .collect::<Vec<_>>(),
                    );
                }
            }
            ReviewAction::Open(session) => {
                if state.session.as_ref() != Some(&session) {
                    state.session = Some(session.clone());
                    state.rows = None;
                    state.base_menu = false;
                }
                // Opening again while its read is in flight asks for
                // nothing new: the files may not have changed since it
                // started, unlike a turn end or Mark reviewed.
                if state.fetch.get(&session).is_some_and(|fetch| fetch.loading) {
                    return prepare(state);
                }
                return Effect::merge(vec![prepare(state), self.fetch(state, &session)]);
            }
            ReviewAction::Prepared(generation, rows) => {
                if generation == state.generation {
                    state.rows = Some(rows);
                }
            }
            ReviewAction::ToggleFile(path) => {
                if let Some(session) = state.session.clone() {
                    let folded = state.saved.folded.entry(session).or_default();
                    if !folded.remove(&path) {
                        folded.insert(path);
                    }
                    return prepare(state);
                }
            }
            ReviewAction::ToggleWrap => state.wrap = !state.wrap,
            ReviewAction::HandedOver(session, comment) => forget(state, &session, &comment),
            ReviewAction::Delegate(_) => {}
        }
        Effect::none()
    }
}

/// What older builds saved, one hand-over per session: its unsent comments
/// in the order they were saved, then the one that was being written.
fn hand_overs(state: &ReviewState) -> Vec<HandOver> {
    let sessions: BTreeSet<&SessionId> = state
        .saved
        .legacy_queued
        .keys()
        .chain(state.saved.legacy_drafts.keys())
        .collect();
    sessions
        .into_iter()
        .filter_map(|session| {
            let mut comments = state
                .saved
                .legacy_queued
                .get(session)
                .cloned()
                .unwrap_or_default();
            if let Some((anchor, text)) = state.saved.legacy_drafts.get(session) {
                comments.push(draft_comment(anchor, text));
            }
            if comments.is_empty() {
                return None;
            }
            let mut repos_with_path: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for diff in state.workspace.diffs.iter().filter(|diff| &diff.session == session) {
                for file in &diff.files {
                    let repos = repos_with_path.entry(file.path.clone()).or_default();
                    if !repos.contains(&diff.repo) {
                        repos.push(diff.repo.clone());
                    }
                }
            }
            Some(HandOver {
                session: session.clone(),
                comments,
                selected_repo: state
                    .saved
                    .selected
                    .get(session)
                    .map(|(repo, _)| repo.clone()),
                repos_with_path,
            })
        })
        .collect()
}

/// A comment being written when an older build quit, as a saved comment.
fn draft_comment(anchor: &Anchor, text: &str) -> Comment {
    Comment {
        path: anchor.path.clone(),
        line: anchor.line,
        body: text.trim().to_owned(),
        sent: false,
        context: None,
    }
}

/// Drops the saved comment the daemon took from the legacy maps.
fn forget(state: &mut ReviewState, session: &SessionId, comment: &Comment) {
    if let Some(queued) = state.saved.legacy_queued.get_mut(session)
        && let Some(index) = queued.iter().position(|saved| saved == comment)
    {
        queued.remove(index);
        if queued.is_empty() {
            state.saved.legacy_queued.remove(session);
        }
        return;
    }
    if state
        .saved
        .legacy_drafts
        .get(session)
        .is_some_and(|(anchor, text)| draft_comment(anchor, text) == *comment)
    {
        state.saved.legacy_drafts.remove(session);
    }
}

/// Prepares the shown session's rows in the background.
fn prepare(state: &mut ReviewState) -> Effect<ReviewAction> {
    let Some(session) = state.session.clone() else {
        return Effect::none();
    };
    state.generation += 1;
    let generation = state.generation;
    let workspace = state.workspace.clone();
    let folded = state
        .saved
        .folded
        .get(&session)
        .cloned()
        .unwrap_or_default();
    Effect::future(async move {
        ReviewAction::Prepared(generation, Arc::new(rows(&workspace, &session, &folded)))
    })
    .cancellable("review-prepare", true)
}

/// The rows for `session`'s changes, with `folded` files closed.
#[must_use]
pub fn rows(workspace: &Workspace, session: &SessionId, folded: &BTreeSet<String>) -> Vec<DiffRow> {
    let mut rows = Vec::new();
    for diff in workspace
        .diffs
        .iter()
        .filter(|diff| &diff.session == session)
    {
        for file in &diff.files {
            let open = !folded.contains(&file.path);
            rows.push(DiffRow::File {
                path: file.path.clone(),
                added: file.added,
                removed: file.removed,
                open,
            });
            if !open {
                continue;
            }
            for line in &file.lines {
                rows.push(DiffRow::Line {
                    path: file.path.clone(),
                    line: line.clone(),
                });
            }
        }
    }
    rows
}
