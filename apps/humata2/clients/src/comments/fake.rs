//! A scripted [`CommentsClient`]: it keeps comments the way the daemon's
//! ledger does (SM-6 to SM-21), types the message of SM-9 into a recorded
//! list instead of a terminal, and lets a test play the agent and the file
//! watcher. No timers, no real IO.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::mpsc;

use super::{
    Anchor, Author, Comment, CommentEvent, CommentEvents, CommentId, CommentsClient, Listing, Side,
    Sent, Status, ThreadReply, Which,
};
use crate::daemon::{DaemonError, Reply};
use crate::model::SessionId;

/// Most quoted lines a message carries per comment (SM-9).
const QUOTE_LIMIT: usize = 12;

/// What typing into a session's main agent does on the next sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Typing {
    /// The terminal takes the message: comments become sent.
    #[default]
    Typed,
    /// No terminal, or its shell exited: comments wait, nothing is typed.
    NotTyped,
    /// Humata restarted mid-send: comments wait as unconfirmed, nothing is
    /// recorded as typed.
    Unknown,
}

/// A call a test can read back, one per trait method.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    /// `list`.
    List(SessionId, Which),
    /// `watch`.
    Watch(SessionId),
    /// `create`, with the idempotency key last.
    Create(SessionId, Anchor, String, String),
    /// `edit`.
    Edit(CommentId, String, u64),
    /// `drop_comment`.
    Drop(CommentId),
    /// `reattach`.
    Reattach(CommentId, Anchor, u64),
    /// `send`.
    Send(SessionId, Vec<CommentId>),
    /// `take_back`.
    TakeBack(SessionId, Vec<CommentId>),
    /// `resolve`.
    Resolve(CommentId, u64),
    /// `resolve_addressed`.
    ResolveAddressed(SessionId, Option<(String, String)>),
    /// `reopen`.
    Reopen(CommentId, u64),
    /// `archive`.
    Archive(CommentId),
    /// `unarchive`.
    Unarchive(CommentId),
}

struct SessionState {
    repos: usize,
    agent: String,
    typing: Typing,
    comments: Vec<Comment>,
    next: u32,
    keys: HashMap<String, CommentId>,
    /// The unconfirmed batch each waiting comment was in.
    batches: HashMap<CommentId, u32>,
    next_batch: u32,
    typed: Vec<String>,
    /// Comments that were typed at least once; only these take the agent's
    /// reply or resolve, whatever their status now.
    delivered: HashSet<CommentId>,
    watchers: Vec<mpsc::UnboundedSender<CommentEvent>>,
}

impl SessionState {
    fn new() -> Self {
        Self {
            repos: 1,
            agent: "claude".to_owned(),
            typing: Typing::Typed,
            comments: Vec::new(),
            next: 1,
            keys: HashMap::new(),
            batches: HashMap::new(),
            next_batch: 1,
            typed: Vec::new(),
            delivered: HashSet::new(),
            watchers: Vec::new(),
        }
    }

    fn emit(&mut self, event: &CommentEvent) {
        self.watchers.retain(|watcher| watcher.send(event.clone()).is_ok());
    }

    fn position(&self, id: &CommentId) -> Option<usize> {
        self.comments.iter().position(|comment| &comment.id == id)
    }
}

#[derive(Default)]
struct Inner {
    sessions: HashMap<SessionId, SessionState>,
    failures: HashMap<&'static str, DaemonError>,
    calls: Vec<Call>,
}

impl Inner {
    fn session(&mut self, session: &SessionId) -> &mut SessionState {
        self.sessions.entry(session.clone()).or_insert_with(SessionState::new)
    }

    /// The session and position of a comment.
    fn locate(&self, id: &CommentId) -> Result<(SessionId, usize), DaemonError> {
        for (session, state) in &self.sessions {
            if let Some(index) = state.position(id) {
                return Ok((session.clone(), index));
            }
        }
        Err(gone())
    }

