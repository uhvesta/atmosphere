//! Reviewing a session's changes (RV-1–27): files fold, lines wrap or
//! scroll, and comments on lines are kept as drafts. Each can be sent on its own, or all
//! together; an explicit send is delivered without waiting for a turn to end
//! (RV-4, RV-6, SE-23).
//!
//! The rows the view draws are prepared off the UI thread by an effect and
//! arrive ready to draw ([`ReviewAction::Prepared`]); a newer request
//! replaces an older one still in flight.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use humata2_clients::model::{Comment, CommentContext, DiffLine, LineKind, SessionId};
use humata2_clients::{DaemonError, Workspace};
use serde::{Deserialize, Serialize};
use tca::{Effect, Reducer};

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
    /// A comment under the line it is about.
    Comment {
        /// The file.
        path: String,
        /// The line in the new file.
        line: u32,
        /// What it says.
        body: String,
        /// Whether it was sent.
        sent: bool,
    },
}

/// Where a comment goes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Anchor {
    /// The file.
    pub path: String,
    /// The line in the new file.
    pub line: u32,
}

/// What survives a restart: folded files and unsent comments.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewSaved {
    /// The selected repository and file, per session.
    #[serde(default)]
    pub selected: BTreeMap<SessionId, (String, String)>,
    /// Folded files per session.
    pub folded: BTreeMap<SessionId, BTreeSet<String>>,
    /// Comments written but not sent, per session.
    pub queued: BTreeMap<SessionId, Vec<Comment>>,
    /// A comment being written, per session.
    pub drafts: BTreeMap<SessionId, (Anchor, String)>,
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
    /// Comments are on their way to the agent.
    pub sending: bool,
    /// What went wrong last.
    pub error: Option<String>,
}

impl ReviewState {
    /// The comments waiting to be sent for the shown session.
    #[must_use]
    pub fn queued(&self) -> &[Comment] {
        self.session
            .as_ref()
            .and_then(|id| self.saved.queued.get(id))
            .map_or(&[], Vec::as_slice)
    }

    /// The comment being written for the shown session.
    #[must_use]
    pub fn draft(&self) -> Option<&(Anchor, String)> {
        self.session
            .as_ref()
            .and_then(|id| self.saved.drafts.get(id))
    }
}

/// What the review page asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Deliver this batch to the session's terminal now (SE-23); the session
    /// does not wait for a turn to end. The batch leaves the drafts only once
    /// the session has retained it.
    QueueComments(SessionId, Vec<Comment>),
    /// Say this in a toast.
    Toast(String),
}

