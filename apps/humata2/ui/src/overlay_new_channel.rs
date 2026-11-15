//! The New channel sheet (CH-1).

use super::{Overlays, field, sheet, sync};
use crate::bridge::Ui;
use crate::theme::Look;
use crate::widgets::{self, row};
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::input::Input;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, NewChannel, NewChannelAction};
use std::rc::Rc;

impl Overlays {
    /// The New channel sheet.
    pub(super) fn new_channel(
        &self,
        sheet_state: &NewChannel,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        sync(&self.name, &sheet_state.name, window, cx);
        sync(&self.repos, &sheet_state.repos, window, cx);
        sheet("new-channel", "New channel", look)
            .test_support()
            .child(widgets::title("New channel"))
            .child(field(
                "Name",
                Input::new(&self.name).aria_label("Channel name"),
            ))
            .child(field(
                "Repositories, separated by commas",
                Input::new(&self.repos).aria_label("Repositories"),
            ))
            .children(sheet_state.error.clone().map(|error| {
                widgets::error_banner(ui, "new-channel-error", error, AppAction::Escape)
            }))
            .child(
                row()
                    .justify_end()
                    .gap_2()
                    .child(widgets::ghost(
                        ui,
                        "new-channel-cancel",
                        "Cancel",
                        AppAction::Escape,
                    ))
                    .child(
                        widgets::primary(
                            ui,
                            "new-channel-create",
                            if sheet_state.creating {
                                "Making…"
                            } else {
                                "Make channel"
                            },
                            AppAction::NewChannel(NewChannelAction::Create),
                        )
                        .disabled(sheet_state.creating),
                    ),
            )
            .focus_trap("new-channel-trap", &self.name.focus_handle(cx))
            .into_any_element()
    }
}
