//! The host takes a machine's Inbox journal into its own Inbox (AWAY T7,
//! D226). A request the machine holds shows here as a mirror with the same
//! id, marked by `journal` as held on that machine; the machine's answer or
//! withdrawal then settles it, credited to the user at that machine.
//!
//! Entries are applied one at a time in the order they arrive. Each one,
//! applied or rejected, moves the machine's mark up in the same store
//! transaction as the Inbox row it made, so a restart or a second delivery
//! changes nothing. Only a failed store write holds the link: nothing moves
//! and the caller asks the machine to send the entry again.

use std::sync::Arc;
use std::time::SystemTime;

use humata2_access::{Answer, GrantId};
use humata2_daemon_ports::{Caller, UserVia};
use humata2_grpc::humata::v1 as pb;
use humata2_rpc_kit::{helpers as status, log};
use humata2_store::{SessionRecord, Store, StoreError};
use humata2_store_proto::humata::store::v1 as stored;
use humata2_svc_away::{JOURNAL_ANSWER, JOURNAL_REQUEST, JOURNAL_WITHDRAWN};
use prost::Message;
use tokio::task::JoinError;

use super::{Entry, Held, Inbox, kept, next_revision};
use crate::ports;

/// What became of one journal entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replayed {
    /// Taken, or already taken before. The machine may drop it.
    Applied,
    /// Not one the host can take. Logged; the machine may drop it.
    Rejected,
    /// The host's store failed. Nothing moved: the entry must come again.
    Held,
}

/// Why the host's store couldn't take an entry.
#[derive(Debug, thiserror::Error)]
enum ReplayError {
    #[error(
        "couldn't save {machine}'s journal entry {seq} in this Mac's store; check that the disk has room and the state folder is writable. Nothing after it is applied until it is saved, and Humata asks {machine} to send it again"
    )]
    Store {
        machine: String,
        seq: u64,
        #[source]
        source: StoreError,
    },

    #[error(
        "a background task saving {machine}'s journal entry {seq} stopped; Humata asks {machine} to send it again, and restarting it helps if this keeps happening"
    )]
    Task {
        machine: String,
        seq: u64,
        #[source]
        source: JoinError,
    },
}

/// Why an entry isn't one the host can take.
#[derive(Debug, thiserror::Error)]
enum Rejection {
    #[error(
        "`{operation}` isn't an Inbox operation the host takes; nothing to do, the machine drops it"
    )]
    UnknownOperation { operation: String },

    #[error(
        "its request can't be read; update Humata on the machine so both ends speak the same version"
    )]
    Undecodable {
        #[source]
        source: prost::DecodeError,
    },

    #[error("its request id `{id}` isn't a valid id")]
    BadId { id: String },

    #[error(
        "it names session `{session}`, which this Mac doesn't know; the session may have been deleted here"
    )]
    SessionUnknown { session: String },

    #[error(
        "it names session `{session}`, which this Mac places on {placed}, not on the machine that sent it"
    )]
    SessionElsewhere { session: String, placed: String },

    #[error("the session that sent it isn't the one that made the request")]
    NotRequester,

    #[error(
        "request `{id}` isn't this machine's: the Inbox already holds one with that id from another source"
    )]
    Foreign { id: String },

    #[error("`{operation}` can't carry a request in state {state}")]
    StateDisagrees { operation: String, state: i32 },

    #[error(
        "request `{id}` already stands as state {first}, so the later state {later} is left out; the first recorded stands"
    )]
    Conflict { id: String, first: i32, later: i32 },

    #[error("browser request `{id}` carried an answer kind that isn't allowed for it")]
    InvalidBrowserAnswer { id: String },
}

/// What one entry brings, with what the host already holds.
struct Arrival<'a> {
    machine: &'a str,
    seq: u64,
    operation: &'a str,
    at: Option<humata2_grpc::Timestamp>,
    /// The session the entry says made the call.
    session: &'a str,
    /// The host's own machine.
    host: &'a str,
    /// The answer the host has sent down for this request and not yet
    /// seen settled.
    relayed: Option<pb::RequestAnswer>,
    request: pb::InboxRequest,
}

/// What the host knows about it.
struct Known<'a> {
    session: Option<&'a SessionRecord>,
    /// The host's request with that id: the session it is kept under, and
    /// the request as it stands.
    existing: Option<(&'a str, &'a pb::InboxRequest)>,
}

fn valid_id(id: &str) -> bool {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    !id.is_empty() && id.len() <= 128 && !id.starts_with('.') && id.chars().all(allowed)
}

