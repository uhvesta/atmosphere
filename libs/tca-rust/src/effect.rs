//! What a reducer asks to happen after it changes the state: work that
//! sends actions back to the store.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::clock::Clock;

/// A future that can be moved to another thread and outlives its creator.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// Sends an effect's actions back to the store that ran it. Once the store
/// is gone, or the effect is cancelled, what's sent is dropped.
pub struct Sender<A> {
    send: Arc<dyn Fn(A) + Send + Sync>,
}

impl<A> Clone for Sender<A> {
    fn clone(&self) -> Self {
        Self {
            send: self.send.clone(),
        }
    }
}

impl<A: 'static> Sender<A> {
    pub(crate) fn new(send: impl Fn(A) + Send + Sync + 'static) -> Self {
        Self {
            send: Arc::new(send),
        }
    }

    /// Sends `action` to the store.
    pub fn send(&self, action: A) {
        (self.send)(action)
    }

    /// A sender of `B`s that sends each as the `A` that `f` makes of it.
    fn before<B: 'static>(self, f: Arc<dyn Fn(B) -> A + Send + Sync>) -> Sender<B> {
        Sender::new(move |action| self.send(f(action)))
    }
}

/// Names effects so they can be cancelled, or replaced by the next one of
/// the same name.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CancelId(Arc<str>);

impl CancelId {
    /// This id within `scope`, kept apart from the same id in other scopes.
    pub(crate) fn within(&self, scope: &str) -> Self {
        Self(format!("{scope}/{}", self.0).into())
    }
}

impl From<&str> for CancelId {
    fn from(id: &str) -> Self {
        Self(id.into())
    }
}

impl From<String> for CancelId {
    fn from(id: String) -> Self {
        Self(id.into())
    }
}

impl fmt::Debug for CancelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0, f)
    }
}

impl fmt::Display for CancelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Work a reducer returns for its store to run: nothing, an action to send
/// at once, async work that sends actions, or several of these together or
/// in turn, any of them cancellable by name.
#[must_use = "an effect does nothing unless the reducer returns it"]
pub struct Effect<A> {
    pub(crate) kind: Kind<A>,
}

pub(crate) type Job<A> = Box<dyn FnOnce(Sender<A>) -> BoxFuture<()> + Send>;

pub(crate) enum Kind<A> {
    None,
    Send(A),
    Run(Job<A>),
    Merge(Vec<Effect<A>>),
    Concat(Vec<Effect<A>>),
    Cancellable {
        id: CancelId,
        cancel_in_flight: bool,
        effect: Box<Effect<A>>,
    },
    Cancel(CancelId),
}

impl<A> fmt::Debug for Effect<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            Kind::None => f.write_str("Effect::none"),
            Kind::Send(_) => f.write_str("Effect::send"),
            Kind::Run(_) => f.write_str("Effect::run"),
            Kind::Merge(effects) => f.debug_tuple("Effect::merge").field(effects).finish(),
            Kind::Concat(effects) => f.debug_tuple("Effect::concat").field(effects).finish(),
            Kind::Cancellable { id, effect, .. } => f
                .debug_tuple("Effect::cancellable")
                .field(id)
                .field(effect)
                .finish(),
            Kind::Cancel(id) => f.debug_tuple("Effect::cancel").field(id).finish(),
        }
    }
}

