//! InboxService: questions an agent asks the user, access it asks for
//! ahead (IN-26), the user's answers, and waiting for them (MCP.md §5).
//! Approving an ask-ahead or answering "Always allow …" makes a grant in the
//! policy (IN-14, IN-22). Each request, its answer and the grant it made
//! are saved before anyone hears of them, so a restart keeps the Inbox
//! and the grants (D96). Each new request and each change to one goes on
//! the event feed.

mod deferred;
mod kept;
pub(crate) mod permission;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use humata2_access::{
    AccessRequest, Agent, Answer, AnswerError, Call, Capability, CheckError, Decision, GrantTarget,
    PresentUser, Refusal, RequestId, Source, Target, Verdict,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;
use humata2_store_proto::humata::store::v1 as stored;
use pb::invalid_field::Problem;
use prost::Message;
use tokio::sync::{Mutex, mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Request, Response, Status};

use crate::attach;
use crate::events::{Feed, Scope};
use crate::grant_target;
use crate::guard::{self, Guard};
use crate::identity::{self, AgentCaller, Caller, UserVia};
use crate::status;

use deferred::Outcome;
pub(crate) use deferred::{BoardAsk, Deferred, Run};

/// How often a wait says it is still waiting (MCP.md §5).
const KEEPALIVE: Duration = Duration::from_secs(15);

/// The most choices a question offers.
const CHOICES_MAX: usize = 4;

/// What a wait's `result` holds when the answer made a grant.
pub(crate) const GRANT_TYPE: &str = "humata.v1.Grant";

pub(crate) struct Inbox {
    guard: Arc<Guard>,
    /// Where the sessions an ask-ahead names are looked up.
    store: Arc<Store>,
    /// This machine's id.
    machine: String,
    /// Where new requests and their answers are told.
    feed: Arc<Feed>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    requests: Vec<Entry>,
    /// The request each asking session's idempotency key made.
    keys: HashMap<String, String>,
}

struct Entry {
    id: String,
    /// The session that asked.
    session: String,
    /// What the policy asks the user, for an access request; `None` for a
    /// question.
    access: Option<AccessRequest>,
    /// The target an ask-ahead named, as the agent sent it.
    asked: Option<pb::GrantTarget>,
    /// The "session/key" idempotency keys of the calls that made it.
    keys: Vec<String>,
    request: watch::Sender<Held>,
    /// What a deferred call does once approved (D38.6); taken when it
    /// starts, and gone after a restart.
    run: std::sync::Mutex<Option<Run>>,
    /// An agent's permission hook holds it (D191): Approve or Deny only,
    /// left out of Answer all, and withdrawn once no hook holds it.
    permission: bool,
}

impl Entry {
    fn new(
        id: String,
        session: String,
        access: Option<AccessRequest>,
        asked: Option<pb::GrantTarget>,
        key: Option<String>,
        held: Held,
    ) -> Self {
        Self {
            id,
            session,
            access,
            asked,
            keys: key.into_iter().collect(),
            request: watch::channel(held).0,
            run: std::sync::Mutex::new(None),
            permission: false,
        }
    }
}

/// A request as it stands, and the grant its answer made.
#[derive(Clone)]
struct Held {
    request: pb::InboxRequest,
    grant: Option<pb::Grant>,
    /// What the policy keeps beside the grant, as it's saved; its `grant`
    /// is filled in only on the way to the store.
    granted: Option<stored::InboxGrant>,
    /// The request is for an ordinary call that waits for the answer
    /// (D38.6): approving it runs the call.
    deferred: bool,
    /// The approved call is running now.
    running: bool,
    /// How the approved call ended. Kept in memory only.
    outcome: Option<Outcome>,
}

impl Held {
    fn new(request: pb::InboxRequest) -> Self {
        Self {
            request,
            grant: None,
            granted: None,
            deferred: false,
            running: false,
            outcome: None,
        }
    }

    /// Whether a wait on this request has more to wait for.
    fn waiting(&self) -> bool {
        self.request.state == pb::RequestState::Pending as i32 || self.running
    }
}

/// How the user answered one request.
struct Decided {
    state: pb::RequestState,
    answer: pb::request_answer::Answer,
    grant: Option<(pb::Grant, stored::InboxGrant)>,
}

impl Inbox {
    /// The Inbox as the store kept it, with the grants its answers made
    /// back in the policy. Reads the store: call it before serving.
    pub(crate) fn new(
        guard: Arc<Guard>,
        store: Arc<Store>,
        machine: String,
        feed: Arc<Feed>,
    ) -> Self {
        let state = kept::restore(&guard, &store, &machine);
        Self {
            guard,
            store,
            machine,
            feed,
            state: Mutex::new(state),
        }
    }

    /// Saves `next` as `entry`'s request, and only then tells its waits.
    /// When the store refuses, the grant the answer made, if any, is taken
    /// back out of the policy, so what's in force is what a restart keeps.
    async fn commit(
        &self,
        entry: &Entry,
        next: Held,
        user: Option<&PresentUser>,
        doing: &str,
    ) -> Result<(), Status> {
        let record = kept::record(entry, &next);
        let store = Arc::clone(&self.store);
        let saved = tokio::task::spawn_blocking(move || store.put_inbox_request(&record))
            .await
            .map_err(|error| status::internal(format!("a background task failed: {error}"), ""))?;
        if let Err(error) = saved {
            let made = next
                .grant
                .as_ref()
                .filter(|_| entry.request.borrow().grant.is_none());
            if let (Some(user), Some(id)) = (user, made.and_then(grant_target::grant_number)) {
                // It was just made, so taking it back can't fail for want
                // of the host; if it did, a restart drops it anyway.
                let _ = self.guard.with_policy(|policy, _| {
                    policy.revoke_grant(user, humata2_access::GrantId::new(id))
                });
            }
            return Err(status::store(error, &self.machine, doing));
        }
        entry.request.send_replace(next);
        Ok(())
    }

