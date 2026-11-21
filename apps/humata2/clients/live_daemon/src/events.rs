//! What the daemon pushes, and the app's subscribers to it.

use std::sync::Arc;

use humata2_clients::model::{Session, SessionFile};
use humata2_clients::{DaemonError, EVENT_QUEUE, Event, Events, FileChange};
use crate::humata2_grpc::humata::v1 as pb;
use tokio::sync::mpsc;
use tonic::Streaming;

use crate::connection::Shared;
use crate::map;

/// A moving session as last shown, and the move's operation being watched.
pub(crate) struct Move {
    operation: String,
    session: Session,
}

pub(crate) struct Subscriber {
    events: mpsc::Sender<Result<Event, DaemonError>>,
    /// Events were dropped; a Resync goes first when there is room.
    behind: bool,
}

impl Shared {
    /// Pushes an event to every subscriber. One that has fallen behind is
    /// told to reload once it has room.
    pub(crate) fn publish(&self, event: Event) {
        self.state().subscribers.retain_mut(|subscriber| {
            if subscriber.behind {
                match subscriber.events.try_send(Ok(Event::Resync)) {
                    Ok(()) => subscriber.behind = false,
                    Err(mpsc::error::TrySendError::Full(_)) => return true,
                    Err(mpsc::error::TrySendError::Closed(_)) => return false,
                }
            }
            match subscriber.events.try_send(Ok(event.clone())) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    subscriber.behind = true;
                    true
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
        });
    }

    /// The session as the app shows it. While it moves, it carries the move's
    /// step and progress (MV-4, MV-17): the operation is read once, then
    /// watched, and every change is pushed as a session event.
    pub(crate) async fn map_session(self: &Arc<Self>, session: &pb::Session, tabs: &[pb::Tab]) -> Session {
        let mut mapped = map::session(session, tabs, &self.agents);
        let key = mapped.id.0.clone();
        let Some(moving) = mapped.moving.clone() else {
            self.state().moves.remove(&key);
            return mapped;
        };
        let operation = moving.operation.0;
        let known = self
            .state()
            .moves
            .get(&key)
            .filter(|seen| seen.operation == operation)
            .and_then(|seen| seen.session.moving.clone());
        if let Some(known) = known {
            if let Some(now) = mapped.moving.as_mut() {
                now.step = known.step;
                now.done_units = known.done_units;
                now.total_units = known.total_units;
            }
            self.state().moves.insert(
                key,
                Move {
                    operation,
                    session: mapped.clone(),
                },
            );
            return mapped;
        }
        if operation.is_empty() {
            return mapped;
        }
        let read = self
            .operations()
            .get_operation(pb::GetOperationRequest {
                id: Some(pb::OperationId {
                    value: operation.clone(),
                }),
            })
            .await;
        // An operation that can't be read leaves the step empty; the
        // session still reads Moving.
        let Ok(read) = read else {
            return mapped;
        };
        let read = read.into_inner();
        map::with_operation(&mut mapped, &read);
        let fresh = self
            .state()
            .moves
            .insert(
                key.clone(),
                Move {
                    operation: operation.clone(),
                    session: mapped.clone(),
                },
            )
            .is_none_or(|old| old.operation != operation);
        if let (true, true, Some(id)) = (fresh, read.state == pb::OperationState::Running as i32, read.id) {
            self.spawn(watch_move(self.clone(), key, id, read.version));
        }
        mapped
    }

    /// Tells every subscriber the daemon is gone, once, and lets them go.
    fn end_all(&self) {
        end_all_subscribers(&mut self.state(), &self.machine);
    }