impl<A: Send + 'static> Effect<A> {
    /// No work.
    pub fn none() -> Self {
        Self { kind: Kind::None }
    }

    /// Sends `action` to the store as soon as the reducer returns, behind
    /// actions already waiting. Prefer calling the reducer's own code for
    /// it directly; this is for a child telling its parent.
    pub fn send(action: A) -> Self {
        Self {
            kind: Kind::Send(action),
        }
    }

    /// Async work that sends what it finds with the sender it's given.
    pub fn run<F, Fut>(work: F) -> Self
    where
        F: FnOnce(Sender<A>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Self {
            kind: Kind::Run(Box::new(move |send| Box::pin(work(send)))),
        }
    }

    /// Async work that ends in one action.
    pub fn future<Fut>(work: Fut) -> Self
    where
        Fut: Future<Output = A> + Send + 'static,
    {
        Self::run(move |send| async move { send.send(work.await) })
    }

    /// All of `effects` at once.
    pub fn merge(effects: impl IntoIterator<Item = Self>) -> Self {
        let effects: Vec<_> = effects
            .into_iter()
            .filter(|effect| !effect.is_none())
            .collect();
        match effects.len() {
            0 => Self::none(),
            _ => Self {
                kind: Kind::Merge(effects),
            },
        }
    }

    /// Each of `effects` in turn, the next once the last has finished.
    pub fn concat(effects: impl IntoIterator<Item = Self>) -> Self {
        let effects: Vec<_> = effects
            .into_iter()
            .filter(|effect| !effect.is_none())
            .collect();
        match effects.len() {
            0 => Self::none(),
            _ => Self {
                kind: Kind::Concat(effects),
            },
        }
    }

    /// This and `other` at once.
    pub fn and(self, other: Self) -> Self {
        Self::merge([self, other])
    }

    /// This, then `other` once this has finished.
    pub fn then(self, other: Self) -> Self {
        Self::concat([self, other])
    }

    /// This effect, which [`Effect::cancel`] with `id` stops. With
    /// `cancel_in_flight`, it first stops any running with that id, so only
    /// the newest runs: a search as the person types, say.
    pub fn cancellable(self, id: impl Into<CancelId>, cancel_in_flight: bool) -> Self {
        if self.is_none() {
            return self;
        }
        Self {
            kind: Kind::Cancellable {
                id: id.into(),
                cancel_in_flight,
                effect: Box::new(self),
            },
        }
    }

    /// Stops the effects running with `id`. Actions they send from then on
    /// are dropped.
    pub fn cancel(id: impl Into<CancelId>) -> Self {
        Self {
            kind: Kind::Cancel(id.into()),
        }
    }

    /// This effect once `after` has passed on `clock` without another with
    /// the same `id` starting: what's asked for again in the meantime
    /// replaces it.
    pub fn debounce(self, id: impl Into<CancelId>, after: Duration, clock: Arc<dyn Clock>) -> Self {
        Self::run(move |_| async move { clock.sleep(after).await })
            .then(self)
            .cancellable(id, true)
    }

    /// Whether this is no work at all.
    pub fn is_none(&self) -> bool {
        matches!(self.kind, Kind::None)
    }

    /// This effect, its actions made into another kind: a child's effect
    /// as its parent's, say.
    pub fn map<B: Send + 'static>(self, f: impl Fn(A) -> B + Send + Sync + 'static) -> Effect<B> {
        self.map_with(Arc::new(f))
    }

    fn map_with<B: Send + 'static>(self, f: Arc<dyn Fn(A) -> B + Send + Sync>) -> Effect<B> {
        let kind = match self.kind {
            Kind::None => Kind::None,
            Kind::Send(action) => Kind::Send(f(action)),
            Kind::Run(job) => Kind::Run(Box::new(move |send: Sender<B>| job(send.before(f)))),
            Kind::Merge(effects) => {
                Kind::Merge(effects.into_iter().map(|e| e.map_with(f.clone())).collect())
            }
            Kind::Concat(effects) => {
                Kind::Concat(effects.into_iter().map(|e| e.map_with(f.clone())).collect())
            }
            Kind::Cancellable {
                id,
                cancel_in_flight,
                effect,
            } => Kind::Cancellable {
                id,
                cancel_in_flight,
                effect: Box::new(effect.map_with(f)),
            },
            Kind::Cancel(id) => Kind::Cancel(id),
        };
        Effect { kind }
    }

    /// This effect with its cancel ids kept within `scope`, so the same id
    /// in two children, such as two rows of a list, names two effects.
    pub fn scoped(self, scope: &str) -> Self {
        let kind = match self.kind {
            Kind::Merge(effects) => {
                Kind::Merge(effects.into_iter().map(|e| e.scoped(scope)).collect())
            }
            Kind::Concat(effects) => {
                Kind::Concat(effects.into_iter().map(|e| e.scoped(scope)).collect())
            }
            Kind::Cancellable {
                id,
                cancel_in_flight,
                effect,
            } => Kind::Cancellable {
                id: id.within(scope),
                cancel_in_flight,
                effect: Box::new(effect.scoped(scope)),
            },
            Kind::Cancel(id) => Kind::Cancel(id.within(scope)),
            kind => kind,
        };
        Self { kind }
    }
}