    /// Marks the grant request `id`'s answer made as revoked, so a restart
    /// doesn't bring it back.
    pub(crate) async fn revoked(&self, id: &str, at: SystemTime) -> Result<(), Status> {
        let state = self.state.lock().await;
        let Some(entry) = state.requests.iter().find(|entry| entry.id == id) else {
            return Ok(());
        };
        let mut next = entry.request.borrow().clone();
        let Some(granted) = next.granted.as_mut() else {
            return Ok(());
        };
        granted.revoked_unix_ms = kept::unix_ms(at);
        let doing = format!(
            "save that grant {} is revoked, so until you revoke it again after the next restart, that restart brings it back",
            next.grant
                .as_ref()
                .and_then(grant_target::grant_number)
                .unwrap_or_default()
        );
        self.commit(entry, next, None, &doing).await
    }

    /// The request `id`, if `caller` may see it: the user sees every
    /// request, an agent its own session's.
    async fn find(&self, caller: &Caller, id: &str) -> Result<watch::Sender<Held>, Status> {
        self.state
            .lock()
            .await
            .requests
            .iter()
            .find(|entry| entry.id == id && sees(caller, entry))
            .map(|entry| entry.request.clone())
            .ok_or_else(|| no_request(id))
    }

    /// The ids of the pending requests of any of `sessions`, for a plan
    /// that lists what deleting them would withdraw (CH-24).
    pub(crate) async fn pending_ids_of(&self, sessions: &[String]) -> Vec<String> {
        let state = self.state.lock().await;
        state
            .requests
            .iter()
            .filter(|entry| {
                sessions.contains(&entry.session)
                    && entry.request.borrow().request.state == pb::RequestState::Pending as i32
            })
            .map(|entry| entry.id.clone())
            .collect()
    }

    /// Withdraws every pending request of `session` for `reason`, as when
    /// the session stops. A deferred call's closure is dropped, so it never
    /// runs. The daemon withdraws, so the feed names no one for any reason
    /// but `CallEnded`.
    pub(crate) async fn withdraw_pending_of(
        &self,
        session: &str,
        reason: pb::WithdrawReason,
    ) -> Result<(), Status> {
        let state = self.state.lock().await;
        for entry in state
            .requests
            .iter()
            .filter(|entry| entry.session == session)
        {
            let mut next = entry.request.borrow().clone();
            if next.request.state != pb::RequestState::Pending as i32 {
                continue;
            }
            withdraw(&mut next.request, reason);
            let current = next.request.clone();
            self.commit(entry, next, None, "save the withdrawal")
                .await?;
            entry
                .run
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            self.tell(pb::Change::Updated, &entry.session, &current);
        }
        Ok(())
    }

    /// Tells the feed of `session`'s request as it now stands.
    fn tell(&self, change: pb::Change, session: &str, request: &pb::InboxRequest) {
        let scope = Scope {
            channel: request
                .channel
                .as_ref()
                .map(|channel| channel.value.clone())
                .unwrap_or_default(),
            session: session.to_owned(),
        };
        // The asker made and withdraws it (D38.10); the answer is the
        // answerer's; a time-out is the daemon's.
        let by = match pb::RequestState::try_from(request.state) {
            Ok(pb::RequestState::Pending) => requester(request),
            Ok(pb::RequestState::Withdrawn)
                if request.withdraw_reason == pb::WithdrawReason::CallEnded as i32 =>
            {
                requester(request)
            }
            _ => request
                .answered
                .as_ref()
                .and_then(|answered| answered.by.clone()),
        };
        self.feed.publish_in(
            change,
            pb::event::Payload::Request(request.clone()),
            scope,
            by,
        );
    }

    /// Tells the feed of the grant `entry`'s answer made, if any, then of
    /// the answered request.
    fn tell_answer(&self, entry: &Entry) -> pb::InboxRequest {
        let held = entry.request.borrow().clone();
        if let Some(grant) = held.grant {
            let scope = Scope {
                channel: grant
                    .channel
                    .as_ref()
                    .map(|channel| channel.value.clone())
                    .unwrap_or_default(),
                session: entry.session.clone(),
            };
            let by = held
                .request
                .answered
                .as_ref()
                .and_then(|answered| answered.by.clone());
            self.feed.publish_in(
                pb::Change::Created,
                pb::event::Payload::Grant(grant),
                scope,
                by,
            );
        }
        self.tell(pb::Change::Updated, &entry.session, &held.request);
        held.request
    }

    /// The user behind `caller`, who has shown they are there.
    fn present_user(&self, caller: &Caller) -> Result<PresentUser, Status> {
        let principal = guard::principal(caller);
        self.guard
            .with_policy(|_, context| principal.present_user(context))
            .ok_or_else(|| {
                status::refused(&Refusal::UserOnly {
                    capability: Some(Capability::InboxAnswer),
                })
            })
    }

    /// Approves `access` in the policy, which checks it against the rules
    /// as they are now (IN-21) and makes the grant, if any.
    fn approve(
        &self,
        user: &PresentUser,
        id: &str,
        access: &AccessRequest,
        answer: &Answer,
        at: SystemTime,
    ) -> Result<Option<(pb::Grant, stored::InboxGrant)>, Status> {
        let approval = self
            .guard
            .with_policy(|policy, _| policy.approve(user, &RequestId::new(id), access, answer, at))
            .map_err(|error| grant_target::answer_failed(&error))?;
        Ok(approval
            .grant
            .map(|grant| (grant_target::grant(&grant, false), kept::granted(&grant))))
    }

