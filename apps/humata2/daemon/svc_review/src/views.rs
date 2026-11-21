//! Draft views (VW-1, VW-2, VW-10, VW-14; D87): what a session's agent
//! prepares for the user with `view_prepare`. A view is a draft, which
//! only its agent sees and revises, until MarkReady offers it; from then
//! on only the user moves it between the offered states. Each change is
//! saved to the store before it is answered, with the idempotency keys of
//! the calls that made it, so a restart keeps views and a repeated call
//! acts once.

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_access::{Call, Capability, SessionRef, Target};
use humata2_daemon_ports::Caller;
use humata2_grpc_core::Timestamp;
use crate::pb;
use humata2_review_diff::{NoteState, NoteTarget, wire};
use humata2_store::{SessionRecord, StoreError, ViewRecord};
use pb::invalid_field::Problem;
use prost::Message;
use tonic::{Code, Status};

use super::{Review, blocking, idempotency, own_session};
use crate::ports::{self, Checked, Scope, actor};

/// A view keeps the idempotency keys of this many of the calls that made
/// or changed it; a retry comes soon after its call.
const KEYS_MAX: usize = 16;

/// The longest title a view takes, in characters (MCP.md's schema).
const TITLE_MAX: usize = 200;

/// A view as the store keeps it, read back.
struct Held {
    record: ViewRecord,
    view: pb::PreparedView,
}

impl Held {
    fn id(&self) -> &str {
        &self.record.id
    }

    fn answered(&self, key: Option<&String>) -> bool {
        key.is_some_and(|key| self.record.keys.contains(key))
    }

    fn state(&self) -> pb::ViewState {
        pb::ViewState::try_from(self.view.state).unwrap_or(pb::ViewState::Unspecified)
    }

    fn revision(&self) -> u64 {
        self.view.revision.as_ref().map_or(0, |revision| revision.value)
    }

    /// Moves the view on a revision.
    fn bump(&mut self) {
        let next = self.revision() + 1;
        self.view.revision = Some(pb::Revision { value: next });
    }
}

impl Review {
    /// PrepareView of anything but a review plan in the open review chat:
    /// a new draft, saved.
    pub(super) async fn prepare_draft(
        &self,
        caller: &Caller,
        record: &SessionRecord,
        session: &SessionRef,
        request: pb::PrepareViewRequest,
    ) -> Result<pb::PreparedView, Status> {
        let Caller::Agent(agent) = caller else {
            return Err(agents_only("PrepareView"));
        };
        self.require_prepare(caller, session)?;
        let key = idempotency(request.idempotency_key.as_ref());
        let _writing = self.inner.views.lock().await;
        if let Some(key) = &key {
            for held in self.views_of(&record.id)? {
                if held.answered(Some(key)) {
                    return Ok(held.view);
                }
            }
        }
        let title = checked_title(&request.title)?.ok_or_else(|| ports::missing("title"))?;
        let content = request.content.map(from_prepare).ok_or_else(|| ports::missing("content"))?;
        let content = self.checked_content(caller, record, session, content).await?;
        // An offered stack with no summary says what it is.
        let summary = match &content {
            pb::prepared_view::Content::ReviewPlan(plan) if request.summary.is_empty() => {
                super::stack::offer_text(plan).unwrap_or_default()
            }
            _ => request.summary,
        };
        let (at, at_ms) = now();
        let view = pb::PreparedView {
            id: Some(pb::ViewId {
                value: format!("vw_{}", uuid::Uuid::new_v4().simple()),
            }),
            session: Some(pb::SessionId {
                value: record.id.clone(),
            }),
            title,
            summary,
            content: Some(content),
            prepared_by: Some(pb::AgentActor {
                session: Some(pb::SessionId {
                    value: record.id.clone(),
                }),
                machine: Some(pb::MachineId {
                    value: record.machine.clone(),
                }),
                agent: Some(pb::AgentId {
                    value: agent.agent.clone(),
                }),
                session_title: agent.title.clone(),
            }),
            prepared_at: Some(at),
            state: pb::ViewState::Draft as i32,
            revision: Some(pb::Revision { value: 1 }),
            journal: None,
            undo_id: String::new(),
        };
        let held = Held {
            record: ViewRecord {
                id: view.id.as_ref().map(|id| id.value.clone()).unwrap_or_default(),
                session: record.id.clone(),
                view: Vec::new(),
                keys: Vec::new(),
                prepared_unix_ms: at_ms,
                updated_unix_ms: at_ms,
            },
            view,
        };
        self.put(held, key, Some(pb::Change::Created), caller).await
    }