/// The request the entry makes the host's request, or `None` when the host
/// already stands as it says.
fn decide(arrival: &Arrival<'_>, known: &Known<'_>) -> Result<Option<pb::InboxRequest>, Rejection> {
    let request = &arrival.request;
    let id = request
        .id
        .as_ref()
        .map(|id| id.value.as_str())
        .unwrap_or_default();
    if !valid_id(id) {
        return Err(Rejection::BadId { id: id.to_owned() });
    }
    let session = known.session.ok_or_else(|| Rejection::SessionUnknown {
        session: arrival.session.to_owned(),
    })?;
    if session.machine != arrival.machine {
        return Err(Rejection::SessionElsewhere {
            session: session.id.clone(),
            placed: session.machine.clone(),
        });
    }
    let requested_by = request
        .requester
        .as_ref()
        .and_then(|agent| agent.session.as_ref())
        .map(|session| session.value.as_str());
    if requested_by != Some(arrival.session) {
        return Err(Rejection::NotRequester);
    }
    let existing = match known.existing {
        Some((owner, held)) => {
            let from = held
                .journal
                .as_ref()
                .and_then(|position| position.machine.as_ref())
                .map(|machine| machine.value.as_str());
            if from != Some(arrival.machine) || owner != arrival.session {
                return Err(Rejection::Foreign { id: id.to_owned() });
            }
            Some(held)
        }
        None => None,
    };
    let state = request.state();
    let disagrees = || Rejection::StateDisagrees {
        operation: arrival.operation.to_owned(),
        state: state as i32,
    };
    let conflict = |held: &pb::InboxRequest| Rejection::Conflict {
        id: id.to_owned(),
        first: held.state,
        later: state as i32,
    };
    match arrival.operation {
        JOURNAL_REQUEST => {
            if state != pb::RequestState::Pending {
                return Err(disagrees());
            }
            // A request sent again is the mirror already made.
            Ok(existing.is_none().then(|| mirrored(arrival, session)))
        }
        JOURNAL_ANSWER => {
            if !matches!(
                state,
                pb::RequestState::Approved | pb::RequestState::Denied | pb::RequestState::Answered
            ) {
                return Err(disagrees());
            }
            if !super::browser::valid_replayed_answer(request) {
                return Err(Rejection::InvalidBrowserAnswer { id: id.to_owned() });
            }
            let credit = Some(credited(arrival));
            let Some(held) = existing else {
                // A request never seen is made already settled.
                let mut made = mirrored(arrival, session);
                made.answered = credit;
                return Ok(Some(made));
            };
            match held.state() {
                pb::RequestState::Pending => {
                    let mut next = held.clone();
                    next.state = state as i32;
                    next.answer = request.answer.clone();
                    next.answered = credit;
                    next.revision = revised(arrival, held);
                    Ok(Some(next))
                }
                before if before == state => Ok(None),
                _ => Err(conflict(held)),
            }
        }
        JOURNAL_WITHDRAWN => {
            if state != pb::RequestState::Withdrawn {
                return Err(disagrees());
            }
            let Some(held) = existing else {
                return Ok(Some(mirrored(arrival, session)));
            };
            match held.state() {
                pb::RequestState::Pending => {
                    let mut next = held.clone();
                    next.state = state as i32;
                    next.withdraw_reason = request.withdraw_reason;
                    next.revision = revised(arrival, held);
                    Ok(Some(next))
                }
                pb::RequestState::Withdrawn => Ok(None),
                _ => Err(conflict(held)),
            }
        }
        other => Err(Rejection::UnknownOperation {
            operation: other.to_owned(),
        }),
    }
}

/// The entry's request as the host holds it: the same id, marked held on
/// the machine, in the channel the host's own record gives the session.
fn mirrored(arrival: &Arrival<'_>, session: &SessionRecord) -> pb::InboxRequest {
    let mut request = arrival.request.clone();
    request.journal = Some(pb::JournalPosition {
        machine: Some(pb::MachineId {
            value: arrival.machine.to_owned(),
        }),
        seq: arrival.seq,
    });
    request.channel = Some(pb::ChannelId {
        value: session.channel.clone(),
    });
    super::browser::clamp_preview(&mut request);
    request
}

