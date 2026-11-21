//! EventService (events.proto): the host's change feed. One ordered
//! sequence; a subscriber gets the kept events after its `after`, then live
//! ones, with nothing missed or repeated between the two. The feed lives in
//! memory (D82): its numbers start from the daemon's start time, so a
//! client resuming from an earlier run is older than anything kept and gets
//! `Resync`, as one that falls too far behind does.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_access::{
    Call, Capability, ChannelId, MachineId, Refusal, SessionId, SessionRef, Target,
};
use humata2_grpc::humata::v1 as pb;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::attach::now;
use crate::guard::{Checked, Guard};
use crate::identity::{self, Caller};
use crate::status;

/// Events kept for subscribers that resume.
const KEPT: usize = 1024;

struct Kept {
    next: u64,
    events: VecDeque<Arc<Published>>,
}

/// The channel and session an event belongs to, for subscriptions
/// narrowed to some.
pub struct Scope {
    pub channel: String,
    pub session: String,
}

/// An event and where it belongs; machines belong nowhere.
pub(crate) struct Published {
    event: Arc<pb::Event>,
    scope: Option<Scope>,
}

impl std::ops::Deref for Published {
    type Target = pb::Event;

    fn deref(&self) -> &pb::Event {
        &self.event
    }
}

/// The change feed every service publishes to.
pub struct Feed {
    kept: Mutex<Kept>,
    live: broadcast::Sender<Arc<Published>>,
    /// Shared `Arc<pb::Event>` broadcast for internal subscribers that do not
    /// need `Scope`, avoiding per-adapter bridge tasks and deep `pb::Event` clones.
    live_events: broadcast::Sender<Arc<pb::Event>>,
    /// Turned to `true` once the daemon stops, which ends every subscription.
    stopping: watch::Sender<bool>,
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
            live_events: broadcast::channel(KEPT).0,
            stopping: watch::channel(false).0,
        }
    }
}

impl Feed {
    /// Ends every subscription, as the daemon stops: each one's stream
    /// finishes, so the app's open subscriptions don't hold up the stop.
    pub(crate) fn end_subscriptions(&self) {
        self.stopping.send_replace(true);
    }

    /// Resolves to `true` once [`Self::end_subscriptions`] has run.
    fn end_watch(&self) -> watch::Receiver<bool> {
        self.stopping.subscribe()
    }

    /// Adds one change to the feed, numbered after the last. `by` is who
    /// made it; unset, the daemon did (D95).
    pub fn publish(
        &self,
        change: pb::Change,
        payload: pb::event::Payload,
        by: Option<pb::Actor>,
    ) {
        self.publish_scoped(change, payload, None, by);
    }

    /// Adds one change of something in `scope`'s channel and session.
    pub fn publish_in(
        &self,
        change: pb::Change,
        payload: pb::event::Payload,
        scope: Scope,
        by: Option<pb::Actor>,
    ) {
        self.publish_scoped(change, payload, Some(scope), by);
    }

    fn publish_scoped(
        &self,
        change: pb::Change,
        payload: pb::event::Payload,
        scope: Option<Scope>,
        by: Option<pb::Actor>,
    ) {
        let mut kept = lock(&self.kept);
        let pb_event = Arc::new(pb::Event {
            seq: Some(pb::EventSeq { value: kept.next }),
            at: Some(now()),
            by,
            change: change as i32,
            journal: None,
            payload: Some(payload),
        });
        let event = Arc::new(Published {
            event: Arc::clone(&pb_event),
            scope,
        });
        kept.next += 1;
        if kept.events.len() == KEPT {
            kept.events.pop_front();
        }
        kept.events.push_back(event.clone());
        // Sent under the lock, so a subscriber's backlog and live events
        // meet without a gap. No subscriber is not an error.
        let _ = self.live.send(event);
        let _ = self.live_events.send(pb_event);
    }

    /// Tells every subscriber to reload now. For a source of changes, such
    /// as read marks, that lost some: the feed can't say which, so every
    /// client must reload. The `Resync` has no number and is not kept, so a
    /// later subscriber doesn't get it.
    pub(crate) fn resync_all(&self) {
        let kept = lock(&self.kept);
        let earliest = kept.events.front().map_or(kept.next, |event| seq(event));
        // Sent under the lock, as publish sends, so it lands at the same
        // place in every subscriber's stream. No subscriber is not an error.
        let synced = resync(earliest);
        let pb_event = Arc::clone(&synced.event);
        let _ = self.live.send(synced);
        let _ = self.live_events.send(pb_event);
    }

    /// Subscribes to live events as shared `Arc<pb::Event>`s, with no extra
    /// bridge task or deep cloning.
    pub fn subscribe_events(&self) -> broadcast::Receiver<Arc<pb::Event>> {
        self.live_events.subscribe()
    }

