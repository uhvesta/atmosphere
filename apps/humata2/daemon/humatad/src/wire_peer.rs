//! Concrete `humatad` adapters for `humata2_svc_peer::ports` (Wiring Wave 2).
//!
//! Bridges `humata2_svc_peer`'s narrow port traits onto `humatad`'s concrete
//! `PolicySource`, `BoardSvc`, `Inbox`, `Relays`, `Review`, `Watch`, `Feed`,
//! `Operations`, `Progress`, and `Work` types.

use std::sync::{Arc, Mutex, PoisonError};

use humata2_daemon_ports::{Caller, UserVia};
use humata2_grpc::humata::v1 as pb;
use humata2_svc_peer::ports::{
    MachineWorkPort, PeerBoardPort, PeerFeedPort, PeerInboxPort, PeerOperationsPort,
    PeerRelaysPort, PeerReviewPort, PeerUpgradeWatchPort, PolicySourcePort, Progress,
    ProgressPort, Replayed, Started,
};
use tokio::sync::{broadcast, mpsc, watch};
use tonic::Status;

use crate::away::PolicySource;
#[cfg(feature = "svc_board")]
use crate::board_service::{BoardSvc, Replayed as LocalReplayed};
use crate::events::Feed;
use crate::inbox::{Inbox, Relays};
use crate::machine_service::Work;
use crate::operations::{Operations, Progress as LocalProgress, Started as LocalStarted};
#[cfg(feature = "svc_review")]
use crate::review::Review;
use crate::upgrade::items::Watch;

#[cfg(feature = "svc_board")]
fn map_board_replayed(outcome: LocalReplayed) -> Replayed {
    match outcome {
        LocalReplayed::Held => Replayed::Held,
        LocalReplayed::Applied => Replayed::Applied,
        LocalReplayed::Rejected => Replayed::Rejected,
    }
}

fn map_inbox_replayed(outcome: humata2_svc_inbox::inbox::Replayed) -> Replayed {
    match outcome {
        humata2_svc_inbox::inbox::Replayed::Held => Replayed::Held,
        humata2_svc_inbox::inbox::Replayed::Applied => Replayed::Applied,
        humata2_svc_inbox::inbox::Replayed::Rejected => Replayed::Rejected,
    }
}

#[cfg(feature = "svc_review")]
fn map_review_replayed(outcome: humata2_svc_review::Replayed) -> Replayed {
    match outcome {
        humata2_svc_review::Replayed::Held => Replayed::Held,
        humata2_svc_review::Replayed::Applied => Replayed::Applied,
        humata2_svc_review::Replayed::Rejected => Replayed::Rejected,
    }
}

/// Adapts `crate::away::PolicySource` to [`PolicySourcePort`].
pub(crate) struct PolicySourceAdapter(pub(crate) Arc<PolicySource>);

impl PolicySourcePort for PolicySourceAdapter {
    fn snapshot(&self, machine: &str) -> Result<pb::PolicySnapshot, std::io::Error> {
        self.0
            .snapshot(machine)
            .map_err(|err| std::io::Error::other(crate::status::chain(&err)))
    }

    fn changes(&self) -> watch::Receiver<u64> {
        self.0.changes()
    }

    fn linked(&self, machine: &str) {
        self.0.linked(machine);
    }

    fn acked(&self, machine: &str, rev: u64) {
        self.0.acked(machine, rev);
    }
}

/// Adapts `crate::board_service::BoardSvc` to [`PeerBoardPort`].
#[cfg(feature = "svc_board")]
pub(crate) struct PeerBoardAdapter(pub(crate) Arc<BoardSvc>);

#[cfg(feature = "svc_board")]
#[tonic::async_trait]
impl PeerBoardPort for PeerBoardAdapter {
    async fn replay_at(&self, machine: &str, entry: &pb::JournalEntry, link_up: i64) -> Replayed {
        map_board_replayed(self.0.replay_at(machine, entry, link_up).await)
    }

    async fn forwarded(&self, machine: &str, call: pb::ForwardedCall) -> pb::ForwardedResult {
        self.0.forwarded(machine, call).await
    }
}

