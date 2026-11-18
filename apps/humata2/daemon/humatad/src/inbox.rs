//! InboxService: questions an agent asks the user, access it asks for
//! ahead (IN-26), the user's answers, and waiting for them (MCP.md §5).
//! Approving an ask-ahead or answering "Always allow …" makes a grant in the
//! policy (IN-14, IN-22). Each request, its answer and the grant it made
//! are saved before anyone hears of them, so a restart keeps the Inbox
//! and the grants (D96). Each new request and each change to one goes on
//! the event feed.

mod deferred;
mod forwarded;
mod kept;
mod host_asks;
mod migrate;
mod migrated;
pub(crate) mod permission;
mod relay;
mod removed;
mod replay;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use humata2_access::{
    AccessRequest, Agent, Answer, AnswerError, Call, Capability, CheckError, Decision, GrantTarget,
    PresentUser, Principal, Refusal, RequestId, Source, Target, Verdict,
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
use crate::away::Forwarder;
use crate::events::{Feed, Scope};
use crate::grant_target;
use crate::guard::{self, Guard};
use crate::identity::{self, AgentCaller, Caller, UserVia};
use crate::status;

use deferred::Outcome;
pub(crate) use deferred::{BoardAsk, Deferred, Run};
pub(crate) use replay::Replayed;
use relay::{Relayed, Relays};

/// How often a wait says it is still waiting (MCP.md §5).
const KEEPALIVE: Duration = Duration::from_secs(15);
/// D158.7: how long a session stays Needs you after a pending return or a
/// wait that ended with no answer, for the agent's next wait to come.
const ASK_GAP: Duration = Duration::from_secs(30);

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
    /// The machines' live links, and the answers the host passed down them (D226).
    relays: Arc<Relays>,
    /// On a machine: the host's link, where a wait on a request the host holds
    /// goes up (D268). Set once the link's parts are made.
    forwarder: std::sync::OnceLock<Arc<Forwarder>>,
    /// What carries out a click on a Migrate item (UP-1). Set once the
    /// host's Migrate items are made.
    migrator: std::sync::OnceLock<crate::upgrade::items::Approver>,
    /// The agents this daemon runs, whose `agents.toml` says where each
    /// kind's own asks go (D782). Unset, every kind asks in the Inbox.
    agents: std::sync::OnceLock<humata2_sessions::Agents>,
}

#[derive(Default)]
struct State {
    requests: Vec<Entry>,
    /// The request each asking session's idempotency key made.
    keys: HashMap<String, String>,
}

/// `REQUEST_KIND_MIGRATE` (D281): a needed update outlives a stop, so the
/// stop's sweep leaves it pending. Becomes `pb::RequestKind::Migrate` once
/// UPGRADE adds it to the proto.
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
    /// Its reply must be one of the card's choices (D767): a question the
    /// agent asks on its screen takes only the answers the screen offers.
    only_choices: bool,
    /// For a question asked on the agent's screen (D767 amendment): whether
    /// the screen still asks it, with the chosen reply, when a reply comes.
    still_asked: Option<permission::StillAsked>,
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
            still_asked: None,
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

    /// Whether the request still waits for the user's answer.
    fn pending(&self) -> bool {
        self.request.state == pb::RequestState::Pending as i32
    }
}

