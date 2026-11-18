//! The `PeerUplink` stream. The peer link holds it open while the host is
//! reachable (D213); the newest open replaces the oldest.

use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use humata2_grpc::humata::v1 as pb;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Status};

use super::journal;
use super::link::HostLink;
use super::policy_copy::PolicySource;
use super::registry::Forwarder;
use crate::events::Feed;
use crate::inbox::Inbox;
use crate::identity::{Caller, UserVia};
use crate::status;

/// The daemon's side of a `PeerUplink` call.
pub(crate) type UplinkStream = ReceiverStream<Result<pb::MachineToHost, Status>>;

/// Opens and holds the host's uplinks, and reports the host link's changes
/// to the feed.
pub(crate) struct Uplinks {
    link: Arc<HostLink>,
    feed: Arc<Feed>,
    /// Where the policy copies the host sends are put in force (AW-8).
    policy: Arc<PolicySource>,
    /// Where the answers the host relays are applied (D226).
    inbox: Arc<Inbox>,
    /// Where this machine's agents' board calls go up, and the host's answers come back (D227).
    forwarder: Arc<Forwarder>,
    /// The holders on old holder files, which each uplink reports to the host (D378).
    holders: OnceLock<watch::Receiver<Option<pb::HoldersReport>>>,
}

impl Uplinks {
    pub(crate) fn new(link: Arc<HostLink>, feed: Arc<Feed>, policy: Arc<PolicySource>, inbox: Arc<Inbox>) -> Self {
        let forwarder = Arc::new(Forwarder::new(link.on_host()));
        Self {
            link,
            feed,
            policy,
            inbox,
            forwarder,
            holders: OnceLock::new(),
        }
    }

    /// Each uplink sends the host `holders` once the host's Resume came, and
    /// again whenever it changes (D378).
    pub(crate) fn report_holders(&self, holders: watch::Receiver<Option<pb::HoldersReport>>) {
        let _ = self.holders.set(holders);
    }

    /// The machine's end of the calls it forwards to the host (D227).
    pub(crate) fn forwarder(&self) -> Arc<Forwarder> {
        Arc::clone(&self.forwarder)
    }

    pub(crate) fn reachable(&self) -> bool {
        self.link.reachable()
    }

