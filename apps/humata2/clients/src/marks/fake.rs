//! A scripted [`MarksClient`]: it keeps pins and VIP marks the way the host
//! does. Clones share them, so two clones stand for two apps open at once
//! (NV-24). No timers, no real IO.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::mpsc;

use super::{MarksClient, Pinned, PinnedList, PinnedLists, Thing};
use crate::daemon::{DaemonError, Reply};

/// A call a test can read back, one per trait method.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    /// `pinned`.
    Pinned,
    /// `set`.
    Set(Thing, Option<bool>, Option<bool>),
    /// `reorder`, with the revision last.
    Reorder(Vec<Thing>, u64),
    /// `watch`.
    Watch,
}

/// Lists a watcher can fall behind by before the fake drops it.
const WATCH_BUFFER: usize = 64;

#[derive(Default)]
struct Inner {
    order: Vec<Thing>,
    vip: HashSet<Thing>,
    titles: HashMap<Thing, String>,
    events: HashMap<Thing, String>,
    revision: u64,
    watchers: Vec<mpsc::Sender<PinnedList>>,
    failure: Option<DaemonError>,
    calls: Vec<Call>,
}

impl Inner {
    fn list(&self) -> PinnedList {
        let items = self
            .order
            .iter()
            .map(|thing| Pinned {
                thing: thing.clone(),
                title: self.titles.get(thing).cloned().unwrap_or_else(|| id_of(thing)),
                event: self.events.get(thing).cloned().unwrap_or_default(),
                vip: self.vip.contains(thing),
            })
            .collect();
        PinnedList { items, revision: self.revision }
    }

    /// Raises the revision and tells every watcher that is still there.
    fn changed(&mut self) -> PinnedList {
        self.revision += 1;
        let list = self.list();
        self.watchers.retain(|watcher| watcher.try_send(list.clone()).is_ok());
        list
    }
}

fn id_of(thing: &Thing) -> String {
    match thing {
        Thing::Session(id) | Thing::PrReview(id) => id.as_str().to_owned(),
        Thing::Channel(id) => id.as_str().to_owned(),
        Thing::Machine(id) => id.as_str().to_owned(),
        Thing::BoardItem(id) => id.clone(),
    }
}

/// Marks that start empty at revision 0. Clones share them all: a change
/// through one clone is what the others list, and what their watchers are
/// told.
///
/// A change that changes something raises the revision by one and sends
/// the new list to every watcher before its reply comes. A reorder from
/// another revision than the current one, or one that doesn't name each
/// pinned thing exactly once, is refused and changes nothing. A thing's
/// title is its id until [`set_title`](Self::set_title) names it.
#[derive(Clone, Default)]
pub struct FakeMarks {
    shared: Arc<Mutex<Inner>>,
}

impl FakeMarks {
    /// Marks with nothing pinned and nothing VIP.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The name `thing` shows when it is pinned, now and later.
    pub fn set_title(&self, thing: &Thing, title: &str) {
        self.inner().titles.insert(thing.clone(), title.to_owned());
    }

    /// The event `thing` shows when it is pinned (NV-21); empty for none.
    /// Like the daemon, a new event on a pinned thing is a change to what
    /// the list shows: it raises the revision and every watcher is told.
    /// On a thing that isn't pinned it is only kept for when it is.
    pub fn set_event(&self, thing: &Thing, event: &str) {
        let mut inner = self.inner();
        let before = inner.events.insert(thing.clone(), event.to_owned()).unwrap_or_default();
        if before != event && inner.order.contains(thing) {
            inner.changed();
        }
    }

    /// Makes the next call fail with `error`, whichever method it is, and
    /// change nothing. The call is still recorded.
    pub fn fail_next(&self, error: DaemonError) {
        self.inner().failure = Some(error);
    }

    /// The calls made so far through any clone, oldest first.
    #[must_use]
    pub fn calls(&self) -> Vec<Call> {
        self.inner().calls.clone()
    }

    /// Records `call`, then answers with the scripted failure or `run`.
    fn call<T: Send + 'static>(
        &self,
        call: Call,
        run: impl FnOnce(&mut Inner) -> Result<T, DaemonError>,
    ) -> Reply<T> {
        let mut inner = self.inner();
        inner.calls.push(call);
        let result = match inner.failure.take() {
            Some(error) => Err(error),
            None => run(&mut inner),
        };
        Box::pin(std::future::ready(result))
    }
}

fn refused(what: &str, reason: &str) -> DaemonError {
    DaemonError::Refused { what: what.to_owned(), reason: reason.to_owned() }
}

impl MarksClient for FakeMarks {
    fn pinned(&self) -> Reply<PinnedList> {
        self.call(Call::Pinned, |inner| Ok(inner.list()))
    }

    fn set(&self, thing: &Thing, pinned: Option<bool>, vip: Option<bool>) -> Reply<PinnedList> {
        self.call(Call::Set(thing.clone(), pinned, vip), |inner| {
            let mut changed = false;
            match pinned {
                Some(true) if !inner.order.contains(thing) => {
                    inner.order.push(thing.clone());
                    changed = true;
                }
                Some(false) => {
                    if let Some(at) = inner.order.iter().position(|t| t == thing) {
                        inner.order.remove(at);
                        changed = true;
                    }
                }
                _ => {}
            }
            match vip {
                Some(true) => changed |= inner.vip.insert(thing.clone()),
                Some(false) => changed |= inner.vip.remove(thing),
                None => {}
            }
            Ok(if changed { inner.changed() } else { inner.list() })
        })
    }

    fn reorder(&self, order: &[Thing], revision: u64) -> Reply<PinnedList> {
        self.call(Call::Reorder(order.to_vec(), revision), |inner| {
            if revision != inner.revision {
                return Err(refused(
                    "This order",
                    "the pins changed since you read them; load the pins again and redo the move",
                ));
            }
            let named: HashSet<&Thing> = order.iter().collect();
            let same = order.len() == inner.order.len()
                && named.len() == order.len()
                && inner.order.iter().all(|thing| named.contains(thing));
            if !same {
                return Err(refused(
                    "This order",
                    "it must name each pinned thing exactly once; load the pins again and redo the move",
                ));
            }
            inner.order = order.to_vec();
            Ok(inner.changed())
        })
    }

    fn watch(&self) -> Reply<PinnedLists> {
        self.call(Call::Watch, |inner| {
            let (send, receive) = mpsc::channel(WATCH_BUFFER);
            inner.watchers.push(send);
            Ok(receive)
        })
    }
}