    /// ReviseView: new parts for a draft the caller prepared.
    pub(super) async fn revise_draft(
        &self,
        caller: &Caller,
        request: pb::ReviseViewRequest,
    ) -> Result<pb::PreparedView, Status> {
        let _writing = self.inner.views.lock().await;
        let (mut held, record, session) = self.own_view(caller, request.id.as_ref(), "ReviseView")?;
        let key = idempotency(request.idempotency_key.as_ref());
        if held.answered(key.as_ref()) {
            return Ok(held.view);
        }
        if held.state() != pb::ViewState::Draft {
            return Err(ports::invalid_field(
                "id",
                Problem::NotAllowedHere,
                format!("view {} was offered to the user already, so it can't be revised", held.id()),
                "prepare a new view with view_prepare for the changes; the user keeps the one offered",
            ));
        }
        if let Some(expected) = request.expected_revision
            && expected.value != held.revision()
        {
            return Err(revision_conflict(held.id(), expected.value, held.revision()));
        }
        let mut changed = false;
        if let Some(title) = checked_title(&request.title)? {
            held.view.title = title;
            changed = true;
        }
        if !request.summary.is_empty() {
            held.view.summary = request.summary;
            changed = true;
        }
        if let Some(content) = request.content {
            let content = self.checked_content(caller, &record, &session, from_revise(content)).await?;
            held.view.content = Some(content);
            changed = true;
        }
        if changed {
            held.bump();
        } else if key.is_none() {
            return Ok(held.view);
        }
        let told = changed.then_some(pb::Change::Updated);
        self.put(held, key, told, caller).await
    }

    /// MarkReady: the draft is offered (VW-2). An offered view stays as
    /// it is.
    pub(super) async fn mark_draft_ready(
        &self,
        caller: &Caller,
        request: pb::MarkReadyRequest,
    ) -> Result<pb::PreparedView, Status> {
        let _writing = self.inner.views.lock().await;
        let (mut held, _record, _) = self.own_view(caller, request.id.as_ref(), "MarkReady")?;
        let key = idempotency(request.idempotency_key.as_ref());
        if held.answered(key.as_ref()) {
            return Ok(held.view);
        }
        let offered = held.state() == pb::ViewState::Draft;
        if offered {
            held.view.state = pb::ViewState::Waiting as i32;
            held.bump();
            // VW-12: on a machine the offer reaches the host through the
            // journal, kept before the offer is taken (D225).
            if let Some(journal) = self.inner.view_journal.get() {
                journal.offered(&held.record.session, &held.view).await?;
            }
        } else if key.is_none() {
            return Ok(held.view);
        }
        let told = offered.then_some(pb::Change::Updated);
        self.put(held, key, told, caller).await
    }

    /// ListViews: a session's views, newest first. With `include_forks`, the
    /// views of its forks and their forks too (VW-10). Drafts only to the
    /// session's own agent (VW-14). With no session, the user gets every
    /// session's offered views, and an agent its own session's.
    pub(super) async fn list_prepared(
        &self,
        caller: &Caller,
        request: pb::ListViewsRequest,
    ) -> Result<pb::ListViewsResponse, Status> {
        let asked = request
            .session
            .filter(|session| !session.value.is_empty())
            .or_else(|| own_session(caller));
        let asked = match (asked, caller) {
            (Some(asked), _) => asked,
            (None, Caller::User(_)) => return self.every_offered(),
            (None, _) => return Err(ports::missing("session")),
        };
        let (record, session) = self.session(Some(&asked))?;
        self.require(caller, &Call::ReadSession(session))?;
        let mut sessions = vec![record.id.clone()];
        if request.include_forks {
            sessions.extend(self.forks_below(&record.id, caller)?);
        }
        let mut listed = Vec::new();
        for id in &sessions {
            let own = own_session(caller).is_some_and(|own| own.value == *id);
            listed.extend(
                self.views_of(id)?
                    .into_iter()
                    .filter(|held| own || held.state() != pb::ViewState::Draft),
            );
        }
        listed.sort_by(|a, b| {
            (b.record.prepared_unix_ms, &b.record.id).cmp(&(a.record.prepared_unix_ms, &a.record.id))
        });
        let views = listed.into_iter().map(|held| held.view).collect();
        let views = self.recheck_plans(caller, views).await;
        Ok(pb::ListViewsResponse { views, page: None })
    }

    /// The forks of `root` and their forks, each once, that `caller` may
    /// read; one it may not read is left out, not refused, as a board's is.
    /// The walk is over the store's fork origins, and a cycle in them ends
    /// it.
    fn forks_below(&self, root: &str, caller: &Caller) -> Result<Vec<String>, Status> {
        let origins = self
            .inner
            .store
            .fork_origins()
            .map_err(|error| ports::store_error(error, &self.inner.machine, "list the forks"))?;
        let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
        for origin in &origins {
            children
                .entry(origin.parent.as_str())
                .or_default()
                .push(origin.session_id.as_str());
        }
        let mut seen: HashSet<&str> = HashSet::from([root]);
        let mut todo = vec![root];
        let mut forks = Vec::new();
        while let Some(at) = todo.pop() {
            for &child in children.get(at).into_iter().flatten() {
                if seen.insert(child) {
                    todo.push(child);
                    forks.push(child);
                }
            }
        }
        let mut readable = Vec::new();
        for fork in forks {
            // A fork this store no longer holds has no views to list.
            let Ok((_, session)) = self.session(Some(&pb::SessionId {
                value: fork.to_owned(),
            })) else {
                continue;
            };
            if matches!(
                self.inner.guard.check(caller, &Call::ReadSession(session)),
                Ok(Checked::Allowed)
            ) {
                readable.push(fork.to_owned());
            }
        }
        Ok(readable)
    }

