//! The store: holds the state, runs the reducer for each action on its own
//! thread, runs the effects and tells observers what changed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc;

use crate::effect::Sender;
use crate::reducer::Reducer;
use crate::runner::Runner;

/// Why a store couldn't start.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The threads the store runs on couldn't be made, as when the process
    /// is out of threads or memory. Close other work and try again, or
    /// give the store a runtime with [`Store::on`].
    #[error("couldn't start the store's threads: {source}. Close other work and try again.")]
    Runtime {
        #[source]
        source: std::io::Error,
    },
    /// A [`LocalStore`](crate::LocalStore) was made outside a tokio
    /// runtime, so its effects have nowhere to run. Make it inside one,
    /// or give it a runtime with [`LocalStore::on`](crate::LocalStore::on).
    #[error(
        "couldn't make the store: there's no tokio runtime here for its effects to run on. Make it inside a runtime, or give it one with LocalStore::on."
    )]
    NoRuntime {
        #[source]
        source: tokio::runtime::TryCurrentError,
    },
}

/// Holds a state and changes it only through its reducer, one action at a
/// time, on its own task, so [`Store::send`] never waits. After the
/// actions waiting have been reduced, each observer whose part of the
/// state changed is told, once however many actions there were.
pub struct Store<R: Reducer> {
    shared: Arc<Shared<R::State>>,
    actions: mpsc::UnboundedSender<R::Action>,
    runner: Runner,
    /// The runtime the store made for itself, if it wasn't given one.
    runtime: Option<Runtime>,
}

struct Shared<S> {
    state: RwLock<S>,
    /// Observers added since the store last looked.
    added: Mutex<Vec<Observer<S>>>,
}

type Observer<S> = Box<dyn FnMut(&S) -> bool + Send>;

/// Keeps an observer told of changes; dropping it stops them.
#[must_use = "the observer stops as soon as this is dropped"]
pub struct Observation {
    stopped: Arc<AtomicBool>,
}

impl Observation {
    pub(crate) fn new(stopped: Arc<AtomicBool>) -> Self {
        Self { stopped }
    }

    /// Keeps the observer for as long as the store lives.
    pub fn forever(self) {
        std::mem::forget(self);
    }
}

impl Drop for Observation {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

impl<R> Store<R>
where
    R: Reducer + Send + Sync + 'static,
    R::State: Send + Sync + 'static,
    R::Action: Send + 'static,
{
    /// A store on threads of its own, which stop when it's dropped.
    pub fn new(state: R::State, reducer: R) -> Result<Self, StoreError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("tca-store")
            .enable_all()
            .build()
            .map_err(|source| StoreError::Runtime { source })?;
        let mut store = Self::on(runtime.handle(), state, reducer);
        store.runtime = Some(runtime);
        Ok(store)
    }

    /// A store on `runtime`, which must be running for as long as it is.
    pub fn on(runtime: &Handle, state: R::State, reducer: R) -> Self {
        let shared = Arc::new(Shared {
            state: RwLock::new(state),
            added: Mutex::new(Vec::new()),
        });
        let (actions, receiver) = mpsc::unbounded_channel();
        let runner = Runner::new(runtime.clone());
        // Weak, so the store's own effects don't keep it running once
        // it's dropped.
        let weak = actions.downgrade();
        let send = Sender::new(move |action| {
            if let Some(actions) = weak.upgrade() {
                let _ = actions.send(action);
            }
        });
        runtime.spawn(reduce_all(
            reducer,
            shared.clone(),
            receiver,
            runner.clone(),
            send,
        ));
        Self {
            shared,
            actions,
            runner,
            runtime: None,
        }
    }
}

impl<R: Reducer> Store<R> {
    /// Queues `action` for the reducer, without waiting.
    pub fn send(&self, action: R::Action) {
        let _ = self.actions.send(action);
    }

    /// What `read` makes of the state as it is now.
    pub fn with_state<T>(&self, read: impl FnOnce(&R::State) -> T) -> T {
        read(
            &self
                .shared
                .state
                .read()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    /// Tells `changed` the part of the state `select` picks: now, on this
    /// thread, then on the store's thread each time it's no longer equal
    /// to what it was last told. Only what's selected is compared, so pick
    /// only what the observer shows.
    pub fn observe<V, S, F>(&self, select: S, mut changed: F) -> Observation
    where
        V: PartialEq + Send + 'static,
        S: Fn(&R::State) -> V + Send + 'static,
        F: FnMut(&V) + Send + 'static,
    {
        let stopped = Arc::new(AtomicBool::new(false));
        let mut last = self.with_state(&select);
        changed(&last);
        let still = stopped.clone();
        let observer: Observer<R::State> = Box::new(move |state| {
            if still.load(Ordering::SeqCst) {
                return false;
            }
            let now = select(state);
            if now != last {
                changed(&now);
                last = now;
            }
            true
        });
        self.shared
            .added
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(observer);
        Observation { stopped }
    }
}

impl<R: Reducer> Drop for Store<R> {
    fn drop(&mut self) {
        self.runner.cancel_all();
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// Reduces actions as they come until the store is dropped, then tells
/// the observers once for all that were waiting.
async fn reduce_all<R>(
    reducer: R,
    shared: Arc<Shared<R::State>>,
    mut receiver: mpsc::UnboundedReceiver<R::Action>,
    runner: Runner,
    send: Sender<R::Action>,
) where
    R: Reducer,
    R::Action: Send + 'static,
{
    let mut observers: Vec<Observer<R::State>> = Vec::new();
    let mut waiting = Vec::new();
    while receiver.recv_many(&mut waiting, 256).await > 0 {
        for action in waiting.drain(..) {
            let effect = {
                let mut state = shared.state.write().unwrap_or_else(PoisonError::into_inner);
                reducer.reduce(&mut state, action)
            };
            runner.run(effect, &send);
        }
        observers.append(&mut shared.added.lock().unwrap_or_else(PoisonError::into_inner));
        if observers.is_empty() {
            continue;
        }
        let state = shared.state.read().unwrap_or_else(PoisonError::into_inner);
        observers.retain_mut(|observer| observer(&state));
    }
}
