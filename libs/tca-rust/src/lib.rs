//! A Rust take on [The Composable Architecture]: a feature's state, the
//! actions that change it, and a reducer that changes the state for each
//! action and returns the work, or [`Effect`], that follows. A [`Store`]
//! runs the reducer on its own thread and tells observers what changed,
//! taking actions from any thread; a [`LocalStore`] stays on the thread
//! that made it and reduces each action before `send` returns; a
//! [`TestStore`] runs it in tests, checking every change.
//!
//! It borrows the ideas, not the Swift: no macros or key paths. A parent
//! reducer hands its children their part of the state with plain code and
//! maps their effects back ([`Effect::map`], [`for_each`], [`if_let`]), and
//! a reducer holds its dependencies itself, so tests build it with fakes.
//!
//! For an app there's more: navigation as state ([`StackState`],
//! [`PresentationState`], [`AlertState`]) run by [`for_each_in_stack`] and
//! [`if_presented`]; one value many features read, [`Shared`]; clients by
//! type in [`Dependencies`]; state saved and restored through a
//! [`Storage`] with [`Persisting`]; and streams as effects with
//! [`Effect::stream`].
//!
//! ```
//! use std::sync::Arc;
//! use std::time::Duration;
//! use tca::{Clock, Effect, Reducer, TestClock, TestStore};
//!
//! #[derive(Clone, Debug, Default, PartialEq)]
//! struct Search { query: String, results: Vec<String> }
//!
//! #[derive(Debug, PartialEq)]
//! enum Action { Typed(String), Found(Vec<String>) }
//!
//! struct Searching { clock: Arc<dyn Clock> }
//!
//! impl Reducer for Searching {
//!     type State = Search;
//!     type Action = Action;
//!     fn reduce(&self, state: &mut Search, action: Action) -> Effect<Action> {
//!         match action {
//!             Action::Typed(query) => {
//!                 state.query = query.clone();
//!                 // Searches once typing pauses.
//!                 Effect::future(async move { Action::Found(vec![format!("{query}.rs")]) })
//!                     .debounce("search", Duration::from_millis(300), self.clock.clone())
//!             }
//!             Action::Found(results) => {
//!                 state.results = results;
//!                 Effect::none()
//!             }
//!         }
//!     }
//! }
//!
//! let clock = TestClock::new();
//! let mut store = TestStore::new(Search::default(), Searching { clock: Arc::new(clock.clone()) });
//! store.send(Action::Typed("ma".into()), |s| s.query = "ma".into());
//! store.send(Action::Typed("main".into()), |s| s.query = "main".into());
//! clock.advance(Duration::from_millis(300));
//! store.receive(Action::Found(vec!["main.rs".into()]), |s| s.results = vec!["main.rs".into()]);
//! ```
//!
//! [The Composable Architecture]: https://github.com/pointfreeco/swift-composable-architecture

mod clock;
mod dependencies;
mod diff;
mod effect;
mod identified;
mod local_store;
mod navigation;
mod persistence;
mod reducer;
mod runner;
mod shared;
mod store;
mod stream;
mod test_store;

pub use clock::{Clock, SystemClock, TestClock};
pub use dependencies::{Dependencies, DependencyError};
pub use effect::{BoxFuture, CancelId, EFFECT_QUEUE, Effect, Sender};
pub use identified::{Change, Identified, IdentifiedVec};
pub use local_store::LocalStore;
pub use navigation::{
    AlertState, ButtonRole, ButtonState, PresentationAction, PresentationState, StackAction,
    StackElementId, StackState, alert, for_each_in_stack, if_presented,
};
pub use persistence::{
    Codec, FileStorage, MemoryStorage, PersistError, Persistence, Persisting, Restored, Storage,
    StorageError,
};
pub use reducer::{FnReducer, Reducer, for_each, if_let, reducer};
pub use shared::Shared;
pub use store::{Observation, Store, StoreError};
pub use stream::{EventStream, StreamEnd};
pub use test_store::{TestFailure, TestStore};