    /// Every session's offered views on this daemon, newest first.
    fn every_offered(&self) -> Result<pb::ListViewsResponse, Status> {
        let sessions = self
            .inner
            .store
            .sessions()
            .map_err(|error| ports::store_error(error, &self.inner.machine, "list the sessions"))?;
        let mut offered = Vec::new();
        for session in sessions {
            offered.extend(
                self.views_of(&session.id)?
                    .into_iter()
                    // VW-5: Not now hides the offer; the view stays in the
                    // Journal.
                    .filter(|held| !matches!(held.state(), pb::ViewState::Draft | pb::ViewState::NotNow)),
            );
        }
        offered.sort_by(|a, b| {
            (b.record.prepared_unix_ms, &b.record.id).cmp(&(a.record.prepared_unix_ms, &a.record.id))
        });
        let views = offered.into_iter().map(|held| held.view).collect();
        Ok(pb::ListViewsResponse { views, page: None })
    }

    /// SetViewState: the user's answer to an offered view (VW-4..9).
    pub(super) async fn set_state(
        &self,
        caller: &Caller,
        request: pb::SetViewStateRequest,
    ) -> Result<pb::PreparedView, Status> {
        let id = view_id(request.id.as_ref())?;
        let state = pb::ViewState::try_from(request.state)
            .ok()
            .filter(|state| !matches!(state, pb::ViewState::Unspecified | pb::ViewState::Draft))
            .ok_or_else(|| {
                ports::invalid_field(
                    "state",
                    Problem::UnknownValue,
                    "a view can be set Waiting, Later, Not now, Opened, Reverted, Opened and closed or Kept; only its agent makes a draft",
                    "send one of those states",
                )
            })?;
        let _writing = self.inner.views.lock().await;
        let mut held = self.held(&id)?.ok_or_else(|| no_view(&id))?;
        let (_, session) = self.session(Some(&pb::SessionId {
            value: held.record.session.clone(),
        }))?;
        self.require_use(caller, Capability::UserReviewState, &session)?;
        let key = idempotency(request.idempotency_key.as_ref());
        if held.answered(key.as_ref()) {
            return Ok(held.view);
        }
        if held.state() == pb::ViewState::Draft {
            return Err(ports::invalid_field(
                "id",
                Problem::NotAllowedHere,
                format!("view {id} is still a draft its agent hasn't offered"),
                "wait until the agent marks it ready; a draft isn't offered or in the Journal",
            ));
        }
        let moved = held.state() != state;
        if moved {
            held.view.state = state as i32;
            held.bump();
        } else if key.is_none() {
            return Ok(held.view);
        }
        let told = moved.then_some(pb::Change::Updated);
        self.put(held, key, told, caller).await
    }

    /// The caller's own view `id`, with its session: only a session's
    /// agent changes its views, and another session's are not found.
    fn own_view(
        &self,
        caller: &Caller,
        id: Option<&pb::ViewId>,
        method: &str,
    ) -> Result<(Held, SessionRecord, SessionRef), Status> {
        let own = own_session(caller).ok_or_else(|| agents_only(method))?;
        let id = view_id(id)?;
        let held = self
            .held(&id)?
            .filter(|held| held.record.session == own.value)
            .ok_or_else(|| no_view(&id))?;
        let (record, session) = self.session(Some(&own))?;
        self.require_prepare(caller, &session)?;
        Ok((held, record, session))
    }

    /// Preparing a view is offering it in the session's channel (VW-1).
    fn require_prepare(&self, caller: &Caller, session: &SessionRef) -> Result<(), Status> {
        self.require(
            caller,
            &Call::Use {
                capability: Capability::ViewsPrepare,
                target: Target::Channel(session.channel.clone()),
            },
        )
    }

    fn held(&self, id: &str) -> Result<Option<Held>, Status> {
        match self.inner.store.view(id) {
            Ok(record) => record.map(read).transpose(),
            // Not an id the daemon gives out, so no view has it.
            Err(StoreError::InvalidId { .. }) => Ok(None),
            Err(error) => Err(ports::store_error(error, &self.inner.machine, "read a view")),
        }
    }

    fn views_of(&self, session: &str) -> Result<Vec<Held>, Status> {
        self.inner
            .store
            .views(session)
            .map_err(|error| ports::store_error(error, &self.inner.machine, "list the session's views"))?
            .into_iter()
            .map(read)
            .collect()
    }

    /// Saves `held` with `key` among its keys, and answers with it. When
    /// the view changed, `told` is the change, for the event feed.
    async fn put(
        &self,
        mut held: Held,
        key: Option<String>,
        told: Option<pb::Change>,
        caller: &Caller,
    ) -> Result<pb::PreparedView, Status> {
        if let Some(key) = key {
            let keys = &mut held.record.keys;
            keys.retain(|held| *held != key);
            keys.push(key);
            let over = keys.len().saturating_sub(KEYS_MAX);
            keys.drain(..over);
        }
        held.record.view = held.view.encode_to_vec();
        held.record.updated_unix_ms = now().1;
        let store = self.inner.store.clone();
        let record = held.record;
        let session = record.session.clone();
        blocking(move || store.put_view(&record))
            .await?
            .map_err(|error| ports::store_error(error, &self.inner.machine, "save the view"))?;
        if let Some(change) = told {
            self.inner.feed.send(
                Scope::Session(session),
                change,
                pb::event::Payload::View(held.view.clone()),
                Some(actor(caller)),
            );
        }
        Ok(held.view)
    }