    /// Subscribes to live events as shared `Arc<pb::Event>`s and returns the
    /// latest kept sequence number after `after` (or `after.unwrap_or(0)` if
    /// none).
    pub(crate) fn subscribe_events_after(
        &self,
        after: Option<u64>,
    ) -> (u64, broadcast::Receiver<Arc<pb::Event>>) {
        let kept = lock(&self.kept);
        let live = self.live_events.subscribe();
        let Some(after) = after else {
            return (0, live);
        };
        let earliest = kept.events.front().map_or(kept.next, |event| seq(event));
        if after + 1 < earliest || after >= kept.next {
            return (after, live);
        }
        let last = kept
            .events
            .iter()
            .filter(|event| seq(event) > after)
            .next_back()
            .map_or(after, |event| seq(event));
        (last, live)
    }

    /// What a subscriber resuming after `after` gets first, and the live
    /// events from there on.
    pub(crate) fn subscribe(
        &self,
        after: Option<u64>,
    ) -> (Vec<Arc<Published>>, broadcast::Receiver<Arc<Published>>) {
        let kept = lock(&self.kept);
        let live = self.live.subscribe();
        let Some(after) = after else {
            return (Vec::new(), live);
        };
        let earliest = kept.events.front().map_or(kept.next, |event| seq(event));
        // `after` names an event this feed numbered: the one just before the
        // oldest kept, or later, but not past the newest.
        if after + 1 < earliest || after >= kept.next {
            return (vec![resync(earliest)], live);
        }
        let backlog = kept
            .events
            .iter()
            .filter(|event| seq(event) > after)
            .cloned()
            .collect();
        (backlog, live)
    }

    fn earliest(&self) -> u64 {
        let kept = lock(&self.kept);
        kept.events.front().map_or(kept.next, |event| seq(event))
    }
}

fn seq(published: &Published) -> u64 {
    published.event.seq.as_ref().map_or(0, |seq| seq.value)
}

fn resync(earliest: u64) -> Arc<Published> {
    Arc::new(Published {
        event: Arc::new(pb::Event {
            seq: None,
            at: Some(now()),
            by: None,
            change: pb::Change::Unspecified as i32,
            journal: None,
            payload: Some(pb::event::Payload::Resync(pb::Resync {
                earliest: Some(pb::EventSeq { value: earliest }),
            })),
        }),
        scope: None,
    })
}

/// Who sees what, and what the request asked for.
struct Filter {
    caller: Caller,
    guard: Arc<Guard>,
    /// This daemon's machine, where the sessions it tells of run.
    machine: String,
    user: bool,
    /// An agent's own session.
    own_session: Option<String>,
    machines: bool,
    views: bool,
    inbox: bool,
    access: bool,
    comments: bool,
    operations: bool,
    session_topic: bool,
    channel_topic: bool,
    boards: bool,
    marks: bool,
    settings: bool,
    channels: Vec<String>,
    sessions: Vec<String>,
}

impl Filter {
    fn of(
        caller: &Caller,
        request: &pb::SubscribeRequest,
        guard: Arc<Guard>,
        machine: String,
    ) -> Result<Self, Status> {
        let (user, own_session) = match caller {
            Caller::User(_) => (true, None),
            Caller::Agent(agent) => (false, Some(agent.session.id.to_string())),
            Caller::Script(_) => return Err(status::script_has_no_event_feed()),
            Caller::Peer => return Err(status::refused(&Refusal::PeerCannotCall)),
        };
        let topics = &request.topics;
        // Machines belong to no channel or session: a feed narrowed to
        // some leaves them out unless it names the topic.
        let machines = topics.contains(&(pb::Topic::Machines as i32))
            || (topics.is_empty() && request.channels.is_empty() && request.sessions.is_empty());
        let views = topics.is_empty() || topics.contains(&(pb::Topic::Views as i32));
        let inbox = topics.is_empty() || topics.contains(&(pb::Topic::Inbox as i32));
        let access = topics.is_empty() || topics.contains(&(pb::Topic::Access as i32));
        let comments = topics.is_empty() || topics.contains(&(pb::Topic::Comments as i32));
        let operations = topics.is_empty() || topics.contains(&(pb::Topic::Operations as i32));
        let session_topic = topics.is_empty() || topics.contains(&(pb::Topic::Sessions as i32));
        let channel_topic = topics.is_empty() || topics.contains(&(pb::Topic::Channels as i32));
        let boards = topics.is_empty() || topics.contains(&(pb::Topic::Boards as i32));
        // Marks belong to no channel or session, as machines.
        let marks = topics.contains(&(pb::Topic::Marks as i32))
            || (topics.is_empty() && request.channels.is_empty() && request.sessions.is_empty());
        // Settings are the user's, and belong to no channel or session.
        let settings = topics.contains(&(pb::Topic::Settings as i32))
            || (topics.is_empty() && request.channels.is_empty() && request.sessions.is_empty());
        Ok(Self {
            caller: caller.clone(),
            guard,
            machine,
            user,
            own_session,
            machines,
            views,
            inbox,
            access,
            comments,
            operations,
            session_topic,
            channel_topic,
            boards,
            marks,
            settings,
            channels: request.channels.iter().map(|id| id.value.clone()).collect(),
            sessions: request.sessions.iter().map(|id| id.value.clone()).collect(),
        })
    }

