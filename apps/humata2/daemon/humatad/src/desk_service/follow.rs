//! Tells the feed each Desk session's state as it changes (DK-5, CD-1), so a
//! client keeps the Desk by following the feed instead of listing it again.
//!
//! Every write to a Desk session already tells the feed of its Session
//! (start, close, reopen, keep, access, repos, versions, move, a channel Desk
//! close). This reads the Desk row of each such session and publishes it as a
//! Desk event: the session as ListDeskSessions shows it, or, when it is no
//! longer on the Desk (deleted, or moved into a channel), a deletion with only
//! its id. A session on another machine is told as well: its machine's report
//! of the change makes the host's Session event, and this reads what the
//! machine holds of it, as the list does.

use std::collections::HashSet;
use std::sync::Arc;

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::DESK_CHANNEL;
use humata2_store::StoreError;
use tokio::sync::broadcast::error::RecvError;

use super::DeskSvc;
use crate::channel_service::blocking;
use crate::events::{Feed, Scope};

/// A Desk session read back from the store.
struct Seen {
    /// The channel it is filed under: the real channel, or "desk".
    channel: String,
    wire: pb::DeskSession,
    /// The machine it works on when that isn't this one: what that machine
    /// holds of it (files, repos, versions) is read there, once, because it
    /// just reported a change; nothing polls.
    machine: Option<String>,
    /// The channel to ask that machine about, for a channel's own session.
    asked: Option<String>,
}

/// Starts the follow.
pub(super) fn spawn_follow(svc: DeskSvc, feed: Arc<Feed>) {
    tokio::spawn(async move {
        // Subscribed before the first look, so nothing in between is missed.
        let (_, mut events) = feed.subscribe(None);
        let mut known = known_ids(&svc).await;
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                // Missed some: the sessions known now are the ones to watch.
                Err(RecvError::Lagged(_)) => {
                    known.extend(known_ids(&svc).await);
                    continue;
                }
                Err(RecvError::Closed) => return,
            };
            let Some(pb::event::Payload::Session(session)) = &event.payload else {
                continue;
            };
            let Some(id) = session.id.as_ref().map(|id| id.value.clone()) else {
                continue;
            };
            let channel = session
                .channel
                .as_ref()
                .map(|channel| channel.value.clone())
                .unwrap_or_default();
            let by = event.by.clone();
            tell(&svc, &feed, &mut known, &id, &channel, event.change, by).await;
        }
    });
}

/// Tells that Desk session `session` is gone from the Desk, as a clean-up
/// deleted it.
pub(super) fn tell_gone(svc: &DeskSvc, session: &str) {
    publish(&svc.feed, pb::Change::Deleted, gone(session), DESK_CHANNEL, session, None);
}

/// Every Desk session on this machine that isn't deleted.
async fn known_ids(svc: &DeskSvc) -> HashSet<String> {
    let store = svc.starter.store.clone();
    let read = blocking(move || -> Result<HashSet<String>, StoreError> {
        let mut ids: HashSet<String> = store
            .desks()?
            .into_iter()
            .filter(|row| row.deleted_unix_ms.is_none())
            .map(|row| row.session_id)
            .collect();
        ids.extend(store.channel_desks()?.into_iter().map(|row| row.session_id));
        Ok(ids)
    })
    .await;
    match read {
        Ok(Ok(ids)) => ids,
        Ok(Err(error)) => {
            crate::log::say!("humatad2: couldn't list the Desk sessions to follow: {error}");
            HashSet::new()
        }
        Err(_) => HashSet::new(),
    }
}

/// Publishes what the Desk now holds of `id`, which the feed just told of.
async fn tell(
    svc: &DeskSvc,
    feed: &Feed,
    known: &mut HashSet<String>,
    id: &str,
    event_channel: &str,
    change: i32,
    by: Option<pb::Actor>,
) {
    let (reader, key) = (svc.clone(), id.to_owned());
    match blocking(move || read(&reader, &key)).await {
        Ok(Ok(Some(seen))) => {
            let wire = match seen.machine {
                Some(machine) => {
                    let asked = pb::ListDeskSessionsRequest {
                        channel: seen.asked.map(|value| pb::ChannelId { value }),
                        target: None,
                    };
                    let shown = svc.fill_remote(vec![(Some(machine), seen.wire.clone())], asked).await;
                    shown.into_iter().next().unwrap_or(seen.wire)
                }
                None => seen.wire,
            };
            known.insert(id.to_owned());
            let change = if change == pb::Change::Deleted as i32 {
                pb::Change::Deleted
            } else if change == pb::Change::Created as i32 {
                pb::Change::Created
            } else {
                pb::Change::Updated
            };
            publish(feed, change, wire, &seen.channel, id, by);
        }
        // Not on the Desk any more: deleted, or moved into a channel.
        Ok(Ok(None)) => {
            if known.remove(id) {
                publish(feed, pb::Change::Deleted, gone(id), event_channel, id, by);
            }
        }
        Ok(Err(error)) => {
            crate::log::say!("humatad2: couldn't read the Desk session {id} to tell of it: {error}");
        }
        Err(_) => {}
    }
}

/// The Desk session `id` as the Desk shows it, or None when it isn't on this
/// machine's Desk. Blocking: it reads the folder.
fn read(svc: &DeskSvc, id: &str) -> Result<Option<Seen>, StoreError> {
    let store = &svc.starter.store;
    let Some(record) = store.session(id)? else {
        return Ok(None);
    };
    let machine = svc.elsewhere(&record);
    if let Some(row) = store.channel_desk(id)? {
        return Ok(Some(Seen {
            channel: row.channel.clone(),
            wire: svc.channel_desk_view(&record, &row),
            machine,
            asked: Some(row.channel),
        }));
    }
    if record.channel != DESK_CHANNEL {
        return Ok(None);
    }
    match store.desk(id)? {
        Some(row) if row.deleted_unix_ms.is_none() => Ok(Some(Seen {
            channel: DESK_CHANNEL.to_owned(),
            wire: super::view::desk_pb(&record, &row),
            machine,
            asked: None,
        })),
        _ => Ok(None),
    }
}

/// What a deletion carries: only the session.
fn gone(id: &str) -> pb::DeskSession {
    pb::DeskSession {
        session: Some(pb::SessionId {
            value: id.to_owned(),
        }),
        ..pb::DeskSession::default()
    }
}

fn publish(
    feed: &Feed,
    change: pb::Change,
    wire: pb::DeskSession,
    channel: &str,
    session: &str,
    by: Option<pb::Actor>,
) {
    feed.publish_in(
        change,
        pb::event::Payload::Desk(wire),
        Scope {
            channel: channel.to_owned(),
            session: session.to_owned(),
        },
        by,
    );
}
