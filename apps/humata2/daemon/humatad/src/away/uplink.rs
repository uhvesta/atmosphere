//! The `PeerUplink` stream. The peer link holds it open while the host is
//! reachable (D213); the newest open replaces the oldest.

use std::sync::Arc;

use humata2_grpc::humata::v1 as pb;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Status};

use super::link::HostLink;
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
}

impl Uplinks {
    pub(crate) fn new(link: Arc<HostLink>, feed: Arc<Feed>) -> Self {
        Self { link, feed }
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
                        // The host's messages are ignored for now. The policy
                        // snapshot they will carry is taken in AWAY T4.
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
