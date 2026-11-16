//! MarkService (settings.proto; NV-20, NV-21, NV-24 to NV-26; D170): the
//! user's pins, the order they gave them and their VIP marks, kept on the
//! host in `humata2_marks`'s own file, `<state>/marks.db`.
//!
//! Only the user sets or clears a mark or changes the order
//! (`Capability::MarksChange`, user-only): an agent or a script is refused,
//! never asked about. Anyone may list the pins. Every call that changes
//! something is published to the feed as a `Marks` event (TOPIC_MARKS), so
//! a second client shows the same pins without asking again; a call that
//! changes nothing, or that is refused, publishes nothing.
//!
//! NV-21: a pinned session shows its event ("needs input", "finished",
//! "failed"). The service follows the feed's Session events; when a pinned
//! session's event changes, the marks' revision goes up and a `Marks`
//! event tells every client to read the list again.
//!
//! The file is opened the first time it is needed, so one that can't be
//! used (a full disk, wrong permissions) fails the calls that need it and
//! never stops the daemon.
//!
//! How failures are told (errors.proto):
//! - `MarksError::Stale` is `revision_conflict` (ABORTED) carrying the
//!   revision the list is at now; an unset `expected_revision` skips the
//!   check (GRPC.md §5.3).
//! - `MarksError::OrderMismatch` is `invalid_field` on `order`.
//! - An entity of a kind that can't be marked (a request, a comment, a
//!   view) or no entity is `invalid_field` on `entity`.
//! - `MarksError::Database` is `internal`, naming the file.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use humata2_access::Capability;
use humata2_grpc::humata::v1 as pb;
use humata2_marks::{Event, Marks, MarksError, Thing};
use humata2_store::Store;
use pb::entity_ref::Entity;
use tokio::sync::broadcast::error::RecvError;
use tonic::{Code, Request, Response, Status};

use crate::channel_service::{blocking, timestamp_ms};
use crate::events::Feed;
use crate::guard::Guard;
use crate::identity::{self, Caller};
use crate::status;

/// The marks file and what a call needs to check its caller, name a
/// pinned thing and tell the feed.
pub(crate) struct MarkSvc {
    shared: Arc<Shared>,
}

/// What the service and its feed follower share.
struct Shared {
    /// `<state>/marks.db`.
    path: PathBuf,
    /// The file, once something has opened it.
    marks: OnceLock<Marks>,
    feed: Arc<Feed>,
    /// For a pinned thing's title (NV-21).
    store: Arc<Store>,
    /// Each session's latest event as the feed told it, so a session
    /// pinned after its event shows it at once.
    latest: Mutex<HashMap<String, Option<Event>>>,
}

impl MarkSvc {
    /// `path` is `<state>/marks.db`. Never fails and touches no disk. On a
    /// tokio runtime it starts following the feed's Session events for
    /// NV-21.
    pub(crate) fn new(path: PathBuf, _guard: Arc<Guard>, feed: Arc<Feed>, store: Arc<Store>) -> Self {
        let shared = Arc::new(Shared {
            path,
            marks: OnceLock::new(),
            feed,
            store,
            latest: Mutex::new(HashMap::new()),
        });
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(follow_sessions(Arc::downgrade(&shared)));
        }
        Self { shared }
    }
}

impl Shared {
    /// The marks file, opened now if no call has yet.
    fn marks(&self) -> Result<Marks, MarksError> {
        if let Some(marks) = self.marks.get() {
            return Ok(marks.clone());
        }
        let opened = Marks::open(&self.path)?;
        // Another thread may have opened it first; either copy will do.
        Ok(self.marks.get_or_init(|| opened).clone())
    }

    /// The marks file, unless it was never made: nothing is pinned then,
    /// and nothing needs a file made for it.
    fn marks_if_made(&self) -> Result<Option<Marks>, MarksError> {
        if self.marks.get().is_none() && !self.path.exists() {
            return Ok(None);
        }
        self.marks().map(Some)
    }