    /// Whether `scope` is in the channels or sessions asked for, if any.
    fn asked(&self, scope: Option<&Scope>) -> bool {
        if self.channels.is_empty() && self.sessions.is_empty() {
            return true;
        }
        scope.is_some_and(|scope| {
            self.channels.contains(&scope.channel) || self.sessions.contains(&scope.session)
        })
    }

    /// Whether the caller sees what belongs to `scope`'s session: the user
    /// every session's, an agent its own.
    fn sees_session(&self, scope: Option<&Scope>) -> bool {
        self.user
            || scope
                .is_some_and(|scope| self.own_session.as_deref() == Some(scope.session.as_str()))
    }

    /// Whether the caller may read `scope`'s session, as ListComments
    /// checks it: its own always, another as its grants and rules allow
    /// now. A check that would ask the user counts as no.
    fn reads_session(&self, scope: Option<&Scope>) -> bool {
        if self.sees_session(scope) {
            return true;
        }
        let Some(scope) = scope else {
            return false;
        };
        let call = Call::ReadSession(SessionRef {
            id: SessionId::new(scope.session.as_str()),
            channel: ChannelId::new(scope.channel.as_str()),
            machine: MachineId::new(self.machine.as_str()),
        });
        matches!(self.guard.check(&self.caller, &call), Ok(Checked::Allowed))
    }

    /// Whether the caller may read the channel `scope` names, as
    /// ListChannels and GetChannel check it. A check that would ask the
    /// user counts as no.
    fn reads_channel(&self, scope: Option<&Scope>) -> bool {
        let Some(scope) = scope else {
            return false;
        };
        let call = Call::Use {
            capability: Capability::ChannelRead,
            target: Target::Channel(ChannelId::new(scope.channel.as_str())),
        };
        matches!(self.guard.check(&self.caller, &call), Ok(Checked::Allowed))
    }

