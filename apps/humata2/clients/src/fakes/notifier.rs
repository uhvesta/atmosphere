//! The desktop notification fake the tests run on.

use std::sync::{Arc, Mutex};

use crate::{DesktopNotice, Notifier};

#[derive(Debug, Default)]
struct State {
    posted: Vec<DesktopNotice>,
    withdrawn: Vec<String>,
    permission_requests: usize,
}

/// An in-memory notifier that records every post, withdrawal, and permission request.
#[derive(Clone, Debug, Default)]
pub struct FakeNotifier {
    state: Arc<Mutex<State>>,
}

impl FakeNotifier {
    /// Every notice posted so far, oldest first.
    #[must_use]
    pub fn posted(&self) -> Vec<DesktopNotice> {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.posted.clone()
    }

    /// Every notification identifier withdrawn so far, oldest first.
    #[must_use]
    pub fn withdrawn(&self) -> Vec<String> {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.withdrawn.clone()
    }

    /// How many times permission was requested.
    #[must_use]
    pub fn permission_requests(&self) -> usize {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.permission_requests
    }

    /// Clears all recorded posts, withdrawals, and permission requests.
    pub fn clear(&self) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.posted.clear();
        state.withdrawn.clear();
        state.permission_requests = 0;
    }
}

impl Notifier for FakeNotifier {
    fn post(&self, notice: DesktopNotice) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.posted.push(notice);
    }

    fn withdraw(&self, ids: &[String]) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.withdrawn.extend(ids.iter().cloned());
    }

    fn request_permission(&self) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.permission_requests += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::FakeNotifier;
    use crate::model::{RequestId, SessionId};
    use crate::{DesktopNotice, NoopNotifier, Notifier, NotifyTarget};

    #[test]
    fn fake_notifier_records_and_clears() {
        let notifier = FakeNotifier::default();
        let notice = DesktopNotice {
            id: "humata.session.s1".to_owned(),
            title: "claude is stopped: Run tests".to_owned(),
            body: "Open the Agent inbox to answer.".to_owned(),
            target: NotifyTarget::Request {
                session: SessionId::new("s1"),
                request: RequestId::new("r1"),
            },
        };
        notifier.request_permission();
        notifier.post(notice.clone());
        notifier.withdraw(&["humata.session.s1".to_owned()]);

        assert_eq!(notifier.permission_requests(), 1);
        assert_eq!(notifier.posted(), vec![notice]);
        assert_eq!(notifier.withdrawn(), vec!["humata.session.s1".to_owned()]);

        notifier.clear();
        assert_eq!(notifier.permission_requests(), 0);
        assert!(notifier.posted().is_empty());
        assert!(notifier.withdrawn().is_empty());
    }

    #[test]
    fn noop_notifier_accepts_all_calls() {
        let notifier = NoopNotifier;
        notifier.request_permission();
        notifier.post(DesktopNotice {
            id: "id".to_owned(),
            title: "t".to_owned(),
            body: "b".to_owned(),
            target: NotifyTarget::Inbox,
        });
        notifier.withdraw(&["id".to_owned()]);
    }
}
