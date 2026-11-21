//! A machine's offered view on the host (VW-12, D226): the host takes each
//! `view.offered` entry of a machine's journal into its own views once, and
//! from then on owns the view's state. A re-sent entry, or an id the host
//! knows, never overwrites what the user answered.

use std::sync::Arc;

use humata2_grpc_core::Timestamp;
use crate::pb;
use humata2_store::{SessionRecord, Store, StoreError, ViewRecord};
use prost::Message;
use tokio::task::JoinError;

use super::Review;
use crate::ports::{Scope, chain};

/// Journal operation name for a view offered while the host was away.
pub const JOURNAL_VIEW_OFFERED: &str = "view.offered";

/// Outcome of replaying a journal entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replayed {
    Applied,
    Rejected,
    Held,
}

/// Why a session isn't placed on the machine that journaled the entry.
#[derive(Debug, thiserror::Error)]
enum Placement {
    #[error("there is no session `{session}` on this Mac")]
    Missing { session: String },
    #[error("session `{session}` is placed on `{actual}`, not `{expected}`")]
    WrongMachine {
        session: String,
        expected: String,
        actual: String,
    },
    #[error("couldn't read session `{session}` from this Mac's store")]
    Store {
        session: String,
        #[source]
        source: StoreError,
    },
}

fn placed_on(store: &Store, machine: &str, session: &str) -> Result<SessionRecord, Placement> {
    match store.session(session) {
        Ok(Some(record)) if record.machine == machine => Ok(record),
        Ok(Some(record)) => Err(Placement::WrongMachine {
            session: session.to_owned(),
            expected: machine.to_owned(),
            actual: record.machine,
        }),
        Ok(None) | Err(StoreError::InvalidId { .. }) => Err(Placement::Missing {
            session: session.to_owned(),
        }),
        Err(source) => Err(Placement::Store {
            session: session.to_owned(),
            source,
        }),
    }
}

/// Why the host leaves a journal entry out.
#[derive(Debug, thiserror::Error)]
enum Rejection {
    #[error(
        "its operation is `{operation}`, and the only view operation this Mac takes is `view.offered`; update humatad2 on the machine so it sends what this Mac understands"
    )]
    UnknownOperation { operation: String },

    #[error(
        "the view in it can't be read; the machine and this Mac may run different versions, so update humatad2 on both"
    )]
    Undecodable {
        #[source]
        source: prost::DecodeError,
    },

    #[error(
        "it holds `{id}`, which isn't an offered view (a view id starts with `vw_`, and only a view waiting for the user is offered); the agent's drafts stay on its machine"
    )]
    NotOffered { id: String },

    #[error("it isn't for a session this Mac placed on that machine, so it was left out")]
    Placement {
        #[source]
        source: Placement,
    },

    #[error(
        "it was made by session `{by}`, but the view says it belongs to `{session}`; a machine's agent offers views only of its own session"
    )]
    Principal { by: String, session: String },
}