/// Adapts `crate::inbox::Relays` to [`PeerRelaysPort`].
pub(crate) struct PeerRelaysAdapter(pub(crate) Arc<Relays>);

impl PeerRelaysPort for PeerRelaysAdapter {
    fn register(&self, machine: &str, tx: mpsc::Sender<pb::HostToMachine>) {
        let _guard = self.0.register(machine, tx);
    }

    fn refused(&self, machine: &str, refused: pb::InboxAnswerRefused) {
        self.0.refused(machine, refused);
    }
}

/// Adapts `crate::inbox::Inbox` to [`PeerInboxPort`].
pub(crate) struct PeerInboxAdapter(pub(crate) Arc<Inbox>);

#[tonic::async_trait]
impl PeerInboxPort for PeerInboxAdapter {
    async fn replay(&self, machine: &str, entry: &pb::JournalEntry) -> Replayed {
        map_inbox_replayed(self.0.replay(machine, entry).await)
    }

    async fn replayed_through(&self, machine: &str) -> u64 {
        self.0.replayed_through(machine).await
    }

    fn relays(&self) -> Arc<dyn PeerRelaysPort> {
        Arc::new(PeerRelaysAdapter(self.0.relays().clone()))
    }
}

/// Adapts `crate::review::Review` to [`PeerReviewPort`].
#[cfg(feature = "svc_review")]
pub(crate) struct PeerReviewAdapter(pub(crate) Arc<Review>);

#[cfg(feature = "svc_review")]
#[tonic::async_trait]
impl PeerReviewPort for PeerReviewAdapter {
    async fn replay_offered(&self, machine: &str, entry: &pb::JournalEntry) -> Replayed {
        map_review_replayed(self.0.replay_offered(machine, entry).await)
    }
}

/// Adapts `crate::upgrade::items::Watch` to [`PeerUpgradeWatchPort`].
pub(crate) struct PeerUpgradeWatchAdapter(pub(crate) Watch);

#[tonic::async_trait]
impl PeerUpgradeWatchPort for PeerUpgradeWatchAdapter {
    async fn holders(
        &self,
        machine: &str,
        report: pb::HoldersReport,
        first: bool,
        channel: &tonic::transport::Channel,
    ) {
        self.0.holders(machine, report, first, channel).await;
    }

    async fn linked(&self, machine: &str, build: Option<&pb::BuildInfo>) {
        self.0.linked(machine, build).await;
    }

    async fn unsupported(&self, machine: &str) {
        self.0.unsupported(machine).await;
    }
}

/// Adapts `crate::events::Feed` to [`PeerFeedPort`].
pub(crate) struct PeerFeedAdapter(pub(crate) Arc<Feed>);