    /// Applies `change` to a copy of the comment; when it succeeds the
    /// revision goes up, the copy is stored and watchers hear of it.
    fn change(
        &mut self,
        id: &CommentId,
        change: impl FnOnce(&mut Comment, &SessionState) -> Result<(), DaemonError>,
    ) -> Result<Comment, DaemonError> {
        let (session, index) = self.locate(id)?;
        let state = self.session(&session);
        let mut comment = state.comments[index].clone();
        change(&mut comment, state)?;
        comment.revision += 1;
        state.comments[index] = comment.clone();
        state.emit(&CommentEvent::Changed(comment.clone()));
        Ok(comment)
    }
}

fn gone() -> DaemonError {
    DaemonError::NotFound { what: "This comment".to_owned() }
}

fn refused(what: &str, reason: &str) -> DaemonError {
    DaemonError::Refused { what: what.to_owned(), reason: reason.to_owned() }
}

fn check_revision(comment: &Comment, revision: u64) -> Result<(), DaemonError> {
    if comment.revision == revision {
        Ok(())
    } else {
        Err(refused(
            "This change",
            "the comment changed since you opened it; reopen it to see the latest and try again",
        ))
    }
}

fn not_archived(comment: &Comment) -> Result<(), DaemonError> {
    if comment.archived {
        Err(refused("This comment", "it is archived; unarchive it first"))
    } else {
        Ok(())
    }
}

fn is_typed(comment: &Comment, state: &SessionState) -> bool {
    state.delivered.contains(&comment.id)
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Old => "old",
        Side::New => "new",
    }
}

/// One comment of a message (SM-9).
fn message_part(comment: &Comment, show_repo: bool) -> String {
    let anchor = &comment.anchor;
    let mut out = String::from("[comment on ");
    // A document of the session's own folder has no repo to lead with.
    if show_repo && !anchor.repo.is_empty() {
        out.push_str(&anchor.repo);
        out.push('/');
    }
    out.push_str(&anchor.path);
    if anchor.first != 0 || anchor.last != 0 {
        if anchor.first == anchor.last {
            out.push_str(&format!(":{}", anchor.first));
        } else {
            out.push_str(&format!(":{}-{}", anchor.first, anchor.last));
        }
    }
    if let Some(base) = &anchor.base {
        out.push_str(" · ");
        out.push_str(side_name(anchor.side));
        if !base.is_empty() {
            out.push_str(" · base ");
            out.push_str(base);
        }
    }
    out.push_str("]\n");
    if anchor.base.is_some() {
        for line in anchor.quoted.iter().take(QUOTE_LIMIT) {
            out.push_str("> ");
            out.push_str(line);
            out.push('\n');
        }
        if anchor.quoted.len() > QUOTE_LIMIT {
            out.push_str(&format!("> ({} more lines)\n", anchor.quoted.len() - QUOTE_LIMIT));
        }
    }
    out.push_str(comment.text.trim_end());
    out
}

/// A comments client that does what a test scripts, and that tests can
/// play the agent and the file watcher against.
#[derive(Clone, Default)]
pub struct FakeComments {
    shared: Arc<Mutex<Inner>>,
}

impl FakeComments {
    /// A fake with no comments, where every session has one repository and
    /// the agent `claude`, and sends are typed.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The repositories a session has; messages lead a path with its repo
    /// only when there are two or more (SM-9).
    pub fn set_repos(&self, session: &SessionId, repos: &[&str]) {
        self.inner().session(session).repos = repos.len();
    }

    /// The agent's name, used when it resolves a comment.
    pub fn set_agent(&self, session: &SessionId, agent: &str) {
        agent.clone_into(&mut self.inner().session(session).agent);
    }

    /// What the next sends of the session do.
    pub fn set_typing(&self, session: &SessionId, typing: Typing) {
        self.inner().session(session).typing = typing;
    }