    /// What answering `entry` with `answer` does, before anything changes
    /// but the grant the policy makes.
    fn decide(
        &self,
        user: &PresentUser,
        entry: &Entry,
        current: &pb::InboxRequest,
        answer: pb::request_answer::Answer,
        at: SystemTime,
    ) -> Result<Decided, Status> {
        use pb::request_answer::Answer as A;
        let id = &entry.id;
        if entry.permission {
            return permission::decide(id, answer);
        }
        let decided = |state, answer, grant| Decided {
            state,
            answer,
            grant,
        };
        match (&entry.access, answer) {
            (_, answer @ A::Deny(_)) => Ok(decided(pb::RequestState::Denied, answer, None)),
            (None, A::Reply(reply)) if reply.text.trim().is_empty() => {
                Err(status::missing("answer.reply.text"))
            }
            (None, answer @ A::Reply(_)) => Ok(decided(pb::RequestState::Answered, answer, None)),
            (None, A::Approve(_) | A::Always(_)) => Err(status::invalid_field(
                "answer",
                Problem::NotAllowedHere,
                format!("request {id} is a question: it takes a reply, not an approval"),
                "reply with text (one of its choices, or your own), or deny it",
            )),
            (Some(_), A::Reply(_)) => Err(status::invalid_field(
                "answer",
                Problem::NotAllowedHere,
                format!("request {id} asks for access: a reply doesn't answer it"),
                "allow it, allow it with Always allow, or deny it",
            )),
            (Some(access), answer @ A::Approve(_)) => {
                let grant = self.approve(user, id, access, &Answer::Once, at)?;
                Ok(decided(pb::RequestState::Approved, answer, grant))
            }
            (Some(access), A::Always(always)) => {
                let offered = current
                    .always
                    .as_ref()
                    .and_then(|always| always.target.as_ref());
                let target = match (&access.always_let, offered) {
                    (Some(target), Some(offered)) if always.target.as_ref() == Some(offered) => {
                        target.clone()
                    }
                    (Some(_), Some(_)) => {
                        return Err(grant_target::answer_failed(&AnswerError::TargetNotOffered));
                    }
                    _ => {
                        return Err(grant_target::answer_failed(
                            &AnswerError::NoStandingApproval,
                        ));
                    }
                };
                let grant = self.approve(user, id, access, &Answer::AlwaysLet(target), at)?;
                Ok(decided(
                    pb::RequestState::Approved,
                    A::Always(always),
                    grant,
                ))
            }
        }
    }
}

fn sees(caller: &Caller, entry: &Entry) -> bool {
    match caller {
        Caller::User(_) => true,
        Caller::Agent(agent) => agent.session.id.as_str() == entry.session,
        // A script has no session and is not the user: it sees nothing.
        Caller::Script(_) => false,
        // The Inbox isn't on the peer link; relayed answers come over Uplink.
        Caller::Peer => false,
    }
}

fn in_scope(
    scope: Option<&pb::inbox_scope::Scope>,
    entry: &Entry,
    request: &pb::InboxRequest,
) -> bool {
    match scope {
        None | Some(pb::inbox_scope::Scope::Global(_)) => true,
        Some(pb::inbox_scope::Scope::Channel(channel)) => request.channel.as_ref() == Some(channel),
        Some(pb::inbox_scope::Scope::Session(inbox)) => inbox
            .session
            .as_ref()
            .is_some_and(|session| session.value == entry.session),
    }
}

#[tonic::async_trait]
impl pb::inbox_service_server::InboxService for Inbox {
    async fn list_requests(
        &self,
        request: Request<pb::ListRequestsRequest>,
    ) -> Result<Response<pb::ListRequestsResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let scope = request.scope.and_then(|scope| scope.scope);
        let state = self.state.lock().await;
        let mut requests = Vec::new();
        let mut pending_count = 0;
        for entry in state.requests.iter().filter(|entry| sees(&caller, entry)) {
            let current = entry.request.borrow().request.clone();
            if !in_scope(scope.as_ref(), entry, &current) {
                continue;
            }
            let pending = current.state == pb::RequestState::Pending as i32;
            if pending {
                pending_count += 1;
            }
            if pending || request.include_answered {
                requests.push(current);
            }
        }
        Ok(Response::new(pb::ListRequestsResponse {
            requests,
            pending_count,
            page: None,
        }))
    }

    async fn get_request(
        &self,
        request: Request<pb::GetRequestRequest>,
    ) -> Result<Response<pb::InboxRequest>, Status> {
        let caller = identity::caller(&request)?;
        let id = id_of(request.into_inner().id.map(|id| id.value))
            .ok_or_else(|| status::missing("id"))?;
        let found = self.find(&caller, &id).await?;
        let current = found.borrow().request.clone();
        Ok(Response::new(current))
    }