/// Why the host's store couldn't take an entry.
#[derive(Debug, thiserror::Error)]
enum Failure {
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

/// What became of one entry.
enum Taken {
    /// Its seq is at or under the mark: taken before.
    Already,
    /// Left out; the mark moved past it.
    Left(Rejection),
    /// Taken: `inserted` is whether the host had no view with its id.
    Offered {
        inserted: bool,
        view: Box<pb::PreparedView>,
    },
}

impl Review {
    /// Takes `machine`'s journal `entry` of an offered view into this Mac's
    /// views, in the order entries come.
    pub async fn replay_offered(&self, machine: &str, entry: &pb::JournalEntry) -> Replayed {
        let seq = entry.seq;
        let (store, name, taken) = (
            Arc::clone(&self.inner.store),
            machine.to_owned(),
            entry.clone(),
        );
        let done = tokio::task::spawn_blocking(move || apply(&store, &name, &taken)).await;
        let taken = match done {
            Ok(Ok(taken)) => taken,
            Ok(Err(source)) => {
                let failure = Failure::Store {
                    machine: machine.to_owned(),
                    seq,
                    source,
                };
                humata2_rpc_kit::log::say!("humatad2: warning: {}", chain(&failure));
                return Replayed::Held;
            }
            Err(source) => {
                let failure = Failure::Task {
                    machine: machine.to_owned(),
                    seq,
                    source,
                };
                humata2_rpc_kit::log::say!("humatad2: warning: {}", chain(&failure));
                return Replayed::Held;
            }
        };
        match taken {
            Taken::Already | Taken::Offered { inserted: false, .. } => Replayed::Applied,
            Taken::Left(rejection) => {
                humata2_rpc_kit::log::say!(
                    "humatad2: warning: {machine}'s journal entry {seq} ({}) was left out: {}",
                    entry.operation,
                    chain(&rejection)
                );
                Replayed::Rejected
            }
            Taken::Offered { view, .. } => {
                let session = view
                    .session
                    .as_ref()
                    .map(|id| id.value.clone())
                    .unwrap_or_default();
                let by = view.prepared_by.clone().map(|agent| pb::Actor {
                    actor: Some(pb::actor::Actor::Agent(agent)),
                });
                self.inner.feed.send(
                    Scope::Session(session),
                    pb::Change::Updated,
                    pb::event::Payload::View(*view),
                    by,
                );
                Replayed::Applied
            }
        }
    }
}

/// Checks and saves `entry`, in the store's one transaction for it.
fn apply(store: &Store, machine: &str, entry: &pb::JournalEntry) -> Result<Taken, StoreError> {
    let seq = entry.seq;
    if seq <= store.machine_journal_mark(machine)? {
        return Ok(Taken::Already);
    }
    let (mut view, placed) = match checked(store, machine, entry)? {
        Ok(checked) => checked,
        Err(rejection) => {
            store.apply_machine_journal(machine, seq, None)?;
            return Ok(Taken::Left(rejection));
        }
    };
    view.journal = Some(pb::JournalPosition {
        machine: Some(pb::MachineId {
            value: machine.to_owned(),
        }),
        seq,
    });
    let at = millis(view.prepared_at.as_ref().or(entry.at.as_ref()));
    let record = ViewRecord {
        id: view
            .id
            .as_ref()
            .map(|id| id.value.clone())
            .unwrap_or_default(),
        session: placed.id.clone(),
        view: view.encode_to_vec(),
        keys: Vec::new(),
        prepared_unix_ms: at,
        updated_unix_ms: at,
    };
    let inserted = store.apply_machine_journal_view(machine, seq, &record)?;
    Ok(Taken::Offered {
        inserted,
        view: Box::new(view),
    })
}

fn checked(
    store: &Store,
    machine: &str,
    entry: &pb::JournalEntry,
) -> Result<Result<(pb::PreparedView, SessionRecord), Rejection>, StoreError> {
    if entry.operation != JOURNAL_VIEW_OFFERED {
        return Ok(Err(Rejection::UnknownOperation {
            operation: entry.operation.clone(),
        }));
    }
    let view = match pb::PreparedView::decode(entry.request.as_slice()) {
        Ok(view) => view,
        Err(source) => return Ok(Err(Rejection::Undecodable { source })),
    };
    let id = view
        .id
        .as_ref()
        .map(|id| id.value.as_str())
        .unwrap_or_default();
    let waiting = matches!(pb::ViewState::try_from(view.state), Ok(pb::ViewState::Waiting));
    if !(waiting && id.starts_with("vw_") && valid_id(id)) {
        return Ok(Err(Rejection::NotOffered { id: id.to_owned() }));
    }
    let session = view
        .session
        .as_ref()
        .map(|id| id.value.clone())
        .unwrap_or_default();
    let by = entry
        .principal
        .as_ref()
        .and_then(|principal| principal.session.as_ref())
        .map(|session| session.value.as_str())
        .unwrap_or_default();
    let prepared_by = view
        .prepared_by
        .as_ref()
        .and_then(|agent| agent.session.as_ref())
        .map(|session| session.value.as_str())
        .unwrap_or_default();
    if by != session || prepared_by != session {
        return Ok(Err(Rejection::Principal {
            by: by.to_owned(),
            session,
        }));
    }
    let placed = match placed_on(store, machine, &session) {
        Ok(placed) => placed,
        Err(Placement::Store { source, .. }) => return Err(source),
        Err(source) => return Ok(Err(Rejection::Placement { source })),
    };
    Ok(Ok((view, placed)))
}

fn valid_id(id: &str) -> bool {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    id.len() <= 128 && !id.starts_with('.') && id.chars().all(allowed)
}

fn millis(at: Option<&Timestamp>) -> i64 {
    at.map_or(0, |at| {
        at.seconds
            .saturating_mul(1000)
            .saturating_add(i64::from(at.nanos) / 1_000_000)
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const BOX: &str = "ma_1";

    fn store() -> (tempfile::TempDir, Store) {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        for (id, machine) in [("se_a", BOX), ("se_local", "")] {
            store
                .put_session(&SessionRecord {
                    id: id.to_owned(),
                    channel: "ch_1".to_owned(),
                    machine: machine.to_owned(),
                    title: "Fix the build".to_owned(),
                    agent: "claude".to_owned(),
                    folder: PathBuf::from("/work/web"),
                    created_unix_ms: 1,
                })
                .unwrap();
        }
        (directory, store)
    }

    fn session(id: &str) -> Option<pb::SessionId> {
        Some(pb::SessionId {
            value: id.to_owned(),
        })
    }

    fn view(id: &str, session_id: &str, state: pb::ViewState) -> pb::PreparedView {
        pb::PreparedView {
            id: Some(pb::ViewId {
                value: id.to_owned(),
            }),
            session: session(session_id),
            title: "Plan".to_owned(),
            prepared_by: Some(pb::AgentActor {
                session: session(session_id),
                ..Default::default()
            }),
            state: state as i32,
            revision: Some(pb::Revision { value: 2 }),
            ..Default::default()
        }
    }

    fn entry(seq: u64, by: &str, view: &pb::PreparedView) -> pb::JournalEntry {
        pb::JournalEntry {
            seq,
            principal: Some(pb::AgentPrincipal {
                session: session(by),
                pid: 0,
            }),
            operation: JOURNAL_VIEW_OFFERED.to_owned(),
            request: view.encode_to_vec(),
            ..Default::default()
        }
    }

    fn offered(taken: &Taken) -> Option<bool> {
        match taken {
            Taken::Offered { inserted, .. } => Some(*inserted),
            _ => None,
        }
    }

    #[test]
    fn an_offered_view_of_a_placed_session_is_kept_once_and_marks_the_journal() {
        let (_directory, store) = store();
        let waiting = view("vw_1", "se_a", pb::ViewState::Waiting);
        let taken = apply(&store, BOX, &entry(1, "se_a", &waiting)).unwrap();
        assert_eq!(offered(&taken), Some(true));
        let kept = store.view("vw_1").unwrap().unwrap();
        assert_eq!(kept.session, "se_a");
        let saved = pb::PreparedView::decode(kept.view.as_slice()).unwrap();
        assert_eq!(saved.state(), pb::ViewState::Waiting);
        assert_eq!(
            saved
                .journal
                .and_then(|position| position.machine)
                .map(|m| m.value),
            Some(BOX.to_owned())
        );
        assert_eq!(store.machine_journal_mark(BOX).unwrap(), 1);
    }

    #[test]
    fn the_same_entry_twice_and_a_known_id_leave_the_hosts_view_as_it_is() {
        let (_directory, store) = store();
        let waiting = view("vw_1", "se_a", pb::ViewState::Waiting);
        apply(&store, BOX, &entry(1, "se_a", &waiting)).unwrap();
        let mut kept = store.view("vw_1").unwrap().unwrap();
        let mut answered = pb::PreparedView::decode(kept.view.as_slice()).unwrap();
        answered.state = pb::ViewState::Kept as i32;
        kept.view = answered.encode_to_vec();
        store.put_view(&kept).unwrap();

        assert!(matches!(
            apply(&store, BOX, &entry(1, "se_a", &waiting)).unwrap(),
            Taken::Already
        ));
        let again = apply(&store, BOX, &entry(2, "se_a", &waiting)).unwrap();
        assert_eq!(offered(&again), Some(false));
        let now = store.view("vw_1").unwrap().unwrap();
        let state = pb::PreparedView::decode(now.view.as_slice()).unwrap().state();
        assert_eq!(state, pb::ViewState::Kept);
        assert_eq!(store.machine_journal_mark(BOX).unwrap(), 2);
    }

    #[test]
    fn what_is_not_an_offered_view_of_a_session_on_that_machine_is_left_out_and_acked() {
        let (_directory, store) = store();
        let cases = [
            ("a draft", entry(1, "se_a", &view("vw_1", "se_a", pb::ViewState::Draft))),
            ("an id that isn't a view's", entry(2, "se_a", &view("rq_1", "se_a", pb::ViewState::Waiting))),
            ("a session placed on this Mac", entry(3, "se_local", &view("vw_2", "se_local", pb::ViewState::Waiting))),
            ("a session this Mac doesn't know", entry(4, "se_gone", &view("vw_3", "se_gone", pb::ViewState::Waiting))),
            ("another session's principal", entry(5, "se_other", &view("vw_4", "se_a", pb::ViewState::Waiting))),
            (
                "another operation",
                pb::JournalEntry {
                    operation: "view.revised".to_owned(),
                    ..entry(6, "se_a", &view("vw_5", "se_a", pb::ViewState::Waiting))
                },
            ),
            (
                "bytes that aren't a view",
                pb::JournalEntry {
                    request: vec![0xff, 0xff, 0xff],
                    ..entry(7, "se_a", &view("vw_6", "se_a", pb::ViewState::Waiting))
                },
            ),
        ];
        for (what, entry) in cases {
            let taken = apply(&store, BOX, &entry).unwrap();
            assert!(matches!(taken, Taken::Left(_)), "{what} is left out");
        }
        assert!(store.views("se_a").unwrap().is_empty());
        assert!(store.views("se_local").unwrap().is_empty());
        assert_eq!(store.machine_journal_mark(BOX).unwrap(), 7);
    }
}
