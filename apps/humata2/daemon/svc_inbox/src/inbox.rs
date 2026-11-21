//! InboxService: questions an agent asks the user, access it asks for
//! ahead (IN-26), the user's answers, and waiting for them (MCP.md §5).
//! Approving an ask-ahead or answering "Always allow …" makes a grant in the
//! policy (IN-14, IN-22). Each request, its answer and the grant it made
//! are saved before anyone hears of them, so a restart keeps the Inbox
//! and the grants (D96). Each new request and each change to one goes on
//! the event feed.

mod agent_inbox;
mod browser;
mod deferred;
mod facts;
mod forwarded;
mod host_asks;
mod kept;
mod migrate;
mod migrated;
pub mod permission;
mod relay;
mod removed;
mod replay;

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use humata2_access::{
    AccessRequest, Agent, Answer, AnswerError, Call, Capability, CheckError, Decision, GrantTarget,
    PresentUser, Principal, Refusal, RequestId, Source, Target, Verdict,
};
use humata2_daemon_ports::{AgentCaller, Caller, UserVia};
use humata2_grpc::humata::v1 as pb;
use humata2_rpc_kit::helpers as status;
use humata2_sessions_core::Agents;
use humata2_store::Store;
use humata2_store_proto::humata::store::v1 as stored;
use humata2_svc_away::Forwarder;
use pb::invalid_field::Problem;
use prost::Message;
use tokio::sync::{Mutex, mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Request, Response, Status};

use crate::ports::{
    AgentGatePort, AskingGuardPort, EventScope, InboxFeedPort, InboxGuardPort,
    MigrateApproverPort, NoopAsking, caller as request_caller, now_timestamp, store_status,
    timestamp, with_policy,
};

use deferred::Outcome;
pub use deferred::{BoardAsk, Deferred, Run};
pub use permission::MoveSwitchError;
pub use relay::{Registered, Relays};
use relay::Relayed;
pub use replay::Replayed;

/// How often a wait says it is still waiting (MCP.md §5).
/// The longest explanation an agent gives for a request (INBOXHOOKS).
const MAX_EXPLANATION: usize = 1000;
const KEEPALIVE: Duration = Duration::from_secs(15);
/// D158.7: how long a session stays Needs you after a pending return or a
/// wait that ended with no answer, for the agent's next wait to come.
const ASK_GAP: Duration = Duration::from_secs(30);

/// The most choices a question offers.
const CHOICES_MAX: usize = 4;

/// What a wait's `result` holds when the answer made a grant.
pub const GRANT_TYPE: &str = "humata.v1.Grant";

pub struct Inbox {
    guard: Arc<dyn InboxGuardPort>,
    /// Where the sessions an ask-ahead names are looked up.
    store: Arc<Store>,
    /// This machine's id.
    machine: String,
    /// Where new requests and their answers are told.
    feed: Arc<dyn InboxFeedPort>,
    state: Mutex<State>,
    /// The machines' live links, and the answers the host passed down them (D226).
    relays: Arc<Relays>,
    /// On a machine: the host's link, where a wait on a request the host holds
    /// goes up (D268). Set once the link's parts are made.
    forwarder: std::sync::OnceLock<Arc<Forwarder>>,
    /// What carries out a click on a Migrate item (UP-1). Set once the
    /// host's Migrate items are made.
    migrator: std::sync::OnceLock<Arc<dyn MigrateApproverPort>>,
    /// The agents this daemon runs, whose `agents.toml` says where each
    /// kind's own asks go (D782). Unset, every kind asks in the Inbox.
    agents: std::sync::OnceLock<Agents>,
    /// Marks a session as currently asking the user.
    asking: Arc<dyn AskingGuardPort>,
    /// Reads per-agent permission switches.
    gate: std::sync::OnceLock<Arc<dyn AgentGatePort>>,
    /// In-memory per-session 30-minute browser read grants and consumed approvals.
    browser: std::sync::Mutex<browser::BrowserGrants>,
}

#[derive(Default)]
struct State {
    requests: Vec<Entry>,
    /// The request each asking session's idempotency key made.
    keys: HashMap<String, String>,
}

/// `REQUEST_KIND_MIGRATE` (D281): a needed update outlives a stop, so the
/// stop's sweep leaves it pending.
const MIGRATE_KIND: i32 = 8;

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
    /// Its reply must be one of the card's choices.
    only_choices: bool,
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
            only_choices: false,
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
    /// An asked read's result (D768): its bytes are dropped once a wait
    /// delivers them, a repeat of the read goes through, or `READ_HELD`
    /// passes, and later waits are told to repeat the read. The approval
    /// keeps its usual keep time.
    once: bool,
    /// The read's result was delivered or its read repeated: bytes that
    /// arrive after this are dropped at once.
    taken: bool,
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
            once: false,
            taken: false,
        }
    }

    /// Whether a wait on this request has more to wait for.
    fn waiting(&self) -> bool {
        self.pending() || self.running
    }

    /// The user pressed Ask why and the agent hasn't answered yet: a wait
    /// ends with this so the agent can explain (INBOXHOOKS).
    fn asking_why(&self) -> bool {
        self.pending()
            && self
                .request
                .asked_why
                .as_ref()
                .is_some_and(|asked| asked.asked_at.is_some() && asked.reply.is_empty())
    }

    /// Whether the request still waits for the user's answer.
    fn pending(&self) -> bool {
        self.request.state == pb::RequestState::Pending as i32
    }
}

/// Keeps `session` Needs you for [`ASK_GAP`] while its request stays
/// pending, for the agent's next wait on it (D158.7). Ends at once when the
/// request is answered or withdrawn.
fn hold_through_gap(asking_port: &Arc<dyn AskingGuardPort>, session: &str, mut answers: watch::Receiver<Held>) {
    if !answers.borrow().pending() {
        return;
    }
    // A runtime shutting down has no gap to keep.
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let asking = asking_port.hold(session);
    // From when the wait ended, not from when the task is first run.
    let ends = tokio::time::Instant::now() + ASK_GAP;
    runtime.spawn(async move {
        let _asking = asking;
        let gap = tokio::time::sleep_until(ends);
        tokio::pin!(gap);
        loop {
            if !answers.borrow_and_update().pending() {
                return;
            }
            tokio::select! {
                () = &mut gap => return,
                changed = answers.changed() => if changed.is_err() { return },
            }
        }
    });
}

/// Starts the gap after a wait when the wait ends, however it ends.
struct Gap(Arc<dyn AskingGuardPort>, String, watch::Receiver<Held>);

impl Drop for Gap {
    fn drop(&mut self) {
        hold_through_gap(&self.0, &self.1, self.2.clone());
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
    pub fn new(
        guard: Arc<dyn InboxGuardPort>,
        store: Arc<Store>,
        machine: String,
        feed: Arc<dyn InboxFeedPort>,
    ) -> Self {
        let state = kept::restore(&*guard, &store, &machine);
        Self {
            guard,
            store,
            machine,
            feed,
            state: Mutex::new(state),
            relays: Arc::new(Relays::new()),
            forwarder: std::sync::OnceLock::new(),
            migrator: std::sync::OnceLock::new(),
            agents: std::sync::OnceLock::new(),
            asking: Arc::new(NoopAsking),
            gate: std::sync::OnceLock::new(),
            browser: std::sync::Mutex::new(browser::BrowserGrants::default()),
        }
    }

    /// Installs the asking guard port used to mark sessions as needing user input.
    pub fn with_asking(mut self, asking: Arc<dyn AskingGuardPort>) -> Self {
        self.asking = asking;
        self
    }

    /// Installs the agent gate port used to read per-agent permission switches.
    pub fn use_gate(&self, gate: Arc<dyn AgentGatePort>) {
        let _ = self.gate.set(gate);
    }

    /// Reads where each agent kind asks from `agents`' switches file from
    /// now on (D782). A second call is ignored.
    pub fn use_agents(&self, agents: Agents) {
        let _ = self.agents.set(agents);
    }

    /// The machines' live links and the answers sent down them: each link
    /// loop of the host registers its machine here, and the replay reads
    /// what was sent.
    pub fn relays(&self) -> &Arc<Relays> {
        &self.relays
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
            if let (Some(user), Some(id)) = (user, made.and_then(|g| self.guard.grant_number(g))) {
                // It was just made, so taking it back can't fail for want
                // of the host; if it did, a restart drops it anyway.
                let _ = with_policy(&*self.guard, |policy, _| {
                    policy.revoke_grant(user, humata2_access::GrantId::new(id))
                });
            }
            return Err(store_status(error, &self.machine, doing));
        }
        // The machine's journal takes the change too, as it is made (D225).
        let before = entry.request.borrow().request.state();
        humata2_svc_away::record_journal(
            &self.store,
            &self.guard.host_link(),
            &entry.session,
            before,
            &next.request,
        )
        .await;
        entry.request.send_replace(next);
        Ok(())
    }