    /// A new subscriber, starting the daemon subscription if none runs.
    pub(crate) async fn subscribe(self: &Arc<Self>) -> Result<Events, DaemonError> {
        let (events, receiver) = mpsc::channel(EVENT_QUEUE);
        // Told once the subscription is up, so a load that missed the daemon
        // while it was restarting is redone: a fresh subscription replays
        // nothing of its own.
        let announce = events.clone();
        let start = {
            let mut state = self.state();
            state.subscribers.push(Subscriber {
                events,
                behind: false,
            });
            !std::mem::replace(&mut state.subscribed, true)
        };
        if start {
            let request = pb::SubscribeRequest {
                after: None,
                channels: Vec::new(),
                sessions: Vec::new(),
                topics: vec![
                    pb::Topic::Sessions as i32,
                    pb::Topic::Channels as i32,
                    pb::Topic::Machines as i32,
                    pb::Topic::Inbox as i32,
                    pb::Topic::Boards as i32,
                    pb::Topic::Access as i32,
                    pb::Topic::Settings as i32,
                ],
            };
            match self.event_stream().subscribe(request).await {
                Ok(response) => {
                    self.spawn(follow(self.clone(), response.into_inner()));
                }
                Err(status) => {
                    // SMB-033 + SMB-003 leak: clear `subscribed` and wake/drain
                    // all waiting subscribers (including any second caller that
                    // joined while this subscribe call was in flight, and this
                    // caller's own entry so dead senders never accumulate).
                    self.end_all();
                    return Err(self.error(&status, "Following changes"));
                }
            }
        }
        // The queue is new and empty, so there is always room.
        let _ = announce.try_send(Ok(Event::Resync));
        Ok(receiver)
    }
}

/// Follows a move's operation, pushing the session each time its step or
/// progress changes, until the operation ends or the session stops moving.
async fn watch_move(shared: Arc<Shared>, key: String, id: pb::OperationId, from_version: u64) {
    let opened = shared
        .operations()
        .watch_operation(pb::WatchOperationRequest {
            id: Some(id.clone()),
            from_version,
        })
        .await;
    if let Ok(response) = opened {
        let mut stream = response.into_inner();
        while let Ok(Some(update)) = stream.message().await {
            let told = {
                let mut state = shared.state();
                state
                    .moves
                    .get_mut(&key)
                    .filter(|seen| seen.operation == id.value)
                    .map(|seen| {
                        map::with_operation(&mut seen.session, &update);
                        seen.session.clone()
                    })
            };
            match told {
                Some(session) => shared.publish(Event::Session(session)),
                None => return,
            }
        }
    }
    let mut state = shared.state();
    if state.moves.get(&key).is_some_and(|seen| seen.operation == id.value) {
        state.moves.remove(&key);
    }
}

/// Maps the daemon's events until the stream ends or fails.
async fn follow(shared: Arc<Shared>, mut stream: Streaming<pb::Event>) {
    loop {
        match stream.message().await {
            Ok(Some(event)) => handle(&shared, event).await,
            Ok(None) | Err(_) => {
                shared.end_all();
                return;
            }
        }
    }
}

