//! The `PeerUplink` stream. The peer link holds it open while the host is
//! reachable (D213); the newest open replaces the oldest.

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use humata2_daemon_ports::{Caller, UserVia};
use crate::pb;
use humata2_rpc_kit::helpers as status;
use tokio::sync::{mpsc, watch};
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tonic::{Code, Status};

use super::journal;
use super::link::HostLink;
use super::policy_copy::PolicySource;
use super::registry::Forwarder;

/// Publishes host-link and policy-copy changes to the daemon's event feed.
pub trait EventSink: Send + Sync {
    fn publish(&self, change: pb::Change, payload: pb::event::Payload, actor: Option<pb::Actor>);
}

/// Narrow inbox port used by [`Uplinks`] to apply relayed answers and mark
/// local grants revoked once the host acks them.
#[tonic::async_trait]
pub trait UplinkInbox: Send + Sync {
    async fn answer_relayed(&self, relayed: &pb::InboxAnswerRelay) -> Option<pb::InboxAnswerRefused>;
    async fn revoked(&self, request: &str, at: SystemTime) -> Result<(), Status>;
}

/// The daemon's side of a `PeerUplink` call.
pub type UplinkStream = ReceiverStream<Result<pb::MachineToHost, Status>>;

/// Opens and holds the host's uplinks, and reports the host link's changes
/// to the feed.
pub struct Uplinks {
    link: Arc<HostLink>,
    feed: Arc<dyn EventSink>,
    /// Where the policy copies the host sends are put in force (AW-8).
    policy: Arc<PolicySource>,
    /// Where the answers the host relays are applied (D226).
    inbox: Arc<dyn UplinkInbox>,
    /// Where this machine's agents' board calls go up, and the host's answers come back (D227).
    forwarder: Arc<Forwarder>,
    /// The holders on old holder files, which each uplink reports to the host (D378).
    holders: OnceLock<watch::Receiver<Option<pb::HoldersReport>>>,
}

