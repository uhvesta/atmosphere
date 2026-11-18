//! The Inbox's part of the Migrate items (UP-1, D281): the three things
//! `upgrade::items::Migrations` does to the Inbox, with the access to its
//! private state they need, and the store that keeps them.
//!
//! A Migrate item belongs to no session, so its store row has none, and
//! only one pending item may name each thing and machine. Each item is saved
//! before its change is told, pending or withdrawn. At start up the pending
//! ones come back into the Inbox and `kept_migrates` hands them to the
//! Migrations; a withdrawn one stays in the store and isn't loaded.

use std::sync::Arc;
use std::time::SystemTime;

use humata2_grpc::humata::v1 as pb;
use humata2_store::{MigrateRecord, Store};
use humata2_store_proto::humata::store::v1 as stored;
use prost::Message;
use tonic::Status;

use super::kept::{created_unix_ms, unix_ms};
use super::{Entry, Held, Inbox, next_revision, withdraw};
use crate::status;

impl Inbox {
    /// The pending Migrate items the store kept, as they were saved, for
    /// `upgrade::items::Migrations` to hold each (thing, machine) by. Reads
    /// the store, so call it before serving.
    pub(crate) fn kept_migrates(&self) -> Vec<pb::InboxRequest> {
        kept_requests(&self.store)
    }

    /// Puts the new Migrate item `request` in the Inbox, saved first, and
    /// tells the feed.
    pub(crate) async fn post_migrate(&self, request: pb::InboxRequest) -> Result<(), Status> {
        let entry = new_entry(request);
        let mut state = self.state.lock().await;
        let current = entry.request.borrow().request.clone();
        self.save_migrate(&current).await?;
        self.tell(pb::Change::Created, &entry.session, &current);
        state.add(entry);
        Ok(())
    }

    /// Applies `change` to the pending Migrate item `id`, bumps its
    /// revision, saves it and tells the feed. `false` when the Inbox holds no
    /// pending item with that id.
    pub(crate) async fn update_migrate(
        &self,
        id: &str,
        change: impl FnOnce(&mut pb::InboxRequest),
    ) -> Result<bool, Status> {
        let state = self.state.lock().await;
        let Some(entry) = pending_migrate(&state.requests, id) else {
            return Ok(false);
        };
        let mut next = entry.request.borrow().clone();
        change(&mut next.request);
        next.request.revision = Some(next_revision(&next.request));
        let current = next.request.clone();
        self.save_migrate(&current).await?;
        entry.request.send_replace(next);
        self.tell(pb::Change::Updated, &entry.session, &current);
        Ok(true)
    }

    /// Withdraws the pending Migrate item `id` for `reason`, saves it and
    /// tells the feed. `false` when the Inbox holds no pending item with that
    /// id.
    pub(crate) async fn withdraw_migrate(
        &self,
        id: &str,
        reason: pb::WithdrawReason,
    ) -> Result<bool, Status> {
        let state = self.state.lock().await;
        let Some(entry) = pending_migrate(&state.requests, id) else {
            return Ok(false);
        };
        let mut next = entry.request.borrow().clone();
        withdraw(&mut next.request, reason);
        let current = next.request.clone();
        self.save_migrate(&current).await?;
        entry.request.send_replace(next);
        self.tell(pb::Change::Updated, &entry.session, &current);
        Ok(true)
    }

    /// Saves Migrate item `request` as it stands, pending or withdrawn.
    pub(super) async fn save_migrate(&self, request: &pb::InboxRequest) -> Result<(), Status> {
        let id = request.id.as_ref().map_or("", |id| id.value.as_str());
        let Some(record) = record_of(request) else {
            return Err(status::internal(
                format!("the Migrate item {id} names no machine or thing, so it can't be saved"),
                "",
            ));
        };
        let store = Arc::clone(&self.store);
        let saved = tokio::task::spawn_blocking(move || store.put_migrate_request(&record))
            .await
            .map_err(|error| status::internal(format!("a background task failed: {error}"), ""))?;
        saved.map_err(|error| status::store(error, &self.machine, "save a Migrate item"))
    }

    /// Request `id` as the Inbox holds it.
    #[cfg(test)]
    pub(crate) async fn held_request(&self, id: &str) -> Option<pb::InboxRequest> {
        let state = self.state.lock().await;
        state
            .requests
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.request.borrow().request.clone())
    }
}

/// The entry a Migrate item is held as. No session asked for it, so its
/// session is empty, which no session's id is.
pub(super) fn new_entry(request: pb::InboxRequest) -> Entry {
    let id = request
        .id
        .as_ref()
        .map(|id| id.value.clone())
        .unwrap_or_default();
    Entry::new(id, String::new(), None, None, None, Held::new(request))
}

/// The pending Migrate items `store` keeps, each as it was saved. One the
/// store can't read is left out and said in the log; the rest still load.
pub(super) fn kept_requests(store: &Store) -> Vec<pb::InboxRequest> {
    let records = match store.pending_migrate_requests() {
        Ok(records) => records,
        Err(error) => {
            crate::log::say!(
                "humatad2: warning: couldn't read the Migrate items from the store, so none is held until the next link-up finds the mismatch again: {}",
                crate::status::chain(&error)
            );
            return Vec::new();
        }
    };
    records
        .into_iter()
        .filter_map(|record| {
            let request = stored::InboxEntry::decode(record.entry.as_slice())
                .ok()
                .and_then(|body| pb::InboxRequest::decode(body.request.as_slice()).ok());
            if request.is_none() {
                crate::log::say!(
                    "humatad2: warning: Migrate item {} in the store is unreadable, so it's left out; the next link-up finds the mismatch again",
                    record.id
                );
            }
            request
        })
        .collect()
}