async fn handle(shared: &Arc<Shared>, event: pb::Event) {
    use pb::event::Payload;
    // Any event may be the first after the daemon published its restart report.
    shared.check_restart();
    match event.payload {
        Some(Payload::Session(session)) => {
            let tabs = match session.id.as_ref() {
                Some(id) => shared.listed_tabs(id).await,
                None => Vec::new(),
            };
            let session = shared.map_session(&session, &tabs).await;
            shared.publish(Event::Session(session));
        }
        Some(Payload::Machine(machine)) => {
            let now = crate::local_time::LocalNow::read();
            shared.publish(Event::Machine(map::machine(&machine, &now)));
        }
        Some(Payload::Request(request)) => {
            let mapped = shared.map_request(&request);
            if request.state == pb::RequestState::Withdrawn as i32 {
                // HOOKS (D191): the agent stopped waiting, so the card goes.
                shared.publish(Event::RequestGone(mapped.id));
            } else {
                shared.publish(Event::Request(mapped));
            }
            let about = match request.preview.as_ref().and_then(|preview| preview.preview.as_ref()) {
                Some(pb::request_preview::Preview::BoardChange(change)) => change.item.clone(),
                _ => None,
            };
            if let Some(item) = about {
                shared.refresh_item(&item).await;
            }
        }
        Some(Payload::BoardItem(wire)) => {
            if event.change == pb::Change::Deleted as i32 {
                shared.publish(Event::ItemGone(humata2_clients::model::ItemId::new(
                    wire.id.map(|id| id.value).unwrap_or_default(),
                )));
            } else {
                shared.publish(Event::Item {
                    item: crate::boards::item(&wire),
                    details: crate::boards::details(&wire),
                });
            }
        }
        Some(Payload::Rules(rules)) => {
            let channel = map::channel_id(rules.channel.as_ref());
            shared.publish(Event::Rules {
                channel,
                rules: shared.map_rules(&rules),
            });
        }
        // A grant was made or removed: read its channel's rules again.
        Some(Payload::Grant(grant)) => {
            let channel = map::channel_id(grant.channel.as_ref());
            match shared.read_rules(&channel).await {
                Ok(rules) => shared.publish(Event::Rules { channel, rules }),
                Err(_) => shared.publish(Event::Resync),
            }
        }
        // Bug 23: a file came or went in a session's clone.
        Some(Payload::FileChanged(changed)) => {
            if let Some(event) = file_event(&changed) {
                shared.publish(event);
            }
        }
        // D438: the history index caught a session up.
        Some(Payload::HistoryIndexed(indexed)) => {
            shared.publish(Event::HistoryIndexed(map::session_id(indexed.session.as_ref())));
        }
        // DESKWATCH: a Desk session changed or left the Desk.
        Some(Payload::Desk(wire)) => {
            if let Some(change) = humata2_clients_live_desk::change(event.change, &wire) {
                shared.publish(Event::Desk(change));
            }
        }
        // MACHINESCRIPTS (D686): a script run on a machine changed; the app reads its runs again.
        Some(Payload::MachineScriptRun(changed)) => {
            let machine = changed.machine.map(|machine| machine.value).unwrap_or_default();
            shared.publish(Event::MachineScriptRun(humata2_clients::model::MachineId::new(machine)));
        }
        // D908: the mount state (and any other setting) changed; Settings reads again.
        Some(Payload::Settings(_)) => shared.publish(Event::SettingsChanged),
        // SETTINGSFIX: a `humata login` arrived or settled/withdrew.
        Some(Payload::PendingCliLogin(login)) => {
            if event.change == pb::Change::Deleted as i32 {
                shared.publish(Event::PendingCliLoginGone(login.id));
            } else {
                shared.publish(Event::PendingCliLogin(crate::cli_auth::pending_login(&login)));
            }
        }
        // The app has no channel event yet: reload instead.
        Some(Payload::Channel(_) | Payload::Resync(_)) => shared.publish(Event::Resync),
        _ => {}
    }
}

/// A file change from the daemon as the app's event.
fn file_event(changed: &pb::FileChanged) -> Option<Event> {
    use pb::file_changed::Kind;
    let session = map::session_id(changed.session.as_ref());
    let repo = changed.repo.as_ref().map(map::repo_text).unwrap_or_default();
    let file = |path: &str| SessionFile {
        repo: repo.clone(),
        path: path.to_owned(),
        folder: changed.folder,
    };
    let change = match Kind::try_from(changed.kind).ok()? {
        Kind::Created => FileChange::Created(file(&changed.path)),
        Kind::Deleted => FileChange::Deleted(file(&changed.path)),
        Kind::Renamed => FileChange::Renamed {
            from: file(&changed.old_path),
            to: file(&changed.path),
        },
        Kind::Unspecified => return None,
    };
    Some(Event::Files { session, change })
}

