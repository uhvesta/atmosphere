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
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{Arc, Mutex};

    use gpui_kit::{App, SharedString, SystemNotification};
    use humata2_clients::model::SessionId;
    use humata2_clients::{DesktopNotice, NoticeSound, Notifier, NotifyTarget, PermissionStatus};
    use tokio::sync::mpsc;

    /// Logs a desktop-notifier gate event to stderr unless `HUMATA2_NOTIFY_TRACE=0`.
    pub(crate) fn trace_notifier(gate: &str, detail: &str) {
        if std::env::var("HUMATA2_NOTIFY_TRACE").ok().as_deref() == Some("0") {
            return;
        }
        eprintln!("[humata2::notify] gate={gate} {detail}");
    }

    /// macOS `UNUserNotificationCenter` authorization state probed from the OS.
    pub(crate) type OsPermissionStatus = PermissionStatus;

    /// Extracts `CFBundleIdentifier` from `<App>.app/Contents/Info.plist` when
    /// `exe` lives at `<App>.app/Contents/MacOS/<exe>`.
    #[must_use]
    pub(crate) fn bundle_identifier_for_exe(exe: &Path) -> Option<String> {
        let macos_dir = exe.parent()?;
        if macos_dir.file_name().and_then(|n| n.to_str()) != Some("MacOS") {
            return None;
        }
        let contents_dir = macos_dir.parent()?;
        if contents_dir.file_name().and_then(|n| n.to_str()) != Some("Contents") {
            return None;
        }
        let app_dir = contents_dir.parent()?;
        if app_dir.extension().and_then(|e| e.to_str()) != Some("app") {
            return None;
        }
        let plist = std::fs::read_to_string(contents_dir.join("Info.plist")).ok()?;
        let key_idx = plist.find("<key>CFBundleIdentifier</key>")?;
        let after_key = &plist[key_idx + "<key>CFBundleIdentifier</key>".len()..];
        let start_tag = after_key.find("<string>")?;
        let after_start = &after_key[start_tag + "<string>".len()..];
        let end_tag = after_start.find("</string>")?;
        let id = after_start[..end_tag].trim();
        if id.is_empty() {
            None
        } else {
            Some(id.to_owned())
        }
    }

    /// Parses the JSON output of
    /// `plutil -convert json -o - ~/Library/Preferences/com.apple.ncprefs.plist`
    /// for `bundle_id` without requiring `serde_json`.
    #[must_use]
    pub(crate) fn parse_ncprefs_status(json: &str, bundle_id: &str) -> OsPermissionStatus {
        let Some(apps_key) = json.find("\"apps\"") else {
            return OsPermissionStatus::Unknown;
        };
        let after_key = &json[apps_key + "\"apps\"".len()..];
        let Some(array_start) = after_key.find('[') else {
            return OsPermissionStatus::Unknown;
        };
        let bytes = after_key.as_bytes();
        let mut i = array_start + 1;
        let mut in_string = false;
        let mut escaped = false;
        let mut array_depth: usize = 1;
        let mut obj_depth: usize = 0;
        let mut obj_start: Option<usize> = None;

        while i < bytes.len() {
            let b = bytes[i];
            if in_string {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == b'"' {
                    in_string = false;
                }
                i += 1;
                continue;
            }
            match b {
                b'"' => in_string = true,
                b'[' => array_depth += 1,
                b']' => {
                    array_depth = array_depth.saturating_sub(1);
                    if array_depth == 0 {
                        return OsPermissionStatus::NotDetermined;
                    }
                }
                b'{' => {
                    if array_depth == 1 && obj_depth == 0 {
                        obj_start = Some(i);
                    }
                    obj_depth += 1;
                }
                b'}' => {
                    obj_depth = obj_depth.saturating_sub(1);
                    if array_depth == 1
                        && obj_depth == 0
                        && let Some(start) = obj_start.take()
                    {
                        let entry = &after_key[start..=i];
                        if extract_json_string(entry, "bundle-id").as_deref() == Some(bundle_id) {
                            return match extract_json_u64(entry, "flags") {
                                Some(flags) if flags & 0x1000 != 0 => {
                                    OsPermissionStatus::Authorized
                                }
                                Some(flags) if flags & 0x2000 != 0 => {
                                    OsPermissionStatus::Provisional
                                }
                                Some(_) => OsPermissionStatus::Denied,
                                None => OsPermissionStatus::Unknown,
                            };
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
        OsPermissionStatus::Unknown
    }

    fn extract_json_string(object: &str, key: &str) -> Option<String> {
        let needle = format!("\"{key}\"");
        let key_idx = object.find(&needle)?;
        let rest = object[key_idx + needle.len()..].trim_start();
        let rest = rest.strip_prefix(':')?.trim_start();
        let rest = rest.strip_prefix('"')?;
        let mut out = String::new();
        let mut chars = rest.chars();
        while let Some(ch) = chars.next() {
            match ch {
                '\\' => {
                    if let Some(esc) = chars.next() {
                        out.push(esc);
                    }
                }
                '"' => return Some(out),
                _ => out.push(ch),
            }
        }
        None
    }

    fn extract_json_u64(object: &str, key: &str) -> Option<u64> {
        let needle = format!("\"{key}\"");
        let key_idx = object.find(&needle)?;
        let rest = object[key_idx + needle.len()..].trim_start();
        let rest = rest.strip_prefix(':')?.trim_start();
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            None
        } else {
            digits.parse::<u64>().ok()
        }
    }

    /// Probes macOS notification authorization status for the running app bundle.
    #[must_use]
    pub(crate) fn probe_macos_permission_status() -> OsPermissionStatus {
        let exe = match std::env::current_exe() {
            Ok(path) => path,
            Err(err) => {
                trace_notifier("permission_probe", &format!("status=unknown err={err}"));
                return OsPermissionStatus::Unknown;
            }
        };
        let Some(bundle_id) = bundle_identifier_for_exe(&exe) else {
            trace_notifier(
                "permission_probe",
                &format!("status=unbundled exe={}", exe.display()),
            );
            return OsPermissionStatus::Unbundled;
        };
        let Some(home) = std::env::var_os("HOME") else {
            trace_notifier("permission_probe", "status=unknown reason=no_home_env");
            return OsPermissionStatus::Unknown;
        };
        let plist_path = PathBuf::from(home)
            .join("Library")
            .join("Preferences")
            .join("com.apple.ncprefs.plist");
        let output = match Command::new("/usr/bin/plutil")
            .args(["-convert", "json", "-o", "-"])
            .arg(&plist_path)
            .output()
        {
            Ok(out) => out,
            Err(err) => {
                trace_notifier(
                    "permission_probe",
                    &format!("status=unknown bundle_id={bundle_id} plutil_err={err}"),
                );
                return OsPermissionStatus::Unknown;
            }
        };
        if !output.status.success() {
            trace_notifier(
                "permission_probe",
                &format!(
                    "status=unknown bundle_id={bundle_id} plutil_exit={}",
                    output.status
                ),
            );
            return OsPermissionStatus::Unknown;
        }
        let json = String::from_utf8_lossy(&output.stdout);
        let status = parse_ncprefs_status(&json, &bundle_id);
        trace_notifier(
            "permission_probe",
            &format!("bundle_id={bundle_id} status={status:?}"),
        );
        status
    }

    /// Resolves a bundled notification sound `.wav` file relative to `exe`.
    ///
    /// Checks `<App>.app/Contents/Resources/<file>` first, then `<exe_dir>/<file>`.
    #[must_use]
    pub(crate) fn resolve_sound_path(exe: &Path, sound: NoticeSound) -> Option<PathBuf> {
        let file_name = sound.file_name();
        let exe_dir = exe.parent()?;
        if exe_dir.file_name().and_then(|n| n.to_str()) == Some("MacOS")
            && let Some(contents_dir) = exe_dir.parent()
            && contents_dir.file_name().and_then(|n| n.to_str()) == Some("Contents")
        {
            let resource = contents_dir.join("Resources").join(file_name);
            if resource.is_file() {
                return Some(resource);
            }
        }
        let sibling = exe_dir.join(file_name);
        if sibling.is_file() {
            return Some(sibling);
        }
        None
    }

    fn play_sound_in_app(sound: NoticeSound) {
        let path = std::env::current_exe()
            .ok()
            .and_then(|exe| resolve_sound_path(&exe, sound));
        let Some(path) = path else {
            trace_notifier(
                "play_sound_in_app",
                &format!("sound={} status=missing_file", sound.file_name()),
            );
            return;
        };
        match Command::new("/usr/bin/afplay").arg(&path).spawn() {
            Ok(_) => {
                trace_notifier(
                    "play_sound_in_app",
                    &format!("sound={} path={} status=spawned", sound.file_name(), path.display()),
                );
            }
            Err(err) => {
                trace_notifier(
                    "play_sound_in_app",
                    &format!("sound={} path={} status=spawn_err err={err}", sound.file_name(), path.display()),
                );
            }
        }
    }

    /// Formats the macOS Dock tile badge label for `count`.
    /// `None` or `Some(0)` clears the badge (`None` -> `nil`).
    #[must_use]
    pub(crate) fn dock_badge_label(count: Option<usize>) -> Option<String> {
        count.filter(|&n| n > 0).map(|n| n.to_string())
    }

    /// Decides how a `NotifierCommand::Post` plays its sound:
    /// - Bundled `.app` runs pass the sound file name to macOS
    ///   `UNUserNotificationCenter` (`show_system_notification_with_sound`) so
    ///   macOS Focus / Do Not Disturb is respected.
    /// - Unbundled dev builds fall back to `play_sound_in_app` because
    ///   `UNUserNotificationCenter` is a no-op without a bundle identifier.
    #[must_use]
    pub(crate) fn post_sound_plan(
        is_bundled: bool,
        sound: Option<NoticeSound>,
    ) -> (Option<&'static str>, Option<NoticeSound>) {
        match (is_bundled, sound) {
            (true, Some(s)) => (Some(s.file_name()), None),
            (false, Some(s)) => (None, Some(s)),
            (_, None) => (None, None),
        }
    }

    /// Sets or clears the macOS Dock tile badge via
    /// `NSApplication.sharedApplication.dockTile.setBadgeLabel:`.
    /// Must be called on the main (GPUI foreground) thread.
    fn set_macos_dock_badge(label: Option<&str>) {
        use std::ffi::{CString, c_char, c_void};

        #[link(name = "objc", kind = "dylib")]
        unsafe extern "C" {
            fn objc_getClass(name: *const c_char) -> *mut c_void;
            fn sel_registerName(name: *const c_char) -> *mut c_void;
            fn objc_msgSend();
        }

        // SAFETY: Called from the GPUI main thread on macOS where AppKit's
        // NSApplication/NSDockTile run loop and autorelease pool are active.
        // `objc_msgSend` is cast to the exact C ABI signature for each selector.
        unsafe {
            let msg_send_0: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                std::mem::transmute(objc_msgSend as *const c_void);
            let msg_send_1_ptr: unsafe extern "C" fn(
                *mut c_void,
                *mut c_void,
                *const c_char,
            ) -> *mut c_void = std::mem::transmute(objc_msgSend as *const c_void);
            let msg_send_1_void: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) =
                std::mem::transmute(objc_msgSend as *const c_void);

            let ns_app_cls = objc_getClass(c"NSApplication".as_ptr());
            if ns_app_cls.is_null() {
                return;
            }
            let app = msg_send_0(ns_app_cls, sel_registerName(c"sharedApplication".as_ptr()));
            if app.is_null() {
                return;
            }
            let dock_tile = msg_send_0(app, sel_registerName(c"dockTile".as_ptr()));
            if dock_tile.is_null() {
                return;
            }
            let badge_ns_string = match label {
                Some(text) if !text.is_empty() => {
                    let Ok(cstr) = CString::new(text) else {
                        return;
                    };
                    let ns_string_cls = objc_getClass(c"NSString".as_ptr());
                    if ns_string_cls.is_null() {
                        return;
                    }
                    msg_send_1_ptr(
                        ns_string_cls,
                        sel_registerName(c"stringWithUTF8String:".as_ptr()),
                        cstr.as_ptr(),
                    )
                }
                _ => std::ptr::null_mut(),
            };
            msg_send_1_void(
                dock_tile,
                sel_registerName(c"setBadgeLabel:".as_ptr()),
                badge_ns_string,
            );
        }
    }

    /// Commands queued from [`Notifier`] calls on any thread and drained on the
    /// GPUI foreground thread.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) enum NotifierCommand {
        Post(DesktopNotice),
        Withdraw(Vec<String>),
        RequestPermission,
        PlaySound(NoticeSound),
        SetDockBadge(Option<usize>),
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

        /// Posts a synthetic test notification to verify end-to-end delivery.
        #[allow(dead_code)]
        pub(crate) fn send_test_notification(&self, play_sound: bool) {
            trace_notifier(
                "live_send_test",
                &format!("play_sound={play_sound}"),
            );
            self.post(DesktopNotice {
                id: "humata.test".to_owned(),
                title: "Humata test notification".to_owned(),
                body: "Notifications are working.".to_owned(),
                target: NotifyTarget::Inbox,
                sound: play_sound.then_some(NoticeSound::Inbox),
            });
        }
    }

    impl Notifier for LiveNotifier {
        fn post(&self, notice: DesktopNotice) {
            trace_notifier(
                "live_post",
                &format!("id={} sound={:?}", notice.id, notice.sound),
            );
            if let Ok(mut map) = self.targets.lock() {
                map.insert(notice.id.clone(), notice.target.clone());
            }
            if self.commands.send(NotifierCommand::Post(notice)).is_err() {
                trace_notifier("live_post", "warning=channel_closed bridge_dropped=true");
            }
        }

        fn withdraw(&self, ids: &[String]) {
            if ids.is_empty() {
                return;
            }
            trace_notifier("live_withdraw", &format!("count={}", ids.len()));
            if let Ok(mut map) = self.targets.lock() {
                for id in ids {
                    map.remove(id);
                }
            }
            if self.commands.send(NotifierCommand::Withdraw(ids.to_vec())).is_err() {
                trace_notifier("live_withdraw", "warning=channel_closed bridge_dropped=true");
            }
        }

        fn request_permission(&self) {
            trace_notifier("live_request_permission", "enqueue=true");
            if self.commands.send(NotifierCommand::RequestPermission).is_err() {
                trace_notifier(
                    "live_request_permission",
                    "warning=channel_closed bridge_dropped=true",
                );
            }
        }

        fn query_permission(&self) -> PermissionStatus {
            let status = probe_macos_permission_status();
            trace_notifier("live_query_permission", &format!("status={status:?}"));
            status
        }

        fn play_sound(&self, sound: NoticeSound) {
            trace_notifier("live_play_sound", &format!("sound={}", sound.file_name()));
            if self.commands.send(NotifierCommand::PlaySound(sound)).is_err() {
                trace_notifier(
                    "live_play_sound",
                    "warning=channel_closed bridge_dropped=true",
                );
            }
        }

        fn set_dock_badge(&self, count: Option<usize>) {
            trace_notifier("live_set_dock_badge", &format!("count={count:?}"));
            if self.commands.send(NotifierCommand::SetDockBadge(count)).is_err() {
                trace_notifier(
                    "live_set_dock_badge",
                    "warning=channel_closed bridge_dropped=true",
                );
            }
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

            let is_bundled = std::env::current_exe()
                .ok()
                .and_then(|exe| bundle_identifier_for_exe(&exe))
                .is_some();
            if !is_bundled {
                trace_notifier(
                    "bridge_attach",
                    "warning=unbundled os_notifications_may_noop=true",
                );
            }

            cx.on_system_notification_response(move |response, cx| {
                cx.activate(true);
                let target = resolve_target(&targets, response.tag.as_str());
                trace_notifier(
                    "bridge_click",
                    &format!("tag={} resolved={}", response.tag, target.is_some()),
                );
                if let Some(target) = target {
                    let _ = clicks_tx.send(target);
                }
            });

            cx.spawn(async move |cx| {
                while let Some(command) = self.commands.recv().await {
                    match command {
                        NotifierCommand::Post(notice) => {
                            trace_notifier(
                                "bridge_dispatch",
                                &format!("cmd=post id={} sound={:?}", notice.id, notice.sound),
                            );
                            let (os_sound, fallback_sound) =
                                post_sound_plan(is_bundled, notice.sound);
                            if let Some(sound) = fallback_sound {
                                play_sound_in_app(sound);
                            }
                            let notification = to_system_notification(&notice);
                            let os_sound = os_sound.map(SharedString::from);
                            cx.update(|cx| {
                                cx.show_system_notification_with_sound(notification, os_sound);
                            });
                        }
                        NotifierCommand::Withdraw(ids) => {
                            trace_notifier(
                                "bridge_dispatch",
                                &format!("cmd=withdraw count={}", ids.len()),
                            );
                            cx.update(|cx| {
                                for id in &ids {
                                    cx.dismiss_system_notification(id);
                                }
                            });
                        }
                        NotifierCommand::RequestPermission => {
                            trace_notifier("bridge_dispatch", "cmd=request_permission");
                            cx.update(|cx| cx.request_system_notification_permission());
                        }
                        NotifierCommand::PlaySound(sound) => {
                            trace_notifier(
                                "bridge_dispatch",
                                &format!("cmd=play_sound sound={}", sound.file_name()),
                            );
                            play_sound_in_app(sound);
                        }
                        NotifierCommand::SetDockBadge(count) => {
                            let label = dock_badge_label(count);
                            trace_notifier(
                                "bridge_dispatch",
                                &format!("cmd=set_dock_badge count={count:?} label={label:?}"),
                            );
                            cx.update(|_| set_macos_dock_badge(label.as_deref()));
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

        #[test]
        fn bundle_identifier_for_exe_reads_bundled_plist_and_ignores_unbundled() {
            let root = std::env::temp_dir().join(format!(
                "humata2_notifier_bundle_test_{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);

            let macos_dir = root.join("Humata.app").join("Contents").join("MacOS");
            std::fs::create_dir_all(&macos_dir).expect("create MacOS dir");
            let exe = macos_dir.join("humata2");
            std::fs::write(&exe, b"").expect("write exe");
            let plist = root.join("Humata.app").join("Contents").join("Info.plist");
            std::fs::write(
                &plist,
                "<?xml version=\"1.0\"?><plist><dict><key>CFBundleIdentifier</key><string>com.humata.desktop</string></dict></plist>",
            )
            .expect("write Info.plist");

            assert_eq!(
                bundle_identifier_for_exe(&exe),
                Some("com.humata.desktop".to_owned())
            );

            let unbundled = root.join("bin").join("humata2");
            assert_eq!(bundle_identifier_for_exe(&unbundled), None);

            let _ = std::fs::remove_dir_all(&root);
        }

        #[test]
        fn parse_ncprefs_status_handles_all_states_and_malformed_json() {
            let json = r#"{
                "apps": [
                    {"bundle-id": "com.humata.authorized", "flags": 41955342},
                    {"flags": 8192, "bundle-id": "com.humata.provisional"},
                    {"bundle-id": "com.humata.denied", "flags": 0}
                ]
            }"#;
            assert_eq!(
                parse_ncprefs_status(json, "com.humata.authorized"),
                OsPermissionStatus::Authorized
            );
            assert_eq!(
                parse_ncprefs_status(json, "com.humata.provisional"),
                OsPermissionStatus::Provisional
            );
            assert_eq!(
                parse_ncprefs_status(json, "com.humata.denied"),
                OsPermissionStatus::Denied
            );
            assert_eq!(
                parse_ncprefs_status(json, "com.humata.absent"),
                OsPermissionStatus::NotDetermined
            );
            assert_eq!(
                parse_ncprefs_status("not json at all", "com.humata.authorized"),
                OsPermissionStatus::Unknown
            );
            assert_eq!(
                parse_ncprefs_status(r#"{"apps": [{"bundle-id": "com.humata.broken""#, "com.humata.broken"),
                OsPermissionStatus::Unknown
            );
        }

        #[test]
        fn resolve_sound_path_finds_bundled_wavs_in_contents_resources() {
            let root = std::env::temp_dir().join(format!(
                "humata2_notifier_sound_test_{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);

            let macos_dir = root.join("Humata.app").join("Contents").join("MacOS");
            let resources_dir = root.join("Humata.app").join("Contents").join("Resources");
            std::fs::create_dir_all(&macos_dir).expect("create MacOS dir");
            std::fs::create_dir_all(&resources_dir).expect("create Resources dir");

            let exe = macos_dir.join("humata2");
            std::fs::write(&exe, b"").expect("write exe");
            let inbox_wav = resources_dir.join(NoticeSound::Inbox.file_name());
            let todo_wav = resources_dir.join(NoticeSound::Todo.file_name());
            std::fs::write(&inbox_wav, b"RIFF").expect("write inbox wav");
            std::fs::write(&todo_wav, b"RIFF").expect("write todo wav");

            assert_eq!(resolve_sound_path(&exe, NoticeSound::Inbox), Some(inbox_wav));
            assert_eq!(resolve_sound_path(&exe, NoticeSound::Todo), Some(todo_wav));

            let _ = std::fs::remove_dir_all(&root);
        }

        #[test]
        fn play_sound_and_send_test_notification_enqueue_commands() {
            let (notifier, mut bridge) = LiveNotifier::new();

            notifier.play_sound(NoticeSound::Todo);
            assert_eq!(
                bridge.commands.try_recv(),
                Ok(NotifierCommand::PlaySound(NoticeSound::Todo))
            );

            notifier.send_test_notification(true);
            assert_eq!(
                bridge.commands.try_recv(),
                Ok(NotifierCommand::Post(DesktopNotice {
                    id: "humata.test".to_owned(),
                    title: "Humata test notification".to_owned(),
                    body: "Notifications are working.".to_owned(),
                    target: NotifyTarget::Inbox,
                    sound: Some(NoticeSound::Inbox),
                }))
            );

            notifier.send_test_notification(false);
            assert_eq!(
                bridge.commands.try_recv(),
                Ok(NotifierCommand::Post(DesktopNotice {
                    id: "humata.test".to_owned(),
                    title: "Humata test notification".to_owned(),
                    body: "Notifications are working.".to_owned(),
                    target: NotifyTarget::Inbox,
                    sound: None,
                }))
            );
        }

        #[test]
        fn set_dock_badge_enqueues_command_and_is_safe_when_bridge_dropped() {
            let (notifier, mut bridge) = LiveNotifier::new();

            notifier.set_dock_badge(Some(2));
            assert_eq!(
                bridge.commands.try_recv(),
                Ok(NotifierCommand::SetDockBadge(Some(2)))
            );

            notifier.set_dock_badge(None);
            assert_eq!(
                bridge.commands.try_recv(),
                Ok(NotifierCommand::SetDockBadge(None))
            );

            drop(bridge);
            notifier.set_dock_badge(Some(2));
            notifier.set_dock_badge(None);
        }

        #[test]
        fn dock_badge_label_formats_positive_counts_and_clears_zero_or_none() {
            assert_eq!(dock_badge_label(None), None);
            assert_eq!(dock_badge_label(Some(0)), None);
            assert_eq!(dock_badge_label(Some(1)), Some("1".to_owned()));
            assert_eq!(dock_badge_label(Some(42)), Some("42".to_owned()));
        }

        #[test]
        fn post_sound_plan_delegates_to_os_when_bundled_and_falls_back_when_unbundled() {
            assert_eq!(
                post_sound_plan(true, Some(NoticeSound::Inbox)),
                (Some(NoticeSound::Inbox.file_name()), None)
            );
            assert_eq!(
                post_sound_plan(true, Some(NoticeSound::Todo)),
                (Some(NoticeSound::Todo.file_name()), None)
            );
            assert_eq!(post_sound_plan(true, None), (None, None));
            assert_eq!(
                post_sound_plan(false, Some(NoticeSound::Inbox)),
                (None, Some(NoticeSound::Inbox))
            );
            assert_eq!(post_sound_plan(false, None), (None, None));
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use gpui_kit::App;
    use humata2_clients::{
        DesktopNotice, NoopNotifier, NoticeSound, Notifier, NotifyTarget, PermissionStatus,
    };
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

        fn query_permission(&self) -> PermissionStatus {
            self.0.query_permission()
        }

        fn play_sound(&self, sound: NoticeSound) {
            self.0.play_sound(sound);
        }

        fn set_dock_badge(&self, count: Option<usize>) {
            self.0.set_dock_badge(count);
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