    /// `content` checked against the session: a walkthrough's steps and a
    /// pull request's repos named, a board in the session's channel, a
    /// review plan's diffs and notes as `review_prepare` checks them.
    async fn checked_content(
        &self,
        caller: &Caller,
        record: &SessionRecord,
        session: &SessionRef,
        mut content: pb::prepared_view::Content,
    ) -> Result<pb::prepared_view::Content, Status> {
        use pb::prepared_view::Content;
        let owners = super::unique_repo_owners(&self.inner, record)?;
        super::normalize_content_repos(&mut content, &owners);
        match content {
            Content::Review(walkthrough) => {
                check_walkthrough(&walkthrough)?;
                Ok(Content::Review(walkthrough))
            }
            Content::Board(offer) => {
                self.check_board(record, &offer)?;
                Ok(Content::Board(offer))
            }
            Content::PullRequest(draft) => {
                check_pull_request(&draft)?;
                Ok(Content::PullRequest(draft))
            }
            Content::ReviewPlan(plan) => {
                let (plan, _, checked) = self.checked_plan(caller, record, session, &plan).await?;
                Ok(Content::ReviewPlan(wire::plan_to_pb(
                    &plan,
                    &[],
                    checked.as_ref(),
                )))
            }
        }
    }

    /// A board view shows a board of the session's own channel (VW-1).
    fn check_board(&self, record: &SessionRecord, offer: &pb::BoardViewOffer) -> Result<(), Status> {
        use pb::board_view_offer::Scope as OfferScope;
        let elsewhere = |what: String| {
            ports::invalid_field(
                "board",
                Problem::NotAllowedHere,
                format!("a board view shows a board of your session's channel {}; {what}", record.channel),
                "offer your channel's board, or the board of a session in it",
            )
        };
        match offer.scope.as_ref() {
            None => Err(ports::missing("board.scope")),
            Some(OfferScope::Channel(channel) | OfferScope::ChannelWithSessions(channel)) => {
                if channel.value == record.channel {
                    Ok(())
                } else {
                    Err(elsewhere(format!("{} is another channel", channel.value)))
                }
            }
            Some(OfferScope::SessionWithForks(other) | OfferScope::OneSession(other)) => {
                if other.value == record.id {
                    return Ok(());
                }
                let (other, _) = self.session(Some(other))?;
                if other.channel == record.channel {
                    Ok(())
                } else {
                    Err(elsewhere(format!("session {} is in channel {}", other.id, other.channel)))
                }
            }
        }
    }
}

fn from_prepare(content: pb::prepare_view_request::Content) -> pb::prepared_view::Content {
    use pb::prepare_view_request::Content as Asked;
    use pb::prepared_view::Content;
    match content {
        Asked::Review(walkthrough) => Content::Review(walkthrough),
        Asked::Board(offer) => Content::Board(offer),
        Asked::PullRequest(draft) => Content::PullRequest(draft),
        Asked::ReviewPlan(plan) => Content::ReviewPlan(plan),
    }
}

fn from_revise(content: pb::revise_view_request::Content) -> pb::prepared_view::Content {
    use pb::prepared_view::Content;
    use pb::revise_view_request::Content as Asked;
    match content {
        Asked::Review(walkthrough) => Content::Review(walkthrough),
        Asked::Board(offer) => Content::Board(offer),
        Asked::PullRequest(draft) => Content::PullRequest(draft),
        Asked::ReviewPlan(plan) => Content::ReviewPlan(plan),
    }
}

/// Each step names a file, and its lines, if any, run forward from 1.
fn check_walkthrough(walkthrough: &pb::ReviewWalkthrough) -> Result<(), Status> {
    if walkthrough.steps.is_empty() {
        return Err(ports::invalid_field(
            "review.steps",
            Problem::Missing,
            "a review walkthrough needs at least one step",
            "give each step a repo, a path and a note",
        ));
    }
    for (index, step) in walkthrough.steps.iter().enumerate() {
        let field = |name: &str| format!("review.steps[{index}].{name}");
        let file = step.file.as_ref().ok_or_else(|| ports::missing(&field("file")))?;
        if file.repo.as_ref().is_none_or(|repo| repo.name.is_empty()) {
            return Err(ports::missing(&field("file.repo")));
        }
        if file.path.is_empty() {
            return Err(ports::missing(&field("file.path")));
        }
        if let Some(lines) = &step.lines
            && (lines.first == 0 || lines.last < lines.first)
        {
            return Err(ports::invalid_field(
                &field("lines"),
                Problem::OutOfRange,
                format!(
                    "step {} has lines {} to {}: lines count from 1 and run forward",
                    index + 1,
                    lines.first,
                    lines.last
                ),
                "give the first and last line of the step, first ≤ last",
            ));
        }
    }
    Ok(())
}