/// Tells every subscriber the daemon is gone, once, and lets them go.
fn end_all_subscribers(state: &mut crate::state::State, machine: &humata2_clients::model::MachineId) {
    state.subscribed = false;
    for subscriber in state.subscribers.drain(..) {
        let _ = subscriber.events.try_send(Err(DaemonError::Offline {
            machine: machine.clone(),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(kind: pb::file_changed::Kind, path: &str, old_path: &str) -> pb::FileChanged {
        pb::FileChanged {
            session: Some(pb::SessionId {
                value: "se_1".to_owned(),
            }),
            repo: Some(pb::RepoRef {
                owner: "acme".to_owned(),
                name: "api".to_owned(),
            }),
            kind: kind as i32,
            path: path.to_owned(),
            old_path: old_path.to_owned(),
            folder: false,
        }
    }

    #[test]
    fn a_created_file_becomes_a_created_change_in_its_session() {
        let Some(Event::Files { session, change }) = file_event(&wire(pb::file_changed::Kind::Created, "src/new.rs", ""))
        else {
            panic!("a created file is a Files event");
        };
        assert_eq!(session.as_str(), "se_1");
        let FileChange::Created(file) = change else {
            panic!("created stays created");
        };
        assert_eq!((file.repo.as_str(), file.path.as_str(), file.folder), ("acme/api", "src/new.rs", false));
    }

    #[test]
    fn a_renamed_file_carries_where_it_was_and_where_it_is() {
        let Some(Event::Files { change, .. }) = file_event(&wire(pb::file_changed::Kind::Renamed, "b.rs", "a.rs")) else {
            panic!("a rename is a Files event");
        };
        let FileChange::Renamed { from, to } = change else {
            panic!("renamed stays renamed");
        };
        assert_eq!((from.path.as_str(), to.path.as_str()), ("a.rs", "b.rs"));
    }

    #[test]
    fn a_deleted_file_is_a_deleted_change() {
        let Some(Event::Files { change, .. }) = file_event(&wire(pb::file_changed::Kind::Deleted, "a.rs", "")) else {
            panic!("a delete is a Files event");
        };
        assert!(matches!(change, FileChange::Deleted(file) if file.path == "a.rs"));
    }

    #[test]
    fn a_change_of_no_kind_is_dropped_and_no_repo_maps_to_session_folder() {
        assert!(file_event(&wire(pb::file_changed::Kind::Unspecified, "a.rs", "")).is_none());
        let mut root_file = wire(pb::file_changed::Kind::Created, "notes.md", "");
        root_file.repo = None;
        let Some(Event::Files { session, change }) = file_event(&root_file) else {
            panic!("a session-folder file change must become a Files event");
        };
        assert_eq!(session.as_str(), "se_1");
        let FileChange::Created(file) = change else {
            panic!("created stays created");
        };
        assert_eq!((file.repo.as_str(), file.path.as_str(), file.folder), ("", "notes.md", false));
    }

    #[tokio::test]
    async fn end_all_notifies_all_waiting_subscribers_and_drains_queue() {
        // SMB-033 + SMB-003 leak: when a subscribe RPC fails, calling end_all()
        // resets `subscribed`, notifies every queued subscriber (including any
        // second caller that joined while subscribe was in flight) with
        // Offline, and drains `subscribers` so dead senders do not accumulate.
        let mut state = crate::state::State::default();
        let machine = humata2_clients::model::MachineId::new("this-mac");
        let (tx1, mut rx1) = mpsc::channel(4);
        let (tx2, mut rx2) = mpsc::channel(4);
        state.subscribed = true;
        state.subscribers.push(Subscriber {
            events: tx1,
            behind: false,
        });
        state.subscribers.push(Subscriber {
            events: tx2,
            behind: false,
        });

        end_all_subscribers(&mut state, &machine);

        assert!(!state.subscribed, "subscribed must be cleared");
        assert!(state.subscribers.is_empty(), "subscribers must be drained");
        assert!(matches!(rx1.recv().await, Some(Err(DaemonError::Offline { .. }))));
        assert!(matches!(rx2.recv().await, Some(Err(DaemonError::Offline { .. }))));
    }
}