/// The answer's credit: when the machine says it was made, by the user at
/// that machine's terminal (AW-6). When the host has just relayed that very
/// answer, it is the host's own user: the one at the Mac who gave it.
fn credited(arrival: &Arrival<'_>) -> pb::Timestamped {
    let at = arrival
        .request
        .answered
        .as_ref()
        .and_then(|answered| answered.at.clone())
        .or_else(|| arrival.at.clone());
    let relayed = arrival
        .relayed
        .as_ref()
        .is_some_and(|sent| arrival.request.answer.as_ref() == Some(sent));
    let (via, machine) = if relayed {
        (pb::user_actor::Via::App, arrival.host)
    } else {
        (pb::user_actor::Via::CliOnMachine, arrival.machine)
    };
    pb::Timestamped {
        at,
        by: Some(pb::Actor {
            actor: Some(pb::actor::Actor::User(pb::UserActor {
                via: via as i32,
                machine: Some(pb::MachineId {
                    value: machine.to_owned(),
                }),
            })),
        }),
    }
}

/// The revision the change leaves: the machine's own, or one more.
fn revised(arrival: &Arrival<'_>, held: &pb::InboxRequest) -> Option<pb::Revision> {
    arrival
        .request
        .revision
        .clone()
        .or_else(|| Some(next_revision(held)))
}

/// The target an "Always allow" answer names.
fn always_target(request: &pb::InboxRequest) -> Option<pb::GrantTarget> {
    match request.answer.as_ref()?.answer.as_ref()? {
        pb::request_answer::Answer::Always(always) => always.target.clone(),
        _ => None,
    }
}

fn say_not_forgotten(machine: &str, why: &str) {
    log::line(&format!(
        "humatad2: warning: couldn't forget how far {machine}'s journal was applied: {why}. If {machine} is added again, the first entries of its new journal may be left out as already applied; forget it again once the disk problem is fixed"
    ));
}

impl Inbox {
    /// How far `machine`'s journal is applied, for the host's Resume. A
    /// mark that can't be read is 0: the machine sends everything again,
    /// and applying an entry twice changes nothing.
    pub async fn replayed_through(&self, machine: &str) -> u64 {
        let store = Arc::clone(&self.store);
        let name = machine.to_owned();
        let read = tokio::task::spawn_blocking(move || store.machine_journal_mark(&name)).await;
        match read {
            Ok(Ok(mark)) => mark,
            Ok(Err(error)) => {
                log::line(&format!(
                    "humatad2: warning: couldn't read how far {machine}'s journal is applied, so it is sent again from the start: {}",
                    status::chain(&error)
                ));
                0
            }
            Err(error) => {
                log::line(&format!(
                    "humatad2: warning: reading how far {machine}'s journal is applied stopped, so it is sent again from the start: {error}"
                ));
                0
            }
        }
    }

    /// Applies `machine`'s journal `entry`. Entries are applied in the order
    /// they come; one at or below the mark was applied before and changes
    /// nothing.
    pub async fn replay(&self, machine: &str, entry: &pb::JournalEntry) -> Replayed {
        match self.replayed(machine, entry).await {
            Ok(outcome) => {
                if outcome != Replayed::Held {
                    self.settle_relay(entry);
                }
                outcome
            }
            Err(error) => {
                log::line(&format!("humatad2: warning: {}", status::chain(&error)));
                Replayed::Held
            }
        }
    }

