//! A session's comments for its agent, as the daemon keeps them, used alike
//! by the Diff panel, the editor and Markdown source (SM-6 to SM-21, SM-13,
//! DK-9, D138, D151).
//!
//! The daemon owns drafts, statuses, threads and delivery. This reducer
//! holds a copy per session that the daemon's watch stream keeps current,
//! the comment box being written, and the user's moves: send, take back,
//! resolve, reopen, archive, drop and re-attach. What a call answers is what
//! shows; a status is never assumed.
//!
//! A box makes the daemon draft on its first non-blank text and saves later
//! text after 400 ms without typing, one write per comment in flight at a
//! time. A failed call keeps what the user wrote and asks the parent for a
//! toast.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use humata2_clients::DaemonError;
use humata2_clients::Reply;
use humata2_clients::comments::{
    Anchor, Comment, CommentEvent, CommentId, CommentsClient, Destination, Listing, Sent, Side, Status, Which,
};
use humata2_clients::Workspace;
use humata2_clients::model::{Comment as LegacyComment, SessionId};
use serde::{Deserialize, Serialize};
use tca::{Clock, Effect, Reducer};

/// How long typing pauses before a draft is saved.
const SAVE_AFTER: Duration = Duration::from_millis(400);

/// What older builds kept across restarts: which sessions showed resolved
/// comments (SM-12). Resolved comments now always show, folded as done
/// (D855), so nothing is saved; the type stays so saved files still parse.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentsSaved {
    /// The sessions that had Show resolved on. Ignored.
    #[serde(default)]
    pub show_resolved: BTreeSet<SessionId>,
}

/// What a comment box does once its text is saved.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Then {
    /// Send this comment and close the box.
    Comment,
    /// Close the box.
    Close,
    /// Drop what the box made, or give back the text it opened with.
    Cancel,
    /// Close the box, then archive the comment.
    Archive(CommentId),
    /// Close the box, then send every draft.
    SendAll,
    /// Close the box, keeping its draft, then open a new one here.
    Open(Anchor),
    /// Make the box's text a draft for a pull request instead (PR-11).
    GitHub {
        /// The repository, `owner/name`.
        repo: String,
        /// The pull request's number.
        number: u32,
    },
}

/// The box a comment is written in (SM-9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentBox {
    /// Where the comment points.
    pub anchor: Anchor,
    /// The daemon's comment, once the first text made it.
    pub comment: Option<CommentId>,
    /// What is typed.
    pub text: String,
    /// The text the comment had when the box opened; empty for a new one.
    opening: String,
    /// This box made the comment.
    created_here: bool,
    /// Makes a retry of the create make nothing more.
    key: String,
    /// The text the daemon holds for it.
    saved: String,
    /// The text of the write in flight.
    sending: String,
    /// A write is in flight.
    writing: bool,
    /// Typing paused while a write was in flight.
    due: bool,
    /// What to do once saved.
    then: Option<Then>,
    /// A stale refusal was already retried.
    retried: bool,
    /// The refusal that is being retried.
    refusal: Option<DaemonError>,
}

/// One session's comments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionComments {
    /// Its comments in the order they were made, resolved ones included,
    /// archived ones not.
    pub comments: Vec<Comment>,
    /// The daemon's count of drafts that aren't archived (SM-15).
    pub draft_count: u32,
    /// The archived ones, once asked for (SM-21).
    pub archived: Option<Vec<Comment>>,
    /// The comment being written.
    pub editing: Option<CommentBox>,
    /// What went wrong last, as text for the person.
    pub error: Option<String>,
    loaded: bool,
    boxes_made: u64,
}

/// Every session's comments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommentsState {
    /// By session.
    pub sessions: BTreeMap<SessionId, SessionComments>,
    /// Unsent overall ("as a whole") comment/request draft per session.
    pub overall_drafts: BTreeMap<SessionId, String>,
}

impl CommentsState {
    /// The unsent overall comment/request text for `session`, or `""`.
    #[must_use]
    pub fn overall_draft(&self, session: &SessionId) -> &str {
        self.overall_drafts.get(session).map_or("", String::as_str)
    }