    /// Every message typed into the session's main agent, oldest first;
    /// one per batch.
    #[must_use]
    pub fn typed(&self, session: &SessionId) -> Vec<String> {
        self.inner().sessions.get(session).map(|state| state.typed.clone()).unwrap_or_default()
    }

    /// Every comment the fake holds for the session, archived ones
    /// included, in the order they were made.
    #[must_use]
    pub fn comments(&self, session: &SessionId) -> Vec<Comment> {
        self.inner().sessions.get(session).map(|state| state.comments.clone()).unwrap_or_default()
    }

    /// Makes calls of `method` fail with `error` until cleared. Names are
    /// the trait's method names, such as `"send"` or `"edit"`.
    pub fn fail(&self, method: &'static str, error: DaemonError) {
        self.inner().failures.insert(method, error);
    }

    /// Stops failing calls of `method`.
    pub fn clear_failure(&self, method: &'static str) {
        self.inner().failures.remove(method);
    }

    /// The calls made so far, oldest first.
    #[must_use]
    pub fn calls(&self) -> Vec<Call> {
        self.inner().calls.clone()
    }

    /// The agent answers over its comment tools: only a comment that was
    /// typed, and not archived, takes a reply (DK-9).
    ///
    /// # Errors
    /// Not found, or refused when the comment was never sent or is archived.
    pub fn agent_reply(&self, id: &CommentId, text: &str) -> Result<Comment, DaemonError> {
        self.inner().change(id, |comment, state| {
            not_archived(comment)?;
            if !is_typed(comment, state) {
                return Err(refused("This reply", "the comment was never sent to the agent"));
            }
            if text.trim().is_empty() {
                return Err(refused("This reply", "it is empty; say something in it"));
            }
            let id = format!("r-{}", comment.replies.len() + 1);
            comment.replies.push(ThreadReply {
                id,
                author: Author::Agent(state.agent.clone()),
                text: text.to_owned(),
            });
            Ok(())
        })
    }

    /// The agent resolves a comment with a reply (SM-19).
    ///
    /// # Errors
    /// Not found, or refused when there is no reply, the comment was never
    /// sent, is resolved already, or is archived.
    pub fn agent_resolve(&self, id: &CommentId, reply: &str) -> Result<Comment, DaemonError> {
        self.inner().change(id, |comment, state| {
            not_archived(comment)?;
            if !is_typed(comment, state) {
                return Err(refused("This resolve", "the comment was never sent to the agent"));
            }
            if comment.status == Status::Resolved {
                return Err(refused("This resolve", "the comment is resolved already"));
            }
            if reply.trim().is_empty() {
                return Err(refused("This resolve", "it needs a reply that says what was done"));
            }
            let agent = Author::Agent(state.agent.clone());
            comment.replies.push(ThreadReply {
                id: format!("r-{}", comment.replies.len() + 1),
                author: agent.clone(),
                text: reply.to_owned(),
            });
            comment.status = Status::Resolved;
            comment.resolved_by = Some(agent);
            Ok(())
        })
    }

    /// The file watcher saw the anchored lines change beyond whitespace to
    /// `now` (empty when deleted). Sent comments become addressed; drafts
    /// and waiting ones detach; resolved and addressed ones stay (SM-16,
    /// SM-17, SM-18).
    ///
    /// # Errors
    /// Not found.
    pub fn lines_changed(&self, id: &CommentId, now: &[&str]) -> Result<Comment, DaemonError> {
        let mut inner = self.inner();
        let (session, index) = inner.locate(id)?;
        let status = inner.session(&session).comments[index].status;
        match status {
            Status::Sent => inner.change(id, |comment, _| {
                comment.status = Status::Addressed;
                comment.addressed = Some(now.iter().map(|line| (*line).to_owned()).collect());
                Ok(())
            }),
            Status::Draft | Status::Waiting => inner.change(id, |comment, _| {
                comment.anchor.detached = true;
                Ok(())
            }),
            Status::Addressed | Status::Resolved => {
                Ok(inner.session(&session).comments[index].clone())
            }
        }
    }