    /// Opens an uplink for the host and makes the host reachable. Only a call
    /// that came through the peer link is the host's, so nothing else can
    /// open one. The uplink ends when the host closes its side, or when a
    /// newer uplink replaces it.
    pub(crate) fn open(
        &self,
        caller: Option<&Caller>,
        from_host: tonic::Streaming<pb::HostToMachine>,
    ) -> Result<UplinkStream, Status> {
        if !matches!(caller, Some(Caller::User(UserVia::Host))) {
            return Err(status::failure(
                Code::PermissionDenied,
                status::error(
                    "only the peer link holds the host's uplink",
                    "the host opens it through the peer link when it links this machine; nothing else can",
                    pb::error::Kind::Denied(pb::Denied {
                        source: pb::denied::Source::Unspecified as i32,
                        capability: 0,
                        request: None,
                        user_note: String::new(),
                    }),
                ),
            ));
        }
        let opened = self.link.open();
        self.feed.publish(
            pb::Change::Updated,
            pb::event::Payload::HostLink(pb::HostLinkChanged { reachable: true }),
            None,
        );
        let (to_host, stream) = mpsc::channel(16);
        // From now until this uplink ends, the agents' board calls go up on it (D227).
        let forwarder = Arc::clone(&self.forwarder);
        let epoch = forwarder.attach(to_host.clone());
        let feed = self.feed.clone();
        let policy = self.policy.clone();
        let store = Arc::clone(self.policy.store());
        let inbox = Arc::clone(&self.inbox);
        let mut written = self.link.journal_writes();
        let mut holders = self.holders.get().cloned();
        tokio::spawn(async move {
            let mut from_host = from_host;
            // Nothing of the journal goes out before the host's Resume says
            // what it already holds; after it, `sent_through` is how far
            // this uplink has sent.
            let mut resumed = false;
            let mut sent_through = 0_u64;
            loop {
                tokio::select! {
                    () = opened.replaced() => {
                        // Sent before `to_host` drops, so the stream ends with it.
                        let _ = to_host
                            .send(Err(Status::aborted(
                                "humatad2 ended this uplink: a newer one from the peer link replaced it; the newer one carries on, so no action is needed",
                            )))
                            .await;
                        break;
                    }
                    Ok(()) = written.changed(), if resumed => {
                        if !send_journal(&store, &to_host, &mut sent_through).await {
                            break;
                        }
                    }
                    changed = holders_changed(&mut holders), if resumed => {
                        if !changed {
                            holders = None;
                        } else if !send_holders(&mut holders, &to_host).await {
                            break;
                        }
                    }
                    message = from_host.message() => match message {
                        Ok(Some(pb::HostToMachine {
                            message: Some(pb::host_to_machine::Message::Resume(resume)),
                        })) => {
                            let through = resume.from.map_or(0, |from| from.journal_through);
                            if !acked(&store, &policy, &inbox, through).await {
                                continue;
                            }
                            resumed = true;
                            // A host that resumes lower gets the entries again.
                            sent_through = through;
                            if !send_journal(&store, &to_host, &mut sent_through).await {
                                break;
                            }
                            // The list is sent whole on every link, so a host
                            // that was away learns what changed meanwhile.
                            if !send_holders(&mut holders, &to_host).await {
                                break;
                            }
                        }
                        Ok(Some(pb::HostToMachine {
                            message: Some(pb::host_to_machine::Message::Ack(ack)),
                        })) => {
                            acked(&store, &policy, &inbox, ack.journal_through).await;
                        }
                        Ok(Some(pb::HostToMachine {
                            message: Some(pb::host_to_machine::Message::Policy(snapshot)),
                        })) => {
                            let Some(ack) = applied(&policy, &feed, snapshot).await else {
                                continue;
                            };
                            if to_host.send(Ok(ack)).await.is_err() {
                                break;
                            }
                        }
                        Ok(Some(pb::HostToMachine {
                            message: Some(pb::host_to_machine::Message::Answer(relayed)),
                        })) => {
                            if let Some(refused) = inbox.answer_relayed(&relayed).await {
                                // So the host answers its user now, not after its wait.
                                let message = pb::MachineToHost {
                                    message: Some(pb::machine_to_host::Message::AnswerRefused(refused)),
                                };
                                if to_host.send(Ok(message)).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Ok(Some(pb::HostToMachine {
                            message: Some(pb::host_to_machine::Message::Result(result)),
                        })) => {
                            forwarder.answered(result);
                        }
                        // The host's other messages are taken in later milestones.
                        Ok(Some(_)) => {}
                        Ok(None) | Err(_) => break,
                    },
                }
            }
            // The calls still waiting on this uplink are host_unreachable now.
            forwarder.detach(epoch);
            if opened.close() {
                feed.publish(
                    pb::Change::Updated,
                    pb::event::Payload::HostLink(pb::HostLinkChanged { reachable: false }),
                    None,
                );
            }
        });
        Ok(ReceiverStream::new(stream))
    }
}

/// Waits until the holders report changes; false once nothing can change it.
async fn holders_changed(holders: &mut Option<watch::Receiver<Option<pb::HoldersReport>>>) -> bool {
    match holders {
        Some(holders) => holders.changed().await.is_ok(),
        None => std::future::pending().await,
    }
}

/// Sends the host the holders report as it stands, if there is one yet. False
/// when the uplink is gone.
async fn send_holders(
    holders: &mut Option<watch::Receiver<Option<pb::HoldersReport>>>,
    to_host: &mpsc::Sender<Result<pb::MachineToHost, Status>>,
) -> bool {
    let Some(holders) = holders else {
        return true;
    };
    let report = holders.borrow_and_update().clone();
    let Some(report) = report else {
        return true;
    };
    let message = pb::MachineToHost {
        message: Some(pb::machine_to_host::Message::Holders(report)),
    };
    to_host.send(Ok(message)).await.is_ok()
}

/// Puts `snapshot` in force, reports it kept on the feed, and acks its
/// revision. An older copy is acked but not reported, since it was dropped.
/// A copy that can't be kept is not acked, so the host's next one is taken
/// instead; the one before stays in force meanwhile.
async fn applied(policy: &Arc<PolicySource>, feed: &Feed, snapshot: pb::PolicySnapshot) -> Option<pb::MachineToHost> {
    let policy = Arc::clone(policy);
    let revision = snapshot.revision;
    let applied = tokio::task::spawn_blocking(move || policy.kept(&snapshot)).await;
    match applied {
        Ok(Ok(kept)) => {
            if let Some(revision) = kept {
                feed.publish(
                    pb::Change::Updated,
                    pb::event::Payload::PolicyCopy(pb::PolicyCopyKept { revision }),
                    None,
                );
            }
            Some(pb::MachineToHost {
                message: Some(pb::machine_to_host::Message::Ack(pb::UplinkAck {
                    policy_revision: revision,
                    ..pb::UplinkAck::default()
                })),
            })
        }
        Ok(Err(error)) => {
            crate::log::say!(
                "humatad2: warning: the host's policy copy wasn't taken: {}",
                crate::status::chain(&error)
            );
            None
        }
        Err(error) => {
            crate::log::say!("humatad2: warning: taking the host's policy copy stopped: {error}");
            None
        }
    }
}

/// Records that the host has every journal entry through `seq`. The grants
/// this machine made for the answers among them go first: the host holds
/// those answers and made its own grants, so these are dropped and marked
/// revoked (D226). False when the ack couldn't be kept; the entries are then
/// sent again on the next link.
async fn acked(store: &Arc<humata2_store::Store>, policy: &Arc<PolicySource>, inbox: &Inbox, seq: u64) -> bool {
    let reading = Arc::clone(store);
    let answered = tokio::task::spawn_blocking(move || journal::answered_through(&reading, seq)).await;
    match answered {
        Ok(Ok(answered)) => dropped(policy, inbox, &answered).await,
        Ok(Err(error)) => crate::log::say!(
            "humatad2: warning: couldn't tell which of this machine's grants the host has taken over, so they stay until the next ack: {}",
            crate::status::chain(&error)
        ),
        Err(error) => crate::log::say!("humatad2: warning: reading the journal for the host's ack stopped: {error}"),
    }
    let store = Arc::clone(store);
    let kept = tokio::task::spawn_blocking(move || journal::acked(&store, seq)).await;
    match kept {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            crate::log::say!("humatad2: warning: the host's journal ack wasn't kept: {}", crate::status::chain(&error));
            false
        }
        Err(error) => {
            crate::log::say!("humatad2: warning: keeping the host's journal ack stopped: {error}");
            false
        }
    }
}

/// Drops the grants this machine made for the requests in `answered`, and
/// marks them revoked in the Inbox so a restart doesn't bring them back.
async fn dropped(policy: &Arc<PolicySource>, inbox: &Inbox, answered: &BTreeSet<String>) {
    if answered.is_empty() {
        return;
    }
    let (source, requests) = (Arc::clone(policy), answered.clone());
    match tokio::task::spawn_blocking(move || source.drop_made_for(&requests)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => crate::log::say!(
            "humatad2: warning: a grant made on this machine wasn't dropped after the host took its answer: {}",
            crate::status::chain(&error)
        ),
        Err(error) => crate::log::say!("humatad2: warning: dropping this machine's grants stopped: {error}"),
    }
    let at = SystemTime::now();
    for request in answered {
        if let Err(refused) = inbox.revoked(request, at).await {
            crate::log::say!(
                "humatad2: warning: request {request}'s grant wasn't marked revoked, so a restart may bring it back until the host's copy replaces it: {}",
                refused.message()
            );
        }
    }
}

/// Sends the journal entries after `sent_through`, in order, and moves it
/// up to the last one sent. False when the uplink is gone.
async fn send_journal(
    store: &Arc<humata2_store::Store>,
    to_host: &mpsc::Sender<Result<pb::MachineToHost, Status>>,
    sent_through: &mut u64,
) -> bool {
    let after = *sent_through;
    let reading = Arc::clone(store);
    let read = tokio::task::spawn_blocking(move || journal::entries_after(&reading, after)).await;
    let entries = match read {
        Ok(Ok(entries)) => entries,
        Ok(Err(error)) => {
            crate::log::say!("humatad2: warning: the journal wasn't sent to the host: {}", crate::status::chain(&error));
            return true;
        }
        Err(error) => {
            crate::log::say!("humatad2: warning: reading the journal stopped: {error}");
            return true;
        }
    };
    for entry in entries {
        let seq = entry.seq;
        let message = pb::MachineToHost {
            message: Some(pb::machine_to_host::Message::Journal(entry)),
        };
        if to_host.send(Ok(message)).await.is_err() {
            return false;
        }
        *sent_through = seq;
    }
    true
}
