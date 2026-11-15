//! Reviewing a session's changes (RV-1–27): files fold, lines wrap or
//! scroll, and comments on lines queue up until they are sent to the agent
//! together (RV-4, RV-6).
//!
//! The rows the view draws are prepared off the UI thread by an effect and
//! arrive ready to draw ([`ReviewAction::Prepared`]); a newer request
//! replaces an older one still in flight.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use humata2_clients::model::{Comment, DiffLine, LineKind, SessionId};
use humata2_clients::{Clients, DaemonError, Workspace};
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
    /// Say this in a toast.
    Toast(String),
}

/// What happens on the review page.
#[derive(Clone, Debug, PartialEq)]
pub enum ReviewAction {
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
    /// Send every queued comment to the agent.
    Send,
    /// The daemon took them, or didn't.
    Sent(SessionId, Result<(), DaemonError>),
    /// For the parent.
    Delegate(Delegate),
}

/// The review page's reducer.
#[derive(Clone)]
pub struct Review {
    clients: Clients,
}

impl Review {
    /// A review page that sends comments with `clients`.
    #[must_use]
    pub const fn new(clients: Clients) -> Self {
        Self { clients }
    }
}

impl Reducer for Review {
    type State = ReviewState;
    type Action = ReviewAction;

    fn reduce(&self, state: &mut ReviewState, action: ReviewAction) -> Effect<ReviewAction> {
        match action {
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
                if let Some(session) = state.session.clone()
                    && let Some((anchor, body)) = state.saved.drafts.remove(&session)
                    && !body.trim().is_empty()
                {
                    state.saved.queued.entry(session).or_default().push(Comment {
                        path: anchor.path,
                        line: anchor.line,
                        body: body.trim().to_owned(),
                        sent: false,
                    });
                    return prepare(state);
                }
            }
            ReviewAction::CancelComment => {
                if let Some(session) = &state.session {
                    state.saved.drafts.remove(session);
                }
            }
            ReviewAction::RemoveComment(index) => {
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
                let daemon = self.clients.daemon.clone();
                return Effect::future(async move {
                    let result = daemon.send_comments(&session, comments).await;
                    ReviewAction::Sent(session, result)
                });
            }
            ReviewAction::Sent(session, result) => {
                state.sending = false;
                match result {
                    Ok(()) => {
                        let count = state.saved.queued.remove(&session).map_or(0, |queued| queued.len());
                        state.error = None;
                        return Effect::send(ReviewAction::Delegate(Delegate::Toast(format!(
                            "Sent {count} comment{} to the agent.",
                            if count == 1 { "" } else { "s" }
                        ))));
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
            }
            ReviewAction::Delegate(_) => {}
        }
        Effect::none()
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
    let folded = state.saved.folded.get(&session).cloned().unwrap_or_default();
    let queued = state.saved.queued.get(&session).cloned().unwrap_or_default();
    Effect::future(async move {
        ReviewAction::Prepared(generation, Arc::new(rows(&workspace, &session, &folded, &queued)))
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
    for diff in workspace.diffs.iter().filter(|diff| &diff.session == session) {
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
                    if comment.path == file.path && comment.line == number {
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