    /// The passage moved unchanged: its lines are re-anchored and its
    /// status kept (SM-7, SM-16).
    ///
    /// # Errors
    /// Not found.
    pub fn moved(&self, id: &CommentId, first: u32, last: u32) -> Result<Comment, DaemonError> {
        self.inner().change(id, |comment, _| {
            comment.anchor.first = first;
            comment.anchor.last = last;
            Ok(())
        })
    }

    /// Every watch misses events: each hears `Resync`.
    pub fn drop_events(&self) {
        for state in self.inner().sessions.values_mut() {
            state.emit(&CommentEvent::Resync);
        }
    }

    fn call<T: Send + 'static>(
        &self,
        method: &'static str,
        call: Call,
        run: impl FnOnce(&mut Inner) -> Result<T, DaemonError>,
    ) -> Reply<T> {
        let mut inner = self.inner();
        inner.calls.push(call);
        let result = match inner.failures.get(method) {
            Some(error) => Err(error.clone()),
            None => run(&mut inner),
        };
        Box::pin(std::future::ready(result))
    }
}

impl Inner {
    fn send(&mut self, session: &SessionId, ids: &[CommentId]) -> Result<Sent, DaemonError> {
        if ids.is_empty() {
            return Err(refused("This send", "no comments were chosen; pick at least one draft"));
        }
        let state = self.session(session);
        let mut chosen: Vec<usize> = Vec::new();
        for id in ids {
            let index = state.position(id).ok_or_else(gone)?;
            if !chosen.contains(&index) {
                chosen.push(index);
            }
            if let Some(batch) = state.batches.get(id).copied() {
                for (other, other_batch) in &state.batches {
                    if *other_batch == batch {
                        let at = state.position(other).ok_or_else(gone)?;
                        if !chosen.contains(&at) {
                            chosen.push(at);
                        }
                    }
                }
            }
        }
        for &index in &chosen {
            let comment = &state.comments[index];
            not_archived(comment)?;
            if !matches!(comment.status, Status::Draft | Status::Waiting) {
                return Err(refused(
                    "This send",
                    "only drafts can be sent; take the sent ones out of the selection",
                ));
            }
        }
        let typing = state.typing;
        let batch = state.next_batch;
        if typing == Typing::Unknown {
            state.next_batch += 1;
        }
        let show_repo = state.repos > 1;
        if typing == Typing::Typed {
            let message = chosen
                .iter()
                .map(|&index| message_part(&state.comments[index], show_repo))
                .collect::<Vec<_>>()
                .join("\n\n");
            state.typed.push(message);
        }
        let mut sent = Vec::new();
        for index in chosen {
            let mut comment = state.comments[index].clone();
            match typing {
                Typing::Typed => {
                    comment.status = Status::Sent;
                    comment.unconfirmed = false;
                    state.batches.remove(&comment.id);
                    state.delivered.insert(comment.id.clone());
                }
                Typing::NotTyped => comment.status = Status::Waiting,
                Typing::Unknown => {
                    comment.status = Status::Waiting;
                    comment.unconfirmed = true;
                    state.batches.insert(comment.id.clone(), batch);
                }
            }
            comment.revision += 1;
            state.comments[index] = comment.clone();
            state.emit(&CommentEvent::Changed(comment.clone()));
            sent.push(comment);
        }
        Ok(Sent { comments: sent, queued: typing != Typing::Typed })
    }

