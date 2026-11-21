//! Settings › Notifications (NOTIFYAUDIT, ATM-48/49/50/52, `DECISIONS.md` §4).
//!
//! Renders the Notifications section:
//! - Live `macOS permission` status row (with a `Request permission` action when
//!   `PermissionStatus::NotDetermined`).
//! - Conditional macOS permission warning banner — per `DECISIONS.md` §4, this
//!   warning lives **only** in `Settings › Notifications`, never as a permanent
//!   strip inside `Agent inbox`.
//! - `Desktop notifications` toggle, `Toasts in Humata` toggle (8s, pauses on
//!   hover, never evicts Undo), **decoupled** `Play a sound` toggle (always
//!   interactive even when desktop notifications are off), `Permission request
//!   alerts` toggle, and a `Send a test notification` button.

use crate::{
    bridge::Ui,
    settings::{button, card, dim},
    theme::{GREEN, GREY, Look, ORANGE},
    widgets::{col, row},
};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_notify::{NotifyConfig, PermissionStatus};
use humata2_settings::{Row, SettingsAction as A};
use std::rc::Rc;

/// Snapshot of notification preferences and OS permission state used to render
/// `Settings › Notifications`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotificationSectionProps<'a> {
    /// Live macOS `UNUserNotificationCenter` authorization status.
    pub permission: PermissionStatus,
    /// User notification preferences.
    pub notify: &'a NotifyConfig,
    /// Whether extra permission-request alerts are enabled.
    pub permission_request_alerts: bool,
}

/// Returns `(heading, body)` for the macOS permission warning card when
/// `status.needs_warning()` is true (`Denied` or `Unbundled`), and `None`
/// otherwise.
///
/// Per `DECISIONS.md` §4, this warning lives **only** in
/// `Settings › Notifications`, never as a permanent strip inside `Agent inbox`.
#[must_use]
pub fn permission_banner_copy(status: PermissionStatus) -> Option<(&'static str, &'static str)> {
    match status {
        PermissionStatus::Denied => Some((
            "macOS notifications are off",
            "Turn on notifications for Humata 2 in System Settings \u{203a} Notifications. In-app toasts and sounds still work while Humata is open.",
        )),
        PermissionStatus::Unbundled => Some((
            "macOS notifications unavailable in dev build",
            "This build has no app bundle identifier, so macOS will not deliver system banners. In-app toasts and sounds still work while Humata is open.",
        )),
        PermissionStatus::Unknown
        | PermissionStatus::Authorized
        | PermissionStatus::NotDetermined
        | PermissionStatus::Provisional
        | PermissionStatus::Ephemeral => None,
    }
}

/// Short status pill label for the `macOS permission` row.
#[must_use]
pub fn permission_status_label(status: PermissionStatus) -> &'static str {
    match status {
        PermissionStatus::Authorized => "Allowed",
        PermissionStatus::Denied => "Off in System Settings",
        PermissionStatus::NotDetermined => "Not requested yet",
        PermissionStatus::Provisional => "Quietly allowed",
        PermissionStatus::Ephemeral => "Temporary",
        PermissionStatus::Unbundled => "Unavailable in dev build",
        PermissionStatus::Unknown => "Checking\u{2026}",
    }
}

/// Subtitle for the decoupled `Play a sound` row.
#[must_use]
pub fn play_sound_description() -> &'static str {
    "Play a short chime when a notice or in-app toast fires. Silent when the exact request is already open in front of you."
}

/// Subtitle for the `Toasts in Humata` row.
#[must_use]
pub fn toasts_description() -> &'static str {
    "Show an in-app toast for 8 seconds (pauses on hover, never replaces Undo) when Humata is frontmost or macOS notifications are off."
}

fn status_dot_color(status: PermissionStatus) -> u32 {
    match status {
        PermissionStatus::Authorized
        | PermissionStatus::Provisional
        | PermissionStatus::Ephemeral => GREEN,
        PermissionStatus::Denied | PermissionStatus::Unbundled => ORANGE,
        PermissionStatus::NotDetermined | PermissionStatus::Unknown => GREY,
    }
}

