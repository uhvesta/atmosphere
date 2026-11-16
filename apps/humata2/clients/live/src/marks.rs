//! [`MarksClient`] for the real daemon, over its MarkService (NV-20, NV-21,
//! NV-24 to NV-26). The host keeps the marks and their order; this client
//! only asks.

use std::sync::Arc;

use humata2_clients::marks::{MarksClient, Pinned, PinnedList, PinnedLists, Thing};
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_clients::{DaemonError, Reply};
use humata2_grpc::humata::v1 as pb;
use humata2_grpc::humata::v1::mark_service_client::MarkServiceClient;
use tokio::sync::mpsc;
use tonic::Streaming;

use crate::connection::{LiveDaemon, Shared, fresh_key};

/// Lists the watcher may have waiting before the feed waits for it.
const QUEUE: usize = 16;

const WHAT: &str = "The pinned list";

/// Pins and VIP marks on the real daemon. Cheap to clone.
#[derive(Clone)]
pub struct LiveMarks {
    shared: Arc<Shared>,
}

impl LiveMarks {
    /// Marks over `daemon`'s connection.
    #[must_use]
    pub fn new(daemon: &LiveDaemon) -> Self {
        Self {
            shared: daemon.shared.clone(),
        }
    }
}

fn client(shared: &Shared) -> MarkServiceClient<crate::connection::Wire> {
    MarkServiceClient::new(shared.wire.clone())
}

fn entity(thing: &Thing) -> pb::EntityRef {
    use pb::entity_ref::Entity;
    let entity = match thing {
        Thing::Session(id) => Entity::Session(pb::SessionId {
            value: id.as_str().to_owned(),
        }),
        Thing::Channel(id) => Entity::Channel(pb::ChannelId {
            value: id.as_str().to_owned(),
        }),
        Thing::Machine(id) => Entity::Machine(pb::MachineId {
            value: id.as_str().to_owned(),
        }),
        Thing::PrReview(id) => Entity::PrReview(pb::SessionId {
            value: id.as_str().to_owned(),
        }),
        Thing::BoardItem(id) => Entity::BoardItem(pb::BoardItemId { value: id.clone() }),
    };
    pb::EntityRef { entity: Some(entity) }
}

/// The thing a pin names; kinds the app does not pin are left out.
fn thing(entity: Option<&pb::EntityRef>) -> Option<Thing> {
    use pb::entity_ref::Entity;
    Some(match entity?.entity.as_ref()? {
        Entity::Session(id) => Thing::Session(SessionId::new(id.value.clone())),
        Entity::Channel(id) => Thing::Channel(ChannelId::new(id.value.clone())),
        Entity::Machine(id) => Thing::Machine(MachineId::new(id.value.clone())),
        Entity::PrReview(id) => Thing::PrReview(SessionId::new(id.value.clone())),
        Entity::BoardItem(id) => Thing::BoardItem(id.value.clone()),
        _ => return None,
    })
}

fn list(wire: &pb::ListPinnedResponse) -> PinnedList {
    PinnedList {
        items: wire
            .entries
            .iter()
            .filter_map(|entry| {
                Some(Pinned {
                    thing: thing(entry.entity.as_ref())?,
                    title: entry.title.clone(),
                    event: entry.event.clone(),
                    vip: entry.vip,
                })
            })
            .collect(),
        revision: wire.revision.as_ref().map_or(0, |revision| revision.value),
    }
}

impl Shared {
    async fn list_pinned(&self) -> Result<PinnedList, DaemonError> {
        let answer = client(self)
            .list_pinned(pb::ListPinnedRequest {})
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(list(&answer))
    }

    /// Opens the feed narrowed to marks. It is open before this answers, so
    /// a change made after it is not missed.
    async fn watch_marks(self: &Arc<Self>) -> Result<PinnedLists, DaemonError> {
        let stream = self
            .event_stream()
            .subscribe(pb::SubscribeRequest {
                after: None,
                channels: Vec::new(),
                sessions: Vec::new(),
                topics: vec![pb::Topic::Marks as i32],
            })
            .await
            .map_err(|status| self.error(&status, "Following the pinned list"))?
            .into_inner();
        let (sender, receiver) = mpsc::channel(QUEUE);
        self.spawn(forward(self.clone(), stream, sender));
        Ok(receiver)
    }
}

