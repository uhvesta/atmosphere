//! Who drops what they keep for a session once it is archived or its
//! folder is freed, such as the PR review's last check of its heads (A8),
//! so what they keep doesn't grow with every session there ever was.

use std::sync::{Arc, Mutex, PoisonError};

type Forget = Box<dyn Fn(&str) + Send + Sync>;

/// The listeners, shared by every clone.
#[derive(Clone, Default)]
pub(crate) struct Ended {
    listeners: Arc<Mutex<Vec<Forget>>>,
}

impl Ended {
    /// Calls `forget` with each session that ends from now on.
    pub(crate) fn listen(&self, forget: impl Fn(&str) + Send + Sync + 'static) {
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Box::new(forget));
    }

    /// Says `session` ended to every listener.
    pub(crate) fn tell(&self, session: &str) {
        let listeners = self
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for forget in listeners.iter() {
            forget(session);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listener_of_any_clone_hears_each_ended_session() {
        let ended = Ended::default();
        let heard = Arc::new(Mutex::new(Vec::new()));
        for listener in ["checks", "other"] {
            let heard = Arc::clone(&heard);
            ended.clone().listen(move |session| {
                heard.lock().unwrap().push(format!("{listener}:{session}"));
            });
        }
        ended.tell("s_1");
        ended.clone().tell("s_2");
        assert_eq!(
            *heard.lock().unwrap(),
            ["checks:s_1", "other:s_1", "checks:s_2", "other:s_2"]
        );
    }
}
