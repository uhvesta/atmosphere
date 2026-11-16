//! What the daemon pushes, and the app's subscribers to it.

use std::sync::Arc;

use humata2_clients::model::Status;
use humata2_clients::{DaemonError, EVENT_QUEUE, Event, Events};
use humata2_grpc::humata::v1 as pb;
use tokio::sync::mpsc;
use tonic::Streaming;

use crate::connection::Shared;
use crate::map;

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

    /// Tells every subscriber the daemon is gone, once, and lets them go.
    fn end_all(&self) {
        let mut state = self.state();
        state.subscribed = false;
        for subscriber in state.subscribers.drain(..) {
            let _ = subscriber.events.try_send(Err(DaemonError::Offline {
                machine: self.machine.clone(),
            }));
        }
    }

    /// A new subscriber, starting the daemon subscription if none runs.
    pub(crate) async fn subscribe(self: &Arc<Self>) -> Result<Events, DaemonError> {
        let (events, receiver) = mpsc::channel(EVENT_QUEUE);
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
                ],
            };
            match self.event_stream().subscribe(request).await {
                Ok(response) => {
                    self.spawn(follow(self.clone(), response.into_inner()));
                }
                Err(status) => {
                    self.state().subscribed = false;
                    return Err(self.error(&status, "Following changes"));
                }
            }
        }
        Ok(receiver)
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
    match event.payload {
        Some(Payload::Session(session)) => {
            let tabs = match session.id.as_ref() {
                Some(id) => shared.refresh_tabs(id).await.unwrap_or_default(),
                None => Vec::new(),
            };
            let session = map::session(&session, &tabs, &shared.agents);
            let runs = !matches!(session.status, Status::Exited { .. } | Status::Offline);
            let id = session.id.clone();
            shared.publish(Event::Session(session));
            if runs {
                shared.reattach(&id);
            }
        }
        Some(Payload::Machine(machine)) => shared.publish(Event::Machine(map::machine(&machine))),
        // The app has no channel event yet: reload instead.
        Some(Payload::Channel(_) | Payload::Resync(_)) => shared.publish(Event::Resync),
        _ => {}
    }
}