    async fn replayed(
        &self,
        machine: &str,
        entry: &pb::JournalEntry,
    ) -> Result<Replayed, ReplayError> {
        let seq = entry.seq;
        let session_id = entry
            .principal
            .as_ref()
            .and_then(|principal| principal.session.as_ref())
            .map(|session| session.value.clone())
            .unwrap_or_default();
        // Held while deciding and saving, so an entry and an answer on the
        // host can't interleave.
        let mut state = self.state.lock().await;

        let (store, name, looking) = (
            Arc::clone(&self.store),
            machine.to_owned(),
            session_id.clone(),
        );
        let read = tokio::task::spawn_blocking(move || {
            let mark = store.machine_journal_mark(&name)?;
            let session = if valid_id(&looking) {
                store.session(&looking)?
            } else {
                None
            };
            Ok::<_, StoreError>((mark, session))
        })
        .await;
        let task = |source| ReplayError::Task {
            machine: machine.to_owned(),
            seq,
            source,
        };
        let store_failed = |source| ReplayError::Store {
            machine: machine.to_owned(),
            seq,
            source,
        };
        let (mark, session) = read.map_err(task)?.map_err(store_failed)?;
        if seq <= mark {
            return Ok(Replayed::Applied);
        }

        let decided = match pb::InboxRequest::decode(entry.request.as_slice()) {
            // `board.` entries go to the boards before they get here (D227); what is no
            // Inbox operation at all is left out, decodable or not.
            Ok(_) if !entry.operation.starts_with("inbox.") => Err(Rejection::UnknownOperation {
                operation: entry.operation.clone(),
            }),
            Ok(request) => {
                let arrival = Arrival {
                    machine,
                    seq,
                    operation: &entry.operation,
                    at: entry.at.clone(),
                    session: &session_id,
                    host: &self.machine,
                    relayed: request
                        .id
                        .as_ref()
                        .and_then(|id| self.relays.outstanding(&id.value)),
                    request,
                };
                let id = arrival
                    .request
                    .id
                    .as_ref()
                    .map(|id| id.value.as_str())
                    .unwrap_or_default();
                let existing = state.requests.iter().find(|held| held.id == id);
                let current = existing.map(|held| held.request.borrow().request.clone());
                let known = Known {
                    session: session.as_ref(),
                    existing: existing
                        .zip(current.as_ref())
                        .map(|(held, current)| (held.session.as_str(), current)),
                };
                decide(&arrival, &known)
            }
            Err(source) => Err(Rejection::Undecodable { source }),
        };
        let next = match decided {
            Ok(next) => next,
            Err(rejection) => {
                log::line(&format!(
                    "humatad2: warning: {machine}'s journal entry {seq} ({}) was left out: {}",
                    entry.operation,
                    status::chain(&rejection)
                ));
                self.save(machine, seq, None).await?;
                return Ok(Replayed::Rejected);
            }
        };
        let Some(next) = next else {
            self.save(machine, seq, None).await?;
            return Ok(Replayed::Applied);
        };

        let id = next
            .id
            .as_ref()
            .map(|id| id.value.clone())
            .unwrap_or_default();
        // "Always allow", answered on the machine or relayed from here,
        // becomes this Mac's own grant (D226).
        let made = match (entry.operation.as_str(), always_target(&next)) {
            (JOURNAL_ANSWER, Some(target)) => self
                .host_grant(machine, &session_id, &next, &target)
                .map(|made| (target, made)),
            _ => None,
        };
        let (asked, grant) = match made {
            Some((target, made)) => (Some(target), Some(made)),
            None => (None, None),
        };
        let grant_number = grant
            .as_ref()
            .and_then(|(grant, _)| self.guard.grant_number(grant));
        let position = state.requests.iter().position(|held| held.id == id);
        let mut held = match position {
            Some(at) => {
                let mut held = state.requests[at].request.borrow().clone();
                held.request = next;
                held
            }
            None => Held::new(next),
        };
        if let Some((grant, granted)) = grant {
            held.grant = Some(grant);
            held.granted = Some(granted);
        }
        let (record, before) = match position {
            Some(at) => {
                let entry = &mut state.requests[at];
                let before = entry.asked.clone();
                if asked.is_some() {
                    entry.asked.clone_from(&asked);
                }
                (kept::record(entry, &held), before)
            }
            None => {
                let made = Entry::new(
                    id.clone(),
                    session_id.clone(),
                    None,
                    asked.clone(),
                    None,
                    held.clone(),
                );
                (kept::record(&made, &held), None)
            }
        };
        if let Err(error) = self.save(machine, seq, Some(record)).await {
            // What is in force is what a restart keeps.
            self.take_back(grant_number);
            if let Some(at) = position {
                state.requests[at].asked = before;
            }
            return Err(error);
        }
        let allow_30m = entry.operation == JOURNAL_ANSWER
            && matches!(
                held.request.answer.as_ref().and_then(|a| a.answer.as_ref()),
                Some(pb::request_answer::Answer::AllowFor30Min(_))
            );
        match position {
            Some(at) => {
                state.requests[at].request.send_replace(held);
                self.record_browser_answer(&state.requests[at], allow_30m, SystemTime::now());
                self.tell_answer(&state.requests[at]);
            }
            None => {
                let told = held.request.clone();
                let made = Entry::new(id, session_id.clone(), None, asked, None, held);
                self.record_browser_answer(&made, allow_30m, SystemTime::now());
                self.tell_grant(&made);
                state.add(made);
                self.tell(pb::Change::Created, &session_id, &told);
            }
        }
        Ok(Replayed::Applied)
    }