    /// The user's answer. Only the first one counts (IN-15); an approval is
    /// checked against the rule as it is now (IN-21), and "Always allow …"
    /// grants exactly the target the card offered (IN-14).
    async fn answer_request(
        &self,
        request: Request<pb::AnswerRequestRequest>,
    ) -> Result<Response<pb::InboxRequest>, Status> {
        let caller = identity::caller(&request)?;
        // An agent is refused here, its own requests included (IN-23).
        self.guard.require(&caller, &answer())?;
        let user = self.present_user(&caller)?;
        let request = request.into_inner();
        let id = id_of(request.id.map(|id| id.value)).ok_or_else(|| status::missing("id"))?;
        let answer = request
            .answer
            .and_then(|answer| answer.answer)
            .ok_or_else(|| status::missing("answer"))?;
        // Held while answering and saving, so an answer, a second answer
        // and a withdrawal can't interleave.
        let state = self.state.lock().await;
        let entry = state
            .requests
            .iter()
            .find(|entry| entry.id == id && sees(&caller, entry))
            .ok_or_else(|| no_request(&id))?;
        // A permission card whose hook let go has no one to hear the answer.
        let _ = self.withdraw_unheld(entry).await;
        let current = entry.request.borrow().request.clone();
        still_open(&current)?;
        if let Some(expected) = request.expected_revision {
            let actual = current.revision.clone().unwrap_or_default();
            if expected.value != actual.value {
                return Err(revision_conflict(&id, expected, actual));
            }
        }
        let at = SystemTime::now();
        let approving = matches!(
            answer,
            pb::request_answer::Answer::Approve(_) | pb::request_answer::Answer::Always(_)
        );
        if let Some(refused) = self
            .deny_if_never(entry, &caller, &user, approving, at)
            .await?
        {
            self.tell_answer(entry);
            return Err(refused);
        }
        let decided = self.decide(&user, entry, &current, answer, at)?;
        let by = answered_by(&caller, &user);
        let mut next = entry.request.borrow().clone();
        settle(&mut next, decided, by, at);
        self.commit(entry, next, Some(&user), "save the answer")
            .await?;
        self.start(entry);
        Ok(Response::new(self.tell_answer(entry)))
    }