impl Uplinks {
    pub fn new(
        link: Arc<HostLink>,
        feed: Arc<dyn EventSink>,
        policy: Arc<PolicySource>,
        inbox: Arc<dyn UplinkInbox>,
    ) -> Self {
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
    pub fn report_holders(&self, holders: watch::Receiver<Option<pb::HoldersReport>>) {
        let _ = self.holders.set(holders);
    }

    /// The machine's end of the calls it forwards to the host (D227).
    pub fn forwarder(&self) -> Arc<Forwarder> {
        Arc::clone(&self.forwarder)
    }

    pub fn reachable(&self) -> bool {
        self.link.reachable()
    }

    /// Opens an uplink for the host and makes the host reachable. Only a call
    /// that came through the peer link is the host's, so nothing else can
    /// open one. The uplink ends when the host closes its side, or when a
    /// newer uplink replaces it.
    pub fn open(
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
        let feed = Arc::clone(&self.feed);
        let policy = Arc::clone(&self.policy);
        let store = Arc::clone(self.policy.store());
        let inbox = Arc::clone(&self.inbox);
        let written = self.link.journal_writes();
        let holders = self.holders.get().cloned();
        tokio::spawn(run_uplink(
            opened,
            to_host,
            forwarder,
            epoch,
            feed,
            policy,
            store,
            inbox,
            written,
            holders,
            from_host,
            |store, policy, inbox, seq| async move { acked(&store, &policy, &*inbox, seq).await },
        ));
        Ok(ReceiverStream::new(stream))
    }
}

/// Runs one host uplink until the host closes it or a newer uplink replaces it.
#[allow(clippy::too_many_arguments)]
async fn run_uplink<S, A, Fut>(
    opened: super::link::Opened,
    to_host: mpsc::Sender<Result<pb::MachineToHost, Status>>,
    forwarder: Arc<Forwarder>,
    epoch: u64,
    feed: Arc<dyn EventSink>,
    policy: Arc<PolicySource>,
    store: Arc<humata2_store::Store>,
    inbox: Arc<dyn UplinkInbox>,
    mut written: watch::Receiver<u64>,
    mut holders: Option<watch::Receiver<Option<pb::HoldersReport>>>,
    mut from_host: S,
    ack: A,
) where
    S: Stream<Item = Result<pb::HostToMachine, Status>> + Unpin,
    A: Fn(Arc<humata2_store::Store>, Arc<PolicySource>, Arc<dyn UplinkInbox>, u64) -> Fut,
    Fut: Future<Output = bool>,
{
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
            message = from_host.next() => match message {
                Some(Ok(pb::HostToMachine {
                    message: Some(pb::host_to_machine::Message::Resume(resume)),
                })) => {
                    let through = resume.from.map_or(0, |from| from.journal_through);
                    // Recording the ack is bookkeeping; a transient store failure
                    // must not leave `resumed` false and wedge journal/holders
                    // delivery for the rest of the link (SMB-066). The next Ack
                    // retries it.
                    let _ = ack(Arc::clone(&store), Arc::clone(&policy), Arc::clone(&inbox), through).await;
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
                Some(Ok(pb::HostToMachine {
                    message: Some(pb::host_to_machine::Message::Ack(ack_msg)),
                })) => {
                    let _ = ack(Arc::clone(&store), Arc::clone(&policy), Arc::clone(&inbox), ack_msg.journal_through).await;
                }
                Some(Ok(pb::HostToMachine {
                    message: Some(pb::host_to_machine::Message::Policy(snapshot)),
                })) => {
                    let Some(ack_msg) = applied(&policy, &*feed, snapshot).await else {
                        continue;
                    };
                    if to_host.send(Ok(ack_msg)).await.is_err() {
                        break;
                    }
                }
                Some(Ok(pb::HostToMachine {
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
                Some(Ok(pb::HostToMachine {
                    message: Some(pb::host_to_machine::Message::Result(result)),
                })) => {
                    forwarder.answered(result);
                }
                // The host's other messages are taken in later milestones.
                Some(Ok(_)) => {}
                None | Some(Err(_)) => break,
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
async fn applied(
    policy: &Arc<PolicySource>,
    feed: &dyn EventSink,
    snapshot: pb::PolicySnapshot,
) -> Option<pb::MachineToHost> {
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
            humata2_rpc_kit::log::line(&format!(
                "humatad2: warning: the host's policy copy wasn't taken: {}",
                status::chain(&error)
            ));
            None
        }
        Err(error) => {
            humata2_rpc_kit::log::line(&format!(
                "humatad2: warning: taking the host's policy copy stopped: {error}"
            ));
            None
        }
    }
}

/// Records that the host has every journal entry through `seq`. The grants
/// this machine made for the answers among them go first: the host holds
/// those answers and made its own grants, so these are dropped and marked
/// revoked (D226). False when the ack couldn't be kept; the entries are then
/// sent again on the next link.
async fn acked(
    store: &Arc<humata2_store::Store>,
    policy: &Arc<PolicySource>,
    inbox: &dyn UplinkInbox,
    seq: u64,
) -> bool {
    let reading = Arc::clone(store);
    let answered = tokio::task::spawn_blocking(move || journal::answered_through(&reading, seq)).await;
    match answered {
        Ok(Ok(answered)) => dropped(policy, inbox, &answered).await,
        Ok(Err(error)) => humata2_rpc_kit::log::line(&format!(
            "humatad2: warning: couldn't tell which of this machine's grants the host has taken over, so they stay until the next ack: {}",
            status::chain(&error)
        )),
        Err(error) => humata2_rpc_kit::log::line(&format!(
            "humatad2: warning: reading the journal for the host's ack stopped: {error}"
        )),
    }
    let store = Arc::clone(store);
    let kept = tokio::task::spawn_blocking(move || journal::acked(&store, seq)).await;
    match kept {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            humata2_rpc_kit::log::line(&format!(
                "humatad2: warning: the host's journal ack wasn't kept: {}",
                status::chain(&error)
            ));
            false
        }
        Err(error) => {
            humata2_rpc_kit::log::line(&format!(
                "humatad2: warning: keeping the host's journal ack stopped: {error}"
            ));
            false
        }
    }
}

/// Drops the grants this machine made for the requests in `answered`, and
/// marks them revoked in the Inbox so a restart doesn't bring them back.
async fn dropped(policy: &Arc<PolicySource>, inbox: &dyn UplinkInbox, answered: &BTreeSet<String>) {
    if answered.is_empty() {
        return;
    }
    let (source, requests) = (Arc::clone(policy), answered.clone());
    match tokio::task::spawn_blocking(move || source.drop_made_for(&requests)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => humata2_rpc_kit::log::line(&format!(
            "humatad2: warning: a grant made on this machine wasn't dropped after the host took its answer: {}",
            status::chain(&error)
        )),
        Err(error) => humata2_rpc_kit::log::line(&format!(
            "humatad2: warning: dropping this machine's grants stopped: {error}"
        )),
    }
    let at = SystemTime::now();
    for request in answered {
        if let Err(refused) = inbox.revoked(request, at).await {
            humata2_rpc_kit::log::line(&format!(
                "humatad2: warning: request {request}'s grant wasn't marked revoked, so a restart may bring it back until the host's copy replaces it: {}",
                refused.message()
            ));
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
            humata2_rpc_kit::log::line(&format!(
                "humatad2: warning: the journal wasn't sent to the host: {}",
                status::chain(&error)
            ));
            return true;
        }
        Err(error) => {
            humata2_rpc_kit::log::line(&format!(
                "humatad2: warning: reading the journal stopped: {error}"
            ));
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use humata2_access::{ChannelRules, Grant, Policy, PolicyError, RulesState};
    use humata2_store::{Store, StoreError};

    use super::*;
    use crate::policy_copy::PolicyGuard;

    struct NoopFeed;
    impl EventSink for NoopFeed {
        fn publish(&self, _change: pb::Change, _payload: pb::event::Payload, _actor: Option<pb::Actor>) {}
    }

    struct NoopInbox;
    #[tonic::async_trait]
    impl UplinkInbox for NoopInbox {
        async fn answer_relayed(&self, _relayed: &pb::InboxAnswerRelay) -> Option<pb::InboxAnswerRefused> {
            None
        }
        async fn revoked(&self, _request: &str, _at: SystemTime) -> Result<(), Status> {
            Ok(())
        }
    }

    struct NoopPolicyGuard {
        changes: watch::Sender<u64>,
    }

    impl NoopPolicyGuard {
        fn new() -> Arc<Self> {
            let (changes, _) = watch::channel(0);
            Arc::new(Self { changes })
        }
    }

    impl PolicyGuard for NoopPolicyGuard {
        fn policy_revision(&self) -> u64 {
            0
        }
        fn policy_changes(&self) -> watch::Receiver<u64> {
            self.changes.subscribe()
        }
        fn policy_clone(&self) -> Policy {
            Policy::default()
        }
        fn replace_policy(&self, _policy: Policy) {}
        fn host_rules_arrived(&self) {}
        fn drop_local_grants(&self, _answered: &BTreeSet<String>) -> Result<(), PolicyError> {
            Ok(())
        }
        fn encode_rules(&self, _rules: &ChannelRules) -> (Vec<pb::Rule>, pb::AwayRule) {
            (Vec::new(), pb::AwayRule::default())
        }
        fn decode_rules(&self, _rules: &[pb::Rule], _away: Option<&pb::AwayRule>) -> Result<RulesState, String> {
            Ok(Policy::new().to_state().new_channels)
        }
        fn encode_grant(&self, _grant: &Grant, _suspended: bool) -> pb::Grant {
            pb::Grant::default()
        }
        fn decode_grant(
            &self,
            _store: &Store,
            _rules: &Policy,
            _grant: &pb::Grant,
        ) -> Result<Option<Grant>, StoreError> {
            Ok(None)
        }
        fn global_contain(&self, _store: &Store) -> Result<bool, StoreError> {
            Ok(false)
        }
        fn put_global_contain(&self, _store: &Store, _on: bool, _revision: u64) -> Result<(), StoreError> {
            Ok(())
        }
        fn channel_contain(&self, _store: &Store, _channel: &str) -> Result<Option<bool>, StoreError> {
            Ok(None)
        }
        fn put_channel_contain(
            &self,
            _store: &Store,
            _channel: &str,
            _on: Option<bool>,
            _revision: u64,
        ) -> Result<(), StoreError> {
            Ok(())
        }
        fn use_worktrees(&self, _store: &Store) -> Result<bool, StoreError> {
            Ok(false)
        }
        fn put_use_worktrees(&self, _store: &Store, _on: bool, _revision: u64) -> Result<(), StoreError> {
            Ok(())
        }
    }

    /// SMB-066: when recording the Resume ack fails (`acked() == false`), the
    /// uplink must still mark `resumed = true` and deliver pending/subsequent
    /// journal entries and holders reports rather than wedging the link.
    #[tokio::test]
    async fn a_failed_resume_ack_does_not_wedge_journal_replay_or_holders() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path()).unwrap());
        let link = Arc::new(HostLink::new(false));
        let seq1 = journal::record_board(&store, &link, "se_1", "board.item_create", vec![1], None, None)
            .await
            .unwrap();
        let seq2 = journal::record_board(&store, &link, "se_1", "board.item_move", vec![2], None, None)
            .await
            .unwrap();

        let opened = link.open();
        let (to_host_tx, mut to_host_rx) = mpsc::channel(16);
        let (from_host_tx, from_host_rx) = mpsc::channel(16);
        let forwarder = Arc::new(Forwarder::new(false));
        let epoch = forwarder.attach(to_host_tx.clone());
        let policy = PolicySource::new(NoopPolicyGuard::new(), Arc::clone(&store));
        let (holders_tx, holders_rx) = watch::channel(Some(pb::HoldersReport::default()));

        let task = tokio::spawn(run_uplink(
            opened,
            to_host_tx,
            forwarder,
            epoch,
            Arc::new(NoopFeed),
            policy,
            Arc::clone(&store),
            Arc::new(NoopInbox),
            link.journal_writes(),
            Some(holders_rx),
            ReceiverStream::new(from_host_rx),
            // Simulate `journal::acked` failing on disk.
            |_store, _policy, _inbox, _seq| async { false },
        ));

        from_host_tx
            .send(Ok(pb::HostToMachine {
                message: Some(pb::host_to_machine::Message::Resume(pb::UplinkResume {
                    from: Some(pb::UplinkAck {
                        journal_through: seq1,
                        ..pb::UplinkAck::default()
                    }),
                })),
            }))
            .await
            .unwrap();

        let bound = Duration::from_secs(2);
        let first = tokio::time::timeout(bound, to_host_rx.recv())
            .await
            .expect("unacked entry after Resume must be sent")
            .unwrap()
            .unwrap();
        match first.message {
            Some(pb::machine_to_host::Message::Journal(entry)) => assert_eq!(entry.seq, seq2),
            other => panic!("expected Journal(seq2), got {other:?}"),
        }

        let second = tokio::time::timeout(bound, to_host_rx.recv())
            .await
            .expect("initial holders report must be sent after Resume")
            .unwrap()
            .unwrap();
        assert!(
            matches!(second.message, Some(pb::machine_to_host::Message::Holders(_))),
            "expected Holders, got {:?}",
            second.message
        );

        // Subsequent journal writes must stream because `resumed` is true.
        let seq3 = journal::record_board(&store, &link, "se_1", "board.item_move", vec![3], None, None)
            .await
            .unwrap();
        let third = tokio::time::timeout(bound, to_host_rx.recv())
            .await
            .expect("subsequent journal write must reach host")
            .unwrap()
            .unwrap();
        match third.message {
            Some(pb::machine_to_host::Message::Journal(entry)) => assert_eq!(entry.seq, seq3),
            other => panic!("expected Journal(seq3), got {other:?}"),
        }

        // Subsequent holders updates must also stream.
        holders_tx.send(Some(pb::HoldersReport::default())).unwrap();
        let fourth = tokio::time::timeout(bound, to_host_rx.recv())
            .await
            .expect("subsequent holders update must reach host")
            .unwrap()
            .unwrap();
        assert!(
            matches!(fourth.message, Some(pb::machine_to_host::Message::Holders(_))),
            "expected updated Holders, got {:?}",
            fourth.message
        );

        drop(from_host_tx);
        task.await.unwrap();
    }
}
