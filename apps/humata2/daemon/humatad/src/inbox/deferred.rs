//! Ordinary calls a channel rule sent to Ask me (D38.6). The call isn't
//! refused: it becomes an Inbox request, the agent waits on it (TP-11,
//! TP-12), and approving it runs the call (TP-13) whether or not anyone
//! still waits. The result is kept in memory for 24 hours; a restart loses
//! the closure that would run the call and any result, so an approved call
//! the restart cut off asks its agent to repeat the call with the same
//! idempotency key, which then runs it without asking again.

use std::future::Future;
use std::pin::Pin;
use std::sync::PoisonError;
use std::time::{Duration, SystemTime};

use humata2_access::{AccessRequest, AnswerError, Decision};
use humata2_grpc::humata::v1 as pb;
use tonic::Status;

use super::{
    Decided, Entry, Held, Inbox, answered_by, id_of, kind_of, lowercase_first, new_id, settle,
};
use crate::attach;
use crate::grant_target;
use crate::identity::Caller;
use crate::status;

/// How long an approved call's result stays available to a wait (TP-12).
const RESULT_KEPT: Duration = Duration::from_secs(24 * 60 * 60);

/// What an approved call produced: its response, encoded, and the response
/// message's full name (e.g. "humata.v1.ListChannelsResponse").
pub(crate) struct Deferred {
    pub(crate) result: Vec<u8>,
    pub(crate) result_type: String,
}

/// The call, to run once the user approves it.
pub(crate) type Run =
    Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = Result<Deferred, Status>> + Send>> + Send>;

/// How an approved call ended.
#[derive(Clone)]
pub(super) struct Outcome {
    pub(super) result: Vec<u8>,
    pub(super) result_type: String,
    /// What it failed with, or why it was denied after all.
    pub(super) error: Option<pb::Error>,
    pub(super) kept_until: SystemTime,
}

impl Outcome {
    fn of(done: Result<Deferred, Status>) -> Self {
        let kept_until = SystemTime::now() + RESULT_KEPT;
        match done {
            Ok(done) => Self {
                result: done.result,
                result_type: done.result_type,
                error: None,
                kept_until,
            },
            Err(failed) => Self {
                result: Vec::new(),
                result_type: String::new(),
                error: Some(status::error_of(&failed).unwrap_or_else(|| pb::Error {
                    message: failed.message().to_owned(),
                    fix: "check the message, then repeat the call".to_owned(),
                    retry: None,
                    kind: None,
                })),
                kept_until,
            },
        }
    }
}

/// Whether `request` is a deferred call's card: it has no preview of its
/// own, which a question and an ask-ahead always do.
pub(super) fn is_deferred(request: &pb::InboxRequest) -> bool {
    request.preview.is_none()
}

/// Approved, but the call neither runs nor has a result: the process that
/// held it restarted.
pub(super) fn lost(held: &Held) -> bool {
    held.deferred
        && held.request.state == pb::RequestState::Approved as i32
        && !held.running
        && held.outcome.is_none()
}

pub(super) fn lost_error(request: &str) -> pb::Error {
    status::error(
        format!(
            "request {request} was approved, but humatad2 restarted before the call finished, so its result is gone"
        ),
        "repeat the original call with the same idempotency key: it runs again without asking the user a second time, unless the rule has changed since",
        pb::error::Kind::RequestWithdrawn(pb::RequestWithdrawn {
            request: Some(pb::RequestId {
                value: request.to_owned(),
            }),
        }),
    )
}

/// The card for a deferred call (IN-9). It offers no "Always let": the
/// rule is the standing answer, and changing it is the user's to do.
fn card(
    id: &str,
    agent: &crate::identity::AgentCaller,
    asked: &AccessRequest,
    decision: Option<Decision>,
) -> pb::InboxRequest {
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
            "{} asks to {}",
            agent.title,
            lowercase_first(asked.capability.label())
        ),
        capability: crate::guard::capability(asked.capability) as i32,
        rule_decision: decision.map_or(pb::Decision::Unspecified, super::decision_of) as i32,
        created_at: Some(attach::now()),
        revision: Some(pb::Revision { value: 1 }),
        ..Default::default()
    }
}

impl Inbox {
    /// Puts the agent's call in the Inbox, to run once the user approves it
    /// (D38.6). Returns the status the handler returns: the call is waiting
    /// on that request (`approval_pending`), or why it can't be (a store
    /// failure, a rule that is Never by now). The same caller and key
    /// return the request already made (TP-14).
    pub(crate) async fn defer(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        key: Option<String>,
        run: Run,
    ) -> Status {
        match self.deferred(caller, asked, key, run).await {
            Ok(id) => status::approval_pending(&id),
            Err(refused) => refused,
        }
    }

