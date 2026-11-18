//! A machine's journal of its Inbox (AWAY T5, D225). Every change to the
//! machine's Inbox is kept in the store's journal, whether or not the host
//! is reachable, so the host takes it in order when it links: a new request,
//! the answer to one, a request withdrawn. The host's ack ends a resend.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::SystemTime;

use humata2_grpc::humata::v1 as pb;
use humata2_store::{Store, StoreError};
use prost::Message;
use tokio::task::JoinError;

use super::HostLink;
use super::policy_copy;
use crate::grant_target;

pub(crate) const REQUEST: &str = "inbox.request";
pub(crate) const ANSWER: &str = "inbox.answer";
pub(crate) const WITHDRAWN: &str = "inbox.withdrawn";
/// A view the session's agent marked ready, kept whole for the host (VW-12).
pub(crate) const VIEW_OFFERED: &str = "view.offered";

/// Why the journal couldn't be written or read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum JournalError {
    #[error(
        "couldn't read or write this machine's journal in its store; check that the disk has room and the state folder is writable. Entries already kept are not lost"
    )]
    Store {
        #[source]
        source: StoreError,
    },

    #[error("a background task of the journal stopped; restart humatad2 on this machine if it keeps happening")]
    Task {
        #[source]
        source: JoinError,
    },
}

/// What a change from `before` to `request` is called in the journal, or
/// none when it isn't a change the host needs: a request kept again in the
/// same state (a grant marked revoked) is only bookkeeping.
fn operation_of(before: pb::RequestState, request: &pb::InboxRequest) -> Option<&'static str> {
    match request.state() {
        pb::RequestState::Pending => Some(REQUEST),
        state if state == before => None,
        pb::RequestState::Withdrawn => Some(WITHDRAWN),
        _ => Some(ANSWER),
    }
}

/// Keeps `request`, `session`'s, as it stands after a change from state
/// `before`, in the journal, if this daemon is a machine's and the change is
/// one the host needs. A failure is logged and never fails the caller: the
/// request itself is kept by the Inbox.
pub(crate) async fn record(
    store: &Arc<Store>,
    link: &HostLink,
    session: &str,
    before: pb::RequestState,
    request: &pb::InboxRequest,
) {
    if link.on_host() {
        return;
    }
    let Some(operation) = operation_of(before, request) else {
        return;
    };
    let store = Arc::clone(store);
    let session = session.to_owned();
    let request = request.clone();
    let appended = tokio::task::spawn_blocking(move || append(&store, &session, operation, &request))
        .await
        .map_err(|source| JournalError::Task { source })
        .and_then(|appended| appended);
    match appended {
        Ok(()) => link.journal_written(),
        Err(error) => crate::log::say!(
            "humatad2: warning: this change to the Inbox wasn't added to the journal for the host: {}",
            crate::status::chain(&error)
        ),
    }
}

/// As [`record`], for the Inbox's start-up, which isn't async.
pub(crate) fn record_now(
    store: &Store,
    link: &HostLink,
    session: &str,
    before: pb::RequestState,
    request: &pb::InboxRequest,
) {
    if link.on_host() {
        return;
    }
    let Some(operation) = operation_of(before, request) else {
        return;
    };
    match append(store, session, operation, request) {
        Ok(()) => link.journal_written(),
        Err(error) => crate::log::say!(
            "humatad2: warning: this change to the Inbox wasn't added to the journal for the host: {}",
            crate::status::chain(&error)
        ),
    }
}

fn append(store: &Store, session: &str, operation: &str, request: &pb::InboxRequest) -> Result<(), JournalError> {
    append_entry(store, session, operation, request.encode_to_vec(), None, None).map(drop)
}

/// Adds one entry made by `session`, with `request` as it was received, and
/// returns its sequence once it is on disk.
fn append_entry(
    store: &Store,
    session: &str,
    operation: &str,
    request: Vec<u8>,
    base_revision: Option<u64>,
    approved_request: Option<String>,
) -> Result<u64, JournalError> {
    let store_error = |source| JournalError::Store { source };
    let revision = policy_copy::held_revision(store).map_err(store_error)?;
    let entry = pb::JournalEntry {
        at: Some(grant_target::timestamp(SystemTime::now())),
        principal: Some(pb::AgentPrincipal {
            session: Some(pb::SessionId {
                value: session.to_owned(),
            }),
            pid: 0,
        }),
        operation: operation.to_owned(),
        request,
        base_revision: base_revision.map(|value| pb::Revision { value }),
        policy_revision: revision,
        approved_request: approved_request.map(|value| pb::RequestId { value }),
        ..Default::default()
    };
    store
        .journal()
        .append(operation, &entry.encode_to_vec())
        .map_err(store_error)
}

