//! The host's answer relay (AWAY T7b, D226). A request a machine holds shows
//! on the host as a mirror; the host's answer to it goes down that machine's
//! link, and the host waits for the machine's own journal to say how the
//! request ended. This module is the host's record of which machines have a
//! live link, which answers it has sent, and the wait.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::pb;
use humata2_rpc_kit::helpers as status;
use tokio::sync::{mpsc, oneshot, watch};
use tonic::{Code, Status};

use super::{Held, machine_offline, still_open};

/// How long a host answer waits for the machine's verdict.
const WAIT: Duration = Duration::from_secs(10);

fn lock<T>(held: &Mutex<T>) -> MutexGuard<'_, T> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The machines' live links, and the answers sent to them that their
/// journals haven't settled yet.
pub struct Relays {
    links: Mutex<HashMap<String, mpsc::Sender<pb::HostToMachine>>>,
    /// The answer sent for each request id, until the machine's journal
    /// entry for it is applied: that entry credits the answer to the host's
    /// user only when it records this very answer.
    sent: Mutex<HashMap<String, Outstanding>>,
    /// Counts the answers sent, so a record is only removed by the answer
    /// that made it.
    generation: AtomicU64,
    wait: Duration,
}

/// An answer sent to `machine` that nothing has settled yet.
struct Outstanding {
    machine: String,
    generation: u64,
    answer: pb::RequestAnswer,
    /// Where the machine's refusal goes, if it sends one.
    refusal: Option<oneshot::Sender<pb::InboxAnswerRefused>>,
}

impl Relays {
    pub(super) fn new() -> Self {
        Self {
            links: Mutex::new(HashMap::new()),
            sent: Mutex::new(HashMap::new()),
            generation: AtomicU64::new(0),
            wait: WAIT,
        }
    }

    /// Says `machine`'s link is up, carrying `link`. The machine is offline
    /// again when the returned guard drops.
    pub fn register(self: &Arc<Self>, machine: &str, link: mpsc::Sender<pb::HostToMachine>) -> Registered {
        lock(&self.links).insert(machine.to_owned(), link.clone());
        Registered {
            relays: Arc::clone(self),
            machine: machine.to_owned(),
            link,
        }
    }

    /// `machine`'s link, if one is up.
    pub(super) fn link_of(&self, machine: &str) -> Option<mpsc::Sender<pb::HostToMachine>> {
        lock(&self.links).get(machine).filter(|link| !link.is_closed()).cloned()
    }

    /// Notes that `answer` is being sent to `machine` for request `id`. The
    /// note goes when the returned [`Sent`] drops, however the wait ends.
    pub(super) fn sent(self: &Arc<Self>, machine: &str, id: &str, answer: pb::RequestAnswer) -> Sent {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed);
        let (refusal, refused) = oneshot::channel();
        lock(&self.sent).insert(
            id.to_owned(),
            Outstanding {
                machine: machine.to_owned(),
                generation,
                answer,
                refusal: Some(refusal),
            },
        );
        Sent {
            relays: Arc::clone(self),
            id: id.to_owned(),
            generation,
            refused,
        }
    }

    /// The answer sent for request `id` that no journal entry has settled.
    pub(super) fn outstanding(&self, id: &str) -> Option<pb::RequestAnswer> {
        lock(&self.sent).get(id).map(|held| held.answer.clone())
    }

    /// Passes `machine`'s refusal of the answer sent for its request to the
    /// wait on it. A refusal for an answer nobody waits on any more, or
    /// from a machine the answer wasn't sent to, is left out.
    pub fn refused(&self, machine: &str, refusal: pb::InboxAnswerRefused) {
        let id = refusal.request.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        let sender = lock(&self.sent)
            .get_mut(&id)
            .filter(|held| held.machine == machine)
            .and_then(|held| held.refusal.take());
        if let Some(sender) = sender {
            // The wait may have just ended; then there is no one to tell.
            let _ = sender.send(refusal);
        }
    }

    /// Forgets the answer sent for request `id`.
    pub(super) fn settle(&self, id: &str) {
        lock(&self.sent).remove(id);
    }

    pub(super) fn wait(&self) -> Duration {
        self.wait
    }
}

/// The record of an answer sent to a machine. Dropping it forgets the
/// answer, unless a newer answer for the same request has replaced it.
pub(super) struct Sent {
    relays: Arc<Relays>,
    id: String,
    generation: u64,
    refused: oneshot::Receiver<pb::InboxAnswerRefused>,
}

impl Drop for Sent {
    fn drop(&mut self) {
        let mut sent = lock(&self.relays.sent);
        if sent.get(&self.id).is_some_and(|held| held.generation == self.generation) {
            sent.remove(&self.id);
        }
    }
}

/// A machine's link being up. Dropping it takes the link away, unless a
/// newer one has replaced it.
pub struct Registered {
    relays: Arc<Relays>,
    machine: String,
    link: mpsc::Sender<pb::HostToMachine>,
}

impl Drop for Registered {
    fn drop(&mut self) {
        let mut links = lock(&self.relays.links);
        if links.get(&self.machine).is_some_and(|held| held.same_channel(&self.link)) {
            links.remove(&self.machine);
        }
    }
}

/// An answer sent to a machine, and what it waits on.
pub(super) struct Relayed {
    pub(super) link: mpsc::Sender<pb::HostToMachine>,
    pub(super) seen: watch::Receiver<Held>,
    pub(super) sent: pb::RequestAnswer,
    /// The record of `sent`, gone when the wait ends.
    pub(super) record: Sent,
    pub(super) wait: Duration,
    /// The mirror as it stood when the answer was sent.
    pub(super) request: pb::InboxRequest,
}

