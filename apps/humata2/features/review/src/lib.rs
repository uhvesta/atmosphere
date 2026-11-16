//! Reviewing a session's changes (RV-1–27): files fold, and lines wrap or
//! scroll. Comments on lines belong to the daemon and live in [`comments`];
//! this page only hands over what older builds saved here (D151).
//!
//! The rows the view draws are prepared off the UI thread by an effect and
//! arrive ready to draw ([`ReviewAction::Prepared`]); a newer request
//! replaces an older one still in flight.

pub mod comments;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use humata2_clients::Workspace;
use humata2_clients::model::{Comment, DiffLine, SessionId};
use serde::{Deserialize, Serialize};
use tca::{Effect, Reducer};

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
}

/// What the review page asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// These comments older builds saved are the daemon's to keep now.
    HandOver(HandOver),
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
    /// For the parent.
    Delegate(Delegate),
}

/// The review page's reducer.
#[derive(Clone)]
pub struct Review;

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
                state.workspace = workspace;
                return prepare(state);
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
                    state.session = Some(session);
                    state.rows = None;
                }
                return prepare(state);
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