    fn take_back(
        &mut self,
        session: &SessionId,
        ids: &[CommentId],
    ) -> Result<Vec<Comment>, DaemonError> {
        let state = self.session(session);
        let mut chosen: Vec<CommentId> = Vec::new();
        for id in ids {
            let index = state.position(id).ok_or_else(gone)?;
            if state.comments[index].status != Status::Waiting {
                return Err(refused(
                    "This take back",
                    "only comments waiting to send can be taken back",
                ));
            }
            let mut add = vec![id.clone()];
            if let Some(batch) = state.batches.get(id).copied() {
                add.extend(
                    state.batches.iter().filter(|(_, b)| **b == batch).map(|(c, _)| c.clone()),
                );
            }
            for each in add {
                if !chosen.contains(&each) {
                    chosen.push(each);
                }
            }
        }
        chosen.sort_by_key(|id| state.position(id));
        let mut back = Vec::new();
        for id in chosen {
            let Some(index) = state.position(&id) else { continue };
            let mut comment = state.comments[index].clone();
            comment.status = Status::Draft;
            comment.unconfirmed = false;
            comment.revision += 1;
            state.batches.remove(&id);
            state.comments[index] = comment.clone();
            state.emit(&CommentEvent::Changed(comment.clone()));
            back.push(comment);
        }
        Ok(back)
    }
}

impl CommentsClient for FakeComments {
    fn list(&self, session: &SessionId, which: Which) -> Reply<Listing> {
        self.call("list", Call::List(session.clone(), which), |inner| {
            let comments = inner.sessions.get(session).map(|s| s.comments.clone()).unwrap_or_default();
            let draft_count =
                u32::try_from(comments.iter().filter(|c| c.counts_as_draft()).count()).unwrap_or(u32::MAX);
            let comments = comments
                .into_iter()
                .filter(|c| match which {
                    Which::Open => !c.archived && c.status != Status::Resolved,
                    Which::WithResolved => !c.archived,
                    Which::Archived => c.archived,
                })
                .collect();
            Ok(Listing { comments, draft_count })
        })
    }

    fn watch(&self, session: &SessionId) -> Reply<CommentEvents> {
        self.call("watch", Call::Watch(session.clone()), |inner| {
            let (sender, receiver) = mpsc::unbounded_channel();
            inner.session(session).watchers.push(sender);
            Ok(receiver)
        })
    }

    fn create(&self, session: &SessionId, anchor: Anchor, text: &str, key: &str) -> Reply<Comment> {
        let call = Call::Create(session.clone(), anchor.clone(), text.to_owned(), key.to_owned());
        self.call("create", call, |inner| {
            let state = inner.session(session);
            if let Some(existing) = state.keys.get(key).and_then(|id| state.position(id)) {
                return Ok(state.comments[existing].clone());
            }
            if text.trim().is_empty() {
                return Err(refused("This comment", "it is empty; write what you want changed"));
            }
            let id = CommentId::new(format!("{session}/c-{}", state.next));
            state.next += 1;
            let comment = Comment {
                id: id.clone(),
                session: session.clone(),
                anchor,
                text: text.to_owned(),
                status: Status::Draft,
                revision: 1,
                unconfirmed: false,
                archived: false,
                replies: Vec::new(),
                resolved_by: None,
                addressed: None,
            };
            state.keys.insert(key.to_owned(), id);
            state.comments.push(comment.clone());
            state.emit(&CommentEvent::Changed(comment.clone()));
            Ok(comment)
        })
    }

    fn edit(&self, id: &CommentId, text: &str, revision: u64) -> Reply<Comment> {
        self.call("edit", Call::Edit(id.clone(), text.to_owned(), revision), |inner| {
            inner.change(id, |comment, _| {
                check_revision(comment, revision)?;
                if comment.status != Status::Draft {
                    return Err(refused(
                        "This edit",
                        "it was sent already; reply in the terminal or write a new comment",
                    ));
                }
                if text.trim().is_empty() {
                    return Err(refused("This edit", "it is empty; drop the comment instead"));
                }
                text.clone_into(&mut comment.text);
                Ok(())
            })
        })
    }

    fn drop_comment(&self, id: &CommentId) -> Reply<()> {
        self.call("drop_comment", Call::Drop(id.clone()), |inner| {
            let (session, index) = inner.locate(id)?;
            let state = inner.session(&session);
            let comment = state.comments.remove(index);
            state.batches.remove(id);
            state.keys.retain(|_, kept| kept != id);
            state.emit(&CommentEvent::Dropped(comment.id));
            Ok(())
        })
    }

