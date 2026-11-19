//! Requests the host raises on its own account, about no session (D348): a
//! machine that can't carry out a change by itself asks the person here.
//! Such a request belongs to no session, is VIP, and takes Approve or Deny
//! only. Approving it runs the call it holds, in a task of its own; denying
//! it drops the call. Only one request is pending for each key, and a second
//! post for the key replaces what the pending one says. It is kept in
//! memory: the work that raises it finds the same trouble again after a
//! restart and raises it again, so nothing is lost.

use std::sync::PoisonError;
use std::time::SystemTime;

use humata2_access::PresentUser;
use humata2_daemon_ports::Caller;
use humata2_grpc::humata::v1 as pb;
use humata2_rpc_kit::helpers as status;
use pb::invalid_field::Problem;
use tonic::Status;

use super::{Decided, Entry, Held, Inbox, Run, answered_by, new_id, next_revision, settle, withdraw};
use crate::ports::now_timestamp;

/// Whether `entry` is a request the host raised for itself: it belongs to no
/// session and holds a call to run once approved.
pub(super) fn is_host_ask(entry: &Entry) -> bool {
    entry.session.is_empty() && entry.request.borrow().deferred
}

/// Makes `card` what a host ask is: a pending, VIP request of the Channel kind.
fn host_card(card: &mut pb::InboxRequest) {
    card.kind = pb::RequestKind::Channel as i32;
    card.state = pb::RequestState::Pending as i32;
    card.vip = true;
}

impl Inbox {
    /// Puts `card` in the Inbox, to run `run` once the user approves it.
    /// When a request for `key` is already pending, it takes `card` and
    /// `run` in place and no second request is made.
    pub async fn post_host_ask(&self, key: &str, mut card: pb::InboxRequest, run: Run) -> Result<(), Status> {
        host_card(&mut card);
        let mut state = self.state.lock().await;
        let found = state
            .id_with(Some(key))
            .and_then(|id| state.requests.iter().position(|entry| entry.id == id));
        if let Some(at) = found {
            let entry = &state.requests[at];
            if entry.request.borrow().request.state == pb::RequestState::Pending as i32 {
                let mut next = entry.request.borrow().clone();
                card.id = next.request.id.clone();
                card.created_at = next.request.created_at.clone();
                card.revision = Some(next_revision(&next.request));
                next.request = card;
                let current = next.request.clone();
                *entry.run.lock().unwrap_or_else(PoisonError::into_inner) = Some(run);
                entry.request.send_replace(next);
                self.tell(pb::Change::Updated, &entry.session, &current);
                return Ok(());
            }
        }
        let id = new_id();
        card.id = Some(pb::RequestId { value: id.clone() });
        card.created_at = Some(now_timestamp());
        card.revision = Some(pb::Revision { value: 1 });
        let mut held = Held::new(card);
        held.deferred = true;
        let entry = Entry::new(id, String::new(), None, None, Some(key.to_owned()), held);
        *entry.run.lock().unwrap_or_else(PoisonError::into_inner) = Some(run);
        let current = entry.request.borrow().request.clone();
        self.tell(pb::Change::Created, &entry.session, &current);
        state.add(entry);
        Ok(())
    }

    /// Withdraws the pending request for `key` for `reason`, as when what it
    /// asked about is no longer a problem. Its call never runs. Does nothing
    /// when none is pending.
    pub async fn withdraw_host_ask(&self, key: &str, reason: pb::WithdrawReason) {
        let state = self.state.lock().await;
        let Some(entry) = state
            .id_with(Some(key))
            .and_then(|id| state.requests.iter().find(|entry| entry.id == id))
        else {
            return;
        };
        let mut next = entry.request.borrow().clone();
        if next.request.state != pb::RequestState::Pending as i32 {
            return;
        }
        withdraw(&mut next.request, reason);
        let current = next.request.clone();
        entry.run.lock().unwrap_or_else(PoisonError::into_inner).take();
        entry.request.send_replace(next);
        self.tell(pb::Change::Updated, &entry.session, &current);
    }

    /// The user's answer to host ask `entry`: Approve runs its call, Deny
    /// drops it, anything else is refused. Answers the request as it is now.
    pub(super) fn answer_host_ask(
        &self,
        entry: &Entry,
        caller: &Caller,
        user: &PresentUser,
        answer: pb::request_answer::Answer,
    ) -> Result<pb::InboxRequest, Status> {
        use pb::request_answer::Answer as A;
        let decided = match answer {
            a @ A::Approve(_) => Decided {
                state: pb::RequestState::Approved,
                answer: a,
                grant: None,
            },
            a @ A::Deny(_) => {
                entry.run.lock().unwrap_or_else(PoisonError::into_inner).take();
                Decided {
                    state: pb::RequestState::Denied,
                    answer: a,
                    grant: None,
                }
            }
            _ => {
                return Err(status::invalid_field(
                    "answer",
                    Problem::NotAllowedHere,
                    format!("request {} changes something on a machine: it takes Approve or Deny", entry.id),
                    "approve it to go ahead, or deny it to leave things as they are",
                ));
            }
        };
        let mut next = entry.request.borrow().clone();
        settle(&mut next, decided, answered_by(caller, user), SystemTime::now());
        entry.request.send_replace(next);
        self.start(entry);
        Ok(self.tell_answer(entry))
    }
}
