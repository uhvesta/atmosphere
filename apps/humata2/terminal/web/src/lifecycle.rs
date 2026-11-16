//! Ownership of an embedded application's asynchronous launch.
use std::{cell::Cell, rc::Rc};

#[derive(Clone)]
pub(crate) struct Launch(Rc<Cell<bool>>);
impl Launch {
    pub fn is_current(&self) -> bool {
        self.0.get()
    }
}
#[derive(Default)]
pub(crate) struct Lifecycle {
    current: Option<Launch>,
}
impl Lifecycle {
    /// Repeated starts are idempotent until stopped.
    pub fn start(&mut self) -> Option<Launch> {
        if self.current.is_some() {
            return None;
        }
        let launch = Launch(Rc::new(Cell::new(true)));
        self.current = Some(launch.clone());
        Some(launch)
    }
    pub fn stop(&mut self) {
        if let Some(launch) = self.current.take() {
            launch.0.set(false);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopping_invalidates_an_unfinished_launch_and_allows_one_fresh_launch() {
        let mut lifecycle = Lifecycle::default();
        let old = lifecycle.start().unwrap();
        assert!(old.is_current());
        assert!(lifecycle.start().is_none());
        lifecycle.stop();
        lifecycle.stop();
        assert!(!old.is_current());
        let fresh = lifecycle.start().unwrap();
        assert!(fresh.is_current());
        assert!(!old.is_current());
        assert!(lifecycle.start().is_none());
    }
}