    fn reattach(&self, id: &CommentId, anchor: Anchor, revision: u64) -> Reply<Comment> {
        self.call("reattach", Call::Reattach(id.clone(), anchor.clone(), revision), |inner| {
            inner.change(id, |comment, _| {
                check_revision(comment, revision)?;
                if comment.status != Status::Draft {
                    return Err(refused(
                        "This re-attach",
                        "only drafts can move; write a new comment on the new passage",
                    ));
                }
                comment.anchor = Anchor { detached: false, ..anchor };
                Ok(())
            })
        })
    }

    fn send(&self, session: &SessionId, ids: &[CommentId]) -> Reply<Sent> {
        self.call("send", Call::Send(session.clone(), ids.to_vec()), |inner| {
            inner.send(session, ids)
        })
    }

    fn take_back(&self, session: &SessionId, ids: &[CommentId]) -> Reply<Vec<Comment>> {
        self.call("take_back", Call::TakeBack(session.clone(), ids.to_vec()), |inner| {
            inner.take_back(session, ids)
        })
    }

    fn resolve(&self, id: &CommentId, revision: u64) -> Reply<Comment> {
        self.call("resolve", Call::Resolve(id.clone(), revision), |inner| {
            inner.change(id, |comment, _| {
                check_revision(comment, revision)?;
                not_archived(comment)?;
                if comment.status == Status::Resolved {
                    return Err(refused("This resolve", "the comment is resolved already"));
                }
                comment.status = Status::Resolved;
                comment.resolved_by = Some(Author::User);
                Ok(())
            })
        })
    }

    fn resolve_addressed(
        &self,
        session: &SessionId,
        file: Option<(&str, &str)>,
    ) -> Reply<Vec<Comment>> {
        let file_owned = file.map(|(repo, path)| (repo.to_owned(), path.to_owned()));
        self.call("resolve_addressed", Call::ResolveAddressed(session.clone(), file_owned), |inner| {
            let ids: Vec<CommentId> = inner
                .session(session)
                .comments
                .iter()
                .filter(|c| {
                    c.status == Status::Addressed
                        && !c.archived
                        && file.is_none_or(|(repo, path)| c.anchor.repo == repo && c.anchor.path == path)
                })
                .map(|c| c.id.clone())
                .collect();
            let mut resolved = Vec::new();
            for id in ids {
                resolved.push(inner.change(&id, |comment, _| {
                    comment.status = Status::Resolved;
                    comment.resolved_by = Some(Author::User);
                    Ok(())
                })?);
            }
            Ok(resolved)
        })
    }

    fn reopen(&self, id: &CommentId, revision: u64) -> Reply<Comment> {
        self.call("reopen", Call::Reopen(id.clone(), revision), |inner| {
            inner.change(id, |comment, _| {
                check_revision(comment, revision)?;
                if comment.status != Status::Resolved {
                    return Err(refused("This reopen", "the comment isn't resolved"));
                }
                comment.status = Status::Draft;
                comment.resolved_by = None;
                comment.addressed = None;
                Ok(())
            })
        })
    }

    fn archive(&self, id: &CommentId) -> Reply<Comment> {
        self.call("archive", Call::Archive(id.clone()), |inner| {
            inner.change(id, |comment, _| {
                if comment.archived {
                    return Err(refused("This archive", "the comment is archived already"));
                }
                comment.archived = true;
                Ok(())
            })
        })
    }

    fn unarchive(&self, id: &CommentId) -> Reply<Comment> {
        self.call("unarchive", Call::Unarchive(id.clone()), |inner| {
            inner.change(id, |comment, _| {
                if !comment.archived {
                    return Err(refused("This unarchive", "the comment isn't archived"));
                }
                comment.archived = false;
                Ok(())
            })
        })
    }
}