fn setting_row(look: Look, title: &str, note: &str, control: AnyElement) -> Div {
    row()
        .gap(px(16.))
        .px(px(14.))
        .py(px(10.))
        .border_b_1()
        .border_color(rgb(look.line))
        .items_center()
        .child(
            col()
                .flex_1()
                .min_w_0()
                .child(title.to_owned())
                .child(dim(look, note.to_owned())),
        )
        .child(control)
}

fn toggle_action(
    ui: &Rc<Ui>,
    id: &str,
    on: bool,
    action: AppAction,
    look: Look,
) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id(SharedString::from(id.to_owned()))
        .role(Role::Button)
        .aria_label(if on { "On" } else { "Off" })
        .aria_selected(on)
        .test_support()
        .w(px(28.))
        .h(px(16.))
        .rounded_full()
        .p(px(2.))
        .bg(rgb(if on { 0x28cd41 } else { look.fill }))
        .flex()
        .when(on, |x| x.justify_end())
        .child(div().size(px(12.)).rounded_full().bg(rgb(0xffffff)))
        .cursor_pointer()
        .on_click(move |_, _, cx| ui.send(action.clone(), cx))
}

/// Live `macOS permission` status row, with a `Request permission` button when
/// macOS has not yet prompted (`PermissionStatus::NotDetermined`).
pub fn permission_status_row(
    ui: &Rc<Ui>,
    permission: PermissionStatus,
    look: Look,
    request_permission_action: AppAction,
) -> Div {
    let pill = row()
        .gap(px(6.))
        .items_center()
        .flex_shrink_0()
        .child(
            div()
                .size(px(7.))
                .rounded_full()
                .bg(rgb(status_dot_color(permission))),
        )
        .child(dim(look, permission_status_label(permission)));

    let control = row()
        .gap(px(10.))
        .items_center()
        .flex_shrink_0()
        .child(pill)
        .when(permission == PermissionStatus::NotDetermined, |r| {
            r.child(button(
                ui,
                "settings-request-notification-permission",
                "Request permission",
                request_permission_action,
                look,
            ))
        })
        .into_any_element();

    setting_row(
        look,
        "macOS permission",
        permission.detail(),
        control,
    )
}

fn permission_warning_card(heading: &'static str, body: &'static str, look: Look) -> Div {
    card(look)
        .border_color(rgb(ORANGE))
        .child(
            col()
                .gap(px(4.))
                .px(px(14.))
                .py(px(10.))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(ORANGE))
                        .child(heading),
                )
                .child(dim(look, body)),
        )
}