    /// The grant an "Always allow" answer makes on this Mac when it is
    /// replayed (D226): the card's target worked out again against the
    /// rules as they are now, and made the way the user's own "Always
    /// allow" here would. No grant, and a line in the log, when the rule is
    /// now Allow or something already covers it, when it is now Never, or
    /// when what the target names is gone: the answer is recorded all the
    /// same.
    fn host_grant(
        &self,
        machine: &str,
        session: &str,
        request: &pb::InboxRequest,
        target: &pb::GrantTarget,
    ) -> Option<(pb::Grant, stored::InboxGrant)> {
        let id = request
            .id
            .as_ref()
            .map(|id| id.value.as_str())
            .unwrap_or_default();
        let ahead = kept::ahead(
            self.guard.as_ref(),
            &self.store,
            &self.machine,
            session,
            request.capability,
            target.clone(),
        );
        let asked = match ahead {
            kept::Ahead::Ask(asked) => asked,
            kept::Ahead::Covered => {
                log::line(&format!(
                    "humatad2: {machine}'s \"Always allow\" for request {id} made no grant on this Mac: the rule allows it already, or a grant covers it"
                ));
                return None;
            }
            kept::Ahead::Refused => {
                log::line(&format!(
                    "humatad2: {machine}'s \"Always allow\" for request {id} made no grant on this Mac: the rule for it is now Never; the answer is recorded, and the agent asks again if it still needs this"
                ));
                return None;
            }
            kept::Ahead::Gone => {
                log::line(&format!(
                    "humatad2: {machine}'s \"Always allow\" for request {id} made no grant on this Mac: the session, or what the grant named, is gone; the answer is recorded"
                ));
                return None;
            }
        };
        // An ask-ahead asks whatever the rule says; Allow means nothing to grant.
        let rule = ports::with_policy(self.guard.as_ref(), |policy, _| {
            policy
                .channel_rules(&asked.channel)
                .and_then(|rules| rules.decision(asked.capability))
        });
        if rule == Some(humata2_access::Decision::Allow) {
            log::line(&format!(
                "humatad2: {machine}'s \"Always allow\" for request {id} made no grant on this Mac: the rule allows it already"
            ));
            return None;
        }
        let offered = asked.always_let.clone()?;
        let user = self.present_user(&Caller::User(UserVia::App)).ok()?;
        match self.approve(
            &user,
            id,
            &asked,
            &Answer::AlwaysLet(offered),
            SystemTime::now(),
        ) {
            Ok(made) => made,
            Err(refused) => {
                log::line(&format!(
                    "humatad2: {machine}'s \"Always allow\" for request {id} made no grant on this Mac: {}",
                    refused.message()
                ));
                None
            }
        }
    }

    /// Takes back the grant number `grant` just made, when its entry
    /// couldn't be saved.
    fn take_back(&self, grant: Option<u64>) {
        let Some(grant) = grant else {
            return;
        };
        let Ok(user) = self.present_user(&Caller::User(UserVia::App)) else {
            return;
        };
        // Just made, so it can't be missing; if it were, a restart has none.
        let _ = ports::with_policy(self.guard.as_ref(), |policy, _| {
            policy.revoke_grant(&user, GrantId::new(grant))
        });
    }

    /// Forgets how far `machine`'s journal was applied: the machine is
    /// forgotten, or enrolled again, and its journal starts over, so the old
    /// mark would skip its first entries (D226).
    pub async fn forget_journal(&self, machine: &str) {
        // Not while an entry of it is being applied.
        let _state = self.state.lock().await;
        let (store, name) = (Arc::clone(&self.store), machine.to_owned());
        match tokio::task::spawn_blocking(move || store.forget_machine_journal(&name)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => say_not_forgotten(machine, &status::chain(&error)),
            Err(error) => say_not_forgotten(machine, &error.to_string()),
        }
    }

    /// As [`Self::forget_journal`], from a thread that may block.
    pub fn forget_journal_now(&self, machine: &str) {
        if let Err(error) = self.store.forget_machine_journal(machine) {
            say_not_forgotten(machine, &status::chain(&error));
        }
    }

    /// An answer or withdrawal of request `entry` is applied: the answer
    /// the host sent for it, if any, has done its part.
    fn settle_relay(&self, entry: &pb::JournalEntry) {
        if !matches!(entry.operation.as_str(), JOURNAL_ANSWER | JOURNAL_WITHDRAWN) {
            return;
        }
        let id = pb::InboxRequest::decode(entry.request.as_slice())
            .ok()
            .and_then(|request| request.id);
        if let Some(id) = id {
            self.relays.settle(&id.value);
        }
    }

    /// Saves `record`, if any, and moves `machine`'s mark to `seq`, in one
    /// transaction.
    async fn save(
        &self,
        machine: &str,
        seq: u64,
        record: Option<humata2_store::InboxRecord>,
    ) -> Result<(), ReplayError> {
        let store: Arc<Store> = Arc::clone(&self.store);
        let name = machine.to_owned();
        let saved = tokio::task::spawn_blocking(move || {
            store.apply_machine_journal(&name, seq, record.as_ref())
        })
        .await
        .map_err(|source| ReplayError::Task {
            machine: machine.to_owned(),
            seq,
            source,
        })?;
        saved.map_err(|source| ReplayError::Store {
            machine: machine.to_owned(),
            seq,
            source,
        })
    }
}