    /// Non-archived session-level overall comments (`anchor.path.is_empty()`)
    /// for `session`, in creation order.
    #[must_use]
    pub fn overall_comments(&self, session: &SessionId) -> Vec<&Comment> {
        self.sessions
            .get(session)
            .map(|held| {
                held.comments
                    .iter()
                    .filter(|c| {
                        !c.archived
                            && c.anchor.path.is_empty()
                            && matches!(c.destination, Destination::Terminal)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    /// What the parent should keep across restarts.
    #[must_use]
    pub fn saved(&self) -> CommentsSaved {
        CommentsSaved::default()
    }

    /// The comments to draw on one file, under the line each ends on: none
    /// archived; resolved ones show as done (SM-8, D855).
    #[must_use]
    pub fn on_file(
        &self,
        session: &SessionId,
        repo: &str,
        path: &str,
    ) -> BTreeMap<u32, Vec<&Comment>> {
        let mut rows: BTreeMap<u32, Vec<&Comment>> = BTreeMap::new();
        let Some(held) = self.sessions.get(session) else {
            return rows;
        };
        for comment in &held.comments {
            let shown = !comment.archived
                && comment.anchor.repo == repo
                && comment.anchor.path == path;
            if shown {
                rows.entry(comment.anchor.last).or_default().push(comment);
            }
        }
        rows
    }

    /// The drafts for GitHub in `session`, in the order they were made: the
    /// ones Submit review would post.
    #[must_use]
    pub fn github_drafts(&self, session: &SessionId) -> Vec<&Comment> {
        self.sessions
            .get(session)
            .map(|held| held.comments.iter().filter(|c| is_github_draft(c)).collect())
            .unwrap_or_default()
    }

    /// The drafts a Send would type, in the order they were made.
    #[must_use]
    pub fn sendable(&self, session: &SessionId) -> Vec<&Comment> {
        self.sessions
            .get(session)
            .map(|held| held.comments.iter().filter(|c| c.counts_as_draft()).collect())
            .unwrap_or_default()
    }

    /// Computes the inclusive `(first, last)` line range when extending an open
    /// comment box in `session` on `(repo, path, side)` to `line` (e.g. via
    /// Shift+click). Returns `(line, line)` when no matching box is open.
    #[must_use]
    pub fn extend_box_range(
        &self,
        session: &SessionId,
        repo: &str,
        path: &str,
        side: Side,
        line: u32,
    ) -> (u32, u32) {
        self.sessions
            .get(session)
            .and_then(|held| held.editing.as_ref())
            .filter(|b| b.anchor.repo == repo && b.anchor.path == path && b.anchor.side == side)
            .map_or((line, line), |b| Anchor::extend_range(b.anchor.first, b.anchor.last, line))
    }
}

/// The pull request a comment on `repo` in `session` can be drafted for on
/// GitHub (PR-11, PR-23), or `None` when the session isn't a pull request
/// review or its pull requests are ambiguous for that file. `repo` is the
/// anchor's repository, either `owner/name` or just the name; with one pull
/// request in the review, it needs no match.
#[must_use]
pub fn github_target(workspace: &Workspace, session: &SessionId, repo: &str) -> Option<(String, u32)> {
    if !workspace.session(session).is_some_and(|found| found.pull_request_review) {
        return None;
    }
    let pulls: Vec<_> = workspace
        .pulls
        .iter()
        .filter(|pull| pull.session.as_ref() == Some(session))
        .collect();
    let by_name: Vec<_> = pulls
        .iter()
        .copied()
        .filter(|pull| humata2_clients::comments::repo_holds_pull(repo, &pull.repo))
        .collect();
    match (by_name.as_slice(), pulls.as_slice()) {
        ([pull], _) | ([], [pull]) => Some((pull.repo.clone(), pull.number)),
        _ => None,
    }
}

/// Drafts saved by older builds, to be handed to the daemon (D151).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandOver {
    /// Their session.
    pub session: SessionId,
    /// The comments, in the order they were saved.
    pub comments: Vec<LegacyComment>,
    /// The repository selected in the session's diff, if any.
    pub selected_repo: Option<String>,
    /// For each path, the repositories whose diffs have it.
    pub repos_with_path: BTreeMap<String, Vec<String>>,
}

/// What the reducer asks its parent to do.
#[derive(Clone, Debug, PartialEq)]
pub enum CommentsDelegate {
    /// Say this in a toast.
    Toast(String),
    /// Say a comment was archived, with Undo that sends `undo` back.
    ArchivedToast {
        /// The comment archived.
        comment: CommentId,
        /// The action Undo sends.
        undo: Box<CommentsAction>,
    },
    /// Show resolved changed; keep this.
    Saved(CommentsSaved),
    /// This saved draft is the daemon's now: drop it from saved state.
    HandedOver {
        /// Its session.
        session: SessionId,
        /// The saved comment.
        comment: LegacyComment,
    },
}

/// What happens to a session's comments.
#[derive(Clone, Debug, PartialEq)]
pub enum CommentsAction {
    /// Saved state came back at launch.
    Restore(CommentsSaved),
    /// Show this session's comments: list and follow them.
    Open(SessionId),
    /// A listing arrived.
    Listed(SessionId, Which, Result<Listing, DaemonError>),
    /// The daemon's stream said something.
    Event(SessionId, CommentEvent),
    /// A call failed.
    Failed(SessionId, DaemonError),

    /// Open a box for a new comment on an anchor.
    OpenBox(SessionId, Anchor),
    /// Open a box on an existing draft.
    EditDraft(SessionId, CommentId),
    /// The box's text changed.
    Typed(SessionId, String),
    /// Typing paused.
    SaveDue(SessionId),
    /// A save answered.
    Written(SessionId, Result<Comment, DaemonError>),
    /// A stale refusal made the session list again.
    Reloaded(SessionId, Result<Listing, DaemonError>),
    /// Save and send this comment, closing the box.
    Comment(SessionId),
    /// Close the box, keeping non-blank text; a blank one is cancelled.
    Close(SessionId),
    /// Close the box without keeping what it made.
    Cancel(SessionId),
    /// Make the box's text a draft for the pull request `repo` (`owner/name`)
    /// `number`, as the user, in place of a comment for the agent (PR-11).
    DraftForGitHub(SessionId, String, u32),
    /// The pull request draft was made; its box closes. The second answer is
    /// the box's earlier comment for the agent being dropped, when there was
    /// one.
    GitHubDrafted(SessionId, Result<Comment, DaemonError>, Option<(CommentId, Result<(), DaemonError>)>),
    /// Turn a note into a draft for a pull request, in place (PR-23). `pr`
    /// names the pull request when the note's repository doesn't say.
    MakeDraft(SessionId, CommentId, Option<(String, u32)>),
    /// Turn an agent's note into the person's own comment and send it to the
    /// session's agent: a comment is made from the note's lines and text,
    /// typed, and the note is dropped.
    SendNote(SessionId, CommentId),
    /// A note was sent: the comments that changed, and whether the note
    /// itself was dropped.
    NoteSent(SessionId, CommentId, Result<Vec<Comment>, DaemonError>, Result<(), DaemonError>),

    /// Update the unsent overall ("as a whole") comment/request draft for a session.
    SetOverallDraft(SessionId, String),
    /// Send every draft held (Send N), along with any overall draft.
    SendAll(SessionId),
    /// Send an unconfirmed comment's batch again.
    SendAgain(SessionId, CommentId),
    /// A send batch answered; clears `overall_drafts` on success when `had_overall` is true.
    SentBatch(SessionId, bool, Result<Vec<Comment>, DaemonError>),
    /// Waiting comments go back to drafts.
    TakeBack(SessionId, CommentId),
    /// Comments changed by one call.
    ManyChanged(SessionId, Result<Vec<Comment>, DaemonError>),
    /// Resolve a comment.
    Resolve(SessionId, CommentId),
    /// Resolve addressed comments of a file, or of the session.
    ResolveAddressed(SessionId, Option<(String, String)>),
    /// Set a resolved comment back to draft.
    Reopen(SessionId, CommentId),
    /// A comment changed by one call.
    Changed(SessionId, Result<Comment, DaemonError>),
    /// Archive a comment.
    Archive(SessionId, CommentId),
    /// Archiving answered.
    Archived(SessionId, Result<Comment, DaemonError>),
    /// Bring an archived comment back (also the toast's Undo).
    Unarchive(SessionId, CommentId),
    /// Delete a comment for good.
    Drop(SessionId, CommentId),
    /// Dropping answered.
    Dropped(SessionId, CommentId, Result<(), DaemonError>),
    /// Point a detached draft at a new passage.
    Reattach(SessionId, CommentId, Anchor),
    /// List the archived comments.
    ShowArchived(SessionId),

    /// Hand drafts saved by older builds to the daemon.
    HandOver(HandOver),
    /// A handed-over draft's create answered.
    HandedOver(SessionId, LegacyComment, Result<Comment, DaemonError>),
    /// For the parent.
    Delegate(CommentsDelegate),
}

/// The comments reducer.
#[derive(Clone)]
pub struct Comments {
    client: Arc<dyn CommentsClient>,
    clock: Arc<dyn Clock>,
}

impl Comments {
    /// A reducer that asks `client` and waits on `clock`.
    #[must_use]
    pub fn new(client: Arc<dyn CommentsClient>, clock: Arc<dyn Clock>) -> Self {
        Self { client, clock }
    }

    fn list(&self, session: &SessionId, which: Which) -> Reply<Listing> {
        self.client.list(session, which)
    }

    /// Every listing but the archived one takes resolved comments too: they
    /// fold as done on their file (D855).
    const SHOWN: Which = Which::WithResolved;

    fn save_key(session: &SessionId) -> String {
        format!("comments-save/{session}")
    }

    fn watch(&self, session: &SessionId) -> Effect<CommentsAction> {
        let call = self.client.watch(session);
        let session = session.clone();
        let id = format!("comments-watch/{session}");
        Effect::run(move |send| async move {
            match call.await {
                Err(error) => send.send(CommentsAction::Failed(session, error)),
                Ok(mut events) => {
                    while let Some(event) = events.recv().await {
                        send.ready().await;
                        send.send(CommentsAction::Event(session.clone(), event));
                    }
                }
            }
        })
        .cancellable(id, true)
    }

    fn toast(text: String) -> Effect<CommentsAction> {
        Effect::send(CommentsAction::Delegate(CommentsDelegate::Toast(text)))
    }

    /// A failed call: the text stays, the error shows and goes to a toast.
    fn fail(s: &mut SessionComments, error: &DaemonError) -> Effect<CommentsAction> {
        let text = error.to_string();
        s.error = Some(text.clone());
        Self::toast(text)
    }

    fn revision(s: &SessionComments, id: &CommentId) -> u64 {
        s.comments.iter().find(|c| &c.id == id).map_or(0, |c| c.revision)
    }

    /// Starts the box's next write, if it has one to make.
    fn start_write(&self, session: &SessionId, s: &mut SessionComments) -> Option<Effect<CommentsAction>> {
        let revision = s
            .editing
            .as_ref()
            .and_then(|b| b.comment.as_ref())
            .map_or(0, |id| Self::revision(s, id));
        let b = s.editing.as_mut()?;
        if b.writing || is_blank(&b.text) {
            return None;
        }
        let text = b.text.clone();
        let call = match &b.comment {
            None => {
                b.created_here = true;
                self.client.create(session, b.anchor.clone(), &text, &b.key)
            }
            Some(id) => {
                if text == b.saved {
                    return None;
                }
                self.client.edit(id, &text, revision)
            }
        };
        b.writing = true;
        b.due = false;
        b.sending = text;
        let session = session.clone();
        Some(Effect::future(async move { CommentsAction::Written(session, call.await) }))
    }

    /// Does what the box is waiting to do once it has nothing in flight.
    fn drive(
        &self,
        session: &SessionId,
        s: &mut SessionComments,
        overall_drafts: &BTreeMap<SessionId, String>,
    ) -> Effect<CommentsAction> {
        let Some(b) = s.editing.as_ref() else {
            return Effect::none();
        };
        if b.writing {
            return Effect::none();
        }
        if b.then == Some(Then::Cancel) {
            return self.cancel_now(session, s);
        }
        if let Some(Then::GitHub { repo, number }) = b.then.clone() {
            return self.draft_for_github(session, s, repo, number);
        }
        if b.then.is_some() || b.due {
            if let Some(effect) = self.start_write(session, s) {
                return effect;
            }
        }
        let Some(b) = s.editing.as_mut() else {
            return Effect::none();
        };
        let Some(then) = b.then.take() else {
            return Effect::none();
        };
        let comment = b.comment.clone();
        s.editing = None;
        let stop = Effect::cancel(Self::save_key(session));
        match then {
            Then::Close | Then::Cancel | Then::GitHub { .. } => stop,
            Then::Comment => match comment {
                // An agent's note, edited, goes as the person's own comment.
                Some(id) if s.comments.iter().any(|c| c.id == id && is_agent_note(c)) => {
                    stop.and(self.send_note(session, s, id))
                }
                Some(id) => stop.and(self.send(session, vec![id], None)),
                None => stop,
            },
            Then::Archive(id) => stop.and(self.archive(session, &id)),
            Then::Open(anchor) => {
                self.open_box(session, s, new_box(anchor, None, String::new()));
                stop
            }
            Then::SendAll => {
                let ids = Self::draft_ids(s);
                let overall = overall_drafts
                    .get(session)
                    .map(|text| text.trim().to_owned())
                    .filter(|text| !text.is_empty());
                stop.and(self.send(session, ids, overall))
            }
        }
    }

    /// Closes the box, dropping a draft it made or giving back its text.
    fn cancel_now(&self, session: &SessionId, s: &mut SessionComments) -> Effect<CommentsAction> {
        let Some(b) = s.editing.take() else {
            return Effect::none();
        };
        let stop = Effect::cancel(Self::save_key(session));
        let Some(id) = b.comment.clone() else {
            return stop;
        };
        let session = session.clone();
        if b.created_here {
            let call = self.client.drop_comment(&id);
            return stop.and(Effect::future(async move { CommentsAction::Dropped(session, id, call.await) }));
        }
        if b.saved != b.opening && !is_blank(&b.opening) {
            let revision = Self::revision(s, &id);
            let call = self.client.edit(&id, &b.opening, revision);
            return stop.and(Effect::future(async move { CommentsAction::Changed(session, call.await) }));
        }
        stop
    }

    /// Makes the box's text a pull request draft. The comment for the agent
    /// that autosaving made is dropped once the draft exists, so nothing is
    /// ever lost half way.
    fn draft_for_github(
        &self,
        session: &SessionId,
        s: &mut SessionComments,
        repo: String,
        number: u32,
    ) -> Effect<CommentsAction> {
        let Some(b) = s.editing.as_mut() else {
            return Effect::none();
        };
        if is_blank(&b.text) {
            b.then = None;
            return Effect::none();
        }
        b.writing = true;
        b.due = false;
        let client = self.client.clone();
        let session = session.clone();
        let (anchor, text) = (b.anchor.clone(), b.text.clone());
        let key = format!("{}/github", b.key);
        let earlier = b.comment.clone().filter(|_| b.created_here);
        let stop = Effect::cancel(Self::save_key(&session));
        stop.and(Effect::future(async move {
            let made = client
                .create_for_pull_request(&session, anchor, &text, &repo, number, &key)
                .await;
            let dropped = match (&made, earlier) {
                (Ok(_), Some(id)) => {
                    let result = client.drop_comment(&id).await;
                    Some((id, result))
                }
                _ => None,
            };
            CommentsAction::GitHubDrafted(session, made, dropped)
        }))
    }

    fn draft_ids(s: &SessionComments) -> Vec<CommentId> {
        s.comments.iter().filter(|c| c.counts_as_draft()).map(|c| c.id.clone()).collect()
    }

    fn send(
        &self,
        session: &SessionId,
        ids: Vec<CommentId>,
        overall_note: Option<String>,
    ) -> Effect<CommentsAction> {
        let overall = overall_note.filter(|text| !is_blank(text));
        if ids.is_empty() && overall.is_none() {
            return Effect::none();
        }
        let had_overall = overall.is_some();
        let call = self.client.send(session, &ids, overall.as_deref());
        let session = session.clone();
        Effect::future(async move {
            CommentsAction::SentBatch(
                session,
                had_overall,
                call.await.map(|sent: Sent| sent.comments),
            )
        })
    }

    /// Makes an agent's note the person's own comment and types it: a
    /// comment is made from the note's lines and text, sent, and the note
    /// dropped once its text is typed, so nothing is lost half way.
    fn send_note(&self, session: &SessionId, s: &SessionComments, id: CommentId) -> Effect<CommentsAction> {
        let Some(note) = s.comments.iter().find(|c| c.id == id && is_agent_note(c)) else {
            return Effect::none();
        };
        let (anchor, text) = (note.anchor.clone(), note.text.clone());
        let client = self.client.clone();
        let session = session.clone();
        let key = format!("{id}/sent");
        Effect::future(async move {
            let sent = async {
                let made = client.create(&session, anchor, &text, &key).await?;
                let sent = client
                    .send(&session, std::slice::from_ref(&made.id), None)
                    .await?;
                Ok(sent.comments)
            }
            .await;
            let dropped = match &sent {
                Ok(_) => client.drop_comment(&id).await,
                Err(_) => Ok(()),
            };
            CommentsAction::NoteSent(session, id, sent, dropped)
        })
    }

    fn archive(&self, session: &SessionId, id: &CommentId) -> Effect<CommentsAction> {
        let call = self.client.archive(id);
        let session = session.clone();
        Effect::future(async move { CommentsAction::Archived(session, call.await) })
    }

    fn changed(
        &self,
        session: &SessionId,
        call: Reply<Comment>,
    ) -> Effect<CommentsAction> {
        let session = session.clone();
        Effect::future(async move { CommentsAction::Changed(session, call.await) })
    }

    /// Opens a box, unless another is open.
    fn open_box(&self, session: &SessionId, s: &mut SessionComments, mut b: CommentBox) {
        if s.editing.is_some() {
            return;
        }
        s.boxes_made += 1;
        b.key = format!("{session}/box-{}-{}", self.clock.now().as_millis(), s.boxes_made);
        s.error = None;
        s.editing = Some(b);
    }

    fn handover_effects(&self, hand: &HandOver) -> Effect<CommentsAction> {
        let mut seen: BTreeMap<(String, String, u32, String), u64> = BTreeMap::new();
        let mut effects = Vec::new();
        for comment in &hand.comments {
            // A blank one says nothing, and a sent one was already typed:
            // neither becomes a draft.
            if comment.sent || is_blank(&comment.body) {
                effects.push(Effect::send(CommentsAction::Delegate(CommentsDelegate::HandedOver {
                    session: hand.session.clone(),
                    comment: comment.clone(),
                })));
                continue;
            }
            let known = comment
                .context
                .as_ref()
                .map(|c| c.repo.clone())
                .or_else(|| hand.selected_repo.clone())
                .or_else(|| match hand.repos_with_path.get(&comment.path).map(Vec::as_slice) {
                    Some([only]) => Some(only.clone()),
                    _ => None,
                });
            let repo = known.clone().unwrap_or_default();
            let occurrence = seen
                .entry((repo.clone(), comment.path.clone(), comment.line, comment.body.clone()))
                .or_insert(0);
            let key = handover_key(&hand.session, &repo, comment, *occurrence);
            *occurrence += 1;
            let anchor = Anchor {
                repo,
                path: comment.path.clone(),
                side: Side::New,
                first: comment.line,
                last: comment.line,
                quoted: comment
                    .context
                    .as_ref()
                    .and_then(|c| c.quoted.clone())
                    .into_iter()
                    .collect(),
                base: comment.context.as_ref().map(|c| c.base.clone()),
                detached: known.is_none(),
                outdated: false,
            };
            let call = self.client.create(&hand.session, anchor, &comment.body, &key);
            let session = hand.session.clone();
            let saved = comment.clone();
            effects.push(Effect::future(async move {
                CommentsAction::HandedOver(session, saved, call.await)
            }));
        }
        Effect::merge(effects)
    }
}

impl Reducer for Comments {
    type State = CommentsState;
    type Action = CommentsAction;

    #[allow(clippy::too_many_lines)]
    fn reduce(&self, state: &mut CommentsState, action: CommentsAction) -> Effect<CommentsAction> {
        use CommentsAction as A;
        match action {
            A::Restore(_) => Effect::none(),
            A::Open(session) => {
                let s = state.sessions.entry(session.clone()).or_default();
                s.error = None;
                let which = Self::SHOWN;
                let watch = self.watch(&session);
                let call = self.list(&session, which);
                watch.and(Effect::future(async move { A::Listed(session, which, call.await) }))
            }
            A::Listed(session, which, result) => {
                let s = state.sessions.entry(session).or_default();
                match result {
                    Ok(listing) if which == Which::Archived => {
                        s.archived = Some(listing.comments);
                        Effect::none()
                    }
                    Ok(listing) => {
                        s.comments = listing.comments;
                        s.draft_count = listing.draft_count;
                        s.loaded = true;
                        Effect::none()
                    }
                    Err(error) => Self::fail(s, &error),
                }
            }
            A::Event(session, event) => match event {
                CommentEvent::Changed(comment) => {
                    upsert(state.sessions.entry(session).or_default(), comment);
                    Effect::none()
                }
                CommentEvent::Dropped(id) => {
                    let s = state.sessions.entry(session).or_default();
                    remove(s, &id);
                    Effect::none()
                }
                CommentEvent::Resync => {
                    let which = Self::SHOWN;
                    let call = self.list(&session, which);
                    Effect::future(async move { A::Listed(session, which, call.await) })
                }
            },
            A::Failed(session, error) => Self::fail(state.sessions.entry(session).or_default(), &error),

            A::OpenBox(session, anchor) => {
                let anchor = anchor.normalized();
                let s = state.sessions.entry(session.clone()).or_default();
                // A box with no daemon comment yet on the same file and side
                // stretches or moves to the new lines and keeps its text.
                if let Some(open) = s.editing.as_mut().filter(|b| {
                    b.comment.is_none()
                        && !b.writing
                        && b.anchor.repo == anchor.repo
                        && b.anchor.path == anchor.path
                        && b.anchor.side == anchor.side
                }) {
                    open.anchor = anchor;
                    return Effect::none();
                }
                // A box holding text keeps it as a draft and the new box
                // opens once it is saved (flow 04 step 6, RV-27).
                if let Some(open) = s
                    .editing
                    .as_mut()
                    .filter(|b| b.then.is_none() && !is_blank(&b.text))
                {
                    open.then = Some(Then::Open(anchor));
                    return self.drive(&session, s, &state.overall_drafts);
                }
                self.open_box(&session, s, new_box(anchor, None, String::new()));
                Effect::none()
            }
            A::EditDraft(session, id) => {
                let s = state.sessions.entry(session.clone()).or_default();
                let Some(comment) = s
                    .comments
                    .iter()
                    .find(|c| c.id == id && (c.counts_as_draft() || is_github_draft(c) || is_agent_note(c)))
                else {
                    return Effect::none();
                };
                let b = new_box(comment.anchor.clone(), Some(id), comment.text.clone());
                self.open_box(&session, s, b);
                Effect::none()
            }
            A::Typed(session, text) => {
                let s = state.sessions.entry(session.clone()).or_default();
                let Some(b) = s.editing.as_mut() else {
                    return Effect::none();
                };
                b.text = text;
                s.error = None;
                let key = Self::save_key(&session);
                if is_blank(&b.text) {
                    return Effect::cancel(key);
                }
                if b.comment.is_none() && !b.writing {
                    return self.start_write(&session, s).unwrap_or_else(Effect::none);
                }
                Effect::send(A::SaveDue(session)).debounce(key, SAVE_AFTER, self.clock.clone())
            }
            A::SaveDue(session) => {
                let s = state.sessions.entry(session.clone()).or_default();
                match s.editing.as_mut() {
                    Some(b) if b.writing => {
                        b.due = true;
                        Effect::none()
                    }
                    Some(_) => self.start_write(&session, s).unwrap_or_else(Effect::none),
                    None => Effect::none(),
                }
            }
            A::Written(session, result) => {
                let s = state.sessions.entry(session.clone()).or_default();
                match result {
                    Ok(comment) => {
                        let id = comment.id.clone();
                        upsert(s, comment);
                        if let Some(b) = s.editing.as_mut() {
                            b.writing = false;
                            b.retried = false;
                            b.refusal = None;
                            b.saved = std::mem::take(&mut b.sending);
                            b.comment = Some(id);
                        }
                        self.drive(&session, s, &state.overall_drafts)
                    }
                    Err(error) => {
                        let Some(b) = s.editing.as_mut() else {
                            return Self::fail(s, &error);
                        };
                        b.writing = false;
                        // The daemon answers a stale revision as a conflict.
                        let conflict = matches!(error, DaemonError::Conflict { .. });
                        let stale = (conflict || matches!(error, DaemonError::Refused { .. }))
                            && b.comment.is_some()
                            && !b.retried
                            && b.then != Some(Then::Cancel);
                        if stale {
                            b.retried = true;
                            b.refusal = Some(error);
                            let which = Self::SHOWN;
                            let call = self.list(&session, which);
                            return Effect::future(async move { A::Reloaded(session, call.await) });
                        }
                        b.due = false;
                        if b.then == Some(Then::Cancel) {
                            s.editing = None;
                        } else {
                            b.then = None;
                        }
                        Self::fail(s, &error)
                    }
                }
            }
            A::Reloaded(session, result) => {
                let s = state.sessions.entry(session.clone()).or_default();
                let refusal = s.editing.as_mut().and_then(|b| b.refusal.take());
                match result {
                    Err(error) => {
                        if let Some(b) = s.editing.as_mut() {
                            b.then = None;
                        }
                        Self::fail(s, &error)
                    }
                    Ok(listing) => {
                        s.comments = listing.comments;
                        s.draft_count = listing.draft_count;
                        s.loaded = true;
                        let still_draft = s
                            .editing
                            .as_ref()
                            .and_then(|b| b.comment.as_ref())
                            .is_some_and(|id| {
                                s.comments
                                    .iter()
                                    .any(|c| &c.id == id && (c.counts_as_draft() || is_github_draft(c)))
                            });
                        if still_draft {
                            if let Some(b) = s.editing.as_mut() {
                                b.due = true;
                            }
                            return self.drive(&session, s, &state.overall_drafts);
                        }
                        if let Some(b) = s.editing.as_mut() {
                            b.then = None;
                            b.due = false;
                        }
                        match refusal {
                            Some(error) => Self::fail(s, &error),
                            None => Effect::none(),
                        }
                    }
                }
            }
            A::Comment(session) => {
                let s = state.sessions.entry(session.clone()).or_default();
                // A draft for GitHub is never typed into the terminal:
                // Comment on its box only saves and closes it.
                let github = s
                    .editing
                    .as_ref()
                    .and_then(|b| b.comment.as_ref())
                    .is_some_and(|id| s.comments.iter().any(|c| &c.id == id && is_github_draft(c)));
                let Some(b) = s.editing.as_mut() else {
                    return Effect::none();
                };
                b.then = Some(if is_blank(&b.text) {
                    Then::Cancel
                } else if github {
                    Then::Close
                } else {
                    Then::Comment
                });
                self.drive(&session, s, &state.overall_drafts)
            }
            A::DraftForGitHub(session, repo, number) => {
                let s = state.sessions.entry(session.clone()).or_default();
                let Some(b) = s.editing.as_mut() else {
                    return Effect::none();
                };
                // A draft of the user's that was already a comment for the
                // agent keeps its destination; only a box this one made can
                // become a pull request draft.
                if is_blank(&b.text) || (b.comment.is_some() && !b.created_here) {
                    return Effect::none();
                }
                b.then = Some(Then::GitHub { repo, number });
                self.drive(&session, s, &state.overall_drafts)
            }
            A::GitHubDrafted(session, made, dropped) => {
                let s = state.sessions.entry(session).or_default();
                match made {
                    Ok(comment) => {
                        upsert(s, comment);
                        s.editing = None;
                        match dropped {
                            Some((id, Ok(()))) => {
                                remove(s, &id);
                                Effect::none()
                            }
                            Some((_, Err(error))) => Self::fail(s, &error),
                            None => Effect::none(),
                        }
                    }
                    Err(error) => {
                        if let Some(b) = s.editing.as_mut() {
                            b.writing = false;
                            b.then = None;
                        }
                        Self::fail(s, &error)
                    }
                }
            }
            A::MakeDraft(session, id, pr) => {
                let call = self
                    .client
                    .make_note_a_draft(&id, pr.as_ref().map(|(repo, number)| (repo.as_str(), *number)));
                self.changed(&session, call)
            }
            A::SendNote(session, id) => {
                let s = state.sessions.entry(session.clone()).or_default();
                self.send_note(&session, s, id)
            }
            A::NoteSent(session, id, sent, dropped) => {
                let s = state.sessions.entry(session).or_default();
                match (sent, dropped) {
                    (Ok(comments), dropped) => {
                        for comment in comments {
                            upsert(s, comment);
                        }
                        match dropped {
                            Ok(()) => {
                                remove(s, &id);
                                Effect::none()
                            }
                            Err(error) => Self::fail(s, &error),
                        }
                    }
                    (Err(error), _) => Self::fail(s, &error),
                }
            }
            A::Close(session) => {
                let s = state.sessions.entry(session.clone()).or_default();
                let Some(b) = s.editing.as_mut() else {
                    return Effect::none();
                };
                b.then = Some(if is_blank(&b.text) { Then::Cancel } else { Then::Close });
                self.drive(&session, s, &state.overall_drafts)
            }
            A::Cancel(session) => {
                let s = state.sessions.entry(session.clone()).or_default();
                let Some(b) = s.editing.as_mut() else {
                    return Effect::none();
                };
                b.then = Some(Then::Cancel);
                self.drive(&session, s, &state.overall_drafts)
            }

            A::SetOverallDraft(session, text) => {
                if text.is_empty() {
                    state.overall_drafts.remove(&session);
                } else {
                    state.overall_drafts.insert(session, text);
                }
                Effect::none()
            }
            A::SendAll(session) => {
                let s = state.sessions.entry(session.clone()).or_default();
                if let Some(b) = s.editing.as_mut() {
                    b.then = Some(Then::SendAll);
                    return self.drive(&session, s, &state.overall_drafts);
                }
                let ids = Self::draft_ids(s);
                let overall = state
                    .overall_drafts
                    .get(&session)
                    .map(|text| text.trim().to_owned())
                    .filter(|text| !text.is_empty());
                self.send(&session, ids, overall)
            }
            A::SendAgain(session, id) => self.send(&session, vec![id], None),
            A::SentBatch(session, had_overall, result) => {
                let s = state.sessions.entry(session.clone()).or_default();
                match result {
                    Ok(comments) => {
                        for comment in comments {
                            upsert(s, comment);
                        }
                        if had_overall {
                            state.overall_drafts.remove(&session);
                        }
                        Effect::none()
                    }
                    Err(error) => Self::fail(s, &error),
                }
            }
            A::TakeBack(session, id) => {
                let call = self.client.take_back(&session, &[id]);
                Effect::future(async move { A::ManyChanged(session, call.await) })
            }
            A::ManyChanged(session, result) => {
                let s = state.sessions.entry(session).or_default();
                match result {
                    Ok(comments) => {
                        for comment in comments {
                            upsert(s, comment);
                        }
                        Effect::none()
                    }
                    Err(error) => Self::fail(s, &error),
                }
            }
            A::Resolve(session, id) => {
                let s = state.sessions.entry(session.clone()).or_default();
                let call = self.client.resolve(&id, Self::revision(s, &id));
                self.changed(&session, call)
            }
            A::ResolveAddressed(session, file) => {
                let call = self
                    .client
                    .resolve_addressed(&session, file.as_ref().map(|(r, p)| (r.as_str(), p.as_str())));
                Effect::future(async move { A::ManyChanged(session, call.await) })
            }
            A::Reopen(session, id) => {
                let s = state.sessions.entry(session.clone()).or_default();
                let call = self.client.reopen(&id, Self::revision(s, &id));
                self.changed(&session, call)
            }
            A::Reattach(session, id, anchor) => {
                let anchor = anchor.normalized();
                let s = state.sessions.entry(session.clone()).or_default();
                let call = self.client.reattach(&id, anchor, Self::revision(s, &id));
                self.changed(&session, call)
            }
            A::Changed(session, result) => {
                let s = state.sessions.entry(session).or_default();
                match result {
                    Ok(comment) => {
                        upsert(s, comment);
                        Effect::none()
                    }
                    Err(error) => Self::fail(s, &error),
                }
            }
            A::Archive(session, id) => {
                let s = state.sessions.entry(session.clone()).or_default();
                if let Some(b) = s.editing.as_mut() {
                    b.then = Some(Then::Archive(id));
                    return self.drive(&session, s, &state.overall_drafts);
                }
                self.archive(&session, &id)
            }
            A::Archived(session, result) => {
                let s = state.sessions.entry(session.clone()).or_default();
                match result {
                    Ok(comment) => {
                        let id = comment.id.clone();
                        upsert(s, comment);
                        Effect::send(A::Delegate(CommentsDelegate::ArchivedToast {
                            comment: id.clone(),
                            undo: Box::new(A::Unarchive(session, id)),
                        }))
                    }
                    Err(error) => Self::fail(s, &error),
                }
            }
            A::Unarchive(session, id) => {
                let call = self.client.unarchive(&id);
                self.changed(&session, call)
            }
            A::Drop(session, id) => {
                let call = self.client.drop_comment(&id);
                Effect::future(async move { A::Dropped(session, id, call.await) })
            }
            A::Dropped(session, id, result) => {
                let s = state.sessions.entry(session).or_default();
                match result {
                    Ok(()) => {
                        remove(s, &id);
                        Effect::none()
                    }
                    Err(error) => Self::fail(s, &error),
                }
            }
            A::ShowArchived(session) => {
                let call = self.list(&session, Which::Archived);
                Effect::future(async move { A::Listed(session, Which::Archived, call.await) })
            }

            A::HandOver(hand) => self.handover_effects(&hand),
            A::HandedOver(session, saved, result) => match result {
                Ok(comment) => {
                    upsert(state.sessions.entry(session.clone()).or_default(), comment);
                    Effect::send(A::Delegate(CommentsDelegate::HandedOver { session, comment: saved }))
                }
                Err(DaemonError::NotFound { .. }) => {
                    Effect::send(A::Delegate(CommentsDelegate::HandedOver { session, comment: saved }))
                }
                Err(error) => Self::fail(state.sessions.entry(session).or_default(), &error),
            },
            A::Delegate(_) => Effect::none(),
        }
    }
}

/// Whether `comment` is a draft for GitHub that is still to be submitted.
fn is_github_draft(comment: &Comment) -> bool {
    comment.status == Status::Draft
        && !comment.archived
        && matches!(comment.destination, Destination::PullRequest { .. })
}

/// Whether `comment` is a note an agent left that the person hasn't dealt
/// with yet.
pub fn is_agent_note(comment: &Comment) -> bool {
    comment.status == Status::Draft
        && !comment.archived
        && comment.destination == Destination::Note
        && matches!(comment.author, humata2_clients::comments::Author::Agent(_))
}

fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

fn new_box(anchor: Anchor, comment: Option<CommentId>, text: String) -> CommentBox {
    CommentBox {
        anchor,
        comment,
        opening: text.clone(),
        saved: text.clone(),
        text,
        created_here: false,
        key: String::new(),
        sending: String::new(),
        writing: false,
        due: false,
        then: None,
        retried: false,
        refusal: None,
    }
}

/// The number after `c-` in an id, to keep comments in made order.
fn made_number(id: &CommentId) -> u64 {
    id.as_str()
        .rsplit_once("/c-")
        .and_then(|(_, n)| n.parse().ok())
        .unwrap_or(u64::MAX)
}

fn upsert_in(list: &mut Vec<Comment>, comment: Comment) {
    match list.iter_mut().find(|held| held.id == comment.id) {
        Some(held) => {
            if comment.revision >= held.revision {
                *held = comment;
            }
        }
        None => {
            list.push(comment);
            list.sort_by_key(|held| made_number(&held.id));
        }
    }
}

fn recount(s: &mut SessionComments) {
    if s.loaded {
        s.draft_count = u32::try_from(s.comments.iter().filter(|c| c.counts_as_draft()).count())
            .unwrap_or(u32::MAX);
    }
}

/// Applies a changed comment: an archived one leaves the held list.
fn upsert(s: &mut SessionComments, comment: Comment) {
    if comment.archived {
        s.comments.retain(|held| held.id != comment.id);
        if let Some(list) = s.archived.as_mut() {
            upsert_in(list, comment);
        }
    } else {
        if let Some(list) = s.archived.as_mut() {
            list.retain(|held| held.id != comment.id);
        }
        upsert_in(&mut s.comments, comment);
    }
    recount(s);
}

fn remove(s: &mut SessionComments, id: &CommentId) {
    s.comments.retain(|held| &held.id != id);
    if let Some(list) = s.archived.as_mut() {
        list.retain(|held| &held.id != id);
    }
    recount(s);
}

/// FNV-1a over the fields, so the key is the same on every run and build.
fn handover_key(session: &SessionId, repo: &str, comment: &LegacyComment, occurrence: u64) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes.iter().chain(&[0xff]) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    feed(session.as_str().as_bytes());
    feed(repo.as_bytes());
    feed(comment.path.as_bytes());
    feed(&comment.line.to_le_bytes());
    feed(comment.body.as_bytes());
    feed(&occurrence.to_le_bytes());
    format!("handover-{hash:016x}")
}

/// Number of queued GitHub reply drafts for `session`.
#[must_use]
pub fn queued_github_reply_count(state: &CommentsState, session: &SessionId) -> usize {
    state.github_drafts(session).len()
}

/// Footer CTA label for posting queued GitHub replies.
#[must_use]
pub fn post_replies_cta_label(count: usize) -> String {
    if count == 1 {
        "Post 1 reply to GitHub".to_owned()
    } else {
        format!("Post {count} replies to GitHub")
    }
}

/// Review status of a pulled GitHub PR feedback item (`ATM-38`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrReviewStatus {
    /// Reviewer requested changes.
    RequestedChanges,
    /// Reviewer left comments.
    Commented,
    /// Reviewer approved.
    Approved,
}

impl PrReviewStatus {
    /// Human-readable status label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::RequestedChanges => "requested changes",
            Self::Commented => "commented",
            Self::Approved => "approved",
        }
    }
}

