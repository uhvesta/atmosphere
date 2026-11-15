//! EventService (events.proto): the host's change feed. One ordered
//! sequence; a subscriber gets the kept events after its `after`, then live
//! ones, with nothing missed or repeated between the two. The feed lives in
//! memory (D82): its numbers start from the daemon's start time, so a
//! client resuming from an earlier run is older than anything kept and gets
//! `Resync`, as one that falls too far behind does.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_grpc::humata::v1 as pb;
use humata2_access::Refusal;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::attach::now;
use crate::identity::{self, Caller};
use crate::status;

/// Events kept for subscribers that resume.
const KEPT: usize = 1024;

struct Kept {
    next: u64,
    events: VecDeque<Arc<pb::Event>>,
}

/// The change feed every service publishes to.
pub(crate) struct Feed {
    kept: Mutex<Kept>,
    live: broadcast::Sender<Arc<pb::Event>>,
}

impl Default for Feed {
    fn default() -> Self {
        let start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX / 2))
            .unwrap_or_default();
        Self {
            kept: Mutex::new(Kept {
                next: start.max(1),
                events: VecDeque::new(),
            }),
            live: broadcast::channel(KEPT).0,
        }
    }
}

impl Feed {
    /// Adds one change to the feed, numbered after the last.
    pub(crate) fn publish(&self, change: pb::Change, payload: pb::event::Payload) {
        let mut kept = lock(&self.kept);
        let event = Arc::new(pb::Event {
            seq: Some(pb::EventSeq { value: kept.next }),
            at: Some(now()),
            by: None,
            change: change as i32,
            journal: None,
            payload: Some(payload),
        });
        kept.next += 1;
        if kept.events.len() == KEPT {
            kept.events.pop_front();
        }
        kept.events.push_back(event.clone());
        // Sent under the lock, so a subscriber's backlog and live events
        // meet without a gap. No subscriber is not an error.
        let _ = self.live.send(event);
    }

    /// What a subscriber resuming after `after` gets first, and the live
    /// events from there on.
    fn subscribe(&self, after: Option<u64>) -> (Vec<Arc<pb::Event>>, broadcast::Receiver<Arc<pb::Event>>) {
        let kept = lock(&self.kept);
        let live = self.live.subscribe();
        let Some(after) = after else {
            return (Vec::new(), live);
        };
        let earliest = kept.events.front().map_or(kept.next, |event| seq(event));
        // `after` names an event this feed numbered: the one just before the
        // oldest kept, or later, but not past the newest.
        if after + 1 < earliest || after >= kept.next {
            return (vec![Arc::new(resync(earliest))], live);
        }
        let backlog = kept.events.iter().filter(|event| seq(event) > after).cloned().collect();
        (backlog, live)
    }

    fn earliest(&self) -> u64 {
        let kept = lock(&self.kept);
        kept.events.front().map_or(kept.next, |event| seq(event))
    }
}

fn seq(event: &pb::Event) -> u64 {
    event.seq.as_ref().map_or(0, |seq| seq.value)
}

fn resync(earliest: u64) -> pb::Event {
    pb::Event {
        seq: None,
        at: Some(now()),
        by: None,
        change: pb::Change::Unspecified as i32,
        journal: None,
        payload: Some(pb::event::Payload::Resync(pb::Resync {
            earliest: Some(pb::EventSeq { value: earliest }),
        })),
    }
}

/// Who sees what, and what the request asked for.
struct Filter {
    user: bool,
    machines: bool,
}

impl Filter {
    fn of(caller: &Caller, request: &pb::SubscribeRequest) -> Result<Self, Status> {
        let user = match caller {
            Caller::User(_) => true,
            Caller::Agent(_) => false,
            Caller::Peer => return Err(status::refused(&Refusal::PeerCannotCall)),
        };
        let topics = &request.topics;
        // Machines belong to no channel or session: a feed narrowed to
        // some leaves them out unless it names the topic.
        let machines = topics.contains(&(pb::Topic::Machines as i32))
            || (topics.is_empty() && request.channels.is_empty() && request.sessions.is_empty());
        Ok(Self { user, machines })
    }

    fn passes(&self, event: &pb::Event) -> bool {
        match &event.payload {
            Some(pb::event::Payload::Resync(_)) => true,
            // An agent sees no machines yet (ListMachines lists none).
            Some(pb::event::Payload::Machine(_)) => self.user && self.machines,
            _ => false,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) struct EventSvc {
    feed: Arc<Feed>,
}

impl EventSvc {
    pub(crate) fn new(feed: Arc<Feed>) -> Self {
        Self { feed }
    }
}

#[tonic::async_trait]
impl pb::event_service_server::EventService for EventSvc {
    type SubscribeStream = ReceiverStream<Result<pb::Event, Status>>;

    /// The kept events after `after` the caller may see, then live ones,
    /// until the caller goes. A subscriber that falls more than the kept
    /// events behind gets `Resync` and goes on from the present.
    async fn subscribe(
        &self,
        request: Request<pb::SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let filter = Filter::of(&caller, &request)?;
        let (backlog, mut live) = self.feed.subscribe(request.after.map(|after| after.value));
        let feed = self.feed.clone();
        let (sender, receiver) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut last = 0;
            for event in backlog {
                last = last.max(seq(&event));
                if filter.passes(&event) && sender.send(Ok((*event).clone())).await.is_err() {
                    return;
                }
            }
            loop {
                let event = tokio::select! {
                    event = live.recv() => event,
                    () = sender.closed() => return,
                };
                let event = match event {
                    Ok(event) if seq(&event) <= last => continue,
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        live = live.resubscribe();
                        Arc::new(resync(feed.earliest()))
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                };
                last = last.max(seq(&event));
                if filter.passes(&event) && sender.send(Ok((*event).clone())).await.is_err() {
                    return;
                }
            }
        });
        Ok(Response::new(ReceiverStream::new(receiver)))
    }
}
