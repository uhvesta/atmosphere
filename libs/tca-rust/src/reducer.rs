//! Reducers: how a state changes for an action, and what work follows.

use std::marker::PhantomData;

use crate::effect::Effect;
use crate::identified::{Identified, IdentifiedVec};

/// Changes a state for an action and returns the work that follows. The
/// reducer itself holds its dependencies, such as a client or a clock, so
/// tests build it with fakes.
///
/// A parent runs its children by handing them their part of its state and
/// its actions for them, and maps their effects back:
///
/// ```
/// use tca::{Effect, Reducer};
///
/// struct Counter;
/// impl Reducer for Counter {
///     type State = i64;
///     type Action = i64;
///     fn reduce(&self, count: &mut i64, by: i64) -> Effect<i64> {
///         *count += by;
///         Effect::none()
///     }
/// }
///
/// #[derive(Default)]
/// struct Two { left: i64, right: i64 }
/// enum TwoAction { Left(i64), Right(i64) }
///
/// struct Both { counter: Counter }
/// impl Reducer for Both {
///     type State = Two;
///     type Action = TwoAction;
///     fn reduce(&self, state: &mut Two, action: TwoAction) -> Effect<TwoAction> {
///         match action {
///             TwoAction::Left(by) => self.counter.reduce(&mut state.left, by).map(TwoAction::Left),
///             TwoAction::Right(by) => self.counter.reduce(&mut state.right, by).map(TwoAction::Right),
///         }
///     }
/// }
/// ```
pub trait Reducer {
    type State;
    type Action;

    fn reduce(&self, state: &mut Self::State, action: Self::Action) -> Effect<Self::Action>;
}

impl<R: Reducer + ?Sized> Reducer for std::sync::Arc<R> {
    type State = R::State;
    type Action = R::Action;

    fn reduce(&self, state: &mut R::State, action: R::Action) -> Effect<R::Action> {
        (**self).reduce(state, action)
    }
}

impl<R: Reducer + ?Sized> Reducer for Box<R> {
    type State = R::State;
    type Action = R::Action;

    fn reduce(&self, state: &mut R::State, action: R::Action) -> Effect<R::Action> {
        (**self).reduce(state, action)
    }
}

/// A reducer from a function.
pub struct FnReducer<S, A, F> {
    reduce: F,
    _types: PhantomData<fn(&mut S, A)>,
}

/// A reducer that calls `reduce`.
pub fn reducer<S, A, F>(reduce: F) -> FnReducer<S, A, F>
where
    F: Fn(&mut S, A) -> Effect<A>,
{
    FnReducer {
        reduce,
        _types: PhantomData,
    }
}

impl<S, A, F> Reducer for FnReducer<S, A, F>
where
    F: Fn(&mut S, A) -> Effect<A>,
{
    type State = S;
    type Action = A;

    fn reduce(&self, state: &mut S, action: A) -> Effect<A> {
        (self.reduce)(state, action)
    }
}

/// Runs `child` on `state` while there is one, as for a sheet that's open.
/// With none, the action is dropped: it came too late, after the state went.
pub fn if_let<R>(child: &R, state: &mut Option<R::State>, action: R::Action) -> Effect<R::Action>
where
    R: Reducer,
    R::Action: Send + 'static,
{
    match state {
        Some(state) => child.reduce(state, action),
        None => Effect::none(),
    }
}

/// Runs `child` on the element of `elements` with `id`, if it's there.
/// Its effects' cancel ids are kept within the element, so two rows'
/// effects of the same name don't cancel each other.
pub fn for_each<R>(
    child: &R,
    elements: &mut IdentifiedVec<R::State>,
    id: &<R::State as Identified>::Id,
    action: R::Action,
) -> Effect<R::Action>
where
    R: Reducer,
    R::State: Identified,
    R::Action: Send + 'static,
{
    elements
        .update(id, |element| child.reduce(element, action))
        .map(|effect| effect.scoped(&format!("{id:?}")))
        .unwrap_or_else(Effect::none)
}
