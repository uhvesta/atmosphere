//! The one place the spike touches tca-rust: a [`LocalStore`] kept on GPUI's
//! main thread, whose sends re-render the view that owns it.
//!
//! tca-rust is still growing, so everything else goes through this module
//! and only this module changes when its API does.
//!
//! The store's effects run on a current-thread tokio runtime that nothing
//! drives, which costs no thread and no CPU while idle. The spike's reducers
//! return no effects. When one does, drive [`LocalStore::run`] from the view
//! (for example with `cx.spawn`) and notify the view after it reduces.

use gpui_kit::Context;
use tca::{LocalStore, Reducer};
use thiserror::Error;
use tokio::runtime::{Builder, Runtime};

/// Why a store could not be made.
#[derive(Debug, Error)]
pub enum StoreBridgeError {
    #[error(
        "could not start the tokio runtime that the view's store runs its effects on: {source}. \
         The process may have run out of threads or file descriptors; close other programs \
         or raise the limit (ulimit -n), then start the spike again"
    )]
    Runtime {
        #[source]
        source: std::io::Error,
    },
}

/// A reducer's state, kept on this thread and changed only by sending it
/// actions.
pub struct StoreBridge<R: Reducer> {
    // Declared before the runtime, so it is dropped first and its effects
    // are cancelled while the runtime still exists.
    store: LocalStore<R>,
    _runtime: Runtime,
}

impl<R> StoreBridge<R>
where
    R: Reducer + 'static,
    R::State: 'static,
    R::Action: Send + 'static,
{
    /// A store holding `state`, changed by `reducer`.
    ///
    /// # Errors
    ///
    /// [`StoreBridgeError::Runtime`] when the effects runtime can't start.
    pub fn new(state: R::State, reducer: R) -> Result<Self, StoreBridgeError> {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|source| StoreBridgeError::Runtime { source })?;
        let store = LocalStore::on(runtime.handle(), state, reducer);
        Ok(Self {
            store,
            _runtime: runtime,
        })
    }

    /// Reduces `action` now and re-renders the view that owns the store.
    pub fn send<V: 'static>(&self, action: R::Action, cx: &mut Context<V>) {
        self.store.send(action);
        cx.notify();
    }

    /// What `read` makes of the state as it is now.
    pub fn with_state<T>(&self, read: impl FnOnce(&R::State) -> T) -> T {
        self.store.with_state(read)
    }
}
