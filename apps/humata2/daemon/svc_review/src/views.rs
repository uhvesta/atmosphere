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
use humata2_grpc::Timestamp;
use humata2_grpc::humata::v1 as pb;
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
        content: pb::prepared_view::Content,
    ) -> Result<pb::prepared_view::Content, Status> {
        use pb::prepared_view::Content;
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
                let (plan, _) = self.checked_plan(caller, record, session, &plan).await?;
                Ok(Content::ReviewPlan(wire::plan_to_pb(&plan, &[])))
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
        if plan.notes.is_empty() {
            return Ok(plan.clone());
        }
        let mut stored = wire::stored_plan_from_pb(plan)
            .map_err(|error| ports::internal(format!("a stored review plan didn't read back: {error}"), ""))?;
        let asked: Vec<pb::RepoRef> = stored.spec.repos.iter().map(|repo| wire::repo_ref(&repo.repo)).collect();
        let keys = super::repo_keys(&self.inner, record, &asked).await?;
        self.require_read(caller, session, &keys)?;
        let clones = super::clones(record, &keys).await?;
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
        let mut outdated = Vec::with_capacity(states.len());
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
        Ok(wire::plan_to_pb(&stored, &outdated))
    }
}
