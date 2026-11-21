//! The desktop notification fake the tests run on.

use std::sync::{Arc, Mutex};

use crate::{DesktopNotice, NoticeSound, Notifier, PermissionStatus};

#[derive(Debug, Default)]
struct State {
    posted: Vec<DesktopNotice>,
    withdrawn: Vec<String>,
    permission_requests: usize,
    permission: PermissionStatus,
    sounds_played: Vec<NoticeSound>,
    dock_badges: Vec<Option<usize>>,
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

    /// Sets the permission status returned by [`Notifier::query_permission`].
    pub fn set_permission(&self, status: PermissionStatus) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.permission = status;
    }

    /// Every sound played via [`Notifier::play_sound`], oldest first.
    #[must_use]
    pub fn sounds_played(&self) -> Vec<NoticeSound> {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.sounds_played.clone()
    }

    /// Every dock badge update passed to [`Notifier::set_dock_badge`], oldest first.
    #[must_use]
    pub fn dock_badges(&self) -> Vec<Option<usize>> {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.dock_badges.clone()
    }

    /// The most recent dock badge update, if any.
    #[must_use]
    pub fn last_dock_badge(&self) -> Option<Option<usize>> {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.dock_badges.last().copied()
    }

    /// Clears all recorded posts, withdrawals, permission requests, sounds,
    /// dock badges, and resets permission to [`PermissionStatus::Unknown`].
    pub fn clear(&self) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.posted.clear();
        state.withdrawn.clear();
        state.permission_requests = 0;
        state.permission = PermissionStatus::Unknown;
        state.sounds_played.clear();
        state.dock_badges.clear();
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

    fn query_permission(&self) -> PermissionStatus {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.permission
    }

    fn play_sound(&self, sound: NoticeSound) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.sounds_played.push(sound);
    }

    fn set_dock_badge(&self, count: Option<usize>) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.dock_badges.push(count);
    }
}

#[cfg(test)]
mod tests {
    use super::FakeNotifier;
    use crate::model::{RequestId, SessionId};
    use crate::{DesktopNotice, NoopNotifier, NoticeSound, Notifier, NotifyTarget, PermissionStatus};

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
            sound: None,
        };
        assert_eq!(notifier.query_permission(), PermissionStatus::Unknown);
        assert_eq!(notifier.last_dock_badge(), None);

        notifier.set_permission(PermissionStatus::Authorized);
        notifier.request_permission();
        notifier.post(notice.clone());
        notifier.withdraw(&["humata.session.s1".to_owned()]);
        notifier.play_sound(NoticeSound::Inbox);
        notifier.set_dock_badge(Some(3));
        notifier.set_dock_badge(None);

        assert_eq!(notifier.query_permission(), PermissionStatus::Authorized);
        assert_eq!(notifier.permission_requests(), 1);
        assert_eq!(notifier.posted(), vec![notice]);
        assert_eq!(notifier.withdrawn(), vec!["humata.session.s1".to_owned()]);
        assert_eq!(notifier.sounds_played(), vec![NoticeSound::Inbox]);
        assert_eq!(notifier.dock_badges(), vec![Some(3), None]);
        assert_eq!(notifier.last_dock_badge(), Some(None));

        notifier.clear();
        assert_eq!(notifier.query_permission(), PermissionStatus::Unknown);
        assert_eq!(notifier.permission_requests(), 0);
        assert!(notifier.posted().is_empty());
        assert!(notifier.withdrawn().is_empty());
        assert!(notifier.sounds_played().is_empty());
        assert!(notifier.dock_badges().is_empty());
        assert_eq!(notifier.last_dock_badge(), None);
    }

    #[test]
    fn noop_notifier_accepts_all_calls() {
        let notifier = NoopNotifier;
        notifier.request_permission();
        assert_eq!(notifier.query_permission(), PermissionStatus::Unknown);
        notifier.play_sound(NoticeSound::Todo);
        notifier.set_dock_badge(Some(1));
        notifier.set_dock_badge(None);
        notifier.post(DesktopNotice {
            id: "id".to_owned(),
            title: "t".to_owned(),
            body: "b".to_owned(),
            target: NotifyTarget::Inbox,
            sound: None,
        });
        notifier.withdraw(&["id".to_owned()]);
    }
}
