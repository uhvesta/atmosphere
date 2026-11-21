//! The Inbox's part of carrying a Migrate out (UP-1, D281): handing a click
//! on a Migrate item to `upgrade::items`, and finishing the item as
//! Approved once the machine is at the new release.

use std::sync::Arc;
use std::time::SystemTime;

use crate::pb;
use humata2_rpc_kit::helpers as status;
use tonic::Status;

use super::{Decided, Entry, Inbox, settle};
use crate::ports::MigrateApproverPort;

impl Inbox {
    /// Sets who carries out a click on a Migrate item. Done once, when the
    /// host's Migrate items are made.
    pub fn migrating(&self, approver: Arc<dyn MigrateApproverPort>) {
        // A second one would be the same items again: the first stays.
        let _ = self.migrator.set(approver);
    }

    /// The user clicked Migrate on item `id`: the item as it stands once it
    /// is migrating (or failed). Called with the Inbox unlocked.
    pub(super) async fn migrate_approved(&self, id: &str) -> Result<pb::InboxRequest, Status> {
        match self.migrator.get() {
            Some(approver) => approver.approve(id).await,
            None => Err(status::not_built(
                "humata.v1.InboxService.AnswerRequest on a Migrate item",
            )),
        }
    }

    /// Finishes the pending Migrate item `id` as Approved, its reason now
    /// `done`, and tells the feed. `false` when the Inbox holds no pending
    /// item with that id.
    pub async fn settle_migrate(&self, id: &str, done: String) -> Result<bool, Status> {
        let state = self.state.lock().await;
        let Some(entry) = state.requests.iter().find(|entry| pending(entry, id)) else {
            return Ok(false);
        };
        let mut next = entry.request.borrow().clone();
        next.request.reason = done;
        // D379: every step of what ended is done.
        if let Some(approver) = self.migrator.get() {
            approver.finish_steps(&mut next.request);
        }
        let by = pb::Actor {
            actor: Some(pb::actor::Actor::System(pb::SystemActor {
                reason: pb::system_actor::Reason::BootReconcile as i32,
            })),
        };
        let decided = Decided {
            state: pb::RequestState::Approved,
            answer: pb::request_answer::Answer::Approve(pb::Approve {}),
            grant: None,
        };
        settle(&mut next, decided, by, SystemTime::now());
        // A Migrate item has no session, so it is saved as one (pick q),
        // not through the session rows `commit` writes.
        self.save_migrate(&next.request).await?;
        entry.request.send_replace(next);
        self.tell_answer(entry);
        Ok(true)
    }
}

fn pending(entry: &Entry, id: &str) -> bool {
    let held = entry.request.borrow();
    entry.id == id
        && held.request.kind() == pb::RequestKind::Migrate
        && held.request.state() == pb::RequestState::Pending
}