/// Keeps a board write `session`'s agent made while the host is away, as the
/// request was received, and returns where it sits in the journal (D227).
/// Unlike [`record`], this fails when the entry can't be kept: the journal is
/// then the only copy of the write, so the call must not say it was taken.
/// `base_revision` is the revision the agent last saw, if the call says.
/// `approved_request` is the request the user approved on this machine for
/// the write, when its rule asked (D268).
pub(crate) async fn record_board(
    store: &Arc<Store>,
    link: &HostLink,
    session: &str,
    operation: &'static str,
    request: Vec<u8>,
    base_revision: Option<u64>,
    approved_request: Option<String>,
) -> Result<u64, JournalError> {
    let store = Arc::clone(store);
    let session = session.to_owned();
    let seq = tokio::task::spawn_blocking(move || {
        append_entry(&store, &session, operation, request, base_revision, approved_request)
    })
    .await
    .map_err(|source| JournalError::Task { source })??;
    link.journal_written();
    Ok(seq)
}

/// Keeps a view `session`'s agent just offered (Draft to Waiting) as it was
/// saved, and returns where it sits in the journal (VW-12). Like
/// [`record_board`], this fails when the entry can't be kept: the journal is
/// then the only way the view reaches the host, so the offer must not be
/// taken. Drafts and revisions are never journaled: they stay on the machine.
pub(crate) async fn record_view(
    store: &Arc<Store>,
    link: &HostLink,
    session: &str,
    view: &pb::PreparedView,
) -> Result<u64, JournalError> {
    let store = Arc::clone(store);
    let session = session.to_owned();
    let request = view.encode_to_vec();
    let seq = tokio::task::spawn_blocking(move || {
        append_entry(&store, &session, VIEW_OFFERED, request, None, None)
    })
    .await
    .map_err(|source| JournalError::Task { source })??;
    link.journal_written();
    Ok(seq)
}

/// The entries the host hasn't acked with a sequence after `after`, in
/// sequence order, with `seq` set. An entry that can't be read back is left
/// out with a warning, so it can't hold the others up.
pub(crate) fn entries_after(store: &Store, after: u64) -> Result<Vec<pb::JournalEntry>, JournalError> {
    let stored = store
        .journal()
        .entries()
        .map_err(|source| JournalError::Store { source })?;
    let mut entries = Vec::new();
    for kept in stored.into_iter().filter(|kept| kept.seq > after) {
        match pb::JournalEntry::decode(kept.payload.as_slice()) {
            Ok(mut entry) => {
                entry.seq = kept.seq;
                entries.push(entry);
            }
            Err(error) => crate::log::say!(
                "humatad2: warning: journal entry {} is damaged and isn't sent to the host: {error}",
                kept.seq
            ),
        }
    }
    Ok(entries)
}

/// The ids of the requests whose answers are in the journal, unacked, with a
/// sequence through `through`. A grant made on this machine for one of them
/// is kept only until the host acks it (D226).
pub(super) fn answered_through(store: &Store, through: u64) -> Result<BTreeSet<String>, JournalError> {
    Ok(entries_after(store, 0)?
        .into_iter()
        .filter(|entry| entry.seq <= through && entry.operation == ANSWER)
        .filter_map(|entry| pb::InboxRequest::decode(entry.request.as_slice()).ok()?.id)
        .map(|id| id.value)
        .collect())
}