    async fn deferred(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        key: Option<String>,
        run: Run,
    ) -> Result<pb::RequestId, Status> {
        let Caller::Agent(agent) = caller else {
            return Err(status::internal(
                "a call was deferred for someone who isn't a session's agent",
                "",
            ));
        };
        let session = agent.session.id.to_string();
        let key = id_of(key).map(|key| format!("{session}/{key}"));
        let mut state = self.state.lock().await;
        if let Some(id) = state.id_with(key.as_deref()) {
            if let Some(entry) = state.requests.iter().find(|entry| entry.id == id) {
                self.rerun(entry, &asked, run);
            }
            return Ok(pb::RequestId { value: id });
        }
        let id = new_id();
        let decision = self.rule_now(&asked);
        let mut held = Held::new(card(&id, agent, &asked, decision));
        held.deferred = true;
        let entry = Entry::new(id.clone(), session, Some(asked), None, key, held.clone());
        *entry.run.lock().unwrap_or_else(PoisonError::into_inner) = Some(run);
        self.commit(&entry, held, None, "save the request").await?;
        self.tell(
            pb::Change::Created,
            &entry.session,
            &entry.request.borrow().request,
        );
        state.add(entry);
        Ok(pb::RequestId { value: id })
    }

    /// A repeat of a call a restart cut off after it was approved runs it
    /// again, unless its rule is Never by now.
    fn rerun(&self, entry: &Entry, asked: &AccessRequest, run: Run) {
        if !lost(&entry.request.borrow()) || self.rule_now(asked) == Some(Decision::Never) {
            return;
        }
        *entry.run.lock().unwrap_or_else(PoisonError::into_inner) = Some(run);
        entry.request.send_modify(|held| held.running = true);
        self.start(entry);
    }

    /// The channel's rule for `asked`'s capability as it is now.
    fn rule_now(&self, asked: &AccessRequest) -> Option<Decision> {
        self.guard.with_policy(|policy, _| {
            policy
                .channel_rules(&asked.channel)
                .and_then(|rules| rules.decision(asked.capability))
        })
    }

    /// Settles `entry` as denied when the user approves a deferred call
    /// whose rule has become Never since it was asked (IN-21). Returns the
    /// status the answer fails with, or `None` when there is nothing to
    /// deny. The caller tells the feed.
    pub(super) async fn deny_if_never(
        &self,
        entry: &Entry,
        caller: &Caller,
        user: &humata2_access::PresentUser,
        approving: bool,
        at: SystemTime,
    ) -> Result<Option<Status>, Status> {
        let Some(asked) = entry.access.as_ref().filter(|_| approving) else {
            return Ok(None);
        };
        if !entry.request.borrow().deferred || self.rule_now(asked) != Some(Decision::Never) {
            return Ok(None);
        }
        let denied = pb::Denied {
            source: pb::denied::Source::NeverRule as i32,
            capability: crate::guard::capability(asked.capability) as i32,
            request: Some(pb::RequestId {
                value: entry.id.clone(),
            }),
            user_note: String::new(),
        };
        let error = status::error(
            format!(
                "the rule for \"{}\" became Never after this was asked, so the user couldn't approve it",
                asked.capability
            ),
            "don't repeat the call; ask the user to change the rule in Agent access if it should run",
            pb::error::Kind::Denied(denied),
        );
        let decided = Decided {
            state: pb::RequestState::Denied,
            answer: pb::request_answer::Answer::Deny(pb::Deny::default()),
            grant: None,
        };
        let mut next = entry.request.borrow().clone();
        settle(&mut next, decided, answered_by(caller, user), at);
        next.outcome = Some(Outcome {
            result: Vec::new(),
            result_type: String::new(),
            error: Some(error),
            kept_until: at + RESULT_KEPT,
        });
        self.commit(entry, next, Some(user), "save the answer")
            .await?;
        Ok(Some(grant_target::answer_failed(
            &AnswerError::RuleIsNever {
                channel: asked.channel.clone(),
                capability: asked.capability,
            },
        )))
    }

    /// Runs `entry`'s call if it was just approved, in a task of its own
    /// that finishes whether or not anyone waits (TP-13).
    pub(super) fn start(&self, entry: &Entry) {
        {
            let held = entry.request.borrow();
            if !held.deferred || held.request.state != pb::RequestState::Approved as i32 {
                return;
            }
        }
        let run = entry
            .run
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let sender = entry.request.clone();
        let Some(run) = run else {
            sender.send_modify(|held| held.running = false);
            return;
        };
        tokio::spawn(async move {
            // A task of its own, so a panic in the call ends it as a failure.
            let done = match tokio::spawn(run()).await {
                Ok(done) => done,
                Err(error) => Err(status::internal(format!("the call failed: {error}"), "")),
            };
            let outcome = Outcome::of(done);
            sender.send_modify(|held| {
                held.running = false;
                held.outcome = Some(outcome);
            });
        });
    }
}
