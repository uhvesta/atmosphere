//! Close session (DV-4, frame 7): the one-line question a Desk session
//! whose folder is not kept asks before closing.

use super::{ORANGE, Overlays, button, header, primary, secondary};
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::theme::Look;
use humata2_ui_core::widgets::{self, row};
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::Workspace;
use humata2_desk::{CloseDialog, DeskAction};
use std::rc::Rc;

impl Overlays {
    /// The Close session question.
    pub(super) fn desk_close(
        &self,
        dialog: &CloseDialog,
        workspace: &Workspace,
        look: Look,
        ui: &Rc<Ui>,
    ) -> AnyElement {
        let title = workspace
            .session(&dialog.session)
            .map_or_else(|| dialog.session.to_string(), |s| s.title.clone());
        let id = dialog.session.clone();
        super::card("desk-close-dialog", "Close session", 520., look)
            .track_focus(&self.sheet)
            .gap(px(12.))
            .child(header(look, "Close session", title))
            .child(
                div()
                    .text_color(rgb(ORANGE))
                    .child(
                        "Nothing in this folder is saved as a version, and the folder goes 7 days after you close it.",
                    ),
            )
            .child(secondary(look, "Save a version to keep the folder and its files."))
            .children(dialog.error.clone().map(|error| {
                widgets::error_banner_in(look, ui, "desk-close-error", error, AppAction::Escape)
            }))
            .child(
                row()
                    .gap(px(8.))
                    .pt(px(4.))
                    .child(div().flex_1())
                    .child(button(ui, look, "desk-close-cancel", "Cancel", AppAction::Escape))
                    .child(
                        button(
                            ui,
                            look,
                            "desk-close-anyway",
                            "Close",
                            AppAction::Desk(DeskAction::Close(id.clone())),
                        )
                        .disabled(dialog.saving),
                    )
                    .child(primary(
                        ui,
                        look,
                        "desk-close-save",
                        if dialog.saving {
                            "Saving…"
                        } else {
                            "Save version and close"
                        },
                        AppAction::Desk(DeskAction::SaveAndClose(id)),
                        dialog.saving,
                    )),
            )
            .focus_trap("desk-close-dialog", &self.sheet)
            .into_any_element()
    }
}