/// Renders the `Settings › Notifications` section.
///
/// The `Play a sound` toggle is intentionally decoupled from `Desktop
/// notifications` so users can keep chimes on for in-app toasts even when
/// macOS notifications are off.
pub fn section(
    ui: &Rc<Ui>,
    state: &AppState,
    permission: PermissionStatus,
    look: Look,
    test_action: AppAction,
    toggle_toasts_action: AppAction,
    request_permission_action: AppAction,
) -> Div {
    let prefs = &state.settings.preferences;
    let props = NotificationSectionProps {
        permission,
        notify: &prefs.notify,
        permission_request_alerts: prefs.permission_request_alerts,
    };

    let mut container = col()
        .gap(px(12.))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Notifications"),
        )
        .child(dim(
            look,
            "Requests that need you always appear in the Agent inbox and beside the session in the sidebar.",
        ));

    if let Some((heading, body)) = permission_banner_copy(props.permission) {
        container = container.child(permission_warning_card(heading, body, look));
    }

    container.child(
        card(look)
            .child(permission_status_row(
                ui,
                props.permission,
                look,
                request_permission_action,
            ))
            .child(setting_row(
                look,
                Row::DesktopNotifications.title(),
                "Post macOS notifications when an agent needs your answer or fails to resume. Quiet while you are looking at that item.",
                toggle_action(
                    ui,
                    "settings-desktop-notifications",
                    props.notify.enabled,
                    AppAction::Settings(A::ToggleDesktopNotifications),
                    look,
                )
                .into_any_element(),
            ))
            .child(setting_row(
                look,
                "Toasts in Humata",
                toasts_description(),
                toggle_action(
                    ui,
                    "settings-in-app-toasts",
                    props.notify.in_app_toasts,
                    toggle_toasts_action,
                    look,
                )
                .into_any_element(),
            ))
            .child(setting_row(
                look,
                Row::PlaySound.title(),
                play_sound_description(),
                toggle_action(
                    ui,
                    "settings-play-sound",
                    props.notify.play_sound,
                    AppAction::Settings(A::TogglePlaySound),
                    look,
                )
                .into_any_element(),
            ))
            .child(setting_row(
                look,
                Row::PermissionRequestAlerts.title(),
                "Show brief in-app toasts for new Agent inbox permission requests. Choose where approvals appear in Agents.",
                toggle_action(
                    ui,
                    "settings-permission-alerts",
                    props.permission_request_alerts,
                    AppAction::Settings(A::TogglePermissionRequestAlerts),
                    look,
                )
                .into_any_element(),
            ))
            .child(setting_row(
                look,
                "Send a test notification",
                "Run the full notification path (desktop banner, in-app toast, and sound) to verify delivery.",
                button(
                    ui,
                    "settings-send-test-notification",
                    "Send test",
                    test_action,
                    look,
                )
                .into_any_element(),
            )),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        permission_banner_copy, permission_status_label, play_sound_description,
        toasts_description,
    };
    use humata2_notify::PermissionStatus;

    #[core::prelude::v1::test]
    fn permission_banner_copy_only_warns_for_denied_and_unbundled() {
        assert!(permission_banner_copy(PermissionStatus::Unknown).is_none());
        assert!(permission_banner_copy(PermissionStatus::Authorized).is_none());
        assert!(permission_banner_copy(PermissionStatus::NotDetermined).is_none());
        assert!(permission_banner_copy(PermissionStatus::Provisional).is_none());
        assert!(permission_banner_copy(PermissionStatus::Ephemeral).is_none());

        let (denied_heading, denied_body) =
            permission_banner_copy(PermissionStatus::Denied).expect("denied should warn");
        assert_eq!(denied_heading, "macOS notifications are off");
        assert!(denied_body.contains("System Settings"));

        let (unbundled_heading, unbundled_body) =
            permission_banner_copy(PermissionStatus::Unbundled).expect("unbundled should warn");
        assert!(unbundled_heading.contains("dev build"));
        assert!(unbundled_body.contains("bundle identifier"));
    }

    #[core::prelude::v1::test]
    fn permission_status_labels_cover_every_variant() {
        assert_eq!(permission_status_label(PermissionStatus::Authorized), "Allowed");
        assert_eq!(
            permission_status_label(PermissionStatus::Denied),
            "Off in System Settings"
        );
        assert_eq!(
            permission_status_label(PermissionStatus::NotDetermined),
            "Not requested yet"
        );
        assert_eq!(
            permission_status_label(PermissionStatus::Provisional),
            "Quietly allowed"
        );
        assert_eq!(
            permission_status_label(PermissionStatus::Ephemeral),
            "Temporary"
        );
        assert_eq!(
            permission_status_label(PermissionStatus::Unbundled),
            "Unavailable in dev build"
        );
        assert_eq!(
            permission_status_label(PermissionStatus::Unknown),
            "Checking\u{2026}"
        );
    }

    #[core::prelude::v1::test]
    fn copy_matches_decisions_spec() {
        assert_eq!(
            play_sound_description(),
            "Play a short chime when a notice or in-app toast fires. Silent when the exact request is already open in front of you."
        );
        assert_eq!(
            toasts_description(),
            "Show an in-app toast for 8 seconds (pauses on hover, never replaces Undo) when Humata is frontmost or macOS notifications are off."
        );
    }
}