/// What happens on the review page.
#[derive(Clone, Debug, PartialEq)]
pub enum ReviewAction {
    /// Show a file from the open session's comparison.
    SelectFile { repo: String, path: String },
    /// The data changed.
    Show(Arc<Workspace>),
    /// Saved folds and comments came back at launch.
    Restore(ReviewSaved),
    /// Show this session's changes.
    Open(SessionId),
    /// Rows were prepared.
    Prepared(u64, Arc<Vec<DiffRow>>),
    /// A file header was clicked.
    ToggleFile(String),
    /// Wrap long lines, or not.
    ToggleWrap,
    /// Start a comment on a line.
    StartComment(Anchor),
    /// The comment's text changed.
    CommentText(String),
    /// Add the comment to the queue.
    SaveComment,
    /// Drop the comment being written.
    CancelComment,
    /// Drop a queued comment.
    RemoveComment(usize),
    /// Send every saved draft of the shown session to the agent together.
    Send,
    /// Send only the saved draft at this index; the others stay drafts.
    SendDraft(usize),
    /// The box's "Comment" button: save the comment being written and send
    /// just that one (SM-9). Other drafts stay.
    SendComment,
    /// The parent retained this batch in the session queue.
    Queued(SessionId, Vec<Comment>),
    /// A queued batch was removed and can be edited again.
    Unqueued(SessionId, Vec<Comment>),
    /// The parent could not retain the batch; the drafts remain.
    QueueFailed(DaemonError),
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
            ReviewAction::Restore(saved) => state.saved = saved,
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
            ReviewAction::StartComment(anchor) => {
                if let Some(session) = state.session.clone() {
                    state.saved.drafts.insert(session, (anchor, String::new()));
                }
            }
            ReviewAction::CommentText(text) => {
                if let Some(session) = &state.session
                    && let Some(draft) = state.saved.drafts.get_mut(session)
                {
                    draft.1 = text;
                }
            }
            ReviewAction::SaveComment => {
                if let Some((session, comment)) = take_comment(state) {
                    state.saved.queued.entry(session).or_default().push(comment);
                    return prepare(state);
                }
            }
            ReviewAction::SendComment => {
                if state.sending {
                    return Effect::none();
                }
                if let Some((session, comment)) = take_comment(state) {
                    // The comment stays a draft until the session has it, so a
                    // failed hand-over loses nothing.
                    state
                        .saved
                        .queued
                        .entry(session.clone())
                        .or_default()
                        .push(comment.clone());
                    state.sending = true;
                    return Effect::merge(vec![
                        prepare(state),
                        Effect::send(ReviewAction::Delegate(Delegate::QueueComments(
                            session,
                            vec![comment],
                        ))),
                    ]);
                }
            }
            ReviewAction::CancelComment => {
                if let Some(session) = &state.session {
                    state.saved.drafts.remove(session);
                }
            }
            ReviewAction::RemoveComment(index) => {
                if state.sending {
                    return Effect::none();
                }
                if let Some(session) = &state.session
                    && let Some(queued) = state.saved.queued.get_mut(session)
                    && index < queued.len()
                {
                    queued.remove(index);
                    return prepare(state);
                }
            }
            ReviewAction::Send => {
                let Some(session) = state.session.clone() else {
                    return Effect::none();
                };
                let comments = state.queued().to_vec();
                if comments.is_empty() || state.sending {
                    return Effect::none();
                }
                state.sending = true;
                return Effect::send(ReviewAction::Delegate(Delegate::QueueComments(
                    session, comments,
                )));
            }
            ReviewAction::SendDraft(index) => {
                if state.sending {
                    return Effect::none();
                }
                let Some(session) = state.session.clone() else {
                    return Effect::none();
                };
                let Some(comment) = state.queued().get(index).cloned() else {
                    return Effect::none();
                };
                state.sending = true;
                return Effect::send(ReviewAction::Delegate(Delegate::QueueComments(
                    session,
                    vec![comment],
                )));
            }
            ReviewAction::Queued(session, comments) => {
                state.sending = false;
                state.error = None;
                if let Some(drafts) = state.saved.queued.get_mut(&session) {
                    for comment in comments {
                        if let Some(index) = drafts.iter().position(|draft| *draft == comment) {
                            drafts.remove(index);
                        }
                    }
                }
                return prepare(state);
            }
            ReviewAction::Unqueued(session, comments) => {
                state
                    .saved
                    .queued
                    .entry(session)
                    .or_default()
                    .extend(comments);
                return prepare(state);
            }
            ReviewAction::QueueFailed(error) => {
                state.sending = false;
                state.error = Some(error.to_string());
                return Effect::send(ReviewAction::Delegate(Delegate::Toast(error.to_string())));
            }
            ReviewAction::Delegate(_) => {}
        }
        Effect::none()
    }
}

/// Takes the comment being written for the shown session out of its box and
/// turns it into a [`Comment`] with the context of the line it is about.
/// Nothing happens for an empty body (the box is still closed).
fn take_comment(state: &mut ReviewState) -> Option<(SessionId, Comment)> {
    let session = state.session.clone()?;
    let (anchor, body) = state.saved.drafts.remove(&session)?;
    if body.trim().is_empty() {
        return None;
    }
    let selected_repo = state.saved.selected.get(&session).map(|(repo, _)| repo);
    let context = state
        .workspace
        .diffs
        .iter()
        .filter(|diff| diff.session == session)
        .find_map(|diff| {
            if selected_repo.is_some_and(|repo| *repo != diff.repo) {
                return None;
            }
            let file = diff.files.iter().find(|file| file.path == anchor.path)?;
            Some(CommentContext {
                repo: diff.repo.clone(),
                base: diff.base.clone(),
                quoted: file
                    .lines
                    .iter()
                    .find(|line| line.new == Some(anchor.line))
                    .map(|line| line.text.clone()),
            })
        });
    Some((
        session,
        Comment {
            path: anchor.path,
            line: anchor.line,
            body: body.trim().to_owned(),
            sent: false,
            context,
        },
    ))
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
    let queued = state
        .saved
        .queued
        .get(&session)
        .cloned()
        .unwrap_or_default();
    Effect::future(async move {
        ReviewAction::Prepared(
            generation,
            Arc::new(rows(&workspace, &session, &folded, &queued)),
        )
    })
    .cancellable("review-prepare", true)
}

/// The rows for `session`'s changes, with `folded` files closed and
/// `queued` comments under their lines.
#[must_use]
pub fn rows(
    workspace: &Workspace,
    session: &SessionId,
    folded: &BTreeSet<String>,
    queued: &[Comment],
) -> Vec<DiffRow> {
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
                let Some(number) = line.new.filter(|_| line.kind != LineKind::Hunk) else {
                    continue;
                };
                for comment in diff.comments.iter().chain(queued) {
                    if comment.path == file.path
                        && comment.line == number
                        && comment
                            .context
                            .as_ref()
                            .is_none_or(|context| context.repo == diff.repo)
                    {
                        rows.push(DiffRow::Comment {
                            path: file.path.clone(),
                            line: number,
                            body: comment.body.clone(),
                            sent: comment.sent,
                        });
                    }
                }
            }
        }
    }
    rows
}
