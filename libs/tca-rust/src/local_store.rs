//! A store kept on one thread: sending reduces at once, on the caller's
//! thread, without a channel or a lock.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::{Future, poll_fn};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Poll, Waker};

use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::effect::Sender;
use crate::reducer::Reducer;
use crate::runner::Runner;
use crate::store::{Observation, StoreError};

/// Holds a state on the thread that made it, and changes it only through
/// its reducer. [`send`](Self::send) reduces the action before it returns,
/// along with any actions the reducer sends at once, then tells each
/// observer whose part changed. Observers are told on this thread too.
///
/// Effects still run on a tokio runtime, and what they send later waits
/// for [`run`](Self::run), which reduces it on this thread: spawn it on a
/// [`LocalSet`](tokio::task::LocalSet), or await it beside the rest of
/// the thread's work.
///
/// It can't leave its thread, so the state needn't be `Send`. Use a
/// [`Store`](crate::Store) for a state many threads send to.
///
/// ```compile_fail
/// fn send_to_another_thread<T: Send>(_: T) {}
/// let store = tca::LocalStore::on(
///     &tokio::runtime::Runtime::new().unwrap().handle(),
///     0,
///     tca::reducer(|n: &mut i64, by: i64| { *n += by; tca::Effect::none() }),
/// );
/// send_to_another_thread(store);
/// ```
pub struct LocalStore<R: Reducer> {
    inner: Rc<Inner<R>>,
}

impl<R: Reducer> Clone for LocalStore<R> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

struct Inner<R: Reducer> {
    reducer: R,
    state: RefCell<R::State>,
    observers: RefCell<Vec<Observer<R::State>>>,
    /// Actions sent while reducing or telling observers, reduced next.
    queue: RefCell<VecDeque<R::Action>>,
    /// Set while a send reduces, so a send from inside it only queues.
    reducing: Cell<bool>,
    /// What effects send.
    received: RefCell<mpsc::UnboundedReceiver<R::Action>>,
    /// Keeps the effects' channel open; they hold it only weakly.
    _actions: mpsc::UnboundedSender<R::Action>,
    send: Sender<R::Action>,
    runner: Runner,
    /// Wakes [`LocalStore::run`] to end once the store is gone.
    running: RefCell<Option<Waker>>,
}

type Observer<S> = Box<dyn FnMut(&S) -> bool>;

