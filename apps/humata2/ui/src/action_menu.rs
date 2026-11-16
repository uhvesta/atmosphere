//! The action menu: a card of items over the content, with a clear layer behind
//! it that closes it on a click outside. Esc closes it through the overlay that
//! owns the menu's open flag, so the menu itself holds no state.

use std::rc::Rc;

use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;

use crate::bridge::Ui;
use crate::theme::Look;
use crate::widgets::col;

/// The card that holds a menu's items, in the frames' menu chrome. It is
/// absolute, so the caller places it; its items are its children.
#[must_use]
pub(crate) fn card(id: &'static str, width: f32, look: Look) -> Stateful<Div> {
    col()
        .id(id)
        .absolute()
        .w(px(width))
        .p(px(5.))
        .gap(px(2.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .rounded(px(7.))
        .shadow_lg()
}

/// A clear layer covering the window from where its container sits. A click on
/// it sends `close`. Draw it before the card, so the card stays above it.
#[must_use]
pub(crate) fn scrim(ui: &Rc<Ui>, id: &'static str, close: AppAction) -> Stateful<Div> {
    let ui = ui.clone();
    div()
        .id(id)
        .absolute()
        // Larger than any window, so the layer reaches every edge from the
        // container's corner.
        .top(px(-10_000.))
        .left(px(-10_000.))
        .w(px(20_000.))
        .h(px(20_000.))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            ui.send(close.clone(), cx);
        })
}
