//! InboxService: questions an agent asks the user, access it asks for
//! ahead (IN-26), the user's answers, and waiting for them (MCP.md §5).
//! Approving an ask-ahead or answering "Always let …" makes a grant in the
//! policy (IN-14, IN-22). Requests and grants live in memory until the
//! daemon stops. Each new request and each change to one goes on the
//! event feed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use humata2_access::{
    AccessRequest, Agent, Answer, AnswerError, Call, Capability, CheckError, Decision, GrantTarget,
    PresentUser, Refusal, RequestId, SessionKind, Source, Target, Verdict,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;
use pb::invalid_field::Problem;
use prost::Message;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Request, Response, Status};

use crate::attach;
use crate::events::{Feed, Scope};
use crate::grant_target;
use crate::guard::{self, Guard};
use crate::identity::{self, AgentCaller, Caller, UserVia};
use crate::status;

/// How often a wait says it is still waiting (MCP.md §5).
const KEEPALIVE: Duration = Duration::from_secs(15);

/// The most choices a question offers.
const CHOICES_MAX: usize = 4;

/// What a wait's `result` holds when the answer made a grant.
const GRANT_TYPE: &str = "humata.v1.Grant";

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
    request: watch::Sender<Held>,
}

/// A request as it stands, and the grant its answer made.
#[derive(Clone)]
struct Held {
    request: pb::InboxRequest,
    grant: Option<pb::Grant>,
}

impl Held {
    fn new(request: pb::InboxRequest) -> Self {
        Self {
            request,
            grant: None,
        }
    }
}

/// How the user answered one request.
struct Decided {
    state: pb::RequestState,
    answer: pb::request_answer::Answer,
    grant: Option<pb::Grant>,
}

impl Inbox {
    pub(crate) fn new(guard: Arc<Guard>, store: Arc<Store>, machine: String, feed: Arc<Feed>) -> Self {
        Self {
            guard,
            store,
            machine,
            feed,
            state: Mutex::new(State::default()),
        }
    }

    /// The request `id`, if `caller` may see it: the user sees every
    /// request, an agent its own session's.
    fn find(&self, caller: &Caller, id: &str) -> Result<watch::Sender<Held>, Status> {
        lock(&self.state)
            .requests
            .iter()
            .find(|entry| entry.id == id && sees(caller, entry))
            .map(|entry| entry.request.clone())
            .ok_or_else(|| no_request(id))
    }

    /// Tells the feed of `session`'s request as it now stands.
    fn tell(&self, change: pb::Change, session: &str, request: &pb::InboxRequest) {
        let scope = Scope {
            channel: request.channel.as_ref().map(|channel| channel.value.clone()).unwrap_or_default(),
            session: session.to_owned(),
        };
        self.feed
            .publish_in(change, pb::event::Payload::Request(request.clone()), scope);
    }

    /// Tells the feed of the grant `entry`'s answer made, if any, then of
    /// the answered request.
    fn tell_answer(&self, entry: &Entry) -> pb::InboxRequest {
        let held = entry.request.borrow().clone();
        if let Some(grant) = held.grant {
            let scope = Scope {
                channel: grant.channel.as_ref().map(|channel| channel.value.clone()).unwrap_or_default(),
                session: entry.session.clone(),
            };
            self.feed
                .publish_in(pb::Change::Created, pb::event::Payload::Grant(grant), scope);
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
    ) -> Result<Option<pb::Grant>, Status> {
        let approval = self
            .guard
            .with_policy(|policy, _| policy.approve(user, &RequestId::new(id), access, answer, at))
            .map_err(|error| grant_target::answer_failed(&error))?;
        Ok(approval.grant.map(|grant| grant_target::grant(&grant, false)))
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
                "approve it, approve it with Always let, or deny it",
            )),
            (Some(access), answer @ A::Approve(_)) => {
                let grant = self.approve(user, id, access, &Answer::Once, at)?;
                Ok(decided(pb::RequestState::Approved, answer, grant))
            }
            (Some(access), A::Always(always)) => {
                let offered = current.always.as_ref().and_then(|always| always.target.as_ref());
                let target = match (&access.always_let, offered) {
                    (Some(target), Some(offered)) if always.target.as_ref() == Some(offered) => {
                        target.clone()
                    }
                    (Some(_), Some(_)) => {
                        return Err(grant_target::answer_failed(&AnswerError::TargetNotOffered));
                    }
                    _ => return Err(grant_target::answer_failed(&AnswerError::NoStandingApproval)),
                };
                let grant = self.approve(user, id, access, &Answer::AlwaysLet(target), at)?;
                Ok(decided(pb::RequestState::Approved, A::Always(always), grant))
            }
        }
    }
}