impl<R> LocalStore<R>
where
    R: Reducer + 'static,
    R::State: 'static,
    R::Action: Send + 'static,
{
    /// A store whose effects run on the runtime this is called in.
    ///
    /// # Errors
    ///
    /// [`StoreError::NoRuntime`] when called outside a tokio runtime.
    pub fn new(state: R::State, reducer: R) -> Result<Self, StoreError> {
        let runtime = Handle::try_current().map_err(|source| StoreError::NoRuntime { source })?;
        Ok(Self::on(&runtime, state, reducer))
    }

    /// A store whose effects run on `runtime`, which must be running for as
    /// long as they do.
    pub fn on(runtime: &Handle, state: R::State, reducer: R) -> Self {
        let (actions, received) = mpsc::unbounded_channel();
        // Weak, so effects still running don't keep the channel open once
        // the store is gone.
        let weak = actions.downgrade();
        let send = Sender::new(move |action| {
            if let Some(actions) = weak.upgrade() {
                let _ = actions.send(action);
            }
        });
        Self {
            inner: Rc::new(Inner {
                reducer,
                state: RefCell::new(state),
                observers: RefCell::new(Vec::new()),
                queue: RefCell::new(VecDeque::new()),
                reducing: Cell::new(false),
                received: RefCell::new(received),
                _actions: actions,
                send,
                runner: Runner::new(runtime.clone()),
                running: RefCell::new(None),
            }),
        }
    }

    /// Reduces `action` now, with what follows from it at once, then tells
    /// the observers. Sent from a reducer's observer, it's reduced right
    /// after the action being reduced.
    pub fn send(&self, action: R::Action) {
        self.inner.queue.borrow_mut().push_back(action);
        self.inner.reduce_all();
    }

    /// What `read` makes of the state as it is now.
    ///
    /// # Panics
    ///
    /// If called from inside the reducer, which has the state.
    pub fn with_state<T>(&self, read: impl FnOnce(&R::State) -> T) -> T {
        read(&self.inner.state.borrow())
    }

    /// Tells `changed` the part of the state `select` picks: now, then each
    /// time a send leaves it no longer equal to what it was last told.
    pub fn observe<V, S, F>(&self, select: S, mut changed: F) -> Observation
    where
        V: PartialEq + 'static,
        S: Fn(&R::State) -> V + 'static,
        F: FnMut(&V) + 'static,
    {
        let mut last = self.with_state(&select);
        changed(&last);
        let stopped = Arc::new(AtomicBool::new(false));
        let still = stopped.clone();
        self.inner
            .observers
            .borrow_mut()
            .push(Box::new(move |state| {
                if still.load(Ordering::SeqCst) {
                    return false;
                }
                let now = select(state);
                if now != last {
                    changed(&now);
                    last = now;
                }
                true
            }));
        Observation::new(stopped)
    }

    /// Reduces what effects send, as it comes, until the store is gone.
    /// Holds the store only weakly, so it doesn't keep it alive.
    pub fn run(&self) -> impl Future<Output = ()> + 'static {
        let store = Rc::downgrade(&self.inner);
        async move {
            while let Some(inner) = next(&store).await {
                inner.reduce_all();
            }
        }
    }
}

/// The store, once an effect has sent it something; None once it's gone.
async fn next<R: Reducer>(store: &Weak<Inner<R>>) -> Option<Rc<Inner<R>>> {
    poll_fn(|cx| {
        let Some(inner) = store.upgrade() else {
            return Poll::Ready(None);
        };
        let polled = inner.received.borrow_mut().poll_recv(cx);
        match polled {
            Poll::Ready(Some(action)) => {
                inner.queue.borrow_mut().push_back(action);
                Poll::Ready(Some(inner))
            }
            // The store keeps the channel open, so it doesn't close first.
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => {
                *inner.running.borrow_mut() = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    })
    .await
}

impl<R> Inner<R>
where
    R: Reducer,
    R::Action: Send + 'static,
{
    /// Reduces every action waiting, then tells the observers, until none
    /// is left. A send from inside only queues; the outer one reduces it.
    fn reduce_all(&self) {
        if self.reducing.replace(true) {
            return;
        }
        // Cleared however this ends, so a reducer that panics doesn't
        // leave every later send only queueing.
        let _reducing = Reducing(&self.reducing);
        loop {
            let mut reduced = false;
            while let Some(action) = self.next_action() {
                let effect = self.reducer.reduce(&mut self.state.borrow_mut(), action);
                self.runner.run(effect, &self.send);
                reduced = true;
            }
            if !reduced {
                break;
            }
            self.tell();
        }
    }

    fn next_action(&self) -> Option<R::Action> {
        let queued = self.queue.borrow_mut().pop_front();
        queued.or_else(|| self.received.borrow_mut().try_recv().ok())
    }

    fn tell(&self) {
        // Taken out, so an observer may observe or send.
        let mut observers = std::mem::take(&mut *self.observers.borrow_mut());
        {
            let state = self.state.borrow();
            observers.retain_mut(|observer| observer(&state));
        }
        let mut current = self.observers.borrow_mut();
        observers.append(&mut current);
        *current = observers;
    }
}

struct Reducing<'a>(&'a Cell<bool>);

impl Drop for Reducing<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

impl<R: Reducer> Drop for Inner<R> {
    fn drop(&mut self) {
        self.runner.cancel_all();
        if let Some(running) = self.running.get_mut().take() {
            running.wake();
        }
    }
}
