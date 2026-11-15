//! The clients features use, such as the daemon, the clock or the
//! clipboard, kept by type so the app gives live ones and tests fakes.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

/// Clients by type. The app builds one with its live clients and hands it
/// to its features, which take what they need when they're made; a test
/// builds one with fakes, or overrides one client of the app's with
/// [`with`](Self::with). Clones share the clients.
///
/// Keep clients behind a trait object, such as `Arc<dyn Clock>`, and ask
/// for that type: it's the type a fake is given under.
///
/// ```
/// use std::sync::Arc;
/// use tca::{Clock, Dependencies, SystemClock, TestClock};
///
/// let live = Dependencies::new().with::<Arc<dyn Clock>>(Arc::new(SystemClock::new()));
/// let test = live.clone().with::<Arc<dyn Clock>>(Arc::new(TestClock::new()));
/// let clock: Arc<dyn Clock> = test.get().unwrap();
/// assert!(clock.now().is_zero());
/// ```
#[derive(Clone, Default)]
pub struct Dependencies {
    clients: Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

/// A client a feature asked for wasn't given.
#[derive(Debug, thiserror::Error)]
pub enum DependencyError {
    #[error(
        "no {type_name} was given to the dependencies. Give one with Dependencies::with where they're built: the live client in the app, a fake in a test."
    )]
    Missing { type_name: &'static str },
}

impl Dependencies {
    /// None yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// These, with `client` as the `T`, in place of any given before.
    pub fn with<T: Clone + Send + Sync + 'static>(mut self, client: T) -> Self {
        Arc::make_mut(&mut self.clients).insert(TypeId::of::<T>(), Arc::new(client));
        self
    }

    /// The `T` given.
    ///
    /// # Errors
    ///
    /// [`DependencyError::Missing`] if none was.
    pub fn get<T: Clone + 'static>(&self) -> Result<T, DependencyError> {
        self.clients
            .get(&TypeId::of::<T>())
            .and_then(|client| (**client).downcast_ref::<T>())
            .cloned()
            .ok_or(DependencyError::Missing {
                type_name: std::any::type_name::<T>(),
            })
    }

    /// Whether a `T` was given.
    pub fn has<T: 'static>(&self) -> bool {
        self.clients.contains_key(&TypeId::of::<T>())
    }
}

impl fmt::Debug for Dependencies {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dependencies")
            .field("clients", &self.clients.len())
            .finish()
    }
}
