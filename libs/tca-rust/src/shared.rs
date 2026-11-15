//! One value many features read without copying, such as settings or the
//! machine list, which tells its readers when it changes.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use tokio::sync::watch;

use crate::effect::Effect;
use crate::store::Observation;

/// A value shared by every clone: a change made through one is seen
/// through all of them at once. Reading hands out the value itself behind
/// an [`Arc`], never a copy; a change makes a new value and leaves readers
/// holding the old one with what they had.
///
/// Features keep a clone in their state. Two are equal when they're
/// clones of each other or hold equal values, so in a
/// [`TestStore`](crate::TestStore), whose expected state is a clone of
/// the state, a change to a shared value doesn't show as a change of
/// state: check it with [`get`](Self::get).
pub struct Shared<T> {
    inner: Arc<Inner<T>>,
}

struct Inner<T> {
    current: RwLock<Current<T>>,
    subscribers: Mutex<Vec<Subscriber<T>>>,
    /// Tells [`Shared::changes`] effects the version, as it changes.
    versions: watch::Sender<u64>,
}

struct Current<T> {
    value: Arc<T>,
    version: u64,
}

struct Subscriber<T> {
    stopped: Arc<AtomicBool>,
    changed: Arc<dyn Fn(&Arc<T>) + Send + Sync>,
}

impl<T> Shared<T> {
    /// `value`, shared, at version 0.
    pub fn new(value: T) -> Self {
        Self {
            inner: Arc::new(Inner {
                current: RwLock::new(Current {
                    value: Arc::new(value),
                    version: 0,
                }),
                subscribers: Mutex::new(Vec::new()),
                versions: watch::channel(0).0,
            }),
        }
    }

    /// The value as it is now.
    pub fn get(&self) -> Arc<T> {
        self.read_current(|current| current.value.clone())
    }

    /// What `read` makes of the value as it is now.
    pub fn with<R>(&self, read: impl FnOnce(&T) -> R) -> R {
        let value = self.get();
        read(&value)
    }

    /// How many times it has changed: equal versions of one shared value
    /// hold the same value.
    pub fn version(&self) -> u64 {
        self.read_current(|current| current.version)
    }

    /// Replaces the value, then tells the subscribers.
    pub fn set(&self, value: T) {
        self.change(|current| current.value = Arc::new(value));
    }

    /// Changes the value in place, copying it first only if someone still
    /// holds it from [`get`](Self::get), then tells the subscribers.
    pub fn update(&self, change: impl FnOnce(&mut T))
    where
        T: Clone,
    {
        self.change(|current| change(Arc::make_mut(&mut current.value)));
    }

    /// Tells `changed` the new value each time it changes, on the thread
    /// that changed it, until the observation is dropped. Not told now:
    /// read [`get`](Self::get) for that.
    pub fn subscribe(&self, changed: impl Fn(&T) + Send + Sync + 'static) -> Observation
    where
        T: 'static,
    {
        let stopped = Arc::new(AtomicBool::new(false));
        self.inner
            .subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Subscriber {
                stopped: stopped.clone(),
                changed: Arc::new(move |value: &Arc<T>| changed(&**value)),
            });
        Observation::new(stopped)
    }

    /// Whether `self` and `other` are clones of one shared value.
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    fn read_current<R>(&self, read: impl FnOnce(&Current<T>) -> R) -> R {
        read(
            &self
                .inner
                .current
                .read()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    fn change(&self, change: impl FnOnce(&mut Current<T>)) {
        let (value, version) = {
            let mut current = self
                .inner
                .current
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            change(&mut current);
            current.version += 1;
            (current.value.clone(), current.version)
        };
        self.inner.versions.send_replace(version);
        // Told outside the locks, so a subscriber may read, change or
        // subscribe.
        let subscribers: Vec<_> = {
            let mut subscribers = self
                .inner
                .subscribers
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            subscribers.retain(|subscriber| !subscriber.stopped.load(Ordering::SeqCst));
            subscribers
                .iter()
                .map(|subscriber| subscriber.changed.clone())
                .collect()
        };
        for changed in subscribers {
            changed(&value);
        }
    }
}

impl<T: Send + Sync + 'static> Shared<T> {
    /// An effect that sends what `changed` makes of the value each time it
    /// changes from now on, for a store to react to a change made
    /// elsewhere. Changes close together may come as one, with the latest
    /// value. It runs until cancelled, so make it
    /// [`cancellable`](Effect::cancellable).
    pub fn changes<A: Send + 'static>(
        &self,
        changed: impl Fn(Arc<T>) -> A + Send + 'static,
    ) -> Effect<A> {
        let mut versions = self.inner.versions.subscribe();
        let shared = self.clone();
        Effect::run(move |send| async move {
            while versions.changed().await.is_ok() {
                send.send(changed(shared.get()));
            }
        })
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: Default> Default for Shared<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: PartialEq> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        self.same(other) || *self.get() == *other.get()
    }
}

impl<T: fmt::Debug> fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Shared").field(&*self.get()).finish()
    }
}
