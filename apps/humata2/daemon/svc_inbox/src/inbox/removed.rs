//! What a removed machine leaves in the host's Inbox (D783): the requests
//! its journal brought here wait on a machine that will never answer, and
//! the host knows machines by name (D61, D776), so a machine added later
//! under that name would seem to hold them. They are withdrawn as their
//! machine removed. The requests of its sessions are withdrawn with the
//! sessions (`session_service/forget.rs`); this catches the rest.

use crate::pb;
use tonic::Status;

use super::Inbox;

impl Inbox {
    /// Withdraws every pending request `machine`'s journal brought here, for
    /// `reason`. Requests made on this machine stay.
    pub async fn withdraw_pending_from(&self, machine: &str, reason: pb::WithdrawReason) -> Result<(), Status> {
        self.withdraw_pending_where(reason, |entry| journaled_by(&entry.request.borrow().request, machine))
            .await
    }
}

/// Whether `request` came here from `machine`'s journal.
fn journaled_by(request: &pb::InboxRequest, machine: &str) -> bool {
    request
        .journal
        .as_ref()
        .and_then(|position| position.machine.as_ref())
        .is_some_and(|from| from.value == machine)
}