impl Relayed {
    /// Waits for the mirror to leave Pending, and says how it ended: the
    /// request as it stands when the recorded answer is the one sent, else
    /// already answered or withdrawn. The machine can also refuse the
    /// answer outright, and its refusal is the result. When no verdict comes
    /// in time, or the link drops first, the answer was sent but isn't
    /// confirmed. However it ends, the record of the sent answer goes.
    pub(super) async fn verdict(self) -> Result<pb::InboxRequest, Status> {
        let Self {
            link,
            mut seen,
            sent,
            mut record,
            wait,
            request,
        } = self;
        let leaves = async {
            loop {
                let current = seen.borrow_and_update().request.clone();
                if current.state() != pb::RequestState::Pending {
                    return Some(current);
                }
                if seen.changed().await.is_err() {
                    return None;
                }
            }
        };
        // The machine's own refusal ends the wait; a machine that settled
        // the request first drops the record, which isn't a refusal.
        let refusal = async {
            match (&mut record.refused).await {
                Ok(refusal) => refusal,
                Err(_) => std::future::pending().await,
            }
        };
        let current = tokio::select! {
            found = tokio::time::timeout(wait, leaves) => found.ok().flatten(),
            () = link.closed() => None,
            refusal = refusal => return Err(refused(refusal)),
        };
        // The machine's end goes away right after it journals, so the link
        // can close in the same moment the mirror settles: look again.
        let current = current.or_else(|| {
            let now = seen.borrow().request.clone();
            (now.state() != pb::RequestState::Pending).then_some(now)
        });
        match current {
            Some(current) if answered_with(&current, &sent) => {
                Ok(current)
            }
            Some(current) => Err(still_open(&current).err().unwrap_or_else(|| machine_offline(&request, true))),
            None => Err(machine_offline(&request, true)),
        }
    }
}

/// What the host answers its user when the machine refused the answer.
fn refused(refusal: pb::InboxAnswerRefused) -> Status {
    let code = Code::from(refusal.code);
    match refusal.error {
        Some(error) => status::failure(code, error),
        None => Status::new(
            code,
            "the machine refused your answer without saying why; check the machine's Humata log, or answer the request on the machine with `humata login` and the approval passphrase",
        ),
    }
}

/// Whether `request` ended with `sent` as its answer.
fn answered_with(request: &pb::InboxRequest, sent: &pb::RequestAnswer) -> bool {
    matches!(
        request.state(),
        pb::RequestState::Approved | pb::RequestState::Denied | pb::RequestState::Answered
    ) && request.answer.as_ref() == Some(sent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link() -> (mpsc::Sender<pb::HostToMachine>, mpsc::Receiver<pb::HostToMachine>) {
        mpsc::channel(4)
    }

    #[test]
    fn a_machine_is_linked_while_its_registration_lives_and_not_after() {
        let relays = Arc::new(Relays::new());
        let (sender, _receiver) = link();
        assert!(relays.link_of("ma_box").is_none());
        let registered = relays.register("ma_box", sender);
        assert!(relays.link_of("ma_box").is_some());
        drop(registered);
        assert!(relays.link_of("ma_box").is_none());
    }

    #[test]
    fn an_older_registration_dropping_leaves_the_newer_link_up() {
        let relays = Arc::new(Relays::new());
        let (older, _older_end) = link();
        let (newer, _newer_end) = link();
        let first = relays.register("ma_box", older);
        let _second = relays.register("ma_box", newer);
        drop(first);
        assert!(relays.link_of("ma_box").is_some());
    }

    #[test]
    fn a_link_whose_other_end_is_gone_is_not_offered() {
        let relays = Arc::new(Relays::new());
        let (sender, receiver) = link();
        let _registered = relays.register("ma_box", sender);
        drop(receiver);
        assert!(relays.link_of("ma_box").is_none());
    }

    #[test]
    fn a_sent_answer_is_forgotten_when_its_wait_ends_however_it_ends() {
        let relays = Arc::new(Relays::new());
        let answer = pb::RequestAnswer {
            answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
        };
        let first = relays.sent("ma_box", "rq_1", answer.clone());
        drop(first);
        assert_eq!(relays.outstanding("rq_1"), None);
    }

    #[test]
    fn an_older_wait_ending_leaves_the_newer_answers_record() {
        let relays = Arc::new(Relays::new());
        let deny = pb::RequestAnswer {
            answer: Some(pb::request_answer::Answer::Deny(pb::Deny::default())),
        };
        let approve = pb::RequestAnswer {
            answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
        };
        let older = relays.sent("ma_box", "rq_1", deny);
        let _newer = relays.sent("ma_box", "rq_1", approve.clone());
        drop(older);
        assert_eq!(relays.outstanding("rq_1"), Some(approve));
    }

    #[test]
    fn a_sent_answer_is_outstanding_until_it_is_settled() {
        let relays = Arc::new(Relays::new());
        let answer = pb::RequestAnswer {
            answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
        };
        let _sent = relays.sent("ma_box", "rq_1", answer.clone());
        assert_eq!(relays.outstanding("rq_1"), Some(answer));
        relays.settle("rq_1");
        assert_eq!(relays.outstanding("rq_1"), None);
    }
}
