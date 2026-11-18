//! The toast stack at the bottom of the screen.

use crate::bridge::Ui;
use crate::widgets::{self, col, row};
use crate::{chrome, theme::Look};
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, Toast, ToastAction};
use humata2_session::CloseThen;
use std::rc::Rc;

/// The words of a toast's button: Open and Resume for the close dialog's
/// toasts (D784), Add details for a report, otherwise Undo.
fn action_label(action: Option<&ToastAction>, working: bool) -> &'static str {
    match action {
        Some(ToastAction::OpenReport(_)) => "Add details",
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
                .text_color(rgb(0xffffff))
                .text_size(px(12.5))
                .shadow_lg()
                .occlude()
                .child(chrome::icon(
                    if toast.action.is_some() {
                        IconName::Archive
                    } else {
                        IconName::Info
                    },
                    0xffffff,
                    12.,
                ))
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
                    x.child(
                        widgets::ghost_in(look,
                            ui,
                            ("toast-dismiss", toast.id as usize),
                            "",
                            AppAction::DismissToast(toast.id),
                        )
                        .icon(IconName::X)
                        .xsmall()
                        .accessibility_label("Dismiss"),
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
    use humata2_clients::model::SessionId;

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