impl PeerFeedPort for PeerFeedAdapter {
    fn subscribe(&self, after: Option<u64>) -> (u64, broadcast::Receiver<pb::Event>) {
        let (missed, mut src) = self.0.subscribe(after);
        let seq = missed
            .last()
            .and_then(|e| e.seq.as_ref())
            .map_or(after.unwrap_or(0), |s| s.value);
        let (tx, rx) = broadcast::channel(256);
        tokio::spawn(async move {
            loop {
                match src.recv().await {
                    Ok(published) => {
                        if tx.send((**published).clone()).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        (seq, rx)
    }
}

/// Adapts `crate::operations::Progress` to [`ProgressPort`].
pub(crate) struct ProgressAdapter(pub(crate) Mutex<Option<LocalProgress>>);

impl ProgressPort for ProgressAdapter {
    fn id(&self) -> String {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|p| p.operation().id)
            .map(|id| id.value)
            .unwrap_or_default()
    }

    fn operation(&self) -> pb::Operation {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(LocalProgress::operation)
            .unwrap_or_default()
    }

    fn set_step(&self, key: &str, state: pb::StepState, output_tail: Option<String>) {
        if let Some(progress) = self.0.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            progress.set_step(key, state, output_tail);
        }
    }

    fn set_units(&self, key: &str, done: u64, total: u64) {
        if let Some(progress) = self.0.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            progress.units().set(key, done, total);
        }
    }

    fn succeed(&self, result: Option<pb::operation::Result>) {
        if let Some(progress) = self.0.lock().unwrap_or_else(PoisonError::into_inner).take() {
            progress.succeed(result);
        }
    }

    fn fail(&self, error: pb::Error) {
        if let Some(progress) = self.0.lock().unwrap_or_else(PoisonError::into_inner).take() {
            progress.fail(error);
        }
    }

    fn end_cancelled(&self) {
        if let Some(progress) = self.0.lock().unwrap_or_else(PoisonError::into_inner).take() {
            progress.end_cancelled();
        }
    }
}

/// Adapts `crate::operations::Operations` to [`PeerOperationsPort`].
pub(crate) struct PeerOperationsAdapter(pub(crate) Arc<Operations>);

impl PeerOperationsPort for PeerOperationsAdapter {
    fn existing(&self, key: &str) -> Option<pb::Operation> {
        let caller = Caller::User(UserVia::App);
        let idempotency = pb::IdempotencyKey {
            value: key.to_owned(),
        };
        self.0.existing(&caller, Some(&idempotency)).ok().flatten()
    }

    fn start_steps(
        &self,
        _kind: &str,
        key: &str,
        _target: &str,
        steps: &[&str],
    ) -> Result<Started, Status> {
        let caller = Caller::User(UserVia::App);
        let named: Vec<(String, String)> = steps
            .iter()
            .map(|step| ((*step).to_owned(), (*step).to_owned()))
            .collect();
        let idempotency = (!key.is_empty()).then(|| pb::IdempotencyKey {
            value: key.to_owned(),
        });
        match self.0.start_steps(
            &caller,
            pb::OperationKind::Unspecified,
            pb::EntityRef::default(),
            &named,
            false,
            idempotency,
        )? {
            LocalStarted::New(progress) => Ok(Started::New(Progress::new(Arc::new(
                ProgressAdapter(Mutex::new(Some(progress))),
            )))),
            LocalStarted::Existing(op) => Ok(Started::Existing(op)),
        }
    }
}

/// Adapts `crate::machine_service::Work` to [`MachineWorkPort`].
pub(crate) struct MachineWorkAdapter(pub(crate) Arc<Work>);

impl MachineWorkPort for MachineWorkAdapter {}

// ---------------------------------------------------------------------------
// Constructor helpers
// ---------------------------------------------------------------------------

pub(crate) fn policy_source(source: Arc<PolicySource>) -> Arc<dyn PolicySourcePort> {
    Arc::new(PolicySourceAdapter(source))
}

#[cfg(feature = "svc_board")]
pub(crate) fn peer_board(board: Arc<BoardSvc>) -> Arc<dyn PeerBoardPort> {
    Arc::new(PeerBoardAdapter(board))
}

pub(crate) fn peer_inbox(inbox: Arc<Inbox>) -> Arc<dyn PeerInboxPort> {
    Arc::new(PeerInboxAdapter(inbox))
}

#[cfg(feature = "svc_review")]
pub(crate) fn peer_review(review: Arc<Review>) -> Arc<dyn PeerReviewPort> {
    Arc::new(PeerReviewAdapter(review))
}

pub(crate) fn peer_upgrade_watch(watch: Watch) -> Arc<dyn PeerUpgradeWatchPort> {
    Arc::new(PeerUpgradeWatchAdapter(watch))
}

pub(crate) fn peer_feed(feed: Arc<Feed>) -> Arc<dyn PeerFeedPort> {
    Arc::new(PeerFeedAdapter(feed))
}

pub(crate) fn peer_operations(ops: Arc<Operations>) -> Arc<dyn PeerOperationsPort> {
    Arc::new(PeerOperationsAdapter(ops))
}

pub(crate) fn machine_work(work: Arc<Work>) -> Arc<dyn MachineWorkPort> {
    Arc::new(MachineWorkAdapter(work))
}