    fn passes(&self, published: &Published) -> bool {
        let scope = published.scope.as_ref();
        match &published.event.payload {
            Some(pb::event::Payload::Resync(_)) => true,
            // Machine changes go to the user only. An agent lists its
            // channel's machines (TP-1) but isn't told when they change yet.
            Some(pb::event::Payload::Machine(_)) => self.user && self.machines,
            // The user sees offered views; an agent sees its own
            // session's, drafts included (VW-14), as ListViews lists them.
            Some(pb::event::Payload::View(view)) => {
                let sees = if self.user {
                    view.state != pb::ViewState::Draft as i32
                } else {
                    self.sees_session(scope)
                };
                self.views && sees && self.asked(scope)
            }
            // The user sees every request; an agent its own session's, as
            // ListRequests lists them.
            Some(pb::event::Payload::Request(_)) => {
                self.inbox && self.sees_session(scope) && self.asked(scope)
            }
            // Grants likewise, as ListGrants lists them.
            Some(pb::event::Payload::Grant(_)) => {
                self.access && self.sees_session(scope) && self.asked(scope)
            }
            // Review chat notes of every session the caller may read, as
            // ListComments lists them.
            Some(pb::event::Payload::Comment(_)) => {
                self.comments && self.asked(scope) && self.reads_session(scope)
            }
            // Operations: the user's and every session's to the user,
            // an agent its own session's, as GetOperation finds them.
            Some(pb::event::Payload::Operation(_)) => {
                self.operations && self.sees_session(scope) && self.asked(scope)
            }
            // Sessions the caller may read, as ListSessions and GetSession
            // show them.
            Some(pb::event::Payload::Session(_)) => {
                self.session_topic && self.asked(scope) && self.reads_session(scope)
            }
            // Channels the caller may read, as ListChannels shows them.
            Some(pb::event::Payload::Channel(_)) => {
                self.channel_topic && self.asked(scope) && self.reads_channel(scope)
            }
            // Board items: every one to the user; an agent sees those of a
            // channel it may read, as ListItems lists them (D155).
            Some(pb::event::Payload::BoardItem(_)) => {
                self.boards && self.asked(scope) && (self.user || self.reads_channel(scope))
            }
            // Pins and VIP marks are the user's own (NV-20, NV-24).
            Some(pb::event::Payload::Marks(_)) => self.user && self.marks,
            Some(pb::event::Payload::Rules(_)) => self.user && self.access && self.asked(scope),
            Some(pb::event::Payload::Settings(_)) => self.user && self.settings,
            // AWAY: whether this machine has its host, as GetServerInfo says.
            Some(pb::event::Payload::HostLink(_)) => self.user && self.machines,
            // AWAY: a policy copy the machine kept, to the user as HostLink is.
            Some(pb::event::Payload::PolicyCopy(_)) => self.user && self.machines,
            // MACHINESCRIPTS (D686): a machine's script run changed, to the user as Machine is.
            Some(pb::event::Payload::MachineScriptRun(_)) => self.user && self.machines,
            // SETTINGSFIX: a pending `humata login` arrived or settled, to the user on Inbox.
            Some(pb::event::Payload::PendingCliLogin(_)) => self.user && self.inbox,
            // D286: a review's pull request heads moved, to whoever may read
            // the session, like its Session events.
            // Bug 23: a clone's files changed, to whoever may read the session.
            // D438: the history index caught the session up, the same way.
            Some(
                pb::event::Payload::PullRequestHeads(_)
                | pb::event::Payload::FileChanged(_)
                | pb::event::Payload::HistoryIndexed(_)
                // DESKWATCH: a Desk session changed, to whoever may read it.
                | pb::event::Payload::Desk(_),
            ) => {
                self.session_topic && self.asked(scope) && self.reads_session(scope)
            }
            _ => false,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) struct EventSvc {
    feed: Arc<Feed>,
    guard: Arc<Guard>,
    machine: String,
}

impl EventSvc {
    pub(crate) fn new(feed: Arc<Feed>, guard: Arc<Guard>, machine: String) -> Self {
        Self {
            feed,
            guard,
            machine,
        }
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
        let filter = Filter::of(&caller, &request, self.guard.clone(), self.machine.clone())?;
        let (backlog, mut live) = self.feed.subscribe(request.after.map(|after| after.value));
        let feed = self.feed.clone();
        let mut end = self.feed.end_watch();
        let (sender, receiver) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut last = 0;
            for event in backlog {
                last = last.max(seq(&event));
                if filter.passes(&event) && sender.send(Ok((*event.event).clone())).await.is_err() {
                    return;
                }
            }
            loop {
                let event = tokio::select! {
                    event = live.recv() => event,
                    () = sender.closed() => return,
                    _ = end.wait_for(|stopping| *stopping) => return,
                };
                let event = match event {
                    // A Resync has no number, so it always goes on.
                    Ok(event) if event.seq.is_some() && seq(&event) <= last => continue,
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        live = live.resubscribe();
                        resync(feed.earliest())
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                };
                last = last.max(seq(&event));
                if filter.passes(&event) && sender.send(Ok((*event.event).clone())).await.is_err() {
                    return;
                }
            }
        });
        // The task above ends this stream once the daemon is leaving, so
        // the stream isn't cut while it gets there (D270).
        let mut response = Response::new(ReceiverStream::new(receiver));
        response.extensions_mut().insert(crate::stop::streams::EndsByItself);
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D395: when a source of changes loses some, every subscriber is told
    /// to reload, and the subscriber's own stream carries that Resync.
    #[test]
    fn a_resync_for_everyone_reaches_a_subscriber() {
        let feed = Feed::default();
        let (backlog, mut live) = feed.subscribe(None);
        assert!(backlog.is_empty());
        feed.resync_all();
        let event = live
            .try_recv()
            .unwrap_or_else(|error| panic!("a subscriber heard nothing: {error}"));
        assert!(matches!(event.payload, Some(pb::event::Payload::Resync(_))));
    }

    /// D341: a restart's Resync is for those subscribed then; a client
    /// resuming from a point after it hears only the numbered changes.
    #[test]
    fn a_resuming_subscriber_does_not_replay_an_earlier_resync() {
        let feed = Feed::default();
        let (_, mut live) = feed.subscribe(None);
        feed.publish(pb::Change::Created, machine("box-1"), None);
        let first = live.try_recv().unwrap();
        feed.resync_all();
        feed.publish(pb::Change::Updated, machine("box-1"), None);

        let (backlog, _) = feed.subscribe(Some(seq(&first)));

        assert_eq!(backlog.len(), 1);
        assert!(backlog.iter().all(|event| event.event.seq.is_some()));
        assert!(!backlog.iter().any(|event| matches!(event.event.payload, Some(pb::event::Payload::Resync(_)))));
    }

    fn machine(name: &str) -> pb::event::Payload {
        pb::event::Payload::Machine(pb::Machine {
            name: name.to_owned(),
            ..Default::default()
        })
    }
}