/// Reads the list after each marks event until the feed ends or nobody
/// listens.
async fn forward(shared: Arc<Shared>, mut stream: Streaming<pb::Event>, sender: mpsc::Sender<PinnedList>) {
    loop {
        let next = tokio::select! {
            () = sender.closed() => return,
            next = stream.message() => next,
        };
        match next {
            Ok(Some(event)) => {
                if !matches!(event.payload, Some(pb::event::Payload::Marks(_))) {
                    continue;
                }
                let Ok(current) = shared.list_pinned().await else {
                    return;
                };
                if sender.send(current).await.is_err() {
                    return;
                }
            }
            Ok(None) | Err(_) => return,
        }
    }
}

impl MarksClient for LiveMarks {
    fn pinned(&self) -> Reply<PinnedList> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.list_pinned().await })
    }

    fn set(&self, thing: &Thing, pinned: Option<bool>, vip: Option<bool>) -> Reply<PinnedList> {
        let shared = self.shared.clone();
        let entity = entity(thing);
        Box::pin(async move {
            client(&shared)
                .set_marks(pb::SetMarksRequest {
                    idempotency_key: fresh_key(),
                    entity: Some(entity),
                    pinned,
                    vip,
                })
                .await
                .map_err(|status| shared.error(&status, "This mark"))?;
            shared.list_pinned().await
        })
    }

    fn reorder(&self, order: &[Thing], revision: u64) -> Reply<PinnedList> {
        let shared = self.shared.clone();
        let order: Vec<pb::EntityRef> = order.iter().map(entity).collect();
        Box::pin(async move {
            let answer = client(&shared)
                .reorder_pinned(pb::ReorderPinnedRequest {
                    idempotency_key: fresh_key(),
                    order,
                    expected_revision: Some(pb::Revision { value: revision }),
                })
                .await
                .map_err(|status| shared.error(&status, WHAT))?
                .into_inner();
            Ok(list(&answer))
        })
    }

    fn watch(&self) -> Reply<PinnedLists> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.watch_marks().await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_of_thing_survives_the_wire() {
        for thing_in in [
            Thing::Session(SessionId::new("se_1")),
            Thing::Channel(ChannelId::new("ch_1")),
            Thing::Machine(MachineId::new("ma_1")),
            Thing::PrReview(SessionId::new("se_2")),
            Thing::BoardItem("item-1".to_owned()),
        ] {
            assert_eq!(thing(Some(&entity(&thing_in))), Some(thing_in));
        }
    }

    #[test]
    fn a_list_takes_its_entries_in_order_with_the_revision() {
        let wire = pb::ListPinnedResponse {
            entries: vec![
                pb::PinnedEntry {
                    entity: Some(entity(&Thing::BoardItem("a".to_owned()))),
                    title: "A".to_owned(),
                    event: "needs input".to_owned(),
                    event_at: None,
                    vip: true,
                },
                pb::PinnedEntry {
                    entity: Some(entity(&Thing::Channel(ChannelId::new("c")))),
                    title: "C".to_owned(),
                    ..pb::PinnedEntry::default()
                },
            ],
            revision: Some(pb::Revision { value: 7 }),
        };
        let mapped = list(&wire);
        assert_eq!(mapped.revision, 7);
        assert_eq!(
            mapped.items,
            [
                Pinned {
                    thing: Thing::BoardItem("a".to_owned()),
                    title: "A".to_owned(),
                    event: "needs input".to_owned(),
                    vip: true,
                },
                Pinned {
                    thing: Thing::Channel(ChannelId::new("c")),
                    title: "C".to_owned(),
                    event: String::new(),
                    vip: false,
                },
            ]
        );
    }
}
