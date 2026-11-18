//! The Rename session dialog (FC-17), opened from the header's ⋯ menu
//! (SE-7). No frame draws it; it follows the Move dialog's card (D295).

use super::Overlays;
use super::fork::{card, heading};
use crate::bridge::Ui;
use crate::chrome;
use crate::theme::Look;
use crate::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::input::Input;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_session::{RenameDraft, SessionAction};
use std::rc::Rc;

impl Overlays {
    /// The Rename session dialog.
    pub(super) fn rename_dialog(
        &self,
        draft: &RenameDraft,
        error: Option<String>,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        super::sync(&self.rename_title, &draft.title, window, cx);
        let on = draft.rename_branch;
        let toggle = {
            let ui = ui.clone();
            let sending = draft.sending;
            row()
                .id("rename-branch")
                .role(Role::CheckBox)
                .aria_label("Rename the branch too")
                .aria_selected(on)
                .test_support()
                .items_start()
                .gap(px(7.))
                .child(
                    row()
                        .mt(px(2.))
                        .size(px(14.))
                        .flex_shrink_0()
                        .rounded(px(4.))
                        .justify_center()
                        .when(on, |x| {
                            x.bg(rgb(look.accent))
                                .child(chrome::icon(IconName::Check, 0xffffff, 10.))
                        })
                        .when(!on, |x| {
                            x.border_1()
                                .border_color(rgb(look.tertiary))
                                .bg(rgb(look.card))
                        }),
                )
                .child(
                    col()
                        .min_w_0()
                        .child(div().child("Rename the branch too"))
                        .child(
                            div()
                                .text_color(rgb(look.secondary))
                                .child("Only a branch that isn't pushed yet; a pushed branch keeps its name."),
                        ),
                )
                .when(!sending, |x| {
                    x.cursor_pointer().on_click(move |_, _, cx| {
                        ui.send(AppAction::Session(SessionAction::RenameBranch(!on)), cx);
                    })
                })
        };
        let mut body = card("rename-sheet", "Rename session", 460., look)
            .child(heading("Rename session".to_owned()))
            .child(
                div()
                    .id("rename-title")
                    .role(Role::Group)
                    .aria_label("Title")
                    .test_support()
                    .child(
                        Input::new(&self.rename_title)
                            .disabled(draft.sending)
                            .aria_label("Session title"),
                    ),
            )
            .child(toggle);
        if let Some(error) = error {
            body = body.child(
                div()
                    .id("rename-error")
                    .role(Role::Alert)
                    .text_color(rgb(0xff453a))
                    .child(error),
            );
        }
        let body = body
            .child(
                row()
                    .gap(px(8.))
                    .pt(px(4.))
                    .justify_end()
                    .child(
                        widgets::push_quiet(
                            ui,
                            "rename-cancel",
                            "Cancel",
                            AppAction::Session(SessionAction::CancelRename),
                            look,
                        )
                        .disabled(draft.sending),
                    )
                    .child(
                        widgets::push_primary(
                            ui,
                            "rename-confirm",
                            if draft.sending { "Renaming…" } else { "Rename" },
                            AppAction::Session(SessionAction::ConfirmRename),
                            look,
                        )
                        .disabled(draft.sending || draft.title.trim().is_empty()),
                    ),
            )
            .test_support()
            .focus_trap("rename-trap", &self.sheet);
        div()
            .id("rename-dialog")
            .role(Role::Group)
            .aria_label("rename dialog")
            .test_support()
            .child(body)
            .into_any_element()
    }
}
