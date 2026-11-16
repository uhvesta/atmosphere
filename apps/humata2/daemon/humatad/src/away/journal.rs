//! A machine's journal of its Inbox (AWAY T5, D225). Every change to the
//! machine's Inbox is kept in the store's journal, whether or not the host
//! is reachable, so the host takes it in order when it links: a new request,
//! the answer to one, a request withdrawn. The host's ack ends a resend.

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
        request: request.encode_to_vec(),
        policy_revision: revision,
        ..Default::default()
    };
    store
        .journal()
        .append(operation, &entry.encode_to_vec())
        .map_err(store_error)?;
    Ok(())
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
}