/// Records that the host has every entry through `seq`.
pub(crate) fn acked(store: &Store, seq: u64) -> Result<(), JournalError> {
    if seq == 0 {
        return Ok(());
    }
    store
        .journal()
        .ack_through(seq)
        .map_err(|source| JournalError::Store { source })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(state: pb::RequestState) -> pb::InboxRequest {
        pb::InboxRequest {
            id: Some(pb::RequestId {
                value: "rq_1".to_owned(),
            }),
            state: state as i32,
            ..Default::default()
        }
    }

    fn store() -> (tempfile::TempDir, Arc<Store>) {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(directory.path()).unwrap());
        (directory, store)
    }

    #[tokio::test]
    async fn changes_are_journaled_in_order_and_acked_ones_are_not_sent_again() {
        let (_directory, store) = store();
        let link = HostLink::new(false);
        let asked = request(pb::RequestState::Pending);
        let approved = request(pb::RequestState::Approved);
        record(&store, &link, "se_a", pb::RequestState::Pending, &asked).await;
        // Marking the grant revoked keeps the same state: not a change.
        record(&store, &link, "se_a", pb::RequestState::Approved, &approved).await;
        record(&store, &link, "se_a", pb::RequestState::Pending, &approved).await;

        let entries = entries_after(&store, 0).unwrap();
        let kinds: Vec<&str> = entries.iter().map(|entry| entry.operation.as_str()).collect();
        assert_eq!(kinds, [REQUEST, ANSWER]);
        assert!(entries[0].seq < entries[1].seq);
        assert_eq!(
            entries[0].principal.as_ref().and_then(|p| p.session.as_ref()).map(|s| s.value.as_str()),
            Some("se_a")
        );

        let rest = entries_after(&store, entries[0].seq).unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].operation, ANSWER);

        acked(&store, entries[1].seq).unwrap();
        assert!(entries_after(&store, 0).unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_host_keeps_no_journal_of_its_own_inbox() {
        let (_directory, store) = store();
        let link = HostLink::new(true);
        record(&store, &link, "se_a", pb::RequestState::Pending, &request(pb::RequestState::Pending)).await;
        assert!(entries_after(&store, 0).unwrap().is_empty());
    }

    #[test]
    fn a_withdrawal_is_its_own_operation() {
        let withdrawn = request(pb::RequestState::Withdrawn);
        assert_eq!(operation_of(pb::RequestState::Pending, &withdrawn), Some(WITHDRAWN));
    }

    #[tokio::test]
    async fn a_board_write_is_kept_whole_with_the_revision_it_was_made_against() {
        let (_directory, store) = store();
        let link = HostLink::new(false);
        let changes = link.journal_writes();
        let seq = record_board(&store, &link, "se_a", "board.item_move", vec![1, 2, 3], Some(4), None)
            .await
            .unwrap();
        assert!(
            changes.has_changed().unwrap(),
            "an open uplink is told there is something to send"
        );
        record_board(&store, &link, "se_a", "board.item_create", vec![9], None, None).await.unwrap();

        let entries = entries_after(&store, 0).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].seq, seq);
        assert_eq!(entries[0].operation, "board.item_move");
        assert_eq!(entries[0].request, vec![1, 2, 3]);
        assert_eq!(entries[0].base_revision, Some(pb::Revision { value: 4 }));
        assert_eq!(
            entries[0]
                .principal
                .as_ref()
                .and_then(|p| p.session.as_ref())
                .map(|s| s.value.as_str()),
            Some("se_a")
        );
        assert_eq!(
            entries[1].base_revision, None,
            "a write that names no revision is the later one that wins"
        );
        assert_eq!(entries[0].approved_request, None, "nothing was approved for these");
    }

    #[tokio::test]
    async fn a_board_write_the_user_approved_names_the_request_it_ran_under() {
        let (_directory, store) = store();
        let link = HostLink::new(false);
        let approved = Some("rq_1".to_owned());
        record_board(&store, &link, "se_a", "board.item_move", vec![1], Some(4), approved)
            .await
            .unwrap();
        let entries = entries_after(&store, 0).unwrap();
        assert_eq!(
            entries[0].approved_request,
            Some(pb::RequestId {
                value: "rq_1".to_owned()
            })
        );
    }

    #[tokio::test]
    async fn an_offered_view_is_kept_whole_under_its_session_and_tells_the_uplink() {
        let (_directory, store) = store();
        let link = HostLink::new(false);
        let changes = link.journal_writes();
        let view = pb::PreparedView {
            id: Some(pb::ViewId {
                value: "vw_1".to_owned(),
            }),
            title: "Plan".to_owned(),
            state: pb::ViewState::Waiting as i32,
            ..Default::default()
        };
        let seq = record_view(&store, &link, "se_a", &view).await.unwrap();
        assert!(changes.has_changed().unwrap(), "an open uplink is told there is something to send");

        let entries = entries_after(&store, 0).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!((entries[0].seq, entries[0].operation.as_str()), (seq, VIEW_OFFERED));
        assert_eq!(pb::PreparedView::decode(entries[0].request.as_slice()).unwrap(), view);
        assert_eq!(
            entries[0]
                .principal
                .as_ref()
                .and_then(|p| p.session.as_ref())
                .map(|s| s.value.as_str()),
            Some("se_a")
        );
    }
}
