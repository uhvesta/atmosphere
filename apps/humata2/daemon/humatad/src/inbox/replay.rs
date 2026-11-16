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

use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store, StoreError};
use prost::Message;
use tokio::task::JoinError;

use super::{Entry, Held, Inbox, kept, next_revision};
use crate::away::{JOURNAL_ANSWER, JOURNAL_REQUEST, JOURNAL_WITHDRAWN};

/// What became of one journal entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Replayed {
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
    #[error("`{operation}` isn't an Inbox operation the host takes; nothing to do, the machine drops it")]
    UnknownOperation { operation: String },

    #[error("its request can't be read; update Humata on the machine so both ends speak the same version")]
    Undecodable {
        #[source]
        source: prost::DecodeError,
    },

    #[error("its request id `{id}` isn't a valid id")]
    BadId { id: String },

    #[error("it names session `{session}`, which this Mac doesn't know; the session may have been deleted here")]
    SessionUnknown { session: String },

    #[error("it names session `{session}`, which this Mac places on {placed}, not on the machine that sent it")]
    SessionElsewhere { session: String, placed: String },

    #[error("the session that sent it isn't the one that made the request")]
    NotRequester,

    #[error("request `{id}` isn't this machine's: the Inbox already holds one with that id from another source")]
    Foreign { id: String },

    #[error("`{operation}` can't carry a request in state {state}")]
    StateDisagrees { operation: String, state: i32 },

    #[error("request `{id}` already stands as state {first}, so the later state {later} is left out; the first recorded stands")]
    Conflict { id: String, first: i32, later: i32 },
}

/// What one entry brings, with what the host already holds.
struct Arrival<'a> {
    machine: &'a str,
    seq: u64,
    operation: &'a str,
    at: Option<humata2_grpc::Timestamp>,
    /// The session the entry says made the call.
    session: &'a str,
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
    let id = request.id.as_ref().map(|id| id.value.as_str()).unwrap_or_default();
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
    request
}

