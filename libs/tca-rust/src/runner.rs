//! Runs the effects reducers return, for the store and the test store.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinSet;

use crate::effect::{BoxFuture, CancelId, Effect, Job, Kind, Sender};

/// Set once its effect is cancelled.
type Token = Arc<watch::Sender<bool>>;

type Cancels = Arc<Mutex<HashMap<CancelId, Vec<Token>>>>;

/// Runs effects on a runtime, keeping which are running under which ids.
#[derive(Clone)]
pub(crate) struct Runner {
    handle: Handle,
    cancels: Cancels,
    running: Arc<AtomicUsize>,
    /// Moved on by [`Runner::cancel_all`]; an effect stops once it moves
    /// past the one it started in, named or not.
    stops: Arc<watch::Sender<u64>>,
}

/// An effect made ready to run: what it does at once is done, and the
/// cancellable parts it starts with are named, so a cancel the reducer
/// returns next stops them even if they haven't started.
enum Ready<A> {
    Job(Job<A>),
    Merge(Vec<Ready<A>>),
    Concat(Vec<Effect<A>>),
    Named {
        id: CancelId,
        token: Token,
        effect: Box<Ready<A>>,
    },
}

impl Runner {
    pub(crate) fn new(handle: Handle) -> Self {
        Self {
            handle,
            cancels: Arc::default(),
            running: Arc::default(),
            stops: Arc::new(watch::channel(0).0),
        }
    }

    /// How many effects are running.
    pub(crate) fn running(&self) -> usize {
        self.running.load(Ordering::SeqCst)
    }

    /// Starts `effect`, its actions sent with `send`. What it does at once,
    /// sending an action or cancelling, is done before this returns.
    pub(crate) fn run<A: Send + 'static>(&self, effect: Effect<A>, send: &Sender<A>) {
        let Some(ready) = prepare(effect, send, &self.cancels) else {
            return;
        };
        let running = Running::start(self.running.clone());
        let send = send.clone();
        let cancels = self.cancels.clone();
        let mut stops = self.stops.subscribe();
        let started = *stops.borrow_and_update();
        self.handle.spawn(async move {
            let _running = running;
            tokio::select! {
                () = drive(ready, send, cancels) => {}
                _ = stops.wait_for(|now| *now != started) => {}
            }
        });
    }

    /// Stops every effect.
    pub(crate) fn cancel_all(&self) {
        self.stops
            .send_modify(|stops| *stops = stops.wrapping_add(1));
        let all = std::mem::take(&mut *lock(&self.cancels));
        for token in all.into_values().flatten() {
            token.send_replace(true);
        }
    }
}

/// Counts an effect as running until dropped, whether it ended or was
/// stopped.
struct Running(Arc<AtomicUsize>);

impl Running {
    fn start(count: Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(count)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn lock(cancels: &Cancels) -> std::sync::MutexGuard<'_, HashMap<CancelId, Vec<Token>>> {
    cancels.lock().unwrap_or_else(PoisonError::into_inner)
}

fn cancel(id: &CancelId, cancels: &Cancels) {
    let tokens = lock(cancels).remove(id);
    for token in tokens.into_iter().flatten() {
        token.send_replace(true);
    }
}

/// Does what `effect` does at once and names what it starts with; nothing
/// is left if it was all done at once.
fn prepare<A: Send + 'static>(
    effect: Effect<A>,
    send: &Sender<A>,
    cancels: &Cancels,
) -> Option<Ready<A>> {
    match effect.kind {
        Kind::None => None,
        Kind::Send(action) => {
            send.send(action);
            None
        }
        Kind::Cancel(id) => {
            cancel(&id, cancels);
            None
        }
        Kind::Run(job) => Some(Ready::Job(job)),
        Kind::Merge(effects) => {
            let ready: Vec<_> = effects
                .into_iter()
                .filter_map(|e| prepare(e, send, cancels))
                .collect();
            (!ready.is_empty()).then_some(Ready::Merge(ready))
        }
        Kind::Concat(effects) => Some(Ready::Concat(effects)),
        Kind::Cancellable {
            id,
            cancel_in_flight,
            effect,
        } => {
            if cancel_in_flight {
                cancel(&id, cancels);
            }
            let token: Token = Arc::new(watch::channel(false).0);
            lock(cancels)
                .entry(id.clone())
                .or_default()
                .push(token.clone());
            let guarded = guard(send, &token);
            match prepare(*effect, &guarded, cancels) {
                Some(ready) => Some(Ready::Named {
                    id,
                    token,
                    effect: Box::new(ready),
                }),
                None => {
                    forget(&id, &token, cancels);
                    None
                }
            }
        }
    }
}

/// A sender that drops what's sent once `token` is set.
fn guard<A: Send + 'static>(send: &Sender<A>, token: &Token) -> Sender<A> {
    let send = send.clone();
    let token = token.clone();
    Sender::new(move |action| {
        if !*token.borrow() {
            send.send(action);
        }
    })
}

fn forget(id: &CancelId, token: &Token, cancels: &Cancels) {
    let mut cancels = lock(cancels);
    if let Some(tokens) = cancels.get_mut(id) {
        tokens.retain(|t| !Arc::ptr_eq(t, token));
        if tokens.is_empty() {
            cancels.remove(id);
        }
    }
}

fn drive<A: Send + 'static>(ready: Ready<A>, send: Sender<A>, cancels: Cancels) -> BoxFuture<()> {
    Box::pin(async move {
        match ready {
            Ready::Job(job) => job(send).await,
            Ready::Merge(all) => {
                // Dropping the set, as cancelling does, stops them all.
                let mut set = JoinSet::new();
                for ready in all {
                    set.spawn(drive(ready, send.clone(), cancels.clone()));
                }
                while set.join_next().await.is_some() {}
            }
            Ready::Concat(effects) => {
                for effect in effects {
                    if let Some(ready) = prepare(effect, &send, &cancels) {
                        drive(ready, send.clone(), cancels.clone()).await;
                    }
                }
            }
            Ready::Named { id, token, effect } => {
                let mut cancelled = token.subscribe();
                let guarded = guard(&send, &token);
                tokio::select! {
                    () = drive(*effect, guarded, cancels.clone()) => {}
                    _ = cancelled.wait_for(|cancelled| *cancelled) => {}
                }
                forget(&id, &token, &cancels);
            }
        }
    })
}