/// Each repo is named and has a title.
fn check_pull_request(draft: &pb::PullRequestDraft) -> Result<(), Status> {
    if draft.repos.is_empty() {
        return Err(ports::invalid_field(
            "pull_request.repos",
            Problem::Missing,
            "a pull request view needs at least one repo",
            "give each repo its title, body, base branch and whether it is a draft",
        ));
    }
    for (index, repo) in draft.repos.iter().enumerate() {
        if repo.repo.as_ref().is_none_or(|repo| repo.name.is_empty()) {
            return Err(ports::missing(&format!("pull_request.repos[{index}].repo")));
        }
        if repo.title.trim().is_empty() {
            return Err(ports::missing(&format!("pull_request.repos[{index}].title")));
        }
    }
    Ok(())
}

/// The title, if one was given, at most [`TITLE_MAX`] characters.
fn checked_title(title: &str) -> Result<Option<String>, Status> {
    let title = title.trim();
    if title.is_empty() {
        return Ok(None);
    }
    let length = title.chars().count();
    if length > TITLE_MAX {
        return Err(ports::invalid_field(
            "title",
            Problem::TooLong,
            format!("a view's title is at most {TITLE_MAX} characters, and this one is {length}"),
            "shorten the title and put the detail in the summary",
        ));
    }
    Ok(Some(title.to_owned()))
}

fn view_id(id: Option<&pb::ViewId>) -> Result<String, Status> {
    id.map(|id| id.value.clone())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ports::missing("id"))
}

fn read(record: ViewRecord) -> Result<Held, Status> {
    let view = pb::PreparedView::decode(record.view.as_slice()).map_err(|error| {
        ports::internal(
            format!("view {} in this machine's store is unreadable: {error}", record.id),
            "",
        )
    })?;
    Ok(Held { record, view })
}

/// A view is prepared by a session's agent, never by the user.
pub(super) fn agents_only(method: &str) -> Status {
    ports::failure(
        Code::FailedPrecondition,
        ports::error(
            format!("a view is prepared by a session's agent: {method} takes the session from the agent's token"),
            "call it from the agent's MCP server (view_prepare, view_revise, view_mark_ready or review_prepare), not as the user",
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: "session".to_owned(),
                problem: Problem::NotAllowedHere as i32,
            }),
        ),
    )
}

fn no_view(id: &str) -> Status {
    ports::not_found(
        pb::EntityKind::View,
        id,
        format!("no view {id} of your session is on this machine"),
        "list the session's views (views_list or ListViews) and use one of their ids",
    )
}

fn revision_conflict(id: &str, expected: u64, actual: u64) -> Status {
    ports::failure(
        Code::Aborted,
        ports::error(
            format!("view {id} changed since you read it (revision {expected}, now {actual})"),
            "read the view again with views_list, then revise what it holds now",
            pb::error::Kind::RevisionConflict(pb::RevisionConflict {
                kind: pb::EntityKind::View as i32,
                id: id.to_owned(),
                expected: Some(pb::Revision { value: expected }),
                actual: Some(pb::Revision { value: actual }),
            }),
        ),
    )
}