/// The store's record of Migrate item `request`, from its card and its id.
/// `None` when the card names no machine or the request no id.
fn record_of(request: &pb::InboxRequest) -> Option<MigrateRecord> {
    let Some(pb::request_preview::Preview::Migrate(card)) = request
        .preview
        .as_ref()
        .and_then(|preview| preview.preview.as_ref())
    else {
        return None;
    };
    Some(MigrateRecord {
        id: request.id.as_ref()?.value.clone(),
        thing: card.thing,
        machine: card.machine.as_ref()?.value.clone(),
        pending: request.state() == pb::RequestState::Pending,
        entry: stored::InboxEntry {
            request: request.encode_to_vec(),
            ..Default::default()
        }
        .encode_to_vec(),
        created_unix_ms: created_unix_ms(request),
        updated_unix_ms: unix_ms(SystemTime::now()),
    })
}

fn pending_migrate<'a>(requests: &'a [Entry], id: &str) -> Option<&'a Entry> {
    requests.iter().find(|entry| {
        let held = entry.request.borrow();
        entry.id == id
            && held.request.kind() == pb::RequestKind::Migrate
            && held.request.state() == pb::RequestState::Pending
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::events::Feed;
    use crate::guard::Guard;

    const HOST: &str = "ma_host";

    /// The Inbox over the store in `directory`, as a start up makes it.
    fn inbox(directory: &Path) -> Inbox {
        let store = Arc::new(Store::open(directory).unwrap());
        let guard = Arc::new(Guard::new(HOST, pb::DaemonRole::Host));
        Inbox::new(guard, store, HOST.to_owned(), Arc::new(Feed::default()))
    }

    /// A pending Migrate item for `thing` on `machine`, as the Migrations post it.
    fn item(id: &str, thing: pb::MigrateThing, machine: &str) -> pb::InboxRequest {
        pb::InboxRequest {
            id: Some(pb::RequestId {
                value: id.to_owned(),
            }),
            kind: pb::RequestKind::Migrate as i32,
            state: pb::RequestState::Pending as i32,
            vip: true,
            revision: Some(pb::Revision { value: 1 }),
            preview: Some(pb::RequestPreview {
                preview: Some(pb::request_preview::Preview::Migrate(pb::MigratePreview {
                    thing: thing as i32,
                    machine: Some(pb::MachineId {
                        value: machine.to_owned(),
                    }),
                    ..Default::default()
                })),
            }),
            ..Default::default()
        }
    }

    fn ids(requests: &[pb::InboxRequest]) -> Vec<String> {
        requests
            .iter()
            .filter_map(|request| request.id.as_ref().map(|id| id.value.clone()))
            .collect()
    }

    #[tokio::test]
    async fn a_pending_item_is_kept_across_a_restart_and_still_changes() {
        let directory = tempfile::tempdir().unwrap();
        let first = inbox(directory.path());
        let posted = item("rq_m1", pb::MigrateThing::Daemon, "gpu");
        first.post_migrate(posted.clone()).await.unwrap();
        drop(first);

        let restarted = inbox(directory.path());
        assert_eq!(restarted.kept_migrates(), vec![posted]);
        assert!(
            restarted.held_request("rq_m1").await.is_some(),
            "the restarted Inbox doesn't hold the pending item"
        );
        assert!(
            restarted
                .update_migrate("rq_m1", |request| request.reason = "changed".to_owned())
                .await
                .unwrap()
        );
        drop(restarted);

        let again = inbox(directory.path());
        let kept = again.kept_migrates();
        assert_eq!(ids(&kept), vec!["rq_m1"]);
        assert_eq!(kept[0].reason, "changed");
    }

    #[tokio::test]
    async fn a_withdrawn_item_is_not_kept_and_blocks_no_new_one() {
        let directory = tempfile::tempdir().unwrap();
        let first = inbox(directory.path());
        first
            .post_migrate(item("rq_m1", pb::MigrateThing::Daemon, "gpu"))
            .await
            .unwrap();
        assert!(
            first
                .withdraw_migrate("rq_m1", pb::WithdrawReason::Resolved)
                .await
                .unwrap()
        );
        drop(first);

        let restarted = inbox(directory.path());
        assert!(restarted.kept_migrates().is_empty());
        restarted
            .post_migrate(item("rq_m2", pb::MigrateThing::Daemon, "gpu"))
            .await
            .unwrap();
        assert_eq!(ids(&restarted.kept_migrates()), vec!["rq_m2"]);
    }

    #[tokio::test]
    async fn a_second_pending_item_for_the_same_thing_and_machine_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        inbox
            .post_migrate(item("rq_m1", pb::MigrateThing::Daemon, "gpu"))
            .await
            .unwrap();

        let refused = inbox
            .post_migrate(item("rq_m2", pb::MigrateThing::Daemon, "gpu"))
            .await;
        assert!(
            refused.is_err(),
            "a second pending item for the same thing and machine was posted"
        );
        assert_eq!(ids(&inbox.kept_migrates()), vec!["rq_m1"]);
    }
}
