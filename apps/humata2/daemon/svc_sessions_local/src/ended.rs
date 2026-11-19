//! Session-ended listener registry.

use std::sync::{Arc, Mutex};

type Listener = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Clone, Default)]
pub struct Ended {
    listeners: Arc<Mutex<Vec<Listener>>>,
}

impl Ended {
    pub fn listen(&self, f: impl Fn(&str) + Send + Sync + 'static) {
        self.listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Arc::new(f));
    }

    pub fn tell(&self, session: &str) {
        let list: Vec<Listener> = self
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for f in list {
            f(session);
        }
    }
}