    /// IN-6: answers the listed requests that are still pending and in
    /// scope; the rest are skipped. Approving answers access requests once
    /// and skips questions and ask-aheads, since it never adds grants.
    async fn answer_all(
        &self,
        request: Request<pb::AnswerAllRequest>,
    ) -> Result<Response<pb::AnswerAllResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.guard.require(&caller, &answer())?;
        let user = self.present_user(&caller)?;
        let request = request.into_inner();
        let scope = request.scope.and_then(|scope| scope.scope);
        let at = SystemTime::now();
        let state = self.state.lock().await;
        let mut answered = Vec::new();
        let mut skipped = Vec::new();
        for id in request.ids {
            let entry = state
                .requests
                .iter()
                .find(|entry| entry.id == id.value && sees(&caller, entry));
            // A permission card is one tool call, approved one at a time.
            let Some(entry) = entry.filter(|entry| !entry.permission) else {
                skipped.push(id);
                continue;
            };
            let current = entry.request.borrow().request.clone();
            if current.state != pb::RequestState::Pending as i32
                || !in_scope(scope.as_ref(), entry, &current)
            {
                skipped.push(id);
                continue;
            }
            if self
                .deny_if_never(entry, &caller, &user, request.approve, at)
                .await?
                .is_some()
            {
                answered.push(self.tell_answer(entry));
                continue;
            }
            let decided = if request.approve {
                match &entry.access {
                    Some(access) if !access.ahead => self
                        .approve(&user, &entry.id, access, &Answer::Once, at)
                        .ok()
                        .map(|grant| Decided {
                            state: pb::RequestState::Approved,
                            answer: pb::request_answer::Answer::Approve(pb::Approve {}),
                            grant,
                        }),
                    // A question needs a reply, and approving an ask-ahead
                    // would make a grant.
                    Some(_) | None => None,
                }
            } else {
                Some(Decided {
                    state: pb::RequestState::Denied,
                    answer: pb::request_answer::Answer::Deny(pb::Deny {
                        note: String::new(),
                    }),
                    grant: None,
                })
            };
            let Some(decided) = decided else {
                skipped.push(id);
                continue;
            };
            let by = answered_by(&caller, &user);
            let mut next = entry.request.borrow().clone();
            settle(&mut next, decided, by, at);
            self.commit(entry, next, Some(&user), "save the answer")
                .await?;
            self.start(entry);
            answered.push(self.tell_answer(entry));
        }
        Ok(Response::new(pb::AnswerAllResponse { answered, skipped }))
    }

    /// Puts an agent's question in the Inbox and answers pending at once;
    /// the agent then waits with WaitForAnswer.
    async fn ask_user(
        &self,
        request: Request<pb::AskUserRequest>,
    ) -> Result<Response<pb::WaitResponse>, Status> {
        let caller = identity::caller(&request)?;
        if matches!(caller, Caller::Script(_)) {
            return Err(status::refused(&Refusal::ScriptTokenHasNoSession));
        }
        let Caller::Agent(agent) = &caller else {
            return Err(status::invalid_field(
                "question",
                Problem::NotAllowedHere,
                "only a session's agent asks the user a question; this connection is the user",
                "ask from a Humata session tab, where HUMATA_SESSION_TOKEN is set",
            ));
        };
        // An Ask verdict is fine too: the question is the request.
        self.guard.check(&caller, &inbox_ask(agent))?;
        let request = request.into_inner();
        if request.question.trim().is_empty() {
            return Err(status::missing("question"));
        }
        if request.choices.len() > CHOICES_MAX {
            return Err(status::invalid_field(
                "choices",
                Problem::OutOfRange,
                format!(
                    "the question offers {} choices, more than the {CHOICES_MAX} the Inbox shows",
                    request.choices.len()
                ),
                format!("offer at most {CHOICES_MAX} choices, or none for a free reply"),
            ));
        }
        let session = agent.session.id.to_string();
        let key = id_of(request.idempotency_key.map(|key| key.value))
            .map(|key| format!("{session}/{key}"));
        let mut state = self.state.lock().await;
        if let Some(made) = state.made_with(key.as_deref()) {
            return Ok(Response::new(made));
        }
        let id = new_id();
        let made = question(&id, agent, request.question, request.choices);
        let held = Held::new(made);
        let reply = settled(&held);
        let entry = Entry::new(id, session, None, None, key, held.clone());
        self.commit(&entry, held, None, "save the request").await?;
        self.tell(
            pb::Change::Created,
            &entry.session,
            &entry.request.borrow().request,
        );
        state.add(entry);
        Ok(Response::new(reply))
    }

    /// IN-26: an agent asks ahead for a capability on a target. A grant
    /// that already covers it approves it at once; otherwise the request
    /// waits in the Inbox, and approving it makes that grant. A Never rule
    /// or a user-only capability is refused at once (IN-18, IN-23).
    async fn request_access(
        &self,
        request: Request<pb::RequestAccessRequest>,
    ) -> Result<Response<pb::WaitResponse>, Status> {
        let caller = identity::caller(&request)?;
        if matches!(caller, Caller::Script(_)) {
            return Err(status::refused(&Refusal::ScriptTokenHasNoSession));
        }
        let Caller::Agent(agent) = &caller else {
            return Err(status::invalid_field(
                "capability",
                Problem::NotAllowedHere,
                "only a session's agent asks ahead for access; this connection is the user, who needs no grant",
                "ask from a Humata session tab, where HUMATA_SESSION_TOKEN is set",
            ));
        };
        // Asking ahead is asking; this also loads the channel's rules.
        self.guard.check(&caller, &inbox_ask(agent))?;
        let request = request.into_inner();
        let capability = match request.capability {
            0 => return Err(status::missing("capability")),
            value => grant_target::capability_of(value).ok_or_else(|| {
                status::invalid_field(
                    "capability",
                    Problem::UnknownValue,
                    format!("capability {value} isn't one this humatad2 knows"),
                    "use a capability from humata.v1.Capability, or update humatad2",
                )
            })?,
        };
        let target = request.target.ok_or_else(|| status::missing("target"))?;
        let asked_target = target.clone();
        // The target's session is looked up in the store, which blocks:
        // off the runtime's threads (TR-8).
        let call = {
            let store = Arc::clone(&self.store);
            let machine = self.machine.clone();
            let asking = agent.clone();
            tokio::task::spawn_blocking(move || {
                grant_target::ask_ahead_call(&store, &machine, &asking, capability, target)
            })
            .await
            .map_err(|error| status::internal(format!("a background task failed: {error}"), ""))??
        };
        let session = agent.session.id.to_string();
        let key = id_of(request.idempotency_key.map(|key| key.value))
            .map(|key| format!("{session}/{key}"));
        let mut state = self.state.lock().await;
        if let Some(made) = state.made_with(key.as_deref()) {
            return Ok(Response::new(made));
        }
        let asker = Agent {
            session: agent.session.clone(),
            kind: agent.session_kind(),
        };
        let (verdict, grant, decision) = self.guard.with_policy(|policy, context| {
            let verdict = policy.check_ask_ahead(&asker, &call, context);
            let grant = match &verdict {
                Ok(Verdict::Allowed {
                    source: Source::Grant(id),
                    ..
                }) => policy
                    .grant(*id)
                    .map(|grant| grant_target::grant(grant, false)),
                _ => None,
            };
            let decision = policy
                .channel_rules(&agent.session.channel)
                .and_then(|rules| rules.decision(capability));
            (verdict, grant, decision)
        });
        let asked = match verdict {
            Ok(Verdict::Ask(asked)) => asked,
            // Covered already, by a grant or because it's its own.
            Ok(Verdict::Allowed { .. }) => {
                return Ok(Response::new(approved_at_once(grant)));
            }
            Ok(Verdict::WaitForHost) => return Err(status::refused(&Refusal::HostUnreachable)),
            Ok(Verdict::Refused(refusal)) => return Err(status::refused(&refusal)),
            Err(error @ CheckError::TargetDoesNotFit { .. }) => {
                return Err(status::invalid_field(
                    "target",
                    Problem::NotAllowedHere,
                    status::chain(&error),
                    "name the target the capability acts on",
                ));
            }
            Err(error @ CheckError::UnknownChannel { .. }) => {
                return Err(status::internal(
                    format!("couldn't check the ask-ahead: {}", status::chain(&error)),
                    "",
                ));
            }
        };
        if asked.capability != capability {
            return Err(status::invalid_field(
                "capability",
                Problem::NotAllowedHere,
                format!(
                    "on this target the call needs \"{}\", not \"{capability}\"",
                    asked.capability
                ),
                format!("ask ahead for \"{}\" instead", asked.capability),
            ));
        }
        let id = new_id();
        let made = access_request(&id, agent, &asked, request.reason, decision);
        let held = Held::new(made);
        let reply = settled(&held);
        let entry = Entry::new(
            id,
            session,
            Some(asked),
            Some(asked_target),
            key,
            held.clone(),
        );
        self.commit(&entry, held, None, "save the request").await?;
        self.tell(
            pb::Change::Created,
            &entry.session,
            &entry.request.borrow().request,
        );
        state.add(entry);
        Ok(Response::new(reply))
    }

    type WaitForAnswerStream = ReceiverStream<Result<pb::WaitResponse, Status>>;

    /// Sends a keepalive at once and then about every 15 seconds, and ends
    /// with the request's state once it's answered or the wait is over.
    async fn wait_for_answer(
        &self,
        request: Request<pb::WaitForAnswerRequest>,
    ) -> Result<Response<Self::WaitForAnswerStream>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let id = id_of(request.id.map(|id| id.value)).ok_or_else(|| status::missing("id"))?;
        let mut answers = self.find(&caller, &id).await?.subscribe();
        let wait = Duration::from_secs(u64::from(request.wait_seconds));
        let (sender, receiver) = mpsc::channel(4);
        tokio::spawn(async move {
            let deadline = tokio::time::sleep(wait);
            tokio::pin!(deadline);
            let mut keepalive = tokio::time::interval(KEEPALIVE);
            loop {
                let current = answers.borrow_and_update().clone();
                if !current.waiting() {
                    let _ = sender.send(Ok(settled(&current))).await;
                    return;
                }
                tokio::select! {
                    biased;
                    () = sender.closed() => return,
                    () = &mut deadline => {
                        let _ = sender.send(Ok(settled(&current))).await;
                        return;
                    }
                    changed = answers.changed() => {
                        if changed.is_err() {
                            let _ = sender.send(Ok(settled(&current))).await;
                            return;
                        }
                    }
                    _ = keepalive.tick() => {
                        let still = pb::WaitResponse {
                            id: current.request.id.clone(),
                            state: current.request.state,
                            keepalive: true,
                            ..Default::default()
                        };
                        if sender.send(Ok(still)).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Ok(Response::new(ReceiverStream::new(receiver)))
    }

    /// The asking agent stopped waiting (TP-15). Withdrawing a request
    /// that's no longer pending changes nothing.
    async fn withdraw_request(
        &self,
        request: Request<pb::WithdrawRequestRequest>,
    ) -> Result<Response<pb::InboxRequest>, Status> {
        let caller = identity::caller(&request)?;
        if matches!(caller, Caller::Script(_)) {
            return Err(status::refused(&Refusal::ScriptTokenHasNoSession));
        }
        if let Caller::User(_) = caller {
            return Err(status::invalid_field(
                "id",
                Problem::NotAllowedHere,
                "only the agent that made a request withdraws it",
                "answer or deny the request in the Inbox instead",
            ));
        }
        let id = id_of(request.into_inner().id.map(|id| id.value))
            .ok_or_else(|| status::missing("id"))?;
        // Under the lock, so it can't land in the middle of an answer.
        let state = self.state.lock().await;
        let entry = state
            .requests
            .iter()
            .find(|entry| entry.id == id && sees(&caller, entry))
            .ok_or_else(|| no_request(&id))?;
        let mut next = entry.request.borrow().clone();
        if next.request.state != pb::RequestState::Pending as i32 {
            return Ok(Response::new(next.request));
        }
        // The RPC carries no reason; the MCP server calls it only when the
        // agent's call ended.
        withdraw(&mut next.request, pb::WithdrawReason::CallEnded);
        let current = next.request.clone();
        self.commit(entry, next, None, "save the withdrawal")
            .await?;
        self.tell(pb::Change::Updated, &entry.session, &current);
        Ok(Response::new(current))
    }
}

impl State {
    /// What a repeat of the call that made a request with `key` returns.
    fn made_with(&self, key: Option<&str>) -> Option<pb::WaitResponse> {
        let id = self.keys.get(key?)?;
        let entry = self.requests.iter().find(|entry| &entry.id == id)?;
        Some(settled(&entry.request.borrow()))
    }

    /// The request id the call with `key` made.
    fn id_with(&self, key: Option<&str>) -> Option<String> {
        self.keys.get(key?).cloned()
    }

    fn add(&mut self, entry: Entry) {
        for key in &entry.keys {
            self.keys.insert(key.clone(), entry.id.clone());
        }
        self.requests.push(entry);
    }
}

fn answer() -> Call {
    Call::Use {
        capability: Capability::InboxAnswer,
        target: Target::Global,
    }
}

fn inbox_ask(agent: &AgentCaller) -> Call {
    Call::Use {
        capability: Capability::InboxAsk,
        target: Target::Channel(agent.session.channel.clone()),
    }
}

fn new_id() -> String {
    format!("rq_{}", uuid::Uuid::new_v4().simple())
}

fn question(
    id: &str,
    agent: &AgentCaller,
    question: String,
    choices: Vec<String>,
) -> pb::InboxRequest {
    pb::InboxRequest {
        id: Some(pb::RequestId {
            value: id.to_owned(),
        }),
        channel: Some(pb::ChannelId {
            value: agent.session.channel.to_string(),
        }),
        kind: pb::RequestKind::Question as i32,
        state: pb::RequestState::Pending as i32,
        requester: Some(agent.actor()),
        operation: agent.operation(),
        action_title: format!("{} asks you a question", agent.title),
        preview: Some(pb::RequestPreview {
            preview: Some(pb::request_preview::Preview::Question(
                pb::QuestionPreview { question, choices },
            )),
        }),
        capability: pb::Capability::InboxAsk as i32,
        created_at: Some(attach::now()),
        revision: Some(pb::Revision { value: 1 }),
        ..Default::default()
    }
}

/// The card for an ask-ahead (IN-9, IN-10).
fn access_request(
    id: &str,
    agent: &AgentCaller,
    asked: &AccessRequest,
    reason: String,
    decision: Option<Decision>,
) -> pb::InboxRequest {
    let capability = guard::capability(asked.capability) as i32;
    let offered = asked.always_let.as_ref();
    let target = offered.map(grant_target::target);
    let target_label = offered.map(describe).unwrap_or_default();
    pb::InboxRequest {
        id: Some(pb::RequestId {
            value: id.to_owned(),
        }),
        channel: Some(pb::ChannelId {
            value: asked.channel.to_string(),
        }),
        kind: kind_of(asked.capability) as i32,
        state: pb::RequestState::Pending as i32,
        requester: Some(agent.actor()),
        operation: agent.operation(),
        action_title: format!(
            "{} asks ahead to {}",
            agent.title,
            lowercase_first(asked.capability.label())
        ),
        reason,
        preview: Some(pb::RequestPreview {
            preview: Some(pb::request_preview::Preview::AccessAhead(
                pb::AccessAheadPreview {
                    capability,
                    target: target.clone(),
                    target_label: target_label.clone(),
                },
            )),
        }),
        capability,
        rule_decision: decision.map_or(pb::Decision::Unspecified, decision_of) as i32,
        // Granting is all an approval does; nothing runs to undo.
        undo: pb::Undo::CanBeUndone as i32,
        always: target.map(|target| pb::AlwaysOption {
            label: format!("Always allow “{}” to {target_label}", agent.title),
            target: Some(target),
        }),
        created_at: Some(attach::now()),
        revision: Some(pb::Revision { value: 1 }),
        ..Default::default()
    }
}

/// A grant target in words, for the card.
fn describe(target: &GrantTarget) -> String {
    match target {
        GrantTarget::Session(session) => format!("reach session {session}"),
        GrantTarget::Board(board) => {
            format!("change others' items on the {} board", board_name(board))
        }
        GrantTarget::Path(scope) => {
            let folder = if scope.prefix.as_str().is_empty() {
                "everything".to_owned()
            } else {
                format!("files under {}", scope.prefix.as_str())
            };
            match &scope.root {
                humata2_access::Root::Clone(owner) => {
                    format!("use {folder} of session {}", owner.id)
                }
                _ => format!("use {folder} on machine {}", scope.machine),
            }
        }
        GrantTarget::Network(network) => format!(
            "connect to {}:{} from machine {}",
            network.host, network.port, network.machine
        ),
        GrantTarget::AgentOnMachine { machine, agent } => {
            format!("start {agent} sessions on machine {machine}")
        }
        GrantTarget::RepoBranch(branch) => {
            format!("push branch {} of {}", branch.branch, branch.repo)
        }
        GrantTarget::MachineAction(action) => {
            use humata2_access::MachineActionKind as K;
            let verb = match action.action {
                K::Add => "add",
                K::Create => "create",
                K::Start => "start",
                K::Stop => "stop",
                K::Reconnect => "reconnect",
                K::SyncSkills => "sync skills to",
                K::Remove => "remove",
                K::Forget => "forget",
                K::Delete => "delete",
            };
            format!("{verb} machine {}", action.machine)
        }
    }
}

fn board_name(board: &humata2_access::BoardRef) -> String {
    match board {
        humata2_access::BoardRef::Global => "global".to_owned(),
        humata2_access::BoardRef::Channel(channel) => format!("channel {channel}'s"),
        humata2_access::BoardRef::Session(session) => format!("session {}'s", session.id),
    }
}

fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// IN-5's icon for a request about `capability`.
fn kind_of(capability: Capability) -> pb::RequestKind {
    use Capability as C;
    match capability {
        C::FilesReadOtherSession | C::FilesCopyIn | C::FilesFetchLocal | C::SharedWrite => {
            pb::RequestKind::Files
        }
        C::FilesReadOtherMachine | C::NetworkReach => pb::RequestKind::Bridge,
        C::ChannelBoardAdd
        | C::ChannelBoardEditOthers
        | C::GlobalBoardWrite
        | C::ChannelBoardEditOwn
        | C::OwnBoardWrite => pb::RequestKind::Board,
        C::SessionNote
        | C::SessionType
        | C::SessionStartSameMachine
        | C::SessionStartOtherMachine
        | C::SessionMove
        | C::SessionStopOther
        | C::GitPush => pb::RequestKind::Session,
        C::MachineProvision | C::MachineDestroy => pb::RequestKind::Machine,
        _ => pb::RequestKind::Channel,
    }
}

fn decision_of(decision: Decision) -> pb::Decision {
    match decision {
        Decision::Allow => pb::Decision::Allow,
        Decision::Ask => pb::Decision::Ask,
        Decision::Never => pb::Decision::Never,
    }
}

/// Records the answer on the request; waits on it wake.
fn settle(held: &mut Held, decided: Decided, by: pb::Actor, at: SystemTime) {
    let request = &mut held.request;
    request.state = decided.state as i32;
    request.answer = Some(pb::RequestAnswer {
        answer: Some(decided.answer),
    });
    request.answered = Some(pb::Timestamped {
        at: Some(grant_target::timestamp(at)),
        by: Some(by),
    });
    request.revision = Some(next_revision(request));
    (held.grant, held.granted) = decided.grant.unzip();
    // An approved deferred call runs next; waits go on until it ends.
    held.running = held.deferred && decided.state == pb::RequestState::Approved;
}

/// Withdraws a pending `request` for `reason`.
fn withdraw(request: &mut pb::InboxRequest, reason: pb::WithdrawReason) {
    request.state = pb::RequestState::Withdrawn as i32;
    request.withdraw_reason = reason as i32;
    request.revision = Some(next_revision(request));
}

fn next_revision(request: &pb::InboxRequest) -> pb::Revision {
    pb::Revision {
        value: request
            .revision
            .as_ref()
            .map_or(0, |revision| revision.value)
            + 1,
    }
}

/// The agent that asked `request`.
fn requester(request: &pb::InboxRequest) -> Option<pb::Actor> {
    request.requester.clone().map(|agent| pb::Actor {
        actor: Some(pb::actor::Actor::Agent(agent)),
    })
}

/// Who answered: the user, the way they reached this daemon.
fn answered_by(caller: &Caller, user: &PresentUser) -> pb::Actor {
    let via = match caller {
        Caller::User(UserVia::CliLogin { .. }) if user.on_host() => pb::user_actor::Via::CliOnHost,
        Caller::User(UserVia::CliLogin { .. }) => pb::user_actor::Via::CliOnMachine,
        _ => pb::user_actor::Via::App,
    };
    pb::Actor {
        actor: Some(pb::actor::Actor::User(pb::UserActor {
            via: via as i32,
            machine: Some(pb::MachineId {
                value: user.machine().to_string(),
            }),
        })),
    }
}

/// An ask-ahead something already covers: approved, with no request made.
fn approved_at_once(grant: Option<pb::Grant>) -> pb::WaitResponse {
    let (result, result_type) = match &grant {
        Some(grant) => (grant.encode_to_vec(), GRANT_TYPE.to_owned()),
        None => (Vec::new(), String::new()),
    };
    pb::WaitResponse {
        state: pb::RequestState::Approved as i32,
        answer: Some(pb::RequestAnswer {
            answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
        }),
        result,
        result_type,
        ..Default::default()
    }
}

/// Fails when `request` can no longer be answered (IN-15).
fn still_open(request: &pb::InboxRequest) -> Result<(), Status> {
    let id = request.id.clone();
    let value = id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
    match pb::RequestState::try_from(request.state) {
        Ok(pb::RequestState::Pending) => Ok(()),
        Ok(pb::RequestState::Withdrawn) => Err(status::failure(
            Code::FailedPrecondition,
            status::error(
                format!(
                    "request {value} was withdrawn: the agent stopped waiting for it before you answered"
                ),
                "nothing to answer; if the agent still needs it, it will ask again",
                pb::error::Kind::RequestWithdrawn(pb::RequestWithdrawn { request: id }),
            ),
        )),
        _ => {
            let approved = request.state == pb::RequestState::Approved as i32;
            Err(status::failure(
                Code::Aborted,
                status::error(
                    format!(
                        "request {value} was already answered: {}",
                        answered_how(request)
                    ),
                    "nothing to do: the first answer stands; if it should change, deny the agent's next request or change the rule in Agent access",
                    pb::error::Kind::AlreadyAnswered(pb::AlreadyAnswered {
                        request: id,
                        answered: request.answered.clone(),
                        approved,
                    }),
                ),
            ))
        }
    }
}

/// "approved by you in the app 12 s ago", for AW-10.
fn answered_how(request: &pb::InboxRequest) -> String {
    let what = match pb::RequestState::try_from(request.state) {
        Ok(pb::RequestState::Approved) => "approved",
        Ok(pb::RequestState::Denied) => "denied",
        _ => "replied to",
    };
    let Some(answered) = &request.answered else {
        return what.to_owned();
    };
    let how = match answered.by.as_ref().and_then(|by| by.actor.as_ref()) {
        Some(pb::actor::Actor::User(user)) => {
            let machine = user
                .machine
                .as_ref()
                .map(|machine| machine.value.as_str())
                .unwrap_or_default();
            match pb::user_actor::Via::try_from(user.via) {
                Ok(pb::user_actor::Via::CliOnHost) => " by you with humata on the Mac".to_owned(),
                Ok(pb::user_actor::Via::CliOnMachine) => {
                    format!(" by you with humata on machine {machine}")
                }
                _ => " by you in the app".to_owned(),
            }
        }
        _ => String::new(),
    };
    let when = answered
        .at
        .as_ref()
        .map(|at| {
            let now = attach::now().seconds;
            format!(" {} s ago", (now - at.seconds).max(0))
        })
        .unwrap_or_default();
    format!("{what}{how}{when}")
}

fn revision_conflict(id: &str, expected: pb::Revision, actual: pb::Revision) -> Status {
    status::failure(
        Code::Aborted,
        status::error(
            format!(
                "request {id} changed since you saw it (revision {}, now {})",
                expected.value, actual.value
            ),
            "look at the request again and answer what it asks now",
            pb::error::Kind::RevisionConflict(pb::RevisionConflict {
                kind: pb::EntityKind::Request as i32,
                id: id.to_owned(),
                expected: Some(expected),
                actual: Some(actual),
            }),
        ),
    )
}

/// What a wait ends with for `held` as it stands.
fn settled(held: &Held) -> pb::WaitResponse {
    let request = &held.request;
    let id = request.id.clone();
    let value = id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
    let error = match pb::RequestState::try_from(request.state) {
        Ok(pb::RequestState::Withdrawn) => Some(pb::Error {
            message: format!("request {value} was withdrawn"),
            fix: "ask again if it still matters".to_owned(),
            retry: None,
            kind: Some(pb::error::Kind::RequestWithdrawn(pb::RequestWithdrawn {
                request: id.clone(),
            })),
        }),
        Ok(pb::RequestState::Denied) => Some(pb::Error {
            message: format!("the user denied request {value}"),
            fix: "don't ask again for the same thing; ask the user what they want instead"
                .to_owned(),
            retry: None,
            kind: Some(pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::UserDenied as i32,
                capability: request.capability,
                request: id.clone(),
                user_note: match &request.answer {
                    Some(pb::RequestAnswer {
                        answer: Some(pb::request_answer::Answer::Deny(deny)),
                    }) => deny.note.clone(),
                    _ => String::new(),
                },
            })),
        }),
        _ => None,
    };
    let (mut result, mut result_type) = match &held.grant {
        Some(grant) => (grant.encode_to_vec(), GRANT_TYPE.to_owned()),
        None => (Vec::new(), String::new()),
    };
    let mut error = error;
    let mut result_kept_until = None;
    if let Some(outcome) = &held.outcome {
        if outcome.error.is_some() {
            error.clone_from(&outcome.error);
        } else {
            result_kept_until = Some(grant_target::timestamp(outcome.kept_until));
            // After the keep time the state is still told, the result isn't.
            if SystemTime::now() <= outcome.kept_until {
                result.clone_from(&outcome.result);
                result_type.clone_from(&outcome.result_type);
            }
        }
    } else if deferred::lost(held) {
        error = Some(deferred::lost_error(&value));
    }
    pb::WaitResponse {
        id,
        state: request.state,
        answer: request.answer.clone(),
        result,
        result_type,
        error,
        result_kept_until,
        ..Default::default()
    }
}

fn no_request(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Request,
        id,
        format!("no Inbox request {id} is yours on this machine"),
        "check the request id; a request goes when its session is deleted",
    )
}

fn id_of(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}