/// Keeps `session` Needs you for [`ASK_GAP`] while its request stays
/// pending, for the agent's next wait on it (D158.7). Ends at once when the
/// request is answered or withdrawn.
fn hold_through_gap(session: &str, mut answers: watch::Receiver<Held>) {
    if !answers.borrow().pending() {
        return;
    }
    // A runtime shutting down has no gap to keep.
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let asking = crate::session_service::Asking::hold(session);
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
struct Gap(String, watch::Receiver<Held>);

impl Drop for Gap {
    fn drop(&mut self) {
        hold_through_gap(&self.0, self.1.clone());
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
            relays: Arc::new(Relays::new()),
            forwarder: std::sync::OnceLock::new(),
            migrator: std::sync::OnceLock::new(),
            agents: std::sync::OnceLock::new(),
        }
    }

    /// Reads where each agent kind asks from `agents`' switches file from
    /// now on (D782). A second call is ignored.
    pub(crate) fn use_agents(&self, agents: humata2_sessions::Agents) {
        let _ = self.agents.set(agents);
    }

    /// The machines' live links and the answers sent down them: each link
    /// loop of the host registers its machine here, and the replay reads
    /// what was sent.
    pub(crate) fn relays(&self) -> &Arc<Relays> {
        &self.relays
    }

    /// This Inbox with `relays` in place of its own.
    #[cfg(test)]
    fn with_relays(mut self, relays: Arc<Relays>) -> Self {
        self.relays = relays;
        self
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
        // The machine's journal takes the change too, as it is made (D225).
        let before = entry.request.borrow().request.state();
        crate::away::record_journal(
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

    /// The host's mirror of request `id`, when `machine` holds it, `session`
    /// made it and the user approved it: what a journaled board write names
    /// as its approval (AW-9, D268). `None` when any of those isn't so.
    pub(crate) async fn approved_on(&self, machine: &str, id: &str, session: &str) -> Option<pb::InboxRequest> {
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
        self.withdraw_pending_where(reason, |entry| entry.session == session)
            .await
    }

    /// Withdraws every pending request of this machine for `reason`, as
    /// when its sessions are stopped (D264).
    pub(crate) async fn withdraw_all_pending(
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
            return permission::decide(current, id, answer, entry.only_choices);
        }
        // UP-1: a Migrate item takes Migrate and nothing else.
        if let Some(refused) = crate::upgrade::items::refuse_answer(current, &answer) {
            return Err(refused);
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
        if crate::upgrade::items::is_migrate(&current) && matches!(answer, pb::request_answer::Answer::Approve(_)) {
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
            pb::request_answer::Answer::Approve(_) | pb::request_answer::Answer::Always(_)
        );
        if let Some(refused) = self
            .deny_if_never(entry, caller, user, approving, at)
            .await?
        {
            self.tell_answer(entry);
            return Err(refused);
        }
        let decided = self.decide(user, entry, &current, answer, at)?;
        // D767 amendment: a question asked on the agent's screen that the
        // screen no longer asks (answered in the terminal) takes no reply:
        // it is withdrawn, and the reply says so, not that it was answered.
        // D781: a Deny of one (the app's Dismiss) looks once too and types
        // nothing either way: gone from the screen, it is withdrawn as
        // resolved; still asked, or the screen unreadable, it is denied.
        if let Some(still_asked) = &entry.still_asked {
            match &decided.answer {
                pb::request_answer::Answer::Reply(reply) => {
                    if !still_asked(Some(reply.text.clone())).await? {
                        return Err(self.withdraw_unasked(entry).await);
                    }
                }
                pb::request_answer::Answer::Deny(_) => {
                    if let Ok(false) = still_asked(None).await {
                        return Ok(self.withdraw_resolved(entry).await);
                    }
                }
                _ => {}
            }
        }
        let by = answered_by(caller, user);
        let mut next = entry.request.borrow().clone();
        settle(&mut next, decided, by, at);
        self.commit(entry, next, Some(user), "save the answer")
            .await?;
        self.start(entry);
        Ok(self.tell_answer(entry))
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
        fits_card(current, &answer)?;
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
                at: Some(grant_target::timestamp(SystemTime::now())),
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
    pub(crate) async fn answer_relayed(&self, relayed: &pb::InboxAnswerRelay) -> Option<pb::InboxAnswerRefused> {
        let id = relayed
            .request
            .as_ref()
            .map(|id| id.value.clone())
            .unwrap_or_default();
        let answer = relayed.answer.as_ref().and_then(|answer| answer.answer.clone());
        let (false, Some(answer)) = (id.is_empty(), answer) else {
            crate::log::say!(
                "humatad2: warning: the host passed down an answer with no request or no answer in it, so it was left out; update Humata on the Mac and on this machine so both ends speak the same version"
            );
            return None;
        };
        let refusal = |status: Status| pb::InboxAnswerRefused {
            request: Some(pb::RequestId { value: id.clone() }),
            error: status::error_of(&status),
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
                crate::log::say!(
                    "humatad2: the host's answer to request {id} wasn't applied: {}",
                    refused.message()
                );
                let refused = match status::error_of(&refused).and_then(|error| error.kind) {
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
            // UP-1, D290: an Answer all never migrates anything.
            let entry = entry.filter(|entry| {
                !crate::upgrade::items::is_migrate(&entry.request.borrow().request)
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
        hold_through_gap(&entry.session, entry.request.subscribe());
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
        // A call that needs the Mac waits for it here, with no lock held:
        // each run takes the Inbox lock for itself and gives it back when
        // it fails, and the call is judged afresh when the Mac is back.
        let bound = guard::hold_bound(&caller, request.metadata());
        let message = request.into_inner();
        self.guard
            .held(bound, || async {
                let request = message.clone();
                // Asking ahead is asking; this also loads the channel's rules.
                self.guard.check_inbox_ask(&caller, &inbox_ask(agent))?;
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
                    Ok(Verdict::WaitForHost) => return Err(guard::waits_for_host()),
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
                hold_through_gap(&entry.session, entry.request.subscribe());
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
        let caller = identity::caller(&request)?;
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
        let asking = asker.as_deref().map(crate::session_service::Asking::hold);
        tokio::spawn(async move {
            let _asking = asking;
            let _gap = asker.map(|session| Gap(session, answers.clone()));
            let deadline = tokio::time::sleep(wait);
            tokio::pin!(deadline);
            let mut keepalive = tokio::time::interval(KEEPALIVE);
            loop {
                let current = answers.borrow_and_update().clone();
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
        | C::SessionDeleteOther
        | C::DeskMoveToChannel
        | C::GitPush => pb::RequestKind::Session,
        C::MachineProvision
        | C::MachineDestroy
        | C::MachineSessionsStop
        | C::MachineScriptRun
        | C::ProviderImport => {
            pb::RequestKind::Machine
        },
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
fn fits_card(request: &pb::InboxRequest, answer: &pb::request_answer::Answer) -> Result<(), Status> {
    use pb::request_answer::Answer as A;
    let id = request
        .id
        .as_ref()
        .map(|id| id.value.as_str())
        .unwrap_or_default();
    let question = request.kind == pb::RequestKind::Question as i32;
    match answer {
        A::Deny(_) => Ok(()),
        A::Reply(reply) if question => {
            if reply.text.trim().is_empty() {
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
                Some(_) => Err(grant_target::answer_failed(&AnswerError::TargetNotOffered)),
                None => Err(grant_target::answer_failed(&AnswerError::NoStandingApproval)),
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use humata2_store::SessionRecord;

    use super::*;
    use crate::away::{JOURNAL_ANSWER, JOURNAL_REQUEST, JOURNAL_WITHDRAWN};

    const HOST: &str = "ma_host";
    const BOX: &str = "ma_box";
    /// A wait the test expects to run out.
    const SHORT: Duration = Duration::from_millis(200);
    /// The bound on something that must happen.
    const BOUND: Duration = Duration::from_secs(5);

    fn card(id: &str, state: pb::RequestState) -> pb::InboxRequest {
        pb::InboxRequest {
            id: Some(pb::RequestId { value: id.to_owned() }),
            state: state as i32,
            kind: pb::RequestKind::Files as i32,
            requester: Some(pb::AgentActor {
                session: Some(pb::SessionId {
                    value: "se_a".to_owned(),
                }),
                ..Default::default()
            }),
            action_title: "Read files".to_owned(),
            ..Default::default()
        }
    }

    fn entry(seq: u64, operation: &str, request: &pb::InboxRequest) -> pb::JournalEntry {
        pb::JournalEntry {
            seq,
            principal: Some(pb::AgentPrincipal {
                session: Some(pb::SessionId {
                    value: "se_a".to_owned(),
                }),
                pid: 0,
            }),
            operation: operation.to_owned(),
            request: request.encode_to_vec(),
            ..Default::default()
        }
    }

    /// The host's Inbox, holding a mirror of `request`, whose machine's
    /// verdict is waited for at most `wait`.
    async fn host_holding(directory: &std::path::Path, wait: Duration, request: pb::InboxRequest) -> Arc<Inbox> {
        let store = Arc::new(Store::open(directory).unwrap());
        store
            .put_session(&SessionRecord {
                id: "se_a".to_owned(),
                channel: "ch_a".to_owned(),
                machine: BOX.to_owned(),
                title: "agent".to_owned(),
                agent: "claude".to_owned(),
                folder: PathBuf::new(),
                created_unix_ms: 0,
            })
            .unwrap();
        let guard = Arc::new(Guard::new(HOST, pb::DaemonRole::Host));
        let inbox = Inbox::new(guard, store, HOST.to_owned(), Arc::new(Feed::default()))
            .with_relays(Arc::new(Relays::with_wait(wait)));
        assert_eq!(
            inbox.replay(BOX, &entry(1, JOURNAL_REQUEST, &request)).await,
            Replayed::Applied
        );
        Arc::new(inbox)
    }

    /// What the machine does with the host's answer.
    #[derive(Clone, Copy)]
    enum Does {
        /// Applies it, and journals that.
        Applies,
        /// Had already denied the request: that entry reaches the host.
        HadDenied,
        /// Had already withdrawn the request.
        HadWithdrawn,
        /// Refuses it outright: the request was already answered there.
        Refuses,
        /// Says nothing.
        Silent,
        /// The link drops.
        HangsUp,
    }

    /// `BOX`'s link, up, with a machine behind it that does `does` with the
    /// answer it is sent. Dropping the registration takes the link away.
    fn linked(inbox: &Arc<Inbox>, does: Does) -> (relay::Registered, tokio::task::JoinHandle<()>) {
        let (link, mut from_host) = mpsc::channel(4);
        let registered = inbox.relays().register(BOX, link);
        let host = Arc::clone(inbox);
        let machine = tokio::spawn(async move {
            let Some(pb::HostToMachine {
                message: Some(pb::host_to_machine::Message::Answer(relayed)),
            }) = from_host.recv().await
            else {
                return;
            };
            let journal = match does {
                Does::Applies => {
                    let mut request = card("rq_1", pb::RequestState::Approved);
                    request.answer = relayed.answer.clone();
                    entry(2, JOURNAL_ANSWER, &request)
                }
                Does::HadDenied => {
                    let mut request = card("rq_1", pb::RequestState::Denied);
                    request.answer = Some(pb::RequestAnswer {
                        answer: Some(pb::request_answer::Answer::Deny(pb::Deny::default())),
                    });
                    entry(2, JOURNAL_ANSWER, &request)
                }
                Does::HadWithdrawn => {
                    let mut request = card("rq_1", pb::RequestState::Withdrawn);
                    request.withdraw_reason = pb::WithdrawReason::CallEnded as i32;
                    entry(2, JOURNAL_WITHDRAWN, &request)
                }
                Does::Refuses => {
                    host.relays().refused(
                        BOX,
                        pb::InboxAnswerRefused {
                            request: Some(pb::RequestId { value: "rq_1".to_owned() }),
                            error: status::error_of(&still_open(&card("rq_1", pb::RequestState::Denied)).unwrap_err()),
                            code: Code::Aborted as i32,
                        },
                    );
                    // The link stays up, and no journal entry ever comes.
                    std::future::pending::<()>().await;
                    return;
                }
                Does::Silent => {
                    // The link stays up until the task ends with the test.
                    std::future::pending::<()>().await;
                    return;
                }
                Does::HangsUp => {
                    drop(from_host);
                    return;
                }
            };
            host.replay(BOX, &journal).await;
        });
        (registered, machine)
    }

    async fn host_answers(inbox: &Inbox, answer: pb::request_answer::Answer) -> Result<pb::InboxRequest, Status> {
        let caller = Caller::User(UserVia::App);
        let user = inbox.present_user(&caller).unwrap();
        tokio::time::timeout(BOUND, inbox.answer_as(&caller, &user, "rq_1".to_owned(), answer, None))
            .await
            .expect("the host's answer ends within the bound")
    }

    fn approve() -> pb::request_answer::Answer {
        pb::request_answer::Answer::Approve(pb::Approve {})
    }

    fn kind_of(status: &Status) -> Option<pb::error::Kind> {
        status::error_of(status).and_then(|error| error.kind)
    }

    async fn state_of(inbox: &Inbox) -> pb::RequestState {
        let state = inbox.state.lock().await;
        let held = state.requests.iter().find(|held| held.id == "rq_1").unwrap();
        held.request.borrow().request.state()
    }

    #[tokio::test]
    async fn the_hosts_answer_goes_to_the_machine_and_comes_back_settled_and_credited_to_the_host_user() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = host_holding(directory.path(), BOUND, card("rq_1", pb::RequestState::Pending)).await;
        let (_registered, _machine) = linked(&inbox, Does::Applies);
        let settled = host_answers(&inbox, approve()).await.unwrap();
        assert_eq!(settled.state(), pb::RequestState::Approved);
        let by = settled.answered.and_then(|answered| answered.by).and_then(|actor| actor.actor);
        let Some(pb::actor::Actor::User(user)) = by else {
            panic!("the answer should be credited to the user");
        };
        assert_eq!(user.via(), pb::user_actor::Via::App);
        assert_eq!(state_of(&inbox).await, pb::RequestState::Approved);
    }

    #[tokio::test]
    async fn an_answer_with_the_link_down_is_refused_and_the_request_stays_pending() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = host_holding(directory.path(), BOUND, card("rq_1", pb::RequestState::Pending)).await;
        let refused = host_answers(&inbox, approve()).await.unwrap_err();
        assert!(matches!(kind_of(&refused), Some(pb::error::Kind::MachineOffline(_))));
        assert_eq!(state_of(&inbox).await, pb::RequestState::Pending);
        assert_eq!(inbox.relays().outstanding("rq_1"), None);
    }

    #[tokio::test]
    async fn an_answer_the_machine_beat_with_a_different_one_is_already_answered() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = host_holding(directory.path(), BOUND, card("rq_1", pb::RequestState::Pending)).await;
        let (_registered, _machine) = linked(&inbox, Does::HadDenied);
        let refused = host_answers(&inbox, approve()).await.unwrap_err();
        assert!(matches!(kind_of(&refused), Some(pb::error::Kind::AlreadyAnswered(_))));
        assert_eq!(state_of(&inbox).await, pb::RequestState::Denied);
    }

    #[tokio::test]
    async fn an_answer_the_machine_refuses_ends_the_wait_with_its_refusal() {
        let directory = tempfile::tempdir().unwrap();
        // A wait that would fail the test's own bound if it ran out.
        let inbox = host_holding(directory.path(), BOUND * 10, card("rq_1", pb::RequestState::Pending)).await;
        let (_registered, _machine) = linked(&inbox, Does::Refuses);
        let refused = host_answers(&inbox, approve()).await.unwrap_err();
        assert!(matches!(kind_of(&refused), Some(pb::error::Kind::AlreadyAnswered(_))));
        assert_eq!(refused.code(), Code::Aborted);
        assert_eq!(state_of(&inbox).await, pb::RequestState::Pending);
        assert_eq!(inbox.relays().outstanding("rq_1"), None);
    }

    #[tokio::test]
    async fn an_answer_to_a_machine_whose_link_dropped_fails_at_once_as_machine_offline() {
        let directory = tempfile::tempdir().unwrap();
        // A wait that would fail the test's own bound if it ran out.
        let inbox = host_holding(directory.path(), BOUND * 10, card("rq_1", pb::RequestState::Pending)).await;
        let (registered, machine) = linked(&inbox, Does::Silent);
        drop(registered);
        machine.abort();
        let refused = host_answers(&inbox, approve()).await.unwrap_err();
        assert!(matches!(kind_of(&refused), Some(pb::error::Kind::MachineOffline(_))));
        let fix = status::error_of(&refused).map(|error| error.fix).unwrap_or_default();
        assert!(!fix.is_empty(), "the refusal says what to do");
        assert_eq!(inbox.relays().outstanding("rq_1"), None);
    }

    #[tokio::test]
    async fn a_timed_out_answer_leaves_no_record_so_a_second_answer_after_the_machine_returns_works() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = host_holding(directory.path(), SHORT, card("rq_1", pb::RequestState::Pending)).await;
        let (silent, machine) = linked(&inbox, Does::Silent);
        let unconfirmed = host_answers(&inbox, approve()).await.unwrap_err();
        assert!(matches!(kind_of(&unconfirmed), Some(pb::error::Kind::MachineOffline(_))));
        assert_eq!(inbox.relays().outstanding("rq_1"), None);

        drop(silent);
        machine.abort();
        let (_registered, _machine) = linked(&inbox, Does::Applies);
        let settled = host_answers(&inbox, approve()).await.unwrap();
        assert_eq!(settled.state(), pb::RequestState::Approved);
    }

    #[tokio::test]
    async fn a_stop_withdraws_a_pending_request() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = host_holding(directory.path(), BOUND, card("rq_1", pb::RequestState::Pending)).await;
        inbox
            .withdraw_all_pending(pb::WithdrawReason::MachineStopped)
            .await
            .unwrap();
        assert_eq!(state_of(&inbox).await, pb::RequestState::Withdrawn);
    }

    #[tokio::test]
    async fn a_stop_leaves_a_pending_migrate_item_pending() {
        let directory = tempfile::tempdir().unwrap();
        let migrate = pb::InboxRequest {
            kind: MIGRATE_KIND,
            ..card("rq_1", pb::RequestState::Pending)
        };
        let inbox = host_holding(directory.path(), BOUND, migrate).await;
        inbox
            .withdraw_all_pending(pb::WithdrawReason::MachineStopped)
            .await
            .unwrap();
        assert_eq!(state_of(&inbox).await, pb::RequestState::Pending);
    }

    #[tokio::test]
    async fn an_answer_to_a_request_the_machine_withdrew_is_request_withdrawn() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = host_holding(directory.path(), BOUND, card("rq_1", pb::RequestState::Pending)).await;
        let (_registered, _machine) = linked(&inbox, Does::HadWithdrawn);
        let refused = host_answers(&inbox, approve()).await.unwrap_err();
        assert!(matches!(kind_of(&refused), Some(pb::error::Kind::RequestWithdrawn(_))));
    }

    #[tokio::test]
    async fn no_verdict_in_time_is_machine_offline_and_the_request_stays_pending() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = host_holding(directory.path(), SHORT, card("rq_1", pb::RequestState::Pending)).await;
        let (_registered, _machine) = linked(&inbox, Does::Silent);
        let unconfirmed = host_answers(&inbox, approve()).await.unwrap_err();
        assert!(matches!(kind_of(&unconfirmed), Some(pb::error::Kind::MachineOffline(_))));
        assert_eq!(state_of(&inbox).await, pb::RequestState::Pending);
    }

    #[tokio::test]
    async fn a_link_that_drops_while_waiting_ends_the_wait_at_once() {
        let directory = tempfile::tempdir().unwrap();
        // A wait that would fail the test's own bound if it ran out.
        let inbox = host_holding(directory.path(), BOUND * 10, card("rq_1", pb::RequestState::Pending)).await;
        let (_registered, _machine) = linked(&inbox, Does::HangsUp);
        let unconfirmed = host_answers(&inbox, approve()).await.unwrap_err();
        assert!(matches!(kind_of(&unconfirmed), Some(pb::error::Kind::MachineOffline(_))));
        assert_eq!(inbox.relays().outstanding("rq_1"), None);
    }

    #[tokio::test]
    async fn an_answer_the_card_does_not_take_is_refused_and_nothing_is_sent() {
        let directory = tempfile::tempdir().unwrap();
        let question = pb::InboxRequest {
            kind: pb::RequestKind::Question as i32,
            ..card("rq_1", pb::RequestState::Pending)
        };
        let inbox = host_holding(directory.path(), BOUND, question).await;
        let (link, mut from_host) = mpsc::channel(4);
        let _registered = inbox.relays().register(BOX, link);
        let refused = host_answers(&inbox, approve()).await.unwrap_err();
        assert!(matches!(kind_of(&refused), Some(pb::error::Kind::InvalidField(_))));
        assert!(from_host.try_recv().is_err(), "a wrong answer isn't passed on");
        assert_eq!(state_of(&inbox).await, pb::RequestState::Pending);
    }

    #[tokio::test]
    async fn an_answer_that_is_not_the_one_the_host_sent_is_not_credited_to_the_host() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = host_holding(directory.path(), BOUND, card("rq_1", pb::RequestState::Pending)).await;
        // The machine had approved before the relay got there, and says so.
        let (_registered, _machine) = linked(&inbox, Does::HadDenied);
        let _ = host_answers(&inbox, approve()).await;
        let state = inbox.state.lock().await;
        let held = state.requests.iter().find(|held| held.id == "rq_1").unwrap();
        let by = held
            .request
            .borrow()
            .request
            .answered
            .clone()
            .and_then(|answered| answered.by)
            .and_then(|actor| actor.actor);
        let Some(pb::actor::Actor::User(user)) = by else {
            panic!("the answer should be credited to the user");
        };
        assert_eq!(user.via(), pb::user_actor::Via::CliOnMachine);
    }

    #[tokio::test]
    async fn a_machine_counts_the_hosts_relayed_user_as_on_host() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(directory.path()).unwrap());
        let guard = Arc::new(Guard::new(BOX, pb::DaemonRole::Machine));
        let inbox = Inbox::new(guard, store, BOX.to_owned(), Arc::new(Feed::default()));
        let user = inbox.relayed_user().unwrap();
        assert!(user.on_host(), "an answer from the Mac is at the Mac, so a Host-only card takes it");
        assert_eq!(user.machine().to_string(), BOX);
        // A passphrase-less user on the machine itself is not.
        let local = inbox.present_user(&Caller::User(UserVia::App)).unwrap();
        assert!(!local.on_host());
    }

    /// Lets the gap's task see what changed.
    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_pending_ask_holds_needs_you_through_the_gap_and_an_answer_ends_it() {
        let session = "se_gap_answer";
        let (request, answers) = watch::channel(Held::new(card("rq_1", pb::RequestState::Pending)));
        hold_through_gap(session, answers);
        assert!(crate::session_service::turns_asking(session));
        tokio::time::advance(ASK_GAP / 2).await;
        settle().await;
        assert!(crate::session_service::turns_asking(session), "still in the gap");
        request.send_replace(Held::new(card("rq_1", pb::RequestState::Approved)));
        settle().await;
        assert!(!crate::session_service::turns_asking(session), "the answer ends it");
    }

    #[tokio::test(start_paused = true)]
    async fn a_gap_with_no_next_wait_runs_out() {
        let session = "se_gap_out";
        let (_request, answers) = watch::channel(Held::new(card("rq_2", pb::RequestState::Pending)));
        hold_through_gap(session, answers);
        assert!(crate::session_service::turns_asking(session));
        tokio::time::advance(ASK_GAP + Duration::from_secs(1)).await;
        // A sleep lets the timer driver turn, so the gap's own timer fires.
        tokio::time::sleep(Duration::from_millis(1)).await;
        settle().await;
        assert!(!crate::session_service::turns_asking(session));
    }

    #[tokio::test]
    async fn a_settled_request_holds_nothing() {
        let session = "se_gap_settled";
        let (_request, answers) = watch::channel(Held::new(card("rq_3", pb::RequestState::Denied)));
        hold_through_gap(session, answers);
        assert!(!crate::session_service::turns_asking(session));
    }
}
