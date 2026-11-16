//! The keyboard shortcuts sheet.

use super::{Overlays, sheet};
use crate::bridge::Ui;
use crate::keys::SHORTCUTS;
use crate::theme::Look;
use crate::widgets::{self, col, row};
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use std::rc::Rc;

impl Overlays {
    /// The Keyboard shortcuts sheet.
    pub(super) fn shortcuts(&self, look: Look, ui: &Rc<Ui>) -> AnyElement {
        sheet("shortcuts", "Keyboard shortcuts", look)
            .test_support()
            .track_focus(&self.sheet)
            .child(widgets::title_in(look, "Keyboard shortcuts"))
            .child(
                col()
                    .id("shortcuts-list")
                    .role(Role::List)
                    .aria_label("Keyboard shortcuts")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .overflow_y_scroll()
                    .children(SHORTCUTS.iter().enumerate().map(|(index, (keys, what))| {
                        row()
                            .id(("shortcut", index))
                            .role(Role::ListItem)
                            .aria_label(format!("{what}: {keys}"))
                            .test_support()
                            .gap_3()
                            .child(
                                div()
                                    .w(px(120.))
                                    .text_color(rgb(look.accent))
                                    .child(*keys),
                            )
                            .child(div().text_color(rgb(look.text)).child(*what))
                    })),
            )
            .child(row().justify_end().child(widgets::primary_in(
                look,
                ui,
                "shortcuts-close",
                "Close",
                AppAction::Escape,
            )))
            .focus_trap("shortcuts-trap", &self.sheet)
            .into_any_element()
    }
}