/// Now, for the wire and in Unix milliseconds.
fn now() -> (Timestamp, i64) {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let at = Timestamp {
        seconds: i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(since.subsec_nanos()).unwrap_or(0),
    };
    (at, i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

impl Review {
    /// Each review plan of `views` with its notes read against the files
    /// as they are now (RV-34): a note whose lines only moved is shown at
    /// its new lines, one whose lines changed is marked outdated. The
    /// store keeps the plan as it was prepared. A plan whose clones can't
    /// be read now is listed as prepared.
    async fn recheck_plans(
        &self,
        caller: &Caller,
        views: Vec<pb::PreparedView>,
    ) -> Vec<pb::PreparedView> {
        let mut checked = Vec::with_capacity(views.len());
        for mut view in views {
            // Each plan is read against its own session's clones: with
            // include_forks a fork's plan is listed beside the session's.
            if let Some(pb::prepared_view::Content::ReviewPlan(plan)) = &view.content
                && let Ok((record, session)) = self.session(view.session.as_ref())
                && let Ok(plan) = self.rechecked_plan(caller, &record, &session, plan).await
            {
                view.content = Some(pb::prepared_view::Content::ReviewPlan(plan));
            }
            checked.push(view);
        }
        checked
    }

    async fn rechecked_plan(
        &self,
        caller: &Caller,
        record: &SessionRecord,
        session: &SessionRef,
        plan: &pb::ReviewPlan,
    ) -> Result<pb::ReviewPlan, Status> {
        if plan.notes.is_empty() && plan.stack.is_none() {
            return Ok(plan.clone());
        }
        if let Some((machine, remote)) = super::pull_requests::relayed_for(&record.id) {
            return self
                .rechecked_plan_remote(caller, record, session, plan, &machine, remote)
                .await;
        }
        let mut stored = wire::stored_plan_from_pb(plan)
            .map_err(|error| ports::internal(format!("a stored review plan didn't read back: {error}"), ""))?;
        let mut asked: Vec<pb::RepoRef> = stored.spec.repos.iter().map(|repo| wire::repo_ref(&repo.repo)).collect();
        if let Some(stack) = &stored.stack {
            for layer in &stack.layers {
                for part in &layer.repos {
                    let repo = wire::repo_ref(&part.repo);
                    if !asked.contains(&repo) {
                        asked.push(repo);
                    }
                }
            }
        }
        let keys = super::repo_keys(&self.inner, record, &asked).await?;
        self.require_read(caller, session, &keys)?;
        let clones = super::clones(record, &keys).await?;
        let mut outdated = Vec::new();
        if !stored.notes.is_empty() {
            let diffs = clones
                .diff(&stored.spec, &super::context_of(record))
                .await
                .map_err(|error| super::diff_failed(&error))?;
            let mut states: Vec<Option<NoteState>> = vec![None; stored.notes.len()];
            for diff in &diffs {
                let checkout = clones.get(&diff.repo).map_err(|error| super::diff_failed(&error))?.clone();
                let found = checkout
                    .note_states(diff.clone(), stored.notes.clone())
                    .await
                    .map_err(|error| super::diff_failed(&error))?;
                for (state, found) in states.iter_mut().zip(found) {
                    if state.is_none() {
                        *state = found;
                    }
                }
            }
            outdated.reserve(states.len());
            for (note, state) in stored.notes.iter_mut().zip(&states) {
                outdated.push(match state {
                    Some(NoteState::Current { first, last }) => {
                        match &mut note.target {
                            NoteTarget::Lines {
                                first: was_first,
                                last: was_last,
                                ..
                            } => {
                                *was_first = *first;
                                *was_last = *last;
                            }
                            NoteTarget::Hunk { line, .. } => *line = *first,
                            NoteTarget::File => {}
                        }
                        false
                    }
                    Some(NoteState::Outdated | NoteState::Gone) => true,
                    Some(NoteState::OnFile) | None => false,
                });
            }
        }
        let (resolved, drift) = if let Some(stack) = stored.stack.clone() {
            let kept = plan.stack.as_ref().and_then(wire::checked_stack_from_pb);
            let from = stored.from_stack.clone();
            let runner = self.inner.stack_status.clone();
            blocking(move || {
                let mut drift = Vec::new();
                let checked = match stack.check_blocking(&clones) {
                    Ok(now) => {
                        if let Some(kept) = &kept {
                            for found in kept.drift_blocking(&clones) {
                                if found.kind == humata2_review_diff::DriftKind::HeadMoved {
                                    drift.push(found);
                                }
                            }
                        }
                        if let Some(from) = from
                            && let Ok(checkout) = clones.get(&from.repo)
                            && let Ok(read) = from.read_blocking(checkout, runner.as_ref())
                        {
                            drift.extend(humata2_review_diff::FromStack::drift(
                                &now.stack().layers,
                                &read,
                            ));
                        }
                        Some(now)
                    }
                    Err(_) => {
                        if let Some(kept) = kept {
                            drift = kept.drift_blocking(&clones);
                            if let Some(from) = from
                                && let Ok(checkout) = clones.get(&from.repo)
                                && let Ok(read) = from.read_blocking(checkout, runner.as_ref())
                            {
                                drift.extend(humata2_review_diff::FromStack::drift(
                                    &kept.stack().layers,
                                    &read,
                                ));
                            }
                            Some(kept)
                        } else {
                            None
                        }
                    }
                };
                (checked, drift)
            })
            .await?
        } else {
            (None, Vec::new())
        };
        let mut out = wire::plan_to_pb(&stored, &outdated, resolved.as_ref());
        if let Some(stack) = out.stack.as_mut() {
            wire::apply_drift_to_pb(stack, &drift);
        }
        Ok(out)
    }

    async fn rechecked_plan_remote(
        &self,
        caller: &Caller,
        record: &SessionRecord,
        session: &SessionRef,
        plan: &pb::ReviewPlan,
        machine: &str,
        remote: std::sync::Arc<dyn super::ReviewRemote>,
    ) -> Result<pb::ReviewPlan, Status> {
        let Some(stack) = &plan.stack else {
            return Ok(plan.clone());
        };
        let mut asked: Vec<pb::RepoRef> = Vec::new();
        for layer in &stack.layers {
            for part in &layer.repos {
                if let Some(repo) = &part.repo
                    && !asked.contains(repo)
                {
                    asked.push(repo.clone());
                }
            }
        }
        let keys = super::repo_keys(&self.inner, record, &asked).await?;
        self.require_read(caller, session, &keys)?;
        let mut out = plan.clone();
        if let Some(out_stack) = out.stack.as_mut() {
            recheck_stack_via_remote(out_stack, &record.id, machine, remote.as_ref()).await?;
        }
        Ok(out)
    }
}

async fn run_remote_git(
    remote: &dyn super::ReviewRemote,
    machine: &str,
    session_id: &str,
    repo_folder: &str,
    args: Vec<String>,
) -> Result<pb::RunReviewGitResponse, Status> {
    remote
        .run_review_git(
            machine,
            pb::RunReviewGitRequest {
                session: Some(pb::SessionId {
                    value: session_id.to_owned(),
                }),
                repo: repo_folder.to_owned(),
                args,
                stdin: Vec::new(),
            },
        )
        .await
}

async fn remote_rev_commit(
    remote: &dyn super::ReviewRemote,
    machine: &str,
    session_id: &str,
    repo_folder: &str,
    rev: &str,
) -> Result<Option<String>, Status> {
    let response = run_remote_git(
        remote,
        machine,
        session_id,
        repo_folder,
        vec![
            "rev-parse".to_owned(),
            "--verify".to_owned(),
            format!("{rev}^{{commit}}"),
        ],
    )
    .await?;
    if response.exit_code != 0 {
        return Ok(None);
    }
    let commit = String::from_utf8_lossy(&response.stdout).trim().to_owned();
    Ok((!commit.is_empty()).then_some(commit))
}

async fn recheck_stack_via_remote(
    stack: &mut pb::ReviewStack,
    session_id: &str,
    machine: &str,
    remote: &dyn super::ReviewRemote,
) -> Result<(), Status> {
    use pb::review_layer_repo::Part as P;
    use pb::review_layer_resolved::Drift;
    let mut repo_keys: Vec<String> = Vec::new();
    for layer in &stack.layers {
        for part in &layer.repos {
            if let Some(repo) = &part.repo {
                let key = wire::repo_key(repo);
                if !repo_keys.contains(&key) {
                    repo_keys.push(key);
                }
            }
        }
    }
    for key in repo_keys {
        let folder = super::folder_name(&key);
        let mut below: Option<String> = None;
        let mut top: Option<String> = None;
        for layer in &mut stack.layers {
            let Some(part) = layer
                .repos
                .iter_mut()
                .find(|part| part.repo.as_ref().map(wire::repo_key).as_deref() == Some(key.as_str()))
            else {
                continue;
            };
            let kept = part.resolved.clone();
            match &part.part {
                Some(P::Range(range)) => {
                    let base_now =
                        remote_rev_commit(remote, machine, session_id, folder, &range.base).await?;
                    let head_now =
                        remote_rev_commit(remote, machine, session_id, folder, &range.head).await?;
                    let (Some(base_now), Some(head_now)) = (base_now, head_now) else {
                        below = None;
                        top = None;
                        let mut resolved = kept.unwrap_or_default();
                        resolved.drift = Drift::Missing as i32;
                        resolved.files = 0;
                        part.resolved = Some(resolved);
                        continue;
                    };
                    let ancestor_resp = run_remote_git(
                        remote,
                        machine,
                        session_id,
                        folder,
                        vec![
                            "merge-base".to_owned(),
                            "--is-ancestor".to_owned(),
                            base_now.clone(),
                            head_now.clone(),
                        ],
                    )
                    .await?;
                    let ancestor = ancestor_resp.exit_code == 0;
                    let chains = below.as_ref().is_none_or(|b| *b == base_now);
                    if !(ancestor && chains) {
                        let mut resolved = kept.unwrap_or(pb::ReviewLayerResolved {
                            base_commit: base_now.clone(),
                            head_commit: head_now.clone(),
                            drift: Drift::Moved as i32,
                            files: 0,
                        });
                        resolved.drift = Drift::Moved as i32;
                        resolved.files = 0;
                        part.resolved = Some(resolved);
                    } else {
                        let diff_resp = run_remote_git(
                            remote,
                            machine,
                            session_id,
                            folder,
                            vec![
                                "diff-tree".to_owned(),
                                "-r".to_owned(),
                                "--name-only".to_owned(),
                                "--no-ext-diff".to_owned(),
                                base_now.clone(),
                                head_now.clone(),
                            ],
                        )
                        .await?;
                        let files = if diff_resp.exit_code == 0 {
                            let count = String::from_utf8_lossy(&diff_resp.stdout)
                                .lines()
                                .filter(|line| !line.is_empty())
                                .count();
                            u32::try_from(count).unwrap_or(u32::MAX)
                        } else {
                            kept.as_ref().map_or(0, |r| r.files)
                        };
                        part.resolved = Some(pb::ReviewLayerResolved {
                            base_commit: base_now,
                            head_commit: head_now.clone(),
                            drift: Drift::None as i32,
                            files,
                        });
                    }
                    below = Some(head_now.clone());
                    top = Some(head_now);
                }
                Some(P::Worktree(wt)) => {
                    let head_now =
                        remote_rev_commit(remote, machine, session_id, folder, "HEAD").await?;
                    let kept_head = kept
                        .as_ref()
                        .map(|r| r.head_commit.as_str())
                        .filter(|h| !h.is_empty());
                    let holds = head_now.as_ref().is_some_and(|head| {
                        kept_head.is_none_or(|kept| kept == head)
                            && top.as_ref().is_none_or(|top| top == head)
                    });
                    if !holds {
                        let mut resolved = kept.unwrap_or_default();
                        resolved.drift = Drift::HeadMoved as i32;
                        resolved.files = 0;
                        part.resolved = Some(resolved);
                    } else if let Some(head_now) = head_now {
                        let mut args = vec![
                            "status".to_owned(),
                            "--porcelain".to_owned(),
                            "-uall".to_owned(),
                            "--".to_owned(),
                        ];
                        args.extend(wt.include_paths.iter().cloned());
                        let status_resp =
                            run_remote_git(remote, machine, session_id, folder, args).await?;
                        let files = if status_resp.exit_code == 0 {
                            let count = String::from_utf8_lossy(&status_resp.stdout)
                                .lines()
                                .filter(|line| !line.is_empty())
                                .count();
                            u32::try_from(count).unwrap_or(u32::MAX)
                        } else {
                            kept.as_ref().map_or(0, |r| r.files)
                        };
                        part.resolved = Some(pb::ReviewLayerResolved {
                            base_commit: head_now.clone(),
                            head_commit: head_now,
                            drift: Drift::None as i32,
                            files,
                        });
                    }
                }
                None => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockRemote {
        responses: HashMap<Vec<String>, (i32, Vec<u8>)>,
    }

    #[tonic::async_trait]
    impl super::super::ReviewRemote for MockRemote {
        async fn run_review_git(
            &self,
            _machine: &str,
            request: pb::RunReviewGitRequest,
        ) -> Result<pb::RunReviewGitResponse, Status> {
            let (exit_code, stdout) = self
                .responses
                .get(&request.args)
                .cloned()
                .unwrap_or((1, Vec::new()));
            Ok(pb::RunReviewGitResponse {
                exit_code,
                stdout,
                stderr: Vec::new(),
            })
        }

        async fn type_terminal(&self, _machine: &str, _request: pb::TypeRequest) -> Result<(), Status> {
            Ok(())
        }

        async fn get_diff(
            &self,
            _machine: &str,
            _request: pb::GetDiffRequest,
        ) -> Result<pb::GetDiffResponse, Status> {
            Ok(pb::GetDiffResponse::default())
        }

        async fn get_file_diff(
            &self,
            _machine: &str,
            _request: pb::GetFileDiffRequest,
        ) -> Result<pb::FileDiff, Status> {
            Ok(pb::FileDiff::default())
        }
    }

    #[tokio::test]
    async fn recheck_stack_via_remote_resolves_commits_files_and_drift() {
        let mut responses = HashMap::new();
        responses.insert(
            vec!["rev-parse".into(), "--verify".into(), "main^{commit}".into()],
            (0, b"1111\n".to_vec()),
        );
        responses.insert(
            vec!["rev-parse".into(), "--verify".into(), "feat^{commit}".into()],
            (0, b"2222\n".to_vec()),
        );
        responses.insert(
            vec!["merge-base".into(), "--is-ancestor".into(), "1111".into(), "2222".into()],
            (0, Vec::new()),
        );
        responses.insert(
            vec![
                "diff-tree".into(),
                "-r".into(),
                "--name-only".into(),
                "--no-ext-diff".into(),
                "1111".into(),
                "2222".into(),
            ],
            (0, b"src/a.rs\nsrc/b.rs\n".to_vec()),
        );
        let remote = MockRemote { responses };

        let mut stack = pb::ReviewStack {
            layers: vec![
                pb::ReviewLayer {
                    title: "L1".to_owned(),
                    repos: vec![pb::ReviewLayerRepo {
                        repo: Some(pb::RepoRef {
                            owner: "acme".to_owned(),
                            name: "api".to_owned(),
                        }),
                        part: Some(pb::review_layer_repo::Part::Range(pb::ReviewLayerRange {
                            base: "main".to_owned(),
                            head: "feat".to_owned(),
                        })),
                        resolved: None,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                pb::ReviewLayer {
                    title: "L2".to_owned(),
                    repos: vec![pb::ReviewLayerRepo {
                        repo: Some(pb::RepoRef {
                            owner: "acme".to_owned(),
                            name: "api".to_owned(),
                        }),
                        part: Some(pb::review_layer_repo::Part::Range(pb::ReviewLayerRange {
                            base: "feat".to_owned(),
                            head: "gone".to_owned(),
                        })),
                        resolved: Some(pb::ReviewLayerResolved {
                            base_commit: "2222".to_owned(),
                            head_commit: "3333".to_owned(),
                            drift: pb::review_layer_resolved::Drift::None as i32,
                            files: 4,
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
        };

        recheck_stack_via_remote(&mut stack, "s1", "m2", &remote)
            .await
            .unwrap();

        let r1 = stack.layers[0].repos[0].resolved.as_ref().unwrap();
        assert_eq!(r1.base_commit, "1111");
        assert_eq!(r1.head_commit, "2222");
        assert_eq!(r1.files, 2);
        assert_eq!(r1.drift, pb::review_layer_resolved::Drift::None as i32);

        let r2 = stack.layers[1].repos[0].resolved.as_ref().unwrap();
        assert_eq!(r2.base_commit, "2222");
        assert_eq!(r2.head_commit, "3333");
        assert_eq!(r2.files, 0);
        assert_eq!(r2.drift, pb::review_layer_resolved::Drift::Missing as i32);
    }
}
