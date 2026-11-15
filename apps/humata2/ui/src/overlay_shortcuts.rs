//! The keyboard shortcuts sheet.

use super::{Overlays, sheet};
use crate::bridge::Ui;
use crate::keys::SHORTCUTS;
use crate::theme::Look;
use crate::theme::{self, color};
use crate::widgets::{self, row};
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
            .child(widgets::title("Keyboard shortcuts"))
            .children(SHORTCUTS.iter().map(|(keys, what)| {
                row()
                    .gap_3()
                    .child(
                        div()
                            .w(px(120.))
                            .text_color(color(theme::ACCENT2))
                            .child(*keys),
                    )
                    .child(div().text_color(color(theme::TEXT)).child(*what))
            }))
            .child(row().justify_end().child(widgets::primary(
                ui,
                "shortcuts-close",
                "Close",
                AppAction::Escape,
            )))
            .focus_trap("shortcuts-trap", &self.sheet)
            .into_any_element()
    }
}
