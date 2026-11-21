//! The toast stack at the bottom of the screen.

use humata2_ui_core::bridge::Ui;
use humata2_ui_core::widgets::{col, row};
use crate::{chrome, theme::Look};
use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, Toast, ToastAction};
use humata2_clients::model::{SessionId, Status};
use humata2_session::CloseThen;
use std::rc::Rc;

/// What a toast draws before its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Glyph {
    /// The archive, for Undo.
    Archive,
    /// The info mark, for a plain notice.
    Info,
    /// A session's state dot, as the sidebar draws it.
    Status(Status),
}

/// The glyph of a toast offering `action`. The close dialog's toasts (D784,
/// Fable A3/A4) never draw the archive: Stop's draws frame 73's stopped
/// square, Keep running's the session's own state dot (`status_of`), and
/// End for good's, which is Archive's, keeps the archive.
fn glyph(action: Option<&ToastAction>, status_of: impl Fn(&SessionId) -> Option<Status>) -> Glyph {
    match action {
        None | Some(ToastAction::OpenAgentSettings) => Glyph::Info,
        Some(ToastAction::Close(CloseThen::ResumeAgent(_) | CloseThen::ResumeShell { .. })) => {
            Glyph::Status(Status::AgentStopped)
        }
        Some(ToastAction::Close(CloseThen::Open { session, .. })) => {
            status_of(session).map_or(Glyph::Info, Glyph::Status)
        }
        Some(_) => Glyph::Archive,
    }
}

/// The words of a toast's button: Open and Resume for the close dialog's
/// toasts (D784), Add details for a report, otherwise Undo.
fn action_label(action: Option<&ToastAction>, working: bool) -> &'static str {
    match action {
        Some(ToastAction::OpenReport(_)) => "Add details",
        Some(ToastAction::OpenAgentSettings) => "Open Settings",
        Some(ToastAction::Close(CloseThen::Open { .. })) => "Open",
        Some(ToastAction::Close(CloseThen::ResumeAgent(_) | CloseThen::ResumeShell { .. })) => "Resume",
        _ if working => "Restoring…",
        _ => "Undo",
    }
}

/// The stack of toasts.
pub(super) fn stack(toasts: Vec<Toast>, look: Look, ui: &Rc<Ui>) -> Div {
    col()
        .absolute()
        .bottom(px(24.))
        .left_0()
        .w_full()
        .items_center()
        .gap(px(8.))
        .children(toasts.into_iter().map(|toast| {
            row()
                .id(("toast", toast.id as usize))
                .role(Role::Status)
                .aria_label(toast.text.clone())
                .gap(px(14.))
                .px(px(14.))
                .py(px(9.))
                .max_w(relative(0.94))
                .rounded(px(10.))
                .bg(rgb(if look.content == 0xffffff {
                    0x2c2c2e
                } else {
                    0x3a3a3d
                }))
                .border_1()
                .border_color(rgba(0x0000_0000))
                .hover(|s| s.border_color(rgba(0xffff_ff33)))
                .text_color(rgb(0xffffff))
                .text_size(px(12.5))
                .shadow_lg()
                .occlude()
                .child(
                    match glyph(toast.action.as_ref(), |id| {
                        ui.read(|state| state.workspace.session(id).map(|session| session.status))
                    }) {
                        Glyph::Archive => chrome::icon(IconName::Archive, 0xffffff, 12.).into_any_element(),
                        Glyph::Info => chrome::icon(IconName::Info, 0xffffff, 12.).into_any_element(),
                        Glyph::Status(status) => chrome::status(status).into_any_element(),
                    },
                )
                .child(div().child(toast.text.clone()))
                .when(toast.action.is_some(), |x| {
                    let working = toast.working;
                    let label = action_label(toast.action.as_ref(), working);
                    let undo = AppAction::UndoToast(toast.id);
                    let ui = ui.clone();
                    // Frame 32: a bare semibold link on the text line, one gap after the text.
                    // Restoring dims it and takes no clicks.
                    x.child(
                        div()
                            .id(SharedString::from(format!("toast-undo-{}", toast.id)))
                            .role(Role::Button)
                            .aria_label(label)
                            .test_support()
                            .flex_shrink_0()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0x64a8ff))
                            .when(working, |x| x.opacity(0.5))
                            .when(!working, |x| {
                                x.cursor_pointer().on_click(move |_, _, cx| {
                                    ui.send(undo.clone(), cx);
                                })
                            })
                            .child(label),
                    )
                })
                .when(toast.action.is_none(), |x| {
                    let dismiss = AppAction::DismissToast(toast.id);
                    let ui = ui.clone();
                    x.child(
                        row()
                            .id(("toast-dismiss", toast.id as usize))
                            .role(Role::Button)
                            .aria_label("Dismiss")
                            .test_support()
                            .size(px(20.))
                            .rounded(px(5.))
                            .justify_center()
                            .flex_shrink_0()
                            .cursor_pointer()
                            .text_color(rgb(0xd8d8dc))
                            .hover(|x| x.bg(rgba(0xffff_ff1a)))
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                ui.send(dismiss.clone(), cx);
                            })
                            .child(chrome::icon(IconName::X, 0xd8d8dc, 12.)),
                    )
                })
                .test_support()
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    // The gpui glob above brings in its own `test` attribute, which expands
    // to `#[test]` again; the built-in one wins over a glob by name.
    use ::core::prelude::v1::test;

    #[test]
    fn close_toasts_draw_state_not_the_archive() {
        let session = SessionId::new("s");
        let working = |_: &SessionId| Some(Status::Working);
        let stop = ToastAction::Close(CloseThen::ResumeAgent(session.clone()));
        assert_eq!(glyph(Some(&stop), working), Glyph::Status(Status::AgentStopped));
        let shell = ToastAction::Close(CloseThen::ResumeShell { session: session.clone(), tab: 1 });
        assert_eq!(glyph(Some(&shell), working), Glyph::Status(Status::AgentStopped));
        let keep = ToastAction::Close(CloseThen::Open { session: session.clone(), tab: 0 });
        assert_eq!(glyph(Some(&keep), working), Glyph::Status(Status::Working));
        let undo = ToastAction::Close(CloseThen::RestoreStopped(session));
        assert_eq!(glyph(Some(&undo), working), Glyph::Archive);
        assert_eq!(glyph(None, working), Glyph::Info);
    }

    #[test]
    fn close_toasts_offer_open_and_resume_and_end_offers_undo() {
        let session = SessionId::new("s");
        let label = |then| action_label(Some(&ToastAction::Close(then)), false);
        assert_eq!(label(CloseThen::Open { session: session.clone(), tab: 0 }), "Open");
        assert_eq!(label(CloseThen::ResumeAgent(session.clone())), "Resume");
        assert_eq!(label(CloseThen::ResumeShell { session: session.clone(), tab: 1 }), "Resume");
        assert_eq!(label(CloseThen::RestoreStopped(session)), "Undo");
    }
}