    /// Marks the grant request `id`'s answer made as revoked, so a restart
    /// doesn't bring it back.
    pub async fn revoked(&self, id: &str, at: SystemTime) -> Result<(), Status> {
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
                .and_then(|g| self.guard.grant_number(g))
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

    /// The host's mirror of request `id`, when `machine` holds it, `session`
    /// made it and the user approved it: what a journaled board write names
    /// as its approval (AW-9, D268). `None` when any of those isn't so.
    pub async fn approved_on(&self, machine: &str, id: &str, session: &str) -> Option<pb::InboxRequest> {
        let state = self.state.lock().await;
        let held = state
            .requests
            .iter()
            .find(|entry| entry.id == id && entry.session == session)?
            .request
            .borrow()
            .request
            .clone();
        let from = held
            .journal
            .as_ref()
            .and_then(|position| position.machine.as_ref())
            .map(|from| from.value.as_str());
        (held.state == pb::RequestState::Approved as i32 && from == Some(machine)).then_some(held)
    }

    /// The ids of the pending requests of any of `sessions`, for a plan
    /// that lists what deleting them would withdraw (CH-24).
    pub async fn pending_ids_of(&self, sessions: &[String]) -> Vec<String> {
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
    pub async fn withdraw_pending_of(
        &self,
        session: &str,
        reason: pb::WithdrawReason,
    ) -> Result<(), Status> {
        self.withdraw_pending_where(reason, |entry| entry.session == session)
            .await
    }

    /// Withdraws every pending request of this machine for `reason`, as
    /// when its sessions are stopped (D264).
    pub async fn withdraw_all_pending(
        &self,
        reason: pb::WithdrawReason,
    ) -> Result<(), Status> {
        self.withdraw_pending_where(reason, |entry| {
            entry.request.borrow().request.kind != MIGRATE_KIND && !host_asks::is_host_ask(entry)
        })
        .await
    }

    async fn withdraw_pending_where(
        &self,
        reason: pb::WithdrawReason,
        which: impl Fn(&Entry) -> bool,
    ) -> Result<(), Status> {
        let state = self.state.lock().await;
        // One that can't be saved doesn't stop the others being withdrawn:
        // the first failure is what the caller hears, after the rest.
        let mut failed = None;
        for entry in state.requests.iter().filter(|entry| which(entry)) {
            let mut next = entry.request.borrow().clone();
            if next.request.state != pb::RequestState::Pending as i32 {
                continue;
            }
            withdraw(&mut next.request, reason);
            let current = next.request.clone();
            if let Err(error) = self
                .commit(entry, next, None, "save the withdrawal")
                .await
            {
                failed.get_or_insert(error);
                continue;
            }
            entry
                .run
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            self.tell(pb::Change::Updated, &entry.session, &current);
        }
        failed.map_or(Ok(()), Err)
    }

    /// Tells the feed of `session`'s request as it now stands.
    fn tell(&self, change: pb::Change, session: &str, request: &pb::InboxRequest) {
        let scope = EventScope {
            channel: request.channel.as_ref().map(|channel| channel.value.clone()),
            session: (!session.is_empty()).then(|| session.to_owned()),
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
        self.feed.publish(
            change,
            pb::event::Payload::Request(request.clone()),
            by,
            scope,
        );
    }

    /// Tells the feed of the grant `entry`'s answer made, if any, then of
    /// the answered request.
    fn tell_answer(&self, entry: &Entry) -> pb::InboxRequest {
        self.tell_grant(entry);
        let held = entry.request.borrow().clone();
        self.tell(pb::Change::Updated, &entry.session, &held.request);
        held.request
    }

    /// Tells the feed of the grant `entry`'s answer made, if any.
    fn tell_grant(&self, entry: &Entry) {
        let held = entry.request.borrow().clone();
        let Some(grant) = held.grant else {
            return;
        };
        let scope = EventScope {
            channel: grant.channel.as_ref().map(|channel| channel.value.clone()),
            session: (!entry.session.is_empty()).then(|| entry.session.clone()),
        };
        let by = held
            .request
            .answered
            .as_ref()
            .and_then(|answered| answered.by.clone());
        self.feed.publish(
            pb::Change::Created,
            pb::event::Payload::Grant(grant),
            by,
            scope,
        );
    }

    /// The user behind `caller`, who has shown they are there.
    fn present_user(&self, caller: &Caller) -> Result<PresentUser, Status> {
        self.guard.present_user(caller)
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
        let approval = with_policy(&*self.guard, |policy, _| {
            policy.approve(user, &RequestId::new(id), access, answer, at)
        })
        .map_err(|error| self.guard.answer_failed(&error))?;
        Ok(approval
            .grant
            .map(|grant| (self.guard.grant_pb(&grant, false), kept::granted(&grant))))
    }

    /// Runs `attempt` while holding for the host up to `bound` when it returns
    /// a `HostUnreachable` status.
    async fn held<T, F, Fut>(&self, bound: Duration, mut attempt: F) -> Result<T, Status>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, Status>>,
    {
        let deadline = tokio::time::Instant::now() + bound;
        let link = self.guard.host_link();
        loop {
            match attempt().await {
                Ok(value) => return Ok(value),
                Err(error) => {
                    let waits = matches!(
                        status::status_error_of(&error).and_then(|e| e.kind),
                        Some(pb::error::Kind::HostUnreachable(_))
                    );
                    if !waits || link.reachable() || tokio::time::Instant::now() >= deadline {
                        return Err(error);
                    }
                    if tokio::time::timeout_at(deadline, link.until_reachable())
                        .await
                        .is_err()
                    {
                        return Err(error);
                    }
                }
            }
        }
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
            return permission::decide(current, id, answer, entry.only_choices);
        }
        // UP-1: a Migrate item takes Migrate and nothing else.
        if let Some(migrator) = self.migrator.get()
            && let Some(refused) = migrator.refuse_answer(current, &answer)
        {
            return Err(refused);
        }
        let decided = |state, answer, grant| Decided {
            state,
            answer,
            grant,
        };
        let preview = current
            .preview
            .as_ref()
            .and_then(|preview| preview.preview.as_ref());
        let is_todo_action = matches!(preview, Some(pb::request_preview::Preview::TodoAction(_)));
        let is_todo_review = matches!(preview, Some(pb::request_preview::Preview::TodoReview(_)));
        // BROWSER: Allow once, Allow 30 min (reads only) or Deny; never Always.
        if let Some(read) = browser::is_read(current) {
            return browser::decide(id, read, answer).map(|(state, answer)| decided(state, answer, None));
        }
        match (&entry.access, answer) {
            (_, answer @ A::Deny(_)) => Ok(decided(pb::RequestState::Denied, answer, None)),
            (_, A::AllowFor30Min(_)) => Err(status::invalid_field(
                "answer",
                Problem::NotAllowedHere,
                format!("request {id} is not a browser read request: Allow 30 min doesn't answer it"),
                "allow it once, or deny it",
            )),
            (_, answer @ A::TodoAction(_)) if is_todo_action => {
                Ok(decided(pb::RequestState::Answered, answer, None))
            }
            (_, A::TodoAction(_)) => Err(status::invalid_field(
                "answer",
                Problem::NotAllowedHere,
                format!("request {id} is not a to-do action request"),
                "answer this card with the answer kind it expects, or deny it",
            )),
            (_, answer @ A::TodoReview(_)) if is_todo_review => {
                Ok(decided(pb::RequestState::Answered, answer, None))
            }
            (_, A::TodoReview(_)) => Err(status::invalid_field(
                "answer",
                Problem::NotAllowedHere,
                format!("request {id} is not a to-do review request"),
                "answer this card with the answer kind it expects, or deny it",
            )),
            (_, _) if is_todo_action => Err(status::invalid_field(
                "answer",
                Problem::NotAllowedHere,
                format!("request {id} is a to-do action request: it takes Done or Can't be done"),
                "answer with a to-do action response (Done or Can't be done), or deny it",
            )),
            (_, _) if is_todo_review => Err(status::invalid_field(
                "answer",
                Problem::NotAllowedHere,
                format!("request {id} is a to-do review request: it takes Approved or Changes asked"),
                "answer with a to-do review response (Approved or Changes asked), or deny it",
            )),
            (None, A::Reply(reply))
                if reply.text.trim().is_empty() && reply.selected_choices.is_empty() =>
            {
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
                        return Err(self.guard.answer_failed(&AnswerError::TargetNotOffered));
                    }
                    _ => {
                        return Err(self.guard.answer_failed(
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

impl Inbox {
    /// Answers request `id` as `caller`, whose presence `user` shows. Only
    /// the first answer counts (IN-15). A mirror of a request its machine
    /// holds isn't answered here: the answer goes to that machine, and the
    /// result is what the machine's journal says came of it (D226).
    async fn answer_as(
        &self,
        caller: &Caller,
        user: &PresentUser,
        id: String,
        answer: pb::request_answer::Answer,
        expected_revision: Option<pb::Revision>,
    ) -> Result<pb::InboxRequest, Status> {
        // Held while answering and saving, so an answer, a second answer
        // and a withdrawal can't interleave.
        let state = self.state.lock().await;
        let entry = state
            .requests
            .iter()
            .find(|entry| entry.id == id && sees(caller, entry))
            .ok_or_else(|| no_request(&id))?;
        // A permission card whose hook let go has no one to hear the answer.
        let _ = self.withdraw_unheld(entry).await;
        let current = entry.request.borrow().request.clone();
        still_open(&current)?;
        if let Some(expected) = expected_revision {
            let actual = current.revision.clone().unwrap_or_default();
            if expected.value != actual.value {
                return Err(revision_conflict(&id, expected, actual));
            }
        }
        // D348: a request the host raised for itself, about no session, takes
        // Approve (run its call) or Deny and nothing else.
        if host_asks::is_host_ask(entry) {
            return self.answer_host_ask(entry, caller, user, answer);
        }
        // UP-1: Migrate on a Migrate item is carried out, not decided here:
        // the item stays pending, shown as Migrating, until the machine is
        // at the new release. Held no longer than it takes to mark it.
        if self.is_migrate(&current) && matches!(answer, pb::request_answer::Answer::Approve(_)) {
            drop(state);
            return self.migrate_approved(&id).await;
        }
        if current.journal.is_some() {
            let relayed = self.relay(entry, &current, caller, user, answer)?;
            // Not held while the machine answers: its journal entry is
            // applied under this lock.
            drop(state);
            return relayed.verdict().await;
        }
        let at = SystemTime::now();
        let approving = matches!(
            answer,
            pb::request_answer::Answer::Approve(_)
                | pb::request_answer::Answer::Always(_)
                | pb::request_answer::Answer::AllowFor30Min(_)
        );
        if let Some(refused) = self
            .deny_if_never(entry, caller, user, approving, at)
            .await?
        {
            self.tell_answer(entry);
            return Err(refused);
        }
        let decided = self.decide(user, entry, &current, answer, at)?;
        let allow_30m = matches!(
            decided.answer,
            pb::request_answer::Answer::AllowFor30Min(_)
        );
        let by = answered_by(caller, user);
        let mut next = entry.request.borrow().clone();
        settle(&mut next, decided, by, at);
        self.commit(entry, next, Some(user), "save the answer")
            .await?;
        self.record_browser_answer(entry, allow_30m, at);
        self.start(entry);
        Ok(self.tell_answer(entry))
    }

    fn is_migrate(&self, request: &pb::InboxRequest) -> bool {
        self.migrator
            .get()
            .map_or(request.kind == MIGRATE_KIND, |migrator| migrator.is_migrate(request))
    }

    /// Sends `answer` to the machine that holds the mirror `current`, if
    /// its link is up and the answer fits the card (D226). Nothing is sent
    /// otherwise.
    fn relay(
        &self,
        entry: &Entry,
        current: &pb::InboxRequest,
        caller: &Caller,
        user: &PresentUser,
        answer: pb::request_answer::Answer,
    ) -> Result<Relayed, Status> {
        fits_card(&*self.guard, current, &answer)?;
        let machine = current
            .journal
            .as_ref()
            .and_then(|position| position.machine.as_ref())
            .map(|machine| machine.value.clone())
            .unwrap_or_default();
        let Some(link) = self.relays.link_of(&machine) else {
            return Err(machine_offline(current, false));
        };
        let sent = pb::RequestAnswer {
            answer: Some(answer),
        };
        // Noted first, so the journal entry that comes back finds it. The
        // note goes when the wait on the answer ends, whichever way.
        let record = self.relays.sent(&machine, &entry.id, sent.clone());
        let message = pb::HostToMachine {
            message: Some(pb::host_to_machine::Message::Answer(pb::InboxAnswerRelay {
                seq: 0,
                request: current.id.clone(),
                answer: Some(sent.clone()),
                by: Some(answered_by(caller, user)),
                at: Some(timestamp(SystemTime::now())),
            })),
        };
        if link.try_send(message).is_err() {
            return Err(machine_offline(current, false));
        }
        Ok(Relayed {
            link,
            seen: entry.request.subscribe(),
            sent,
            record,
            wait: self.relays.wait(),
            request: current.clone(),
        })
    }

    /// The host's user, as this machine counts them when the host passes an
    /// answer down: at the Mac, so on host, whatever the card's `answer_at`
    /// (D213 judges the host's relayed calls as reachable).
    fn relayed_user(&self) -> Option<PresentUser> {
        let here = humata2_access::Context::on_host(humata2_access::MachineId::new(&self.machine));
        Principal::App.present_user(&here)
    }

    /// Answers a request as the host's user does, when the host passes the
    /// answer down the link (D226). The first answer wins, here, under this
    /// Inbox's lock: one for a request already settled does nothing, and
    /// the journal entry the first answer made carries the outcome to the
    /// host. What this machine refuses comes back as the refusal to send up
    /// the link, so the host's user hears at once (not after a wait).
    pub async fn answer_relayed(&self, relayed: &pb::InboxAnswerRelay) -> Option<pb::InboxAnswerRefused> {
        let id = relayed
            .request
            .as_ref()
            .map(|id| id.value.clone())
            .unwrap_or_default();
        let answer = relayed.answer.as_ref().and_then(|answer| answer.answer.clone());
        let (false, Some(answer)) = (id.is_empty(), answer) else {
            humata2_rpc_kit::log::line(
                "humatad2: warning: the host passed down an answer with no request or no answer in it, so it was left out; update Humata on the Mac and on this machine so both ends speak the same version",
            );
            return None;
        };
        let refusal = |status: Status| pb::InboxAnswerRefused {
            request: Some(pb::RequestId { value: id.clone() }),
            error: status::status_error_of(&status),
            code: status.code() as i32,
        };
        let Some(user) = self.relayed_user() else {
            return Some(refusal(status::failure(
                Code::PermissionDenied,
                pb::Error {
                    message: format!(
                        "{} didn't accept your answer to request {id}: it can't count you as present there",
                        self.machine
                    ),
                    fix: format!(
                        "answer it on {} with `humata login` and the approval passphrase, or update Humata on both so they agree",
                        self.machine
                    ),
                    retry: None,
                    kind: None,
                },
            )));
        };
        let caller = Caller::User(UserVia::Host);
        match self.answer_as(&caller, &user, id.clone(), answer, None).await {
            Ok(_) => None,
            Err(refused) => {
                humata2_rpc_kit::log::line(&format!(
                    "humatad2: the host's answer to request {id} wasn't applied: {}",
                    refused.message()
                ));
                let refused = match status::status_error_of(&refused).and_then(|error| error.kind) {
                    Some(pb::error::Kind::NotFound(_)) => status::not_found(
                        pb::EntityKind::Request,
                        &id,
                        format!("request {id} is no longer held on {}", self.machine),
                        "nothing to answer: the request ended there, and the card shows how once the machine's journal reaches this Mac",
                    ),
                    _ => refused,
                };
                Some(refusal(refused))
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
        let caller = request_caller(&request)?;
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
            if request.agent_inbox && !agent_inbox::in_agent_inbox(&current) {
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
            allowed_without_asking_today: self.guard.allowed_without_asking_today(),
        }))
    }

    async fn get_request(
        &self,
        request: Request<pb::GetRequestRequest>,
    ) -> Result<Response<pb::InboxRequest>, Status> {
        let caller = request_caller(&request)?;
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
        let caller = request_caller(&request)?;
        // An agent is refused here, its own requests included (IN-23).
        self.guard.require(&caller, &answer())?;
        let user = self.present_user(&caller)?;
        let request = request.into_inner();
        let id = id_of(request.id.map(|id| id.value)).ok_or_else(|| status::missing("id"))?;
        let answer = request
            .answer
            .and_then(|answer| answer.answer)
            .ok_or_else(|| status::missing("answer"))?;
        let settled = self
            .answer_as(&caller, &user, id, answer, request.expected_revision)
            .await?;
        Ok(Response::new(settled))
    }

    /// IN-6: answers the listed requests that are still pending and in
    /// scope; the rest are skipped. Approving answers access requests once
    /// and skips questions and ask-aheads, since it never adds grants.
    async fn answer_all(
        &self,
        request: Request<pb::AnswerAllRequest>,
    ) -> Result<Response<pb::AnswerAllResponse>, Status> {
        let caller = request_caller(&request)?;
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
            // UP-1, D290: an Answer all never migrates anything.
            let entry = entry.filter(|entry| {
                !self.is_migrate(&entry.request.borrow().request)
                    && !host_asks::is_host_ask(entry)
            });
            // A permission card is one tool call, approved one at a time.
            let Some(entry) = entry.filter(|entry| !entry.permission) else {
                skipped.push(id);
                continue;
            };
            let current = entry.request.borrow().request.clone();
            // A request held on its machine is answered there (D226).
            if current.state != pb::RequestState::Pending as i32
                || current.journal.is_some()
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
                match (&entry.access, browser::is_read(&current)) {
                    (Some(access), _) if !access.ahead => self
                        .approve(&user, &entry.id, access, &Answer::Once, at)
                        .ok()
                        .map(|grant| Decided {
                            state: pb::RequestState::Approved,
                            answer: pb::request_answer::Answer::Approve(pb::Approve {}),
                            grant,
                        }),
                    // BROWSER: Approve all allows browser reads once and skips browser writes.
                    (_, Some(true)) => Some(Decided {
                        state: pb::RequestState::Approved,
                        answer: pb::request_answer::Answer::Approve(pb::Approve {}),
                        grant: None,
                    }),
                    // A browser write requires per-action approval; a question
                    // needs a reply; approving an ask-ahead would make a grant.
                    _ => None,
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
        let caller = request_caller(&request)?;
        if matches!(caller, Caller::Script(_)) {
            return Err(self.guard.refused_status(&Refusal::ScriptTokenHasNoSession));
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
        self.guard.check_inbox_ask(&caller, &inbox_ask(agent))?;
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
        if request.multi_select && request.choices.is_empty() {
            return Err(status::invalid_field(
                "multi_select",
                Problem::NotAllowedHere,
                "multi_select requires at least one choice",
                "offer 1 to 4 choices, or set multi_select to false for a free-text question",
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
        let mut made = question_with(
            &id,
            agent,
            pb::QuestionPreview {
                question: request.question,
                choices: request.choices,
                todo_item: request.todo_item,
                step_id: request.step_id,
                multi_select: request.multi_select,
                details: request.details,
            },
        );
        made.working_on = self.working_on(agent);
        let held = Held::new(made);
        let reply = settled(&held);
        let entry = Entry::new(id, session, None, None, key, held.clone());
        self.commit(&entry, held, None, "save the request").await?;
        self.tell(
            pb::Change::Created,
            &entry.session,
            &entry.request.borrow().request,
        );
        hold_through_gap(&self.asking, &entry.session, entry.request.subscribe());
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
        let caller = request_caller(&request)?;
        if matches!(caller, Caller::Script(_)) {
            return Err(self.guard.refused_status(&Refusal::ScriptTokenHasNoSession));
        }
        let Caller::Agent(agent) = &caller else {
            return Err(status::invalid_field(
                "capability",
                Problem::NotAllowedHere,
                "only a session's agent asks ahead for access; this connection is the user, who needs no grant",
                "ask from a Humata session tab, where HUMATA_SESSION_TOKEN is set",
            ));
        };
        // A call that needs the Mac waits for it here, with no lock held:
        // each run takes the Inbox lock for itself and gives it back when
        // it fails, and the call is judged afresh when the Mac is back.
        let bound = self.guard.hold_bound(&caller, request.metadata());
        let message = request.into_inner();
        self.held(bound, || async {
            let request = message.clone();
            // Asking ahead is asking; this also loads the channel's rules.
            self.guard.check_inbox_ask(&caller, &inbox_ask(agent))?;
            let capability = match request.capability {
                0 => return Err(status::missing("capability")),
                value => self.guard.capability_of(value).ok_or_else(|| {
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
                let guard = Arc::clone(&self.guard);
                let store = Arc::clone(&self.store);
                let machine = self.machine.clone();
                let asking = agent.clone();
                tokio::task::spawn_blocking(move || {
                    guard.ask_ahead_call(&store, &machine, &asking, capability, target)
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
            let (verdict, grant, decision) = with_policy(&*self.guard, |policy, context| {
                let verdict = policy.check_ask_ahead(&asker, &call, context);
                let grant = match &verdict {
                    Ok(Verdict::Allowed {
                        source: Source::Grant(id),
                        ..
                    }) => policy
                        .grant(*id)
                        .map(|grant| self.guard.grant_pb(grant, false)),
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
                Ok(Verdict::WaitForHost) => return Err(self.guard.waits_for_host()),
                Ok(Verdict::Refused(refusal)) => return Err(self.guard.refused_status(&refusal)),
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
            let made = access_request(&*self.guard, &id, agent, &asked, request.reason, decision);
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
            hold_through_gap(&self.asking, &entry.session, entry.request.subscribe());
            state.add(entry);
            Ok(Response::new(reply))
        })
        .await
    }

    type WaitForAnswerStream = ReceiverStream<Result<pb::WaitResponse, Status>>;

    /// Sends a keepalive at once and then about every 15 seconds, and ends
    /// with the request's state once it's answered or the wait is over.
    async fn wait_for_answer(
        &self,
        request: Request<pb::WaitForAnswerRequest>,
    ) -> Result<Response<Self::WaitForAnswerStream>, Status> {
        let caller = request_caller(&request)?;
        let request = request.into_inner();
        let id = id_of(request.id.as_ref().map(|id| id.value.clone())).ok_or_else(|| status::missing("id"))?;
        let (held, mut answers) = match self.find(&caller, &id).await {
            Ok(held) => {
                let answers = held.subscribe();
                (held, answers)
            }
            // A request this machine doesn't hold may be the host's (D268).
            Err(missing) => {
                let stream = self.wait_on_host(&caller, request, &id, missing)?;
                return Ok(Response::new(stream));
            }
        };
        let wait = Duration::from_secs(u64::from(request.wait_seconds));
        let (sender, receiver) = mpsc::channel(4);
        // D158.7: an agent waiting on its own request needs you meanwhile,
        // and through the gap before its next wait.
        let asker = match &caller {
            Caller::Agent(agent) if answers.borrow().pending() => Some(agent.session.id.to_string()),
            _ => None,
        };
        let asking_port = Arc::clone(&self.asking);
        let asking = asker.as_deref().map(|session| asking_port.hold(session));
        tokio::spawn(async move {
            let _asking = asking;
            let _gap = asker.map(|session| Gap(asking_port, session, answers.clone()));
            let deadline = tokio::time::sleep(wait);
            tokio::pin!(deadline);
            let mut keepalive = tokio::time::interval(KEEPALIVE);
            loop {
                let current = answers.borrow_and_update().clone();
                if current.asking_why() {
                    let _ = sender.send(Ok(settled(&current))).await;
                    return;
                }
                if !current.waiting() {
                    if sender.send(Ok(settled(&current))).await.is_ok() {
                        deferred::delivered(&held);
                    }
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

    /// INBOXHOOKS: the user asks the agent why. The card stays pending; the
    /// agent's wait ends with `asked_why` so it can explain.
    async fn ask_why(
        &self,
        request: Request<pb::AskWhyRequest>,
    ) -> Result<Response<pb::InboxRequest>, Status> {
        let caller = request_caller(&request)?;
        if !matches!(caller, Caller::User(_)) {
            return Err(self.guard.refused_status(&Refusal::UserOnly {
                capability: Some(Capability::InboxAnswer),
            }));
        }
        self.guard.require(&caller, &answer())?;
        let request = request.into_inner();
        let id = id_of(request.id.as_ref().map(|id| id.value.clone()))
            .ok_or_else(|| status::missing("id"))?;
        match self
            .set_why(
                &id,
                |request, at| {
                    let asked = request.asked_why.get_or_insert_with(Default::default);
                    // Asking again before the agent answered changes nothing.
                    if asked.asked_at.is_some() && asked.reply.is_empty() {
                        return false;
                    }
                    *asked = pb::AskedWhy {
                        asked_at: Some(timestamp(at)),
                        reply: String::new(),
                        replied_at: None,
                    };
                    true
                },
                &caller,
            )
            .await
        {
            Ok(updated) => Ok(Response::new(updated)),
            Err(missing) if missing.code() == Code::NotFound => self
                .ask_why_on_host(&caller, request, &id, missing)
                .await
                .map(Response::new),
            Err(failed) => Err(failed),
        }
    }

    /// INBOXHOOKS: the agent's answer to Ask why, on a request its own
    /// session made. The card stays pending.
    async fn explain_ask(
        &self,
        request: Request<pb::ExplainAskRequest>,
    ) -> Result<Response<pb::InboxRequest>, Status> {
        let caller = request_caller(&request)?;
        let Caller::Agent(_) = &caller else {
            return Err(status::invalid_field(
                "id",
                Problem::NotAllowedHere,
                "only the agent that made a request explains it",
                "ask the agent why from the card's Ask why button instead",
            ));
        };
        let request = request.into_inner();
        let id = id_of(request.id.as_ref().map(|id| id.value.clone()))
            .ok_or_else(|| status::missing("id"))?;
        let reply = request.reply.trim().to_owned();
        let length = reply.chars().count();
        if length == 0 || length > MAX_EXPLANATION {
            return Err(status::invalid_field(
                "reply",
                if length == 0 { Problem::Missing } else { Problem::TooLong },
                format!("the explanation is {length} characters; it must be 1 to {MAX_EXPLANATION}"),
                "send a shorter explanation that says why the request is needed",
            ));
        }
        match self
            .set_why(
                &id,
                |request, at| {
                    let asked = request.asked_why.get_or_insert_with(Default::default);
                    asked.reply.clone_from(&reply);
                    asked.replied_at = Some(timestamp(at));
                    true
                },
                &caller,
            )
            .await
        {
            Ok(updated) => Ok(Response::new(updated)),
            Err(missing) if missing.code() == Code::NotFound => self
                .explain_on_host(&caller, request, &id, missing)
                .await
                .map(Response::new),
            Err(failed) => Err(failed),
        }
    }

    /// BROWSER: an agent asks to run one action in the user's browser.
    async fn request_browser_approval(
        &self,
        request: Request<pb::RequestBrowserApprovalRequest>,
    ) -> Result<Response<pb::WaitResponse>, Status> {
        self.browser_approval(request).await
    }

    /// The asking agent stopped waiting (TP-15). Withdrawing a request
    /// that's no longer pending changes nothing.
    async fn withdraw_request(
        &self,
        request: Request<pb::WithdrawRequestRequest>,
    ) -> Result<Response<pb::InboxRequest>, Status> {
        let caller = request_caller(&request)?;
        if matches!(caller, Caller::Script(_)) {
            return Err(self.guard.refused_status(&Refusal::ScriptTokenHasNoSession));
        }
        if let Caller::User(_) = caller {
            return Err(status::invalid_field(
                "id",
                Problem::NotAllowedHere,
                "only the agent that made a request withdraws it",
                "answer or deny the request in the Inbox instead",
            ));
        }
        let request = request.into_inner();
        let id = id_of(request.id.as_ref().map(|id| id.value.clone()))
            .ok_or_else(|| status::missing("id"))?;
        // Under the lock, so it can't land in the middle of an answer.
        let state = self.state.lock().await;
        let Some(entry) = state
            .requests
            .iter()
            .find(|entry| entry.id == id && sees(&caller, entry))
        else {
            drop(state);
            // A request this machine doesn't hold may be the host's (D268).
            let missing = no_request(&id);
            return self
                .withdraw_on_host(&caller, request, &id, missing)
                .await
                .map(Response::new);
        };
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

impl Inbox {
    /// Runs `change` on the pending request `id` as `caller` sees it; when
    /// it says it changed the request, bumps the revision, saves and
    /// publishes it. Returns the request as it is.
    async fn set_why(
        &self,
        id: &str,
        change: impl FnOnce(&mut pb::InboxRequest, SystemTime) -> bool,
        caller: &Caller,
    ) -> Result<pb::InboxRequest, Status> {
        let state = self.state.lock().await;
        let entry = state
            .requests
            .iter()
            .find(|entry| entry.id == id && sees(caller, entry))
            .ok_or_else(|| no_request(id))?;
        let mut next = entry.request.borrow().clone();
        if !next.pending()
            || entry.session.is_empty()
            || self.is_migrate(&next.request)
            || host_asks::is_host_ask(entry)
        {
            return Err(status::invalid_field(
                "id",
                Problem::NotAllowedHere,
                format!("request {id} is not an agent's call that still waits for an answer"),
                "ask why only on an agent's pending request",
            ));
        }
        if !change(&mut next.request, SystemTime::now()) {
            return Ok(next.request);
        }
        next.request.revision = Some(next_revision(&next.request));
        let current = next.request.clone();
        self.commit(entry, next, None, "save the question").await?;
        self.tell(pb::Change::Updated, &entry.session, &current);
        Ok(current)
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
    question_with(
        id,
        agent,
        pb::QuestionPreview {
            question,
            choices,
            todo_item: None,
            step_id: String::new(),
            multi_select: false,
            details: String::new(),
        },
    )
}

fn question_with(
    id: &str,
    agent: &AgentCaller,
    preview: pb::QuestionPreview,
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
            preview: Some(pb::request_preview::Preview::Question(preview)),
        }),
        capability: pb::Capability::InboxAsk as i32,
        created_at: Some(now_timestamp()),
        revision: Some(pb::Revision { value: 1 }),
        ..Default::default()
    }
}

/// The card for an ask-ahead (IN-9, IN-10).
fn access_request(
    guard: &dyn InboxGuardPort,
    id: &str,
    agent: &AgentCaller,
    asked: &AccessRequest,
    reason: String,
    decision: Option<Decision>,
) -> pb::InboxRequest {
    let capability = guard.capability_pb(asked.capability) as i32;
    let offered = asked.always_let.as_ref();
    let target = offered.map(|target| guard.target_pb(target));
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
        action_title: asks_title(&agent.title, asked.capability, true),
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
            label: if facts::always_covers_any_channel(asked) {
                format!("Always allow “{}” to {target_label}, in any channel", agent.title)
            } else {
                format!("Always allow “{}” to {target_label}", agent.title)
            },
            target: Some(target),
        }),
        created_at: Some(now_timestamp()),
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
                K::StopSessions => "stop all sessions on",
                K::StartSessions => "start all sessions on",
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

/// A card's title: "<who> asks to <what the capability lets it do>". A
/// capability that is itself asking the user ("ask you to import…") reads
/// "<who> asks you to import…", never "asks to ask you to". `ahead` is an
/// ask for access before the call that needs it.
pub fn asks_title(who: &str, capability: Capability, ahead: bool) -> String {
    let what = lowercase_first(capability.label());
    let asks = if ahead { "asks ahead" } else { "asks" };
    match what.strip_prefix("ask you ") {
        Some(rest) => format!("{who} {asks} you {rest}"),
        None => format!("{who} {asks} to {what}"),
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
        | C::OwnBoardWrite
        | C::TodoSuggest
        | C::TodoClose => pb::RequestKind::Board,
        C::SessionNote
        | C::SessionType
        | C::SessionStartSameMachine
        | C::SessionStartOtherMachine
        | C::SessionMove
        | C::SessionStopOther
        | C::SessionDeleteOther
        | C::SessionsDeleteMany
        | C::DeskMoveToChannel
        | C::GitPush => pb::RequestKind::Session,
        C::MachineProvision
        | C::MachineDestroy
        | C::MachineSessionsStop
        | C::MachineScriptRun
        | C::ProviderImport => pb::RequestKind::Machine,
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
        at: Some(timestamp(at)),
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

/// The machine that holds `request` can't be reached, so the Mac's answer
/// can't be passed on, or was passed on and not confirmed (D226).
fn machine_offline(request: &pb::InboxRequest, sent: bool) -> Status {
    let machine = request
        .journal
        .as_ref()
        .and_then(|position| position.machine.as_ref())
        .map(|machine| machine.value.clone())
        .unwrap_or_default();
    let id = request
        .id
        .as_ref()
        .map(|id| id.value.clone())
        .unwrap_or_default();
    let (message, fix) = if sent {
        (
            format!(
                "your answer to request {id} was sent to {machine}, but {machine} didn't confirm it"
            ),
            format!(
                "nothing is lost: the card shows how it ended as soon as {machine} is linked again; if it should end differently, answer it on {machine} with `humata login` and the approval passphrase"
            ),
        )
    } else {
        (
            format!(
                "request {id} is held on {machine}, which isn't linked to this Mac right now, so nothing was sent"
            ),
            format!(
                "reconnect {machine} and answer again, or answer it on {machine} with `humata login` and the approval passphrase; its answer shows here as soon as {machine} is linked"
            ),
        )
    };
    status::failure(
        Code::Unavailable,
        status::error(
            message,
            fix,
            pb::error::Kind::MachineOffline(pb::MachineOffline {
                machine: request
                    .journal
                    .as_ref()
                    .and_then(|position| position.machine.clone()),
                machine_name: machine,
                since: None,
            }),
        ),
    )
}

/// Fails when `answer` isn't one the card takes: a question takes a reply,
/// an access request an approval, and "Always allow" only its own offer.
/// The same as answering on the machine, so a mistake fails here and not
/// on the far side of the link.
fn fits_card(
    guard: &dyn InboxGuardPort,
    request: &pb::InboxRequest,
    answer: &pb::request_answer::Answer,
) -> Result<(), Status> {
    use pb::request_answer::Answer as A;
    let id = request
        .id
        .as_ref()
        .map(|id| id.value.as_str())
        .unwrap_or_default();
    let question = request.kind == pb::RequestKind::Question as i32;
    let preview = request
        .preview
        .as_ref()
        .and_then(|preview| preview.preview.as_ref());
    let is_todo_action = matches!(preview, Some(pb::request_preview::Preview::TodoAction(_)));
    let is_todo_review = matches!(preview, Some(pb::request_preview::Preview::TodoReview(_)));
    if let Some(read) = browser::is_read(request) {
        return browser::decide(id, read, answer.clone()).map(|_| ());
    }
    match answer {
        A::Deny(_) => Ok(()),
        A::AllowFor30Min(_) => Err(status::invalid_field(
            "answer",
            Problem::NotAllowedHere,
            format!("request {id} is not a browser read request: Allow 30 min doesn't answer it"),
            "allow it once, or deny it",
        )),
        A::TodoAction(_) if is_todo_action => Ok(()),
        A::TodoAction(_) => Err(status::invalid_field(
            "answer",
            Problem::NotAllowedHere,
            format!("request {id} is not a to-do action request"),
            "answer this card with the answer kind it expects, or deny it",
        )),
        A::TodoReview(_) if is_todo_review => Ok(()),
        A::TodoReview(_) => Err(status::invalid_field(
            "answer",
            Problem::NotAllowedHere,
            format!("request {id} is not a to-do review request"),
            "answer this card with the answer kind it expects, or deny it",
        )),
        _ if is_todo_action => Err(status::invalid_field(
            "answer",
            Problem::NotAllowedHere,
            format!("request {id} is a to-do action request: it takes Done or Can't be done"),
            "answer with a to-do action response (Done or Can't be done), or deny it",
        )),
        _ if is_todo_review => Err(status::invalid_field(
            "answer",
            Problem::NotAllowedHere,
            format!("request {id} is a to-do review request: it takes Approved or Changes asked"),
            "answer with a to-do review response (Approved or Changes asked), or deny it",
        )),
        A::Reply(reply) if question => {
            if reply.text.trim().is_empty() && reply.selected_choices.is_empty() {
                Err(status::missing("answer.reply.text"))
            } else {
                Ok(())
            }
        }
        A::Approve(_) | A::Always(_) if question => Err(status::invalid_field(
            "answer",
            Problem::NotAllowedHere,
            format!("request {id} is a question: it takes a reply, not an approval"),
            "reply with text (one of its choices, or your own), or deny it",
        )),
        A::Reply(_) => Err(status::invalid_field(
            "answer",
            Problem::NotAllowedHere,
            format!("request {id} asks for access: a reply doesn't answer it"),
            "allow it, allow it with Always allow, or deny it",
        )),
        A::Approve(_) => Ok(()),
        A::Always(always) => {
            let offered = request
                .always
                .as_ref()
                .and_then(|always| always.target.as_ref());
            match offered {
                Some(offered) if always.target.as_ref() == Some(offered) => Ok(()),
                Some(_) => Err(guard.answer_failed(&AnswerError::TargetNotOffered)),
                None => Err(guard.answer_failed(&AnswerError::NoStandingApproval)),
            }
        }
    }
}

/// The refusal of an answer to `request`, which was withdrawn.
fn withdrawn(request: &pb::InboxRequest) -> Status {
    let id = request.id.clone();
    let value = id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
    status::failure(
        Code::FailedPrecondition,
        status::error(
            format!(
                "request {value} was withdrawn: the agent stopped waiting for it before you answered"
            ),
            "nothing to answer; if the agent still needs it, it will ask again",
            pb::error::Kind::RequestWithdrawn(pb::RequestWithdrawn { request: id }),
        ),
    )
}

/// Fails when `request` can no longer be answered (IN-15).
fn still_open(request: &pb::InboxRequest) -> Result<(), Status> {
    let id = request.id.clone();
    let value = id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
    match pb::RequestState::try_from(request.state) {
        Ok(pb::RequestState::Pending) => Ok(()),
        Ok(pb::RequestState::Withdrawn) => Err(withdrawn(request)),
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
            let now = now_timestamp().seconds;
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
            result_kept_until = Some(timestamp(outcome.kept_until));
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
        asked_why: held.asking_why(),
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

#[cfg(test)]
mod tests {
    use humata2_access::{
        AnswerError, Context, Grant, GrantTarget, Policy, PresentUser, Principal, Refusal,
    };
    use humata2_daemon_ports::{Deferred, UserVia};
    use humata2_grpc::humata::v1::inbox_service_server::InboxService;
    use humata2_store::{SessionRecord, StoreError};
    use humata2_svc_away::HostLink;
    use tokio_stream::StreamExt;

    use super::*;

    struct TestGuard {
        host_link: Arc<HostLink>,
    }

    impl TestGuard {
        fn new() -> Self {
            Self {
                host_link: Arc::new(HostLink::new(true)),
            }
        }
    }

    impl InboxGuardPort for TestGuard {
        fn host_link(&self) -> Arc<HostLink> {
            self.host_link.clone()
        }
        fn with_policy_mut(&self, f: &mut dyn FnMut(&mut Policy, &Context)) {
            let mut policy = Policy::default();
            let context = Context::on_host(humata2_access::MachineId::new("local"));
            f(&mut policy, &context);
        }
        fn principal(&self, _caller: &Caller) -> Principal {
            Principal::App
        }
        fn present_user(&self, _caller: &Caller) -> Result<PresentUser, Status> {
            let context = Context::on_host(humata2_access::MachineId::new("local"));
            Principal::App
                .present_user(&context)
                .ok_or_else(|| Status::permission_denied("not present"))
        }
        fn capability_pb(&self, _capability: Capability) -> pb::Capability {
            pb::Capability::Unspecified
        }
        fn capability_of(&self, _capability: i32) -> Option<Capability> {
            None
        }
        fn grant_number(&self, _grant: &pb::Grant) -> Option<u64> {
            None
        }
        fn grant_pb(&self, _grant: &Grant, _suspended: bool) -> pb::Grant {
            pb::Grant::default()
        }
        fn target_pb(&self, _target: &GrantTarget) -> pb::GrantTarget {
            pb::GrantTarget::default()
        }
        fn ask_ahead_call(
            &self,
            _store: &Store,
            _machine: &str,
            _agent: &AgentCaller,
            _capability: Capability,
            _target: pb::GrantTarget,
        ) -> Result<Call, Status> {
            Err(Status::unimplemented("test"))
        }
        fn answer_failed(&self, error: &AnswerError) -> Status {
            Status::invalid_argument(error.to_string())
        }
        fn refused_status(&self, refusal: &Refusal) -> Status {
            Status::permission_denied(refusal.to_string())
        }
        fn require(&self, _caller: &Caller, _call: &Call) -> Result<(), Status> {
            Ok(())
        }
        fn check_inbox_ask(&self, _caller: &Caller, _call: &Call) -> Result<(), Status> {
            Ok(())
        }
        fn hold_bound(
            &self,
            _caller: &Caller,
            _metadata: &tonic::metadata::MetadataMap,
        ) -> std::time::Duration {
            std::time::Duration::from_secs(30)
        }
        fn waits_for_host(&self) -> Status {
            Status::unavailable("host unreachable")
        }
        fn agent_caller(
            &self,
            _record: &SessionRecord,
            _machine: &str,
            _store: &Store,
        ) -> Result<AgentCaller, StoreError> {
            Err(StoreError::Write {
                path: std::path::PathBuf::new(),
                source: std::io::Error::other("unused"),
            })
        }
    }

    struct NoopFeed;

    impl InboxFeedPort for NoopFeed {
        fn publish(
            &self,
            _change: pb::Change,
            _payload: pb::event::Payload,
            _actor: Option<pb::Actor>,
            _scope: EventScope,
        ) {
        }
    }

    pub(super) fn user_req<T>(msg: T) -> Request<T> {
        let mut req = Request::new(msg);
        req.extensions_mut().insert(Caller::User(UserVia::App));
        req
    }

    pub(super) fn test_inbox(dir: &tempfile::TempDir) -> Arc<Inbox> {
        let store = Arc::new(Store::open(dir.path().join("state")).expect("open store"));
        Arc::new(Inbox::new(
            Arc::new(TestGuard::new()),
            store,
            "local".to_owned(),
            Arc::new(NoopFeed),
        ))
    }

    pub(super) fn agent_req<T>(msg: T, session: &str) -> Request<T> {
        let mut req = Request::new(msg);
        req.extensions_mut().insert(Caller::Agent(AgentCaller {
            session: humata2_access::SessionRef {
                id: humata2_access::SessionId::new(session),
                channel: humata2_access::ChannelId::new("ch_1"),
                machine: humata2_access::MachineId::new("local"),
            },
            agent: "claude".to_owned(),
            title: "Claude".to_owned(),
            method: "mcp".to_owned(),
            context_only_parent: None,
            desk: None,
            review_push: None,
            channel_desk: None,
        }));
        req
    }

    /// A pending question from `session`, held in `inbox`.
    async fn hold_question(inbox: &Inbox, session: &str, state: pb::RequestState) -> String {
        let request = pb::InboxRequest {
            id: Some(pb::RequestId { value: "rq_1".to_owned() }),
            kind: pb::RequestKind::Question as i32,
            state: state as i32,
            revision: Some(pb::Revision { value: 1 }),
            ..Default::default()
        };
        inbox
            .store
            .put_session(&SessionRecord {
                id: session.to_owned(),
                channel: "ch_1".to_owned(),
                machine: String::new(),
                title: "t".to_owned(),
                agent: "claude".to_owned(),
                folder: std::path::PathBuf::from("/tmp/x"),
                created_unix_ms: 0,
            })
            .unwrap();
        let entry = Entry::new("rq_1".to_owned(), session.to_owned(), None, None, None, Held::new(request));
        inbox.state.lock().await.add(entry);
        "rq_1".to_owned()
    }

    pub(super) fn id(value: &str) -> Option<pb::RequestId> {
        Some(pb::RequestId { value: value.to_owned() })
    }

    #[tokio::test]
    async fn asking_why_marks_a_pending_agent_card_and_ends_the_agents_wait() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = test_inbox(&dir);
        let rq = hold_question(&inbox, "se_1", pb::RequestState::Pending).await;
        let asked = inbox
            .ask_why(user_req(pb::AskWhyRequest { id: id(&rq), ..Default::default() }))
            .await
            .unwrap()
            .into_inner();
        assert!(asked.asked_why.as_ref().is_some_and(|why| why.asked_at.is_some()));
        assert_eq!(asked.state, pb::RequestState::Pending as i32);
        assert_eq!(asked.revision.as_ref().map(|r| r.value), Some(2));
        // A wait that starts now is told at once, and again until a reply.
        for _ in 0..2 {
            let mut waits = inbox
                .wait_for_answer(agent_req(
                    pb::WaitForAnswerRequest { id: id(&rq), wait_seconds: 30, ..Default::default() },
                    "se_1",
                ))
                .await
                .unwrap()
                .into_inner();
            let told = waits.next().await.unwrap().unwrap();
            assert!(told.asked_why && !told.keepalive);
            assert_eq!(told.state, pb::RequestState::Pending as i32);
        }
    }

    #[tokio::test]
    async fn asking_why_of_an_answered_card_fails() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = test_inbox(&dir);
        let rq = hold_question(&inbox, "se_1", pb::RequestState::Approved).await;
        let refused = inbox
            .ask_why(user_req(pb::AskWhyRequest { id: id(&rq), ..Default::default() }))
            .await;
        assert!(refused.is_err());
    }

    #[tokio::test]
    async fn only_the_asking_session_explains_and_the_reply_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = test_inbox(&dir);
        let rq = hold_question(&inbox, "se_1", pb::RequestState::Pending).await;
        inbox
            .ask_why(user_req(pb::AskWhyRequest { id: id(&rq), ..Default::default() }))
            .await
            .unwrap();
        let other = inbox
            .explain_ask(agent_req(
                pb::ExplainAskRequest { id: id(&rq), reply: "because".to_owned() },
                "se_2",
            ))
            .await;
        assert!(other.is_err());
        let too_long = inbox
            .explain_ask(agent_req(
                pb::ExplainAskRequest { id: id(&rq), reply: "x".repeat(1001) },
                "se_1",
            ))
            .await;
        assert!(too_long.is_err());
        let explained = inbox
            .explain_ask(agent_req(
                pb::ExplainAskRequest { id: id(&rq), reply: "The build needs the data there.".to_owned() },
                "se_1",
            ))
            .await
            .unwrap()
            .into_inner();
        let why = explained.asked_why.unwrap();
        assert!(!why.reply.is_empty() && why.replied_at.is_some());
        assert_eq!(explained.state, pb::RequestState::Pending as i32);
        // The reply ends the asking: the agent's wait goes on waiting.
        let held = inbox.state.lock().await.requests[0].request.borrow().clone();
        assert!(!held.asking_why());
    }

    #[test]
    fn asks_title_formats_normal_and_ahead_requests() {
        let normal = asks_title("claude", Capability::FilesReadOtherSession, false);
        let ahead = asks_title("claude", Capability::FilesReadOtherSession, true);
        assert!(normal.starts_with("claude asks "));
        assert!(ahead.starts_with("claude asks ahead "));
        assert!(!normal.contains("asks to ask you"));
    }

    #[tokio::test]
    async fn post_host_ask_lists_and_withdraw_host_ask_removes_card_over_temp_store() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = test_inbox(&dir);

        let card = pb::InboxRequest {
            action_title: "Script setup failed on box-1".to_owned(),
            reason: "exit 1".to_owned(),
            operation: "machine.script".to_owned(),
            ..Default::default()
        };
        let run: Run = Box::new(|_| {
            Box::pin(async {
                Ok(Deferred {
                    result: Vec::new(),
                    result_type: String::new(),
                })
            })
        });
        inbox
            .post_host_ask("machine-script/box-1/setup", card, run)
            .await
            .unwrap();

        let listed = inbox
            .list_requests(user_req(pb::ListRequestsRequest::default()))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(listed.requests.len(), 1);
        assert_eq!(
            listed.requests[0].action_title,
            "Script setup failed on box-1"
        );
        assert_eq!(listed.requests[0].state, pb::RequestState::Pending as i32);

        inbox
            .withdraw_host_ask("machine-script/box-1/setup", pb::WithdrawReason::Resolved)
            .await;

        let after = inbox
            .list_requests(user_req(pb::ListRequestsRequest::default()))
            .await
            .unwrap()
            .into_inner();
        assert!(after.requests.is_empty());
    }

    #[tokio::test]
    async fn withdraw_pending_from_handles_empty_machine_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = test_inbox(&dir);
        inbox
            .withdraw_pending_from("box-gone", pb::WithdrawReason::MachineRemoved)
            .await
            .unwrap();
        assert!(inbox.pending_ids_of(&["se_1".to_owned()]).await.is_empty());
    }

    #[tokio::test]
    async fn ask_why_refuses_agent_and_accepts_forwarded_host_user() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = test_inbox(&dir);
        let rq = hold_question(&inbox, "se_1", pb::RequestState::Pending).await;

        let by_agent = inbox
            .ask_why(agent_req(
                pb::AskWhyRequest { id: id(&rq), ..Default::default() },
                "se_1",
            ))
            .await;
        assert_eq!(by_agent.unwrap_err().code(), Code::PermissionDenied);

        let mut peer_req = Request::new(pb::AskWhyRequest { id: id(&rq), ..Default::default() });
        peer_req.extensions_mut().insert(Caller::Peer);
        assert_eq!(inbox.ask_why(peer_req).await.unwrap_err().code(), Code::PermissionDenied);

        let asked = inbox
            .ask_why_as(
                Caller::User(UserVia::Host),
                pb::AskWhyRequest { id: id(&rq), ..Default::default() },
            )
            .await
            .unwrap();
        assert!(asked.asked_why.as_ref().is_some_and(|why| why.asked_at.is_some()));
        assert_eq!(asked.state, pb::RequestState::Pending as i32);
    }

    #[tokio::test]
    async fn explain_ask_routes_up_to_host_when_not_held_locally() {
        let host_dir = tempfile::tempdir().unwrap();
        let host = test_inbox(&host_dir);
        let rq = hold_question(&host, "se_1", pb::RequestState::Pending).await;
        host.ask_why(user_req(pb::AskWhyRequest { id: id(&rq), ..Default::default() }))
            .await
            .unwrap();

        let machine_dir = tempfile::tempdir().unwrap();
        let machine = test_inbox(&machine_dir);
        let forwarder = Arc::new(Forwarder::new(false));
        let (to_host, mut from_machine) = mpsc::channel(4);
        forwarder.attach(to_host);
        machine.forwarding(Arc::clone(&forwarder));

        let waiting = {
            let machine = Arc::clone(&machine);
            let rq = rq.clone();
            tokio::spawn(async move {
                machine
                    .explain_ask(agent_req(
                        pb::ExplainAskRequest {
                            id: id(&rq),
                            reply: "need fixture rows from the other clone".to_owned(),
                        },
                        "se_1",
                    ))
                    .await
            })
        };

        let sent = tokio::time::timeout(Duration::from_secs(5), from_machine.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let Some(pb::machine_to_host::Message::Call(call)) = sent.message else {
            panic!("expected forwarded ExplainAsk call");
        };
        assert_eq!(call.method, "/humata.v1.InboxService/ExplainAsk");
        let Some(pb::forwarded_call::Principal::Agent(agent)) = call.principal else {
            panic!("expected agent principal");
        };
        assert_eq!(agent.session.map(|s| s.value).as_deref(), Some("se_1"));

        let decoded = pb::ExplainAskRequest::decode(call.request.as_slice()).unwrap();
        let caller = agent_req((), "se_1").extensions().get::<Caller>().unwrap().clone();
        let reply = host.explain_as(caller, decoded).await.unwrap();
        forwarder.answered(pb::ForwardedResult {
            call_id: call.call_id,
            outcome: Some(pb::forwarded_result::Outcome::Response(reply.encode_to_vec())),
        });

        let updated = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .into_inner();
        assert_eq!(
            updated.asked_why.as_ref().map(|why| why.reply.as_str()),
            Some("need fixture rows from the other clone")
        );
    }

    #[tokio::test]
    async fn ask_why_routes_up_to_host_when_not_held_locally() {
        let host_dir = tempfile::tempdir().unwrap();
        let host = test_inbox(&host_dir);
        let rq = hold_question(&host, "se_1", pb::RequestState::Pending).await;

        let machine_dir = tempfile::tempdir().unwrap();
        let machine = test_inbox(&machine_dir);
        let forwarder = Arc::new(Forwarder::new(false));
        let (to_host, mut from_machine) = mpsc::channel(4);
        forwarder.attach(to_host);
        machine.forwarding(Arc::clone(&forwarder));

        let waiting = {
            let machine = Arc::clone(&machine);
            let rq = rq.clone();
            tokio::spawn(async move {
                machine
                    .ask_why(user_req(pb::AskWhyRequest {
                        id: id(&rq),
                        ..Default::default()
                    }))
                    .await
            })
        };

        let sent = tokio::time::timeout(Duration::from_secs(5), from_machine.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let Some(pb::machine_to_host::Message::Call(call)) = sent.message else {
            panic!("expected forwarded AskWhy call");
        };
        assert_eq!(call.method, "/humata.v1.InboxService/AskWhy");
        assert!(matches!(
            call.principal,
            Some(pb::forwarded_call::Principal::User(pb::MachineUser {
                outside_sessions: true,
                ..
            }))
        ));

        let decoded = pb::AskWhyRequest::decode(call.request.as_slice()).unwrap();
        let reply = host
            .ask_why_as(Caller::User(UserVia::Host), decoded)
            .await
            .unwrap();
        forwarder.answered(pb::ForwardedResult {
            call_id: call.call_id,
            outcome: Some(pb::forwarded_result::Outcome::Response(reply.encode_to_vec())),
        });

        let updated = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .into_inner();
        assert!(updated.asked_why.as_ref().is_some_and(|why| why.asked_at.is_some()));
    }

    #[tokio::test]
    async fn explain_ask_returns_host_unreachable_when_host_is_away_and_holds_request() {
        let machine_dir = tempfile::tempdir().unwrap();
        let machine = test_inbox(&machine_dir);
        let forwarder = Arc::new(Forwarder::new(false));
        forwarder.remember_held("rq_host");
        machine.forwarding(forwarder);

        let err = machine
            .explain_ask(agent_req(
                pb::ExplainAskRequest {
                    id: id("rq_host"),
                    reply: "needed for tests".to_owned(),
                },
                "se_1",
            ))
            .await
            .unwrap_err();
        assert_eq!(err.code(), Code::Unavailable);
        assert!(matches!(
            status::status_error_of(&err).and_then(|e| e.kind),
            Some(pb::error::Kind::HostUnreachable(_))
        ));
    }

    #[tokio::test]
    async fn ask_read_builds_file_access_preview_and_rich_card_details() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = test_inbox(&dir);
        inbox
            .store
            .put_session(&SessionRecord {
                id: "se_asker".to_owned(),
                channel: "ch_1".to_owned(),
                machine: String::new(),
                title: "Ledger entries".to_owned(),
                agent: "claude".to_owned(),
                folder: std::path::PathBuf::from("/tmp/asker"),
                created_unix_ms: 0,
            })
            .unwrap();
        inbox
            .store
            .put_session(&SessionRecord {
                id: "se_other".to_owned(),
                channel: "ch_1".to_owned(),
                machine: String::new(),
                title: "Refund retries".to_owned(),
                agent: "copilot".to_owned(),
                folder: std::path::PathBuf::from("/tmp/other"),
                created_unix_ms: 0,
            })
            .unwrap();
        inbox
            .store
            .put_session_detail(&humata2_store::SessionDetail {
                session: "se_other".to_owned(),
                repos: vec![humata2_store::SessionRepo {
                    owner: "acme".to_owned(),
                    name: "api".to_owned(),
                    branch: "refund-retries".to_owned(),
                    path: std::path::PathBuf::from("/tmp/other/api"),
                }],
                model: None,
                first_message: None,
                start: humata2_store::StartStep::Done,
                operation: None,
                revision: 1,
            })
            .unwrap();

        let place = humata2_access::Place {
            machine: humata2_access::MachineId::new("local"),
            root: humata2_access::Root::Clone(humata2_access::SessionRef {
                id: humata2_access::SessionId::new("se_other"),
                channel: humata2_access::ChannelId::new("ch_1"),
                machine: humata2_access::MachineId::new("local"),
            }),
            path: humata2_access::RepoPath::new("acme/api/src/retry/backoff.rs").unwrap(),
            entry: humata2_access::Entry::File,
        };
        let asked = AccessRequest {
            capability: Capability::FilesReadOtherSession,
            channel: humata2_access::ChannelId::new("ch_1"),
            requester: humata2_access::SessionId::new("se_asker"),
            call: Call::ReadFiles(place),
            always_let: None,
            ahead: false,
            answer_at: humata2_access::AnswerAt::Host,
        };
        let caller = agent_req((), "se_asker").extensions().get::<Caller>().unwrap().clone();

        let pending = inbox
            .ask_read_reason(
                &caller,
                asked,
                "read",
                "Reuse the retry backoff policy".to_owned(),
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(pending.code(), Code::FailedPrecondition);

        let listed = inbox
            .list_requests(user_req(pb::ListRequestsRequest {
                agent_inbox: true,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(listed.requests.len(), 1);
        let card = &listed.requests[0];
        assert_eq!(card.reason, "Reuse the retry backoff policy");
        let Some(pb::request_preview::Preview::FileAccess(access)) =
            card.preview.as_ref().and_then(|p| p.preview.as_ref())
        else {
            panic!("expected FileAccessPreview, got {:?}", card.preview);
        };
        assert_eq!(
            access.source_session.as_ref().map(|s| s.value.as_str()),
            Some("se_other")
        );
        assert_eq!(
            access.source_machine.as_ref().map(|m| m.value.as_str()),
            Some("local")
        );
        assert_eq!(access.mode, pb::file_access_preview::Mode::ReadThisCall as i32);
        assert_eq!(access.paths.len(), 1);
        assert_eq!(access.paths[0].path, "src/retry/backoff.rs");
        assert_eq!(
            access.paths[0].repo.as_ref().map(|r| (r.owner.as_str(), r.name.as_str())),
            Some(("acme", "api"))
        );

        let from = card.facts.iter().find(|f| f.label == "From").unwrap();
        assert_eq!(from.lead, "Refund retries");
        assert!(from.rest.contains("copilot") && from.rest.contains("This Mac"));
        let files = card.facts.iter().find(|f| f.label == "Files").unwrap();
        assert_eq!(files.lead, "acme/api/src/retry/backoff.rs");
        assert!(card.affects.iter().any(|e| e.lead.contains("Refund retries") && !e.warn));
        assert!(!card.undo_text.is_empty());
    }

    #[tokio::test]
    async fn ask_user_preserves_todo_question_fields_and_accepts_selected_choices_reply() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = test_inbox(&dir);
        inbox
            .store
            .put_session(&SessionRecord {
                id: "se_1".to_owned(),
                channel: "ch_1".to_owned(),
                machine: String::new(),
                title: "t".to_owned(),
                agent: "claude".to_owned(),
                folder: std::path::PathBuf::from("/tmp/x"),
                created_unix_ms: 0,
            })
            .unwrap();

        let err = inbox
            .ask_user(agent_req(
                pb::AskUserRequest {
                    question: "Pick?".to_owned(),
                    multi_select: true,
                    ..Default::default()
                },
                "se_1",
            ))
            .await
            .unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);

        let waited = inbox
            .ask_user(agent_req(
                pb::AskUserRequest {
                    question: "Which targets?".to_owned(),
                    choices: vec!["mac".to_owned(), "linux".to_owned()],
                    todo_item: Some(pb::BoardItemId { value: "PAY-3".to_owned() }),
                    step_id: "st_1".to_owned(),
                    multi_select: true,
                    details: "Used to build release artifacts".to_owned(),
                    ..Default::default()
                },
                "se_1",
            ))
            .await
            .unwrap()
            .into_inner();
        let rq_id = waited.id.unwrap().value;

        let got = inbox
            .get_request(user_req(pb::GetRequestRequest { id: id(&rq_id) }))
            .await
            .unwrap()
            .into_inner();
        let Some(pb::request_preview::Preview::Question(q)) =
            got.preview.as_ref().and_then(|p| p.preview.as_ref())
        else {
            panic!("expected QuestionPreview");
        };
        assert_eq!(q.todo_item.as_ref().map(|i| i.value.as_str()), Some("PAY-3"));
        assert_eq!(q.step_id, "st_1");
        assert!(q.multi_select);
        assert_eq!(q.details, "Used to build release artifacts");

        let answered = inbox
            .answer_request(user_req(pb::AnswerRequestRequest {
                id: id(&rq_id),
                answer: Some(pb::RequestAnswer {
                    answer: Some(pb::request_answer::Answer::Reply(pb::Reply {
                        text: String::new(),
                        selected_choices: vec!["mac".to_owned(), "linux".to_owned()],
                    })),
                }),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(answered.state, pb::RequestState::Answered as i32);
        let Some(pb::request_answer::Answer::Reply(reply)) =
            answered.answer.and_then(|a| a.answer)
        else {
            panic!("expected Reply answer");
        };
        assert_eq!(reply.selected_choices, vec!["mac", "linux"]);
    }

    #[tokio::test]
    async fn todo_action_and_review_cards_accept_only_matching_structured_answers() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = test_inbox(&dir);
        inbox
            .store
            .put_session(&SessionRecord {
                id: "se_1".to_owned(),
                channel: "ch_1".to_owned(),
                machine: String::new(),
                title: "t".to_owned(),
                agent: "claude".to_owned(),
                folder: std::path::PathBuf::from("/tmp/x"),
                created_unix_ms: 0,
            })
            .unwrap();

        let action_req = pb::InboxRequest {
            id: id("rq_act"),
            kind: pb::RequestKind::Question as i32,
            state: pb::RequestState::Pending as i32,
            revision: Some(pb::Revision { value: 1 }),
            preview: Some(pb::RequestPreview {
                preview: Some(pb::request_preview::Preview::TodoAction(pb::TodoActionPreview {
                    todo_item: Some(pb::BoardItemId { value: "PAY-5".to_owned() }),
                    step_id: "st_1".to_owned(),
                    action: "Rotate key".to_owned(),
                    details: String::new(),
                })),
            }),
            ..Default::default()
        };
        let review_req = pb::InboxRequest {
            id: id("rq_rev"),
            kind: pb::RequestKind::Question as i32,
            state: pb::RequestState::Pending as i32,
            revision: Some(pb::Revision { value: 1 }),
            preview: Some(pb::RequestPreview {
                preview: Some(pb::request_preview::Preview::TodoReview(pb::TodoReviewPreview {
                    todo_item: Some(pb::BoardItemId { value: "PAY-5".to_owned() }),
                    step_id: String::new(),
                    summary: "Check schema".to_owned(),
                    details: String::new(),
                    round: 1,
                    checkpoint: "defined".to_owned(),
                })),
            }),
            ..Default::default()
        };
        {
            let mut state = inbox.state.lock().await;
            state.add(Entry::new("rq_act".to_owned(), "se_1".to_owned(), None, None, None, Held::new(action_req)));
            state.add(Entry::new("rq_rev".to_owned(), "se_1".to_owned(), None, None, None, Held::new(review_req)));
        }

        // Wrong answer kind on action card is refused.
        let wrong = inbox
            .answer_request(user_req(pb::AnswerRequestRequest {
                id: id("rq_act"),
                answer: Some(pb::RequestAnswer {
                    answer: Some(pb::request_answer::Answer::TodoReview(pb::TodoReviewAnswer {
                        approved: true,
                        note: String::new(),
                    })),
                }),
                ..Default::default()
            }))
            .await;
        assert_eq!(wrong.unwrap_err().code(), Code::InvalidArgument);

        // Matching TodoActionAnswer succeeds.
        let act_done = inbox
            .answer_request(user_req(pb::AnswerRequestRequest {
                id: id("rq_act"),
                answer: Some(pb::RequestAnswer {
                    answer: Some(pb::request_answer::Answer::TodoAction(pb::TodoActionAnswer {
                        done: true,
                        note: "rotated".to_owned(),
                    })),
                }),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(act_done.state, pb::RequestState::Answered as i32);

        // Matching TodoReviewAnswer succeeds.
        let rev_changes = inbox
            .answer_request(user_req(pb::AnswerRequestRequest {
                id: id("rq_rev"),
                answer: Some(pb::RequestAnswer {
                    answer: Some(pb::request_answer::Answer::TodoReview(pb::TodoReviewAnswer {
                        approved: false,
                        note: "add edge case test".to_owned(),
                    })),
                }),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(rev_changes.state, pb::RequestState::Answered as i32);
    }
}
