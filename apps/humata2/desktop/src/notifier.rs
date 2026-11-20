//! Desktop notifications for Humata 2's desktop app (D889).
//!
//! On macOS, [`LiveNotifier`] bridges [`humata2_clients::Notifier`] (called from
//! any thread) onto GPUI's foreground `UNUserNotificationCenter` integration
//! (`gpui_kit::App::show_system_notification`, `dismiss_system_notification`,
//! and `on_system_notification_response`).
//!
//! GPUI's macOS backend guards `UNUserNotificationCenter::currentNotificationCenter()`
//! with `NSBundle::mainBundle().bundleIdentifier().is_some()`, so unbundled dev
//! runs (`bazel run`) and headless tests degrade cleanly to no-op without
//! triggering an ObjC `NSInternalInconsistencyException`.
//!
//! On non-macOS targets, [`LiveNotifier`] degrades to [`NoopNotifier`].

#[cfg(target_os = "macos")]
mod platform {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use gpui_kit::{App, SharedString, SystemNotification};
    use humata2_clients::model::SessionId;
    use humata2_clients::{DesktopNotice, Notifier, NotifyTarget};
    use tokio::sync::mpsc;

    /// Commands queued from [`Notifier`] calls on any thread and drained on the
    /// GPUI foreground thread.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) enum NotifierCommand {
        Post(DesktopNotice),
        Withdraw(Vec<String>),
        RequestPermission,
    }

    /// Platform desktop notifier backed by macOS `UNUserNotificationCenter` via
    /// GPUI's foreground `App` notification API.
    pub(crate) struct LiveNotifier {
        commands: mpsc::UnboundedSender<NotifierCommand>,
        /// Maps each active notification `id` (`SystemNotification::tag`) to its
        /// click navigation target.
        targets: Arc<Mutex<HashMap<String, NotifyTarget>>>,
    }

    /// Foreground half of [`LiveNotifier`], attached to GPUI's [`App`] once the
    /// main window is running.
    pub(crate) struct NotifierBridge {
        commands: mpsc::UnboundedReceiver<NotifierCommand>,
        targets: Arc<Mutex<HashMap<String, NotifyTarget>>>,
    }

    impl LiveNotifier {
        /// Creates a [`LiveNotifier`] and its GPUI foreground [`NotifierBridge`].
        ///
        /// Until [`NotifierBridge::attach`] is called (or if the bridge is
        /// dropped, e.g. in unbundled/headless runs), all [`Notifier`] methods
        /// degrade cleanly to no-op.
        #[must_use]
        pub(crate) fn new() -> (Self, NotifierBridge) {
            let (tx, rx) = mpsc::unbounded_channel();
            let targets = Arc::new(Mutex::new(HashMap::new()));
            let notifier = Self { commands: tx, targets: Arc::clone(&targets) };
            let bridge = NotifierBridge { commands: rx, targets };
            (notifier, bridge)
        }
    }

    impl Notifier for LiveNotifier {
        fn post(&self, notice: DesktopNotice) {
            if let Ok(mut map) = self.targets.lock() {
                map.insert(notice.id.clone(), notice.target.clone());
            }
            let _ = self.commands.send(NotifierCommand::Post(notice));
        }

        fn withdraw(&self, ids: &[String]) {
            if ids.is_empty() {
                return;
            }
            if let Ok(mut map) = self.targets.lock() {
                for id in ids {
                    map.remove(id);
                }
            }
            let _ = self.commands.send(NotifierCommand::Withdraw(ids.to_vec()));
        }

        fn request_permission(&self) {
            let _ = self.commands.send(NotifierCommand::RequestPermission);
        }
    }

    impl NotifierBridge {
        /// Connects this bridge to GPUI's foreground `UNUserNotificationCenter`
        /// integration and returns a channel of clicked [`NotifyTarget`]s.
        ///
        /// Clicking a notification activates the app and emits its
        /// [`NotifyTarget`] on the returned receiver.
        #[allow(dead_code)] // Wired from desktop/src/main.rs in the final seam pass.
        pub(crate) fn attach(mut self, cx: &mut App) -> mpsc::UnboundedReceiver<NotifyTarget> {
            let (clicks_tx, clicks_rx) = mpsc::unbounded_channel();
            let targets = Arc::clone(&self.targets);

            cx.on_system_notification_response(move |response, cx| {
                cx.activate(true);
                let target = resolve_target(&targets, response.tag.as_str());
                if let Some(target) = target {
                    let _ = clicks_tx.send(target);
                }
            });

            cx.spawn(async move |cx| {
                while let Some(command) = self.commands.recv().await {
                    match command {
                        NotifierCommand::Post(notice) => {
                            let notification = to_system_notification(&notice);
                            let sound = notice.sound.map(|s| SharedString::from(s.file_name()));
                            cx.update(|cx| {
                                cx.show_system_notification_with_sound(notification, sound);
                            });
                        }
                        NotifierCommand::Withdraw(ids) => {
                            cx.update(|cx| {
                                for id in &ids {
                                    cx.dismiss_system_notification(id);
                                }
                            });
                        }
                        NotifierCommand::RequestPermission => {
                            cx.update(|cx| cx.request_system_notification_permission());
                        }
                    }
                }
            })
            .detach();

            clicks_rx
        }
    }

    /// Converts a [`DesktopNotice`] to GPUI's [`SystemNotification`].
    pub(crate) fn to_system_notification(notice: &DesktopNotice) -> SystemNotification {
        SystemNotification {
            tag: SharedString::from(notice.id.clone()),
            title: SharedString::from(notice.title.clone()),
            body: SharedString::from(notice.body.clone()),
            actions: Vec::new(),
        }
    }

    /// Looks up the [`NotifyTarget`] for a clicked notification `tag`, falling
    /// back to identifier-prefix routing if the notification outlived an app
    /// restart.
    pub(crate) fn resolve_target(
        targets: &Mutex<HashMap<String, NotifyTarget>>,
        tag: &str,
    ) -> Option<NotifyTarget> {
        if let Ok(map) = targets.lock()
            && let Some(target) = map.get(tag)
        {
            return Some(target.clone());
        }
        fallback_target_for_tag(tag)
    }

    fn fallback_target_for_tag(tag: &str) -> Option<NotifyTarget> {
        if let Some(session_id) = tag.strip_prefix("humata.session.")
            && !session_id.is_empty()
        {
            return Some(NotifyTarget::Session(SessionId::new(session_id)));
        }
        if tag.starts_with("humata.channel.") {
            return Some(NotifyTarget::Inbox);
        }
        if tag.starts_with("humata.restart.") {
            return Some(NotifyTarget::Home);
        }
        if tag.starts_with("humata.todo.") {
            return Some(NotifyTarget::Todo { item: None });
        }
        None
    }

    #[cfg(test)]
    mod tests {
        use humata2_clients::model::{RequestId, SessionId};

        use super::*;

        fn sample_notice(id: &str, target: NotifyTarget) -> DesktopNotice {
            DesktopNotice {
                id: id.to_owned(),
                title: "Agent needs you".to_owned(),
                body: "Allow shell command".to_owned(),
                target,
                sound: None,
            }
        }

        #[test]
        fn post_withdraw_and_request_permission_are_safe_when_bridge_dropped() {
            let (notifier, bridge) = LiveNotifier::new();
            drop(bridge);

            notifier.request_permission();
            notifier.post(sample_notice("humata.session.s1", NotifyTarget::Session(SessionId::new("s1"))));
            notifier.withdraw(&["humata.session.s1".to_owned()]);
        }

        #[test]
        fn post_records_target_and_enqueues_command_and_withdraw_clears_it() {
            let (notifier, mut bridge) = LiveNotifier::new();
            let req_target = NotifyTarget::Request {
                session: SessionId::new("s1"),
                request: RequestId::new("r1"),
            };
            let notice = sample_notice("humata.session.s1", req_target.clone());

            notifier.post(notice.clone());
            assert_eq!(resolve_target(&bridge.targets, "humata.session.s1"), Some(req_target));
            assert_eq!(bridge.commands.try_recv(), Ok(NotifierCommand::Post(notice)));

            // Replacing in place updates the stored target.
            let updated_target = NotifyTarget::Session(SessionId::new("s1"));
            let updated = sample_notice("humata.session.s1", updated_target.clone());
            notifier.post(updated.clone());
            assert_eq!(resolve_target(&bridge.targets, "humata.session.s1"), Some(updated_target));
            assert_eq!(bridge.commands.try_recv(), Ok(NotifierCommand::Post(updated)));

            // Withdrawing removes the stored target and enqueues Withdraw.
            notifier.withdraw(&["humata.session.s1".to_owned()]);
            assert_eq!(
                bridge.commands.try_recv(),
                Ok(NotifierCommand::Withdraw(vec!["humata.session.s1".to_owned()]))
            );
            assert!(bridge.targets.lock().expect("lock").is_empty());
        }

        #[test]
        fn empty_withdraw_is_a_noop() {
            let (notifier, mut bridge) = LiveNotifier::new();
            notifier.withdraw(&[]);
            assert!(bridge.commands.try_recv().is_err());
        }

        #[test]
        fn to_system_notification_maps_fields() {
            let notice = sample_notice("humata.session.s2", NotifyTarget::Inbox);
            let sys = to_system_notification(&notice);
            assert_eq!(sys.tag.as_ref(), "humata.session.s2");
            assert_eq!(sys.title.as_ref(), "Agent needs you");
            assert_eq!(sys.body.as_ref(), "Allow shell command");
            assert!(sys.actions.is_empty());
        }

        #[test]
        fn fallback_target_resolves_known_id_prefixes() {
            let empty = Mutex::new(HashMap::new());
            assert_eq!(
                resolve_target(&empty, "humata.session.abc"),
                Some(NotifyTarget::Session(SessionId::new("abc")))
            );
            assert_eq!(resolve_target(&empty, "humata.channel.c1"), Some(NotifyTarget::Inbox));
            assert_eq!(resolve_target(&empty, "humata.restart.m1"), Some(NotifyTarget::Home));
            assert_eq!(
                resolve_target(&empty, "humata.todo.r1"),
                Some(NotifyTarget::Todo { item: None })
            );
            assert_eq!(resolve_target(&empty, "humata.session."), None);
            assert_eq!(resolve_target(&empty, "other"), None);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use gpui_kit::App;
    use humata2_clients::{DesktopNotice, NoopNotifier, Notifier, NotifyTarget};
    use tokio::sync::mpsc;

    /// Non-macOS fallback notifier backed by [`NoopNotifier`].
    pub(crate) struct LiveNotifier(NoopNotifier);

    /// Non-macOS no-op bridge.
    pub(crate) struct NotifierBridge;

    impl LiveNotifier {
        #[must_use]
        pub(crate) fn new() -> (Self, NotifierBridge) {
            (Self(NoopNotifier), NotifierBridge)
        }
    }

    impl Notifier for LiveNotifier {
        fn post(&self, notice: DesktopNotice) {
            self.0.post(notice);
        }

        fn withdraw(&self, ids: &[String]) {
            self.0.withdraw(ids);
        }

        fn request_permission(&self) {
            self.0.request_permission();
        }
    }

    impl NotifierBridge {
        #[allow(dead_code)] // Wired from desktop/src/main.rs in the final seam pass.
        pub(crate) fn attach(self, _cx: &mut App) -> mpsc::UnboundedReceiver<NotifyTarget> {
            let (_tx, rx) = mpsc::unbounded_channel();
            rx
        }
    }
}

#[allow(unused_imports)] // Re-exported for the final seam pass in desktop/src/main.rs.
pub(crate) use platform::{LiveNotifier, NotifierBridge};
