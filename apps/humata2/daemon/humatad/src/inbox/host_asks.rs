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
use humata2_grpc::humata::v1 as pb;
use pb::invalid_field::Problem;
use tonic::Status;

use super::{Decided, Entry, Held, Inbox, Run, answered_by, new_id, next_revision, settle, withdraw};
use crate::identity::Caller;
use crate::status;

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
    pub(crate) async fn post_host_ask(&self, key: &str, mut card: pb::InboxRequest, run: Run) -> Result<(), Status> {
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
        card.created_at = Some(crate::attach::now());
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
    pub(crate) async fn withdraw_host_ask(&self, key: &str, reason: pb::WithdrawReason) {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use humata2_grpc::humata::v1::inbox_service_server::InboxService;
    use humata2_store::Store;
    use tokio::sync::oneshot;

    use super::*;
    use crate::events::Feed;
    use crate::guard::Guard;
    use crate::identity::UserVia;
    use crate::inbox::Deferred;

    const HOST: &str = "ma_host";

    fn inbox(directory: &std::path::Path) -> Inbox {
        let store = Arc::new(Store::open(directory).unwrap());
        let guard = Arc::new(Guard::new(HOST, pb::DaemonRole::Host));
        Inbox::new(guard, store, HOST.to_owned(), Arc::new(Feed::default()))
    }

    fn card(title: &str) -> pb::InboxRequest {
        pb::InboxRequest {
            action_title: title.to_owned(),
            ..Default::default()
        }
    }

    fn nothing() -> Deferred {
        Deferred {
            result: Vec::new(),
            result_type: String::new(),
        }
    }

    /// A call that tells `done` it ran.
    fn telling(done: oneshot::Sender<()>) -> Run {
        Box::new(move |_| {
            Box::pin(async move {
                let _ = done.send(());
                Ok(nothing())
            })
        })
    }

    fn user<T>(message: T) -> tonic::Request<T> {
        let mut request = tonic::Request::new(message);
        request.extensions_mut().insert(Caller::User(UserVia::App));
        request
    }

    async fn pending(inbox: &Inbox) -> Vec<pb::InboxRequest> {
        inbox
            .list_requests(user(pb::ListRequestsRequest::default()))
            .await
            .unwrap()
            .into_inner()
            .requests
    }

    async fn answer(inbox: &Inbox, request: &pb::InboxRequest, answer: pb::request_answer::Answer) -> Result<pb::InboxRequest, Status> {
        inbox
            .answer_request(user(pb::AnswerRequestRequest {
                id: request.id.clone(),
                answer: Some(pb::RequestAnswer { answer: Some(answer) }),
                ..Default::default()
            }))
            .await
            .map(tonic::Response::into_inner)
    }

    #[tokio::test]
    async fn a_second_post_for_a_key_replaces_the_pending_request_and_makes_no_second() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let (first, _) = oneshot::channel();
        let (second, _) = oneshot::channel();
        inbox.post_host_ask("rename/a/b", card("first"), telling(first)).await.unwrap();
        inbox.post_host_ask("rename/a/b", card("second"), telling(second)).await.unwrap();
        inbox.post_host_ask("rename/a/c", card("other"), telling(oneshot::channel().0)).await.unwrap();

        let held = pending(&inbox).await;

        assert_eq!(held.len(), 2, "one request for each key");
        assert!(held.iter().any(|request| request.action_title == "second"));
        assert!(held.iter().all(|request| request.vip && request.state() == pb::RequestState::Pending));
    }

    #[tokio::test]
    async fn approving_runs_the_call_and_denying_drops_it() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let (ran, heard) = oneshot::channel();
        inbox.post_host_ask("approved", card("approve me"), telling(ran)).await.unwrap();
        let witness = Arc::new(());
        let held = witness.clone();
        let dropped: Run = Box::new(move |_| {
            Box::pin(async move {
                let _keep = held;
                Ok(nothing())
            })
        });
        inbox.post_host_ask("denied", card("deny me"), dropped).await.unwrap();
        let requests = pending(&inbox).await;
        let by_title = |title: &str| requests.iter().find(|request| request.action_title == title).unwrap().clone();

        let approved = answer(&inbox, &by_title("approve me"), pb::request_answer::Answer::Approve(pb::Approve {})).await.unwrap();
        let denied = answer(&inbox, &by_title("deny me"), pb::request_answer::Answer::Deny(pb::Deny::default())).await.unwrap();

        assert_eq!(approved.state(), pb::RequestState::Approved);
        assert_eq!(denied.state(), pb::RequestState::Denied);
        tokio::time::timeout(Duration::from_secs(10), heard).await.unwrap().unwrap();
        assert_eq!(Arc::strong_count(&witness), 1, "the denied call was dropped, not kept");
    }

    #[tokio::test]
    async fn only_approve_and_deny_answer_it_and_a_withdrawn_one_takes_no_answer() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        inbox.post_host_ask("k", card("ask"), telling(oneshot::channel().0)).await.unwrap();
        let request = pending(&inbox).await.remove(0);

        let reply = answer(&inbox, &request, pb::request_answer::Answer::Reply(pb::Reply { text: "no".to_owned() })).await;
        inbox.withdraw_host_ask("k", pb::WithdrawReason::Resolved).await;
        let late = answer(&inbox, &request, pb::request_answer::Answer::Approve(pb::Approve {})).await;

        assert!(reply.is_err(), "a reply isn't an answer to it");
        assert!(late.is_err(), "a withdrawn request takes no answer");
        assert!(pending(&inbox).await.is_empty());
    }
}