    fn latest_of(&self, session: &str) -> Option<Event> {
        self.latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session)
            .cloned()
            .flatten()
    }

    /// Tells the feed that `mark` changed, by `by`.
    fn publish(&self, mark: &humata2_marks::Mark, by: Option<pb::Actor>) {
        self.feed.publish(
            pb::Change::Updated,
            pb::event::Payload::Marks(marks_message(mark)),
            by,
        );
    }

    /// A pinned thing's name: a session's title, a channel's name, else
    /// its id.
    fn title_of(&self, thing: &Thing) -> String {
        let named = match thing {
            Thing::Session(id) | Thing::PrReview(id) => self
                .store
                .session(id)
                .ok()
                .flatten()
                .map(|session| session.title)
                .filter(|title| !title.is_empty()),
            Thing::Channel(id) => self
                .store
                .channel(id)
                .ok()
                .flatten()
                .map(|channel| channel.name)
                .filter(|name| !name.is_empty()),
            Thing::Machine(_) | Thing::BoardItem(_) => None,
        };
        named.unwrap_or_else(|| id_of(thing).to_owned())
    }

    /// The pinned list as ListPinned answers it.
    fn listed(&self, marks: &Marks) -> Result<pb::ListPinnedResponse, MarksError> {
        let (pinned, revision) = marks.pinned()?;
        let events = marks.events()?;
        Ok(self.response(&pinned, revision, &events))
    }

    fn response(
        &self,
        pinned: &[humata2_marks::Mark],
        revision: u64,
        events: &HashMap<Thing, Event>,
    ) -> pb::ListPinnedResponse {
        let entries = pinned
            .iter()
            .map(|mark| {
                let event = events.get(&mark.thing);
                pb::PinnedEntry {
                    entity: Some(entity_of(&mark.thing)),
                    title: self.title_of(&mark.thing),
                    event: event.map(|event| event.text.clone()).unwrap_or_default(),
                    event_at: event.map(|event| timestamp_ms(event.at_unix_ms)),
                    vip: mark.vip,
                }
            })
            .collect();
        pb::ListPinnedResponse {
            entries,
            revision: Some(pb::Revision { value: revision }),
        }
    }
}

/// NV-21: follows the feed's Session events while the service is there,
/// and sets each pinned session's event from them.
async fn follow_sessions(shared: std::sync::Weak<Shared>) {
    let mut live = match shared.upgrade() {
        Some(shared) => shared.feed.subscribe(None).1,
        None => return,
    };
    loop {
        let published = match live.recv().await {
            Ok(published) => published,
            // Events missed while behind: the next ones still count.
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => return,
        };
        let Some(shared) = shared.upgrade() else {
            return;
        };
        let Some(pb::event::Payload::Session(session)) = &published.payload else {
            continue;
        };
        let Some(id) = session.id.as_ref().map(|id| id.value.clone()) else {
            continue;
        };
        let at_unix_ms = published.at.as_ref().map_or(0, unix_ms);
        let event = event_of(session).map(|text| Event {
            text: text.to_owned(),
            at_unix_ms,
        });
        let unchanged = {
            let mut latest = shared.latest.lock().unwrap_or_else(PoisonError::into_inner);
            let same = latest
                .get(&id)
                .is_some_and(|was| was.as_ref().map(|e| &e.text) == event.as_ref().map(|e| &e.text));
            if !same {
                latest.insert(id.clone(), event.clone());
            }
            same
        };
        if unchanged {
            continue;
        }
        let work = shared.clone();
        let changed = blocking(move || -> Result<Option<humata2_marks::Mark>, MarksError> {
            let Some(marks) = work.marks_if_made()? else {
                return Ok(None);
            };
            let thing = Thing::Session(id);
            match marks.set_event(&thing, event.as_ref())? {
                Some(_) => marks.get(&thing).map(Some),
                None => Ok(None),
            }
        })
        .await;
        match changed {
            Ok(Ok(Some(mark))) => shared.publish(&mark, None),
            Ok(Ok(None)) => {}
            Ok(Err(error)) => crate::log::say!(
                "humatad2: couldn't keep a pinned session's event: {}",
                status::chain(&error)
            ),
            Err(error) => crate::log::say!("humatad2: couldn't keep a pinned session's event: {error}"),
        }
    }
}