/// The answer's credit: when the machine says it was made, by the user at
/// that machine's terminal (AW-6).
fn credited(arrival: &Arrival<'_>) -> pb::Timestamped {
    let at = arrival
        .request
        .answered
        .as_ref()
        .and_then(|answered| answered.at.clone())
        .or_else(|| arrival.at.clone());
    pb::Timestamped {
        at,
        by: Some(pb::Actor {
            actor: Some(pb::actor::Actor::User(pb::UserActor {
                via: pb::user_actor::Via::CliOnMachine as i32,
                machine: Some(pb::MachineId {
                    value: arrival.machine.to_owned(),
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

impl Inbox {
    /// How far `machine`'s journal is applied, for the host's Resume. A
    /// mark that can't be read is 0: the machine sends everything again,
    /// and applying an entry twice changes nothing.
    pub(crate) async fn replayed_through(&self, machine: &str) -> u64 {
        let store = Arc::clone(&self.store);
        let name = machine.to_owned();
        let read = tokio::task::spawn_blocking(move || store.machine_journal_mark(&name)).await;
        match read {
            Ok(Ok(mark)) => mark,
            Ok(Err(error)) => {
                crate::log::say!(
                    "humatad2: warning: couldn't read how far {machine}'s journal is applied, so it is sent again from the start: {}",
                    crate::status::chain(&error)
                );
                0
            }
            Err(error) => {
                crate::log::say!(
                    "humatad2: warning: reading how far {machine}'s journal is applied stopped, so it is sent again from the start: {error}"
                );
                0
            }
        }
    }

    /// Applies `machine`'s journal `entry`. Entries are applied in the order
    /// they come; one at or below the mark was applied before and changes
    /// nothing.
    pub(crate) async fn replay(&self, machine: &str, entry: &pb::JournalEntry) -> Replayed {
        match self.replayed(machine, entry).await {
            Ok(outcome) => outcome,
            Err(error) => {
                crate::log::say!("humatad2: warning: {}", crate::status::chain(&error));
                Replayed::Held
            }
        }
    }

    async fn replayed(&self, machine: &str, entry: &pb::JournalEntry) -> Result<Replayed, ReplayError> {
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

        let (store, name, looking) = (Arc::clone(&self.store), machine.to_owned(), session_id.clone());
        let read = tokio::task::spawn_blocking(move || {
            let mark = store.machine_journal_mark(&name)?;
            let session = if valid_id(&looking) { store.session(&looking)? } else { None };
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
            Ok(request) => {
                let arrival = Arrival {
                    machine,
                    seq,
                    operation: &entry.operation,
                    at: entry.at.clone(),
                    session: &session_id,
                    request,
                };
                let id = arrival.request.id.as_ref().map(|id| id.value.as_str()).unwrap_or_default();
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
                crate::log::say!(
                    "humatad2: warning: {machine}'s journal entry {seq} ({}) was left out: {}",
                    entry.operation,
                    crate::status::chain(&rejection)
                );
                self.save(machine, seq, None).await?;
                return Ok(Replayed::Rejected);
            }
        };
        let Some(next) = next else {
            self.save(machine, seq, None).await?;
            return Ok(Replayed::Applied);
        };

        let id = next.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        match state.requests.iter().position(|held| held.id == id) {
            Some(at) => {
                let mut held = state.requests[at].request.borrow().clone();
                held.request = next;
                let record = kept::record(&state.requests[at], &held);
                self.save(machine, seq, Some(record)).await?;
                let told = held.request.clone();
                let owner = state.requests[at].session.clone();
                state.requests[at].request.send_replace(held);
                self.tell(pb::Change::Updated, &owner, &told);
            }
            None => {
                let held = Held::new(next);
                let made = Entry::new(id, session_id.clone(), None, None, None, held.clone());
                let record = kept::record(&made, &held);
                self.save(machine, seq, Some(record)).await?;
                state.add(made);
                self.tell(pb::Change::Created, &session_id, &held.request);
            }
        }
        Ok(Replayed::Applied)
    }

    /// Saves `record`, if any, and moves `machine`'s mark to `seq`, in one
    /// transaction.
    async fn save(&self, machine: &str, seq: u64, record: Option<humata2_store::InboxRecord>) -> Result<(), ReplayError> {
        let store: Arc<Store> = Arc::clone(&self.store);
        let name = machine.to_owned();
        let saved = tokio::task::spawn_blocking(move || store.apply_machine_journal(&name, seq, record.as_ref()))
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use humata2_store::SessionRecord;

    use super::*;
    use crate::events::Feed;
    use crate::guard::Guard;

    const HOST: &str = "ma_host";
    const BOX: &str = "ma_box";

    fn session(id: &str, machine: &str) -> SessionRecord {
        SessionRecord {
            id: id.to_owned(),
            channel: "ch_a".to_owned(),
            machine: machine.to_owned(),
            title: "agent".to_owned(),
            agent: "claude".to_owned(),
            folder: PathBuf::new(),
            created_unix_ms: 0,
        }
    }

    fn card(id: &str, session: &str, state: pb::RequestState) -> pb::InboxRequest {
        pb::InboxRequest {
            id: Some(pb::RequestId { value: id.to_owned() }),
            state: state as i32,
            requester: Some(pb::AgentActor {
                session: Some(pb::SessionId {
                    value: session.to_owned(),
                }),
                ..Default::default()
            }),
            action_title: "Read files".to_owned(),
            ..Default::default()
        }
    }

    fn entry(seq: u64, operation: &str, session: &str, request: &pb::InboxRequest) -> pb::JournalEntry {
        pb::JournalEntry {
            seq,
            principal: Some(pb::AgentPrincipal {
                session: Some(pb::SessionId {
                    value: session.to_owned(),
                }),
                pid: 0,
            }),
            operation: operation.to_owned(),
            request: request.encode_to_vec(),
            ..Default::default()
        }
    }

    fn open(directory: &std::path::Path) -> (Arc<Store>, Inbox) {
        let store = Arc::new(Store::open(directory).unwrap());
        store.put_session(&session("se_a", BOX)).unwrap();
        store.put_session(&session("se_far", "ma_other")).unwrap();
        let guard = Arc::new(Guard::new(HOST, pb::DaemonRole::Host));
        let inbox = Inbox::new(guard, Arc::clone(&store), HOST.to_owned(), Arc::new(Feed::default()));
        (store, inbox)
    }

    async fn mirror(inbox: &Inbox, id: &str) -> Option<pb::InboxRequest> {
        let state = inbox.state.lock().await;
        state
            .requests
            .iter()
            .find(|held| held.id == id)
            .map(|held| held.request.borrow().request.clone())
    }

    fn approved(id: &str) -> pb::InboxRequest {
        pb::InboxRequest {
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
            }),
            ..card(id, "se_a", pb::RequestState::Approved)
        }
    }

    #[tokio::test]
    async fn a_request_then_its_answer_make_a_settled_mirror_credited_to_the_user_on_the_machine() {
        let directory = tempfile::tempdir().unwrap();
        let (store, inbox) = open(directory.path());
        let asked = card("rq_1", "se_a", pb::RequestState::Pending);
        assert_eq!(inbox.replay(BOX, &entry(1, JOURNAL_REQUEST, "se_a", &asked)).await, Replayed::Applied);
        let shown = mirror(&inbox, "rq_1").await.unwrap();
        assert_eq!(shown.state(), pb::RequestState::Pending);
        assert_eq!(shown.journal.as_ref().map(|journal| journal.seq), Some(1));
        assert_eq!(shown.channel.as_ref().map(|channel| channel.value.as_str()), Some("ch_a"));

        let outcome = inbox.replay(BOX, &entry(2, JOURNAL_ANSWER, "se_a", &approved("rq_1"))).await;
        assert_eq!(outcome, Replayed::Applied);
        let settled = mirror(&inbox, "rq_1").await.unwrap();
        assert_eq!(settled.state(), pb::RequestState::Approved);
        let by = settled.answered.and_then(|answered| answered.by).and_then(|actor| actor.actor);
        let Some(pb::actor::Actor::User(user)) = by else {
            panic!("the answer should be credited to the user");
        };
        assert_eq!(user.via(), pb::user_actor::Via::CliOnMachine);
        assert_eq!(user.machine.map(|machine| machine.value), Some(BOX.to_owned()));
        assert_eq!(store.machine_journal_mark(BOX).unwrap(), 2);
    }

    #[tokio::test]
    async fn replaying_the_same_entries_after_reopening_changes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let entries = [
            entry(1, JOURNAL_REQUEST, "se_a", &card("rq_1", "se_a", pb::RequestState::Pending)),
            entry(2, JOURNAL_ANSWER, "se_a", &approved("rq_1")),
        ];
        let before = {
            let (_store, inbox) = open(directory.path());
            for each in &entries {
                inbox.replay(BOX, each).await;
            }
            mirror(&inbox, "rq_1").await.unwrap()
        };
        let (store, inbox) = open(directory.path());
        for each in &entries {
            assert_eq!(inbox.replay(BOX, each).await, Replayed::Applied);
        }
        assert_eq!(mirror(&inbox, "rq_1").await, Some(before));
        assert_eq!(store.machine_journal_mark(BOX).unwrap(), 2);
    }

    #[tokio::test]
    async fn a_second_different_answer_is_rejected_but_advances_the_mark() {
        let directory = tempfile::tempdir().unwrap();
        let (store, inbox) = open(directory.path());
        inbox
            .replay(BOX, &entry(1, JOURNAL_REQUEST, "se_a", &card("rq_1", "se_a", pb::RequestState::Pending)))
            .await;
        inbox.replay(BOX, &entry(2, JOURNAL_ANSWER, "se_a", &approved("rq_1"))).await;
        let denied = card("rq_1", "se_a", pb::RequestState::Denied);
        assert_eq!(inbox.replay(BOX, &entry(3, JOURNAL_ANSWER, "se_a", &denied)).await, Replayed::Rejected);
        assert_eq!(mirror(&inbox, "rq_1").await.unwrap().state(), pb::RequestState::Approved);
        assert_eq!(store.machine_journal_mark(BOX).unwrap(), 3);
    }

    #[tokio::test]
    async fn the_same_answer_again_is_taken_without_change() {
        let directory = tempfile::tempdir().unwrap();
        let (_store, inbox) = open(directory.path());
        inbox.replay(BOX, &entry(1, JOURNAL_ANSWER, "se_a", &approved("rq_1"))).await;
        let before = mirror(&inbox, "rq_1").await;
        assert_eq!(before.as_ref().map(|request| request.state()), Some(pb::RequestState::Approved));
        assert_eq!(inbox.replay(BOX, &entry(2, JOURNAL_ANSWER, "se_a", &approved("rq_1"))).await, Replayed::Applied);
        assert_eq!(mirror(&inbox, "rq_1").await, before);
    }

    #[tokio::test]
    async fn an_entry_whose_session_is_on_another_machine_or_unknown_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let (store, inbox) = open(directory.path());
        let far = card("rq_far", "se_far", pb::RequestState::Pending);
        assert_eq!(inbox.replay(BOX, &entry(1, JOURNAL_REQUEST, "se_far", &far)).await, Replayed::Rejected);
        let unknown = card("rq_gone", "se_gone", pb::RequestState::Pending);
        assert_eq!(inbox.replay(BOX, &entry(2, JOURNAL_REQUEST, "se_gone", &unknown)).await, Replayed::Rejected);
        assert!(mirror(&inbox, "rq_far").await.is_none());
        assert!(mirror(&inbox, "rq_gone").await.is_none());
        assert_eq!(store.machine_journal_mark(BOX).unwrap(), 2);
    }

    #[tokio::test]
    async fn a_session_that_did_not_make_the_request_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let (_store, inbox) = open(directory.path());
        let asked = card("rq_1", "se_far", pb::RequestState::Pending);
        assert_eq!(inbox.replay(BOX, &entry(1, JOURNAL_REQUEST, "se_a", &asked)).await, Replayed::Rejected);
        assert!(mirror(&inbox, "rq_1").await.is_none());
    }

    #[tokio::test]
    async fn a_withdrawal_withdraws_a_pending_mirror_and_not_an_answered_one() {
        let directory = tempfile::tempdir().unwrap();
        let (_store, inbox) = open(directory.path());
        let withdrawn = |id: &str| pb::InboxRequest {
            withdraw_reason: pb::WithdrawReason::CallEnded as i32,
            ..card(id, "se_a", pb::RequestState::Withdrawn)
        };
        inbox
            .replay(BOX, &entry(1, JOURNAL_REQUEST, "se_a", &card("rq_1", "se_a", pb::RequestState::Pending)))
            .await;
        assert_eq!(inbox.replay(BOX, &entry(2, JOURNAL_WITHDRAWN, "se_a", &withdrawn("rq_1"))).await, Replayed::Applied);
        let shown = mirror(&inbox, "rq_1").await.unwrap();
        assert_eq!(shown.state(), pb::RequestState::Withdrawn);
        assert_eq!(shown.withdraw_reason(), pb::WithdrawReason::CallEnded);

        inbox.replay(BOX, &entry(3, JOURNAL_ANSWER, "se_a", &approved("rq_2"))).await;
        assert_eq!(inbox.replay(BOX, &entry(4, JOURNAL_WITHDRAWN, "se_a", &withdrawn("rq_2"))).await, Replayed::Rejected);
        assert_eq!(mirror(&inbox, "rq_2").await.unwrap().state(), pb::RequestState::Approved);
    }

    #[tokio::test]
    async fn an_unknown_operation_is_rejected_and_the_mark_moves_past_it() {
        let directory = tempfile::tempdir().unwrap();
        let (store, inbox) = open(directory.path());
        let odd = entry(1, "board.item_update", "se_a", &card("rq_1", "se_a", pb::RequestState::Pending));
        assert_eq!(inbox.replay(BOX, &odd).await, Replayed::Rejected);
        assert_eq!(store.machine_journal_mark(BOX).unwrap(), 1);
        assert!(mirror(&inbox, "rq_1").await.is_none());
    }

    #[tokio::test]
    async fn a_request_that_cannot_be_read_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let (store, inbox) = open(directory.path());
        let mut broken = entry(1, JOURNAL_REQUEST, "se_a", &card("rq_1", "se_a", pb::RequestState::Pending));
        broken.request = vec![0xff, 0xff, 0xff];
        assert_eq!(inbox.replay(BOX, &broken).await, Replayed::Rejected);
        assert_eq!(store.machine_journal_mark(BOX).unwrap(), 1);
    }
}