/// View model for an on-demand Pull PR Feedback card (`ATM-38`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrFeedbackCardModel {
    /// Reviewer handle or display name (e.g. `"@maya"`).
    pub reviewer: String,
    /// Review status badge.
    pub status: PrReviewStatus,
    /// Repository name (e.g. `"api"` or `"acme/api"`).
    pub repo: String,
    /// Pull request number.
    pub pr_number: u32,
    /// Feedback body text.
    pub body: String,
    /// Optional file path & line reference.
    pub location: Option<String>,
    /// Whether this feedback item is selected for batch send.
    pub selected: bool,
    /// Optional queued GitHub reply draft text.
    pub queued_reply: Option<String>,
}

impl PrFeedbackCardModel {
    /// Short PR link chip label, e.g. `"api #142"`.
    #[must_use]
    pub fn pr_link_label(&self) -> String {
        let short = self.repo.rsplit('/').next().unwrap_or(&self.repo);
        format!("{short} #{}", self.pr_number)
    }

    /// Formats this feedback item for `Send to <agent>`.
    #[must_use]
    pub fn format_for_agent(&self) -> String {
        let loc = self
            .location
            .as_deref()
            .map(|l| format!(" ({l})"))
            .unwrap_or_default();
        format!(
            "PR feedback from {} ({}) on {}{loc}:\n{}",
            self.reviewer,
            self.status.label(),
            self.pr_link_label(),
            self.body.trim()
        )
    }
}