/// What a session shows in the Pinned section, if anything (NV-21).
fn event_of(session: &pb::Session) -> Option<&'static str> {
    use pb::SessionStatus as S;
    match S::try_from(session.status).ok()? {
        S::NeedsInput => Some("needs input"),
        S::Idle => Some("finished"),
        S::Exited if session.exit_code == 0 => Some("finished"),
        S::Exited | S::ResumeFailed => Some("failed"),
        _ => None,
    }
}

fn unix_ms(at: &humata2_grpc::Timestamp) -> i64 {
    at.seconds
        .saturating_mul(1000)
        .saturating_add(i64::from(at.nanos / 1_000_000))
}

fn id_of(thing: &Thing) -> &str {
    match thing {
        Thing::Session(id)
        | Thing::Channel(id)
        | Thing::Machine(id)
        | Thing::PrReview(id)
        | Thing::BoardItem(id) => id,
    }
}

/// The thing an entity names, or `invalid_field` on `field` for a kind
/// that can't be marked or no entity at all.
fn thing_of(entity: Option<&pb::EntityRef>, field: &str) -> Result<Thing, Status> {
    let unmarkable = |what: &str| {
        status::invalid_field(
            field,
            pb::invalid_field::Problem::NotAllowedHere,
            format!("{what} can't be pinned or marked VIP"),
            "pin or mark VIP a session, a channel, a machine, a PR review or a board item",
        )
    };
    let thing = match entity.and_then(|entity| entity.entity.as_ref()) {
        Some(Entity::Session(id)) => Thing::Session(id.value.clone()),
        Some(Entity::Channel(id)) => Thing::Channel(id.value.clone()),
        Some(Entity::Machine(id)) => Thing::Machine(id.value.clone()),
        Some(Entity::PrReview(id)) => Thing::PrReview(id.value.clone()),
        Some(Entity::BoardItem(id)) => Thing::BoardItem(id.value.clone()),
        Some(_) => return Err(unmarkable("this kind of thing")),
        None => return Err(status::missing(field)),
    };
    if id_of(&thing).is_empty() {
        return Err(status::missing(field));
    }
    Ok(thing)
}

fn entity_of(thing: &Thing) -> pb::EntityRef {
    let entity = match thing {
        Thing::Session(id) => Entity::Session(pb::SessionId { value: id.clone() }),
        Thing::Channel(id) => Entity::Channel(pb::ChannelId { value: id.clone() }),
        Thing::Machine(id) => Entity::Machine(pb::MachineId { value: id.clone() }),
        Thing::PrReview(id) => Entity::PrReview(pb::SessionId { value: id.clone() }),
        Thing::BoardItem(id) => Entity::BoardItem(pb::BoardItemId { value: id.clone() }),
    };
    pb::EntityRef {
        entity: Some(entity),
    }
}

fn marks_message(mark: &humata2_marks::Mark) -> pb::Marks {
    pb::Marks {
        entity: Some(entity_of(&mark.thing)),
        pinned: mark.pinned,
        vip: mark.vip,
        revision: Some(pb::Revision {
            value: mark.revision,
        }),
    }
}

/// A failure of the marks file or of a reorder, as a status.
fn failed(error: &MarksError) -> Status {
    match error {
        MarksError::Stale { current } => status::failure(
            Code::Aborted,
            pb::Error {
                message: error.to_string(),
                fix: "read the pinned list again and reorder what it shows now".to_owned(),
                retry: None,
                kind: Some(pb::error::Kind::RevisionConflict(pb::RevisionConflict {
                    kind: pb::EntityKind::Unspecified as i32,
                    id: String::new(),
                    expected: None,
                    actual: Some(pb::Revision { value: *current }),
                })),
            },
        ),
        MarksError::OrderMismatch { .. } => status::invalid_field(
            "order",
            pb::invalid_field::Problem::NotAllowedHere,
            error.to_string(),
            "read the pinned list again and send each of its items once",
        ),
        MarksError::Database { path, .. } => {
            status::internal(status::chain(error), &path.display().to_string())
        }
    }
}