fn sees(caller: &Caller, entry: &Entry) -> bool {
    match caller {
        Caller::User(_) => true,
        Caller::Agent(agent) => agent.session.id.as_str() == entry.session,
        // The Inbox isn't on the peer link; relayed answers come over Uplink.
        Caller::Peer => false,
    }
}

fn in_scope(scope: Option<&pb::inbox_scope::Scope>, entry: &Entry, request: &pb::InboxRequest) -> bool {
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
        let state = lock(&self.state);
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
        let id = id_of(request.into_inner().id.map(|id| id.value)).ok_or_else(|| status::missing("id"))?;
        let found = self.find(&caller, &id)?;
        let current = found.borrow().request.clone();
        Ok(Response::new(current))
    }

    /// The user's answer. Only the first one counts (IN-15); an approval is
    /// checked against the rule as it is now (IN-21), and "Always let …"
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
        // Held while answering, so an answer, a second answer and a
        // withdrawal can't interleave.
        let state = lock(&self.state);
        let entry = state
            .requests
            .iter()
            .find(|entry| entry.id == id && sees(&caller, entry))
            .ok_or_else(|| no_request(&id))?;
        let current = entry.request.borrow().request.clone();
        still_open(&current)?;
        if let Some(expected) = request.expected_revision {
            let actual = current.revision.clone().unwrap_or_default();
            if expected.value != actual.value {
                return Err(revision_conflict(&id, expected, actual));
            }
        }
        let at = SystemTime::now();
        let decided = self.decide(&user, entry, &current, answer, at)?;
        let by = answered_by(&caller, &user);
        entry
            .request
            .send_modify(|held| settle(held, decided, by, at));
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
        let state = lock(&self.state);
        let mut answered = Vec::new();
        let mut skipped = Vec::new();
        for id in request.ids {
            let entry = state
                .requests
                .iter()
                .find(|entry| entry.id == id.value && sees(&caller, entry));
            let Some(entry) = entry else {
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
            entry
                .request
                .send_modify(|held| settle(held, decided, by, at));
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
        let mut state = lock(&self.state);
        if let Some(made) = state.made_with(key.as_deref()) {
            return Ok(Response::new(made));
        }
        let id = new_id();
        let made = question(&id, agent, request.question, request.choices);
        let held = Held::new(made);
        let reply = settled(&held);
        self.tell(pb::Change::Created, &session, &held.request);
        state.add(key, id, session, None, held);
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
        let mut state = lock(&self.state);
        if let Some(made) = state.made_with(key.as_deref()) {
            return Ok(Response::new(made));
        }
        let asker = Agent {
            session: agent.session.clone(),
            kind: SessionKind::Ordinary,
        };
        let (verdict, grant, decision) = self.guard.with_policy(|policy, context| {
            let verdict = policy.check_ask_ahead(&asker, &call, context);
            let grant = match &verdict {
                Ok(Verdict::Allowed {
                    source: Source::Grant(id),
                    ..
                }) => policy.grant(*id).map(|grant| grant_target::grant(grant, false)),
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
        self.tell(pb::Change::Created, &session, &held.request);
        state.add(key, id, session, Some(asked), held);
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
        let mut answers = self.find(&caller, &id)?.subscribe();
        let wait = Duration::from_secs(u64::from(request.wait_seconds));
        let (sender, receiver) = mpsc::channel(4);
        tokio::spawn(async move {
            let deadline = tokio::time::sleep(wait);
            tokio::pin!(deadline);
            let mut keepalive = tokio::time::interval(KEEPALIVE);
            loop {
                let current = answers.borrow_and_update().clone();
                if current.request.state != pb::RequestState::Pending as i32 {
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
        if let Caller::User(_) = caller {
            return Err(status::invalid_field(
                "id",
                Problem::NotAllowedHere,
                "only the agent that made a request withdraws it",
                "answer or deny the request in the Inbox instead",
            ));
        }
        let id = id_of(request.into_inner().id.map(|id| id.value)).ok_or_else(|| status::missing("id"))?;
        // Under the lock, so it can't land in the middle of an answer.
        let state = lock(&self.state);
        let entry = state
            .requests
            .iter()
            .find(|entry| entry.id == id && sees(&caller, entry))
            .ok_or_else(|| no_request(&id))?;
        let withdrawn = entry.request.send_if_modified(|held| {
            let current = &mut held.request;
            if current.state != pb::RequestState::Pending as i32 {
                return false;
            }
            current.state = pb::RequestState::Withdrawn as i32;
            // The RPC carries no reason; the MCP server calls it only
            // when the agent's call ended.
            current.withdraw_reason = pb::WithdrawReason::CallEnded as i32;
            current.revision = Some(next_revision(current));
            true
        });
        let current = entry.request.borrow().request.clone();
        if withdrawn {
            self.tell(pb::Change::Updated, &entry.session, &current);
        }
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

    fn add(
        &mut self,
        key: Option<String>,
        id: String,
        session: String,
        access: Option<AccessRequest>,
        held: Held,
    ) {
        self.requests.push(Entry {
            id: id.clone(),
            session,
            access,
            request: watch::channel(held).0,
        });
        if let Some(key) = key {
            self.keys.insert(key, id);
        }
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

fn question(id: &str, agent: &AgentCaller, question: String, choices: Vec<String>) -> pb::InboxRequest {
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
        action_title: format!("{} asks you a question", agent.title),
        preview: Some(pb::RequestPreview {
            preview: Some(pb::request_preview::Preview::Question(pb::QuestionPreview {
                question,
                choices,
            })),
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
        action_title: format!(
            "{} asks ahead to {}",
            agent.title,
            lowercase_first(asked.capability.label())
        ),
        reason,
        preview: Some(pb::RequestPreview {
            preview: Some(pb::request_preview::Preview::AccessAhead(pb::AccessAheadPreview {
                capability,
                target: target.clone(),
                target_label: target_label.clone(),
            })),
        }),
        capability,
        rule_decision: decision.map_or(pb::Decision::Unspecified, decision_of) as i32,
        // Granting is all an approval does; nothing runs to undo.
        undo: pb::Undo::CanBeUndone as i32,
        always: target.map(|target| pb::AlwaysOption {
            label: format!("Always let “{}” {target_label}", agent.title),
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
        GrantTarget::Board(board) => format!("change others' items on the {} board", board_name(board)),
        GrantTarget::Path(scope) => {
            let folder = if scope.prefix.as_str().is_empty() {
                "everything".to_owned()
            } else {
                format!("files under {}", scope.prefix.as_str())
            };
            match &scope.root {
                humata2_access::Root::Clone(owner) => format!("use {folder} of session {}", owner.id),
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
        C::ChannelBoardAdd | C::ChannelBoardEditOthers | C::ChannelBoardEditOwn | C::OwnBoardWrite => {
            pb::RequestKind::Board
        }
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
    held.grant = decided.grant;
}

fn next_revision(request: &pb::InboxRequest) -> pb::Revision {
    pb::Revision {
        value: request.revision.as_ref().map_or(0, |revision| revision.value) + 1,
    }
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
                format!("request {value} was withdrawn: the agent stopped waiting for it before you answered"),
                "nothing to answer; if the agent still needs it, it will ask again",
                pb::error::Kind::RequestWithdrawn(pb::RequestWithdrawn { request: id }),
            ),
        )),
        _ => {
            let approved = request.state == pb::RequestState::Approved as i32;
            Err(status::failure(
                Code::Aborted,
                status::error(
                    format!("request {value} was already answered: {}", answered_how(request)),
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
            let machine = user.machine.as_ref().map(|machine| machine.value.as_str()).unwrap_or_default();
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
            fix: "don't ask again for the same thing; ask the user what they want instead".to_owned(),
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
    let (result, result_type) = match &held.grant {
        Some(grant) => (grant.encode_to_vec(), GRANT_TYPE.to_owned()),
        None => (Vec::new(), String::new()),
    };
    pb::WaitResponse {
        id,
        state: request.state,
        answer: request.answer.clone(),
        result,
        result_type,
        error,
        ..Default::default()
    }
}

fn no_request(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Request,
        id,
        format!("no Inbox request {id} is yours on this machine"),
        "check the request id; requests are kept only until humatad2 stops",
    )
}

fn id_of(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
