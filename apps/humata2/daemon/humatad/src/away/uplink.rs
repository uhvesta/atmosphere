//! The `PeerUplink` stream. The peer link holds it open while the host is
//! reachable (D213); the newest open replaces the oldest.

use std::sync::Arc;

use humata2_grpc::humata::v1 as pb;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Status};

use super::link::HostLink;
use super::policy_copy::PolicySource;
use crate::events::Feed;
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
}

impl Uplinks {
    pub(crate) fn new(link: Arc<HostLink>, feed: Arc<Feed>, policy: Arc<PolicySource>) -> Self {
        Self { link, feed, policy }
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
        let feed = self.feed.clone();
        let policy = self.policy.clone();
        tokio::spawn(async move {
            let mut from_host = from_host;
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
                    message = from_host.message() => match message {
                        Ok(Some(pb::HostToMachine {
                            message: Some(pb::host_to_machine::Message::Policy(snapshot)),
                        })) => {
                            let Some(ack) = applied(&policy, snapshot).await else {
                                continue;
                            };
                            if to_host.send(Ok(ack)).await.is_err() {
                                break;
                            }
                        }
                        // The host's other messages are taken in AWAY T5–T7.
                        Ok(Some(_)) => {}
                        Ok(None) | Err(_) => break,
                    },
                }
            }
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

/// Puts `snapshot` in force and acks its revision. A copy that can't be kept
/// is not acked, so the host's next one is taken instead; the one before
/// stays in force meanwhile.
async fn applied(policy: &Arc<PolicySource>, snapshot: pb::PolicySnapshot) -> Option<pb::MachineToHost> {
    let policy = Arc::clone(policy);
    let applied = tokio::task::spawn_blocking(move || policy.apply(&snapshot)).await;
    match applied {
        Ok(Ok(revision)) => Some(pb::MachineToHost {
            message: Some(pb::machine_to_host::Message::Ack(pb::UplinkAck {
                policy_revision: revision,
                ..pb::UplinkAck::default()
            })),
        }),
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