/// Refuses anyone but the user.
fn user_only(caller: &Caller, what: &str) -> Result<(), Status> {
    if matches!(caller, Caller::User(_)) {
        return Ok(());
    }
    Err(status::user_only_call(
        Capability::MarksChange,
        what,
        "ask the user to pin, unpin or mark it VIP from the app or with `humata pin`",
    ))
}

#[tonic::async_trait]
impl pb::mark_service_server::MarkService for MarkSvc {
    /// NV-20: sets or clears a thing's pin, its VIP mark or both; a flag
    /// left out stays as it is, and with neither the call only reads.
    /// User-only. A new pin goes at the end (NV-25).
    async fn set_marks(
        &self,
        request: Request<pb::SetMarksRequest>,
    ) -> Result<Response<pb::Marks>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller, "pinning or marking VIP")?;
        let by = identity::actor(&caller);
        let request = request.into_inner();
        let thing = thing_of(request.entity.as_ref(), "entity")?;
        let shared = self.shared.clone();
        let (mark, changed) = blocking(move || -> Result<_, MarksError> {
            let marks = shared.marks()?;
            let before = marks.get(&thing)?;
            let mut mark = marks.set(&thing, request.pinned, request.vip)?;
            if mark.pinned
                && !before.pinned
                && let Thing::Session(id) = &thing
                && let Some(event) = shared.latest_of(id)
                && let Some(revision) = marks.set_event(&thing, Some(&event))?
            {
                mark.revision = revision;
            }
            let changed = mark.revision != before.revision;
            Ok((mark, changed))
        })
        .await?
        .map_err(|error| failed(&error))?;
        if changed {
            self.shared.publish(&mark, Some(by));
        }
        Ok(Response::new(marks_message(&mark)))
    }

    /// NV-21: the pinned things in the user's order, and the revision they
    /// were read at.
    async fn list_pinned(
        &self,
        request: Request<pb::ListPinnedRequest>,
    ) -> Result<Response<pb::ListPinnedResponse>, Status> {
        identity::caller(&request)?;
        let shared = self.shared.clone();
        let listed = blocking(move || shared.marks().and_then(|marks| shared.listed(&marks)))
            .await?
            .map_err(|error| failed(&error))?;
        Ok(Response::new(listed))
    }

    /// NV-25: puts the pinned things in a new order. User-only. `order`
    /// names every pinned thing once and nothing else.
    async fn reorder_pinned(
        &self,
        request: Request<pb::ReorderPinnedRequest>,
    ) -> Result<Response<pb::ListPinnedResponse>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller, "reordering the pinned list")?;
        let by = identity::actor(&caller);
        let request = request.into_inner();
        let order = request
            .order
            .iter()
            .map(|entity| thing_of(Some(entity), "order"))
            .collect::<Result<Vec<_>, _>>()?;
        let expected = request.expected_revision.map(|revision| revision.value);
        let shared = self.shared.clone();
        let (listed, moved) = blocking(move || -> Result<_, MarksError> {
            let marks = shared.marks()?;
            let before = marks.pinned()?.1;
            let (pinned, revision) = marks.reorder(&order, expected)?;
            let events = marks.events()?;
            Ok((
                shared.response(&pinned, revision, &events),
                (revision != before).then(|| pinned.into_iter().next()).flatten(),
            ))
        })
        .await?
        .map_err(|error| failed(&error))?;
        // The event names the first pinned thing: its revision is what
        // tells a client its list is out of date.
        if let Some(first) = moved {
            self.shared.publish(&first, Some(by));
        }
        Ok(Response::new(listed))
    }
}
