//! The Desk in the sidebar (DK-1, DK-12): a Desk row under Board, and on the
//! Desk page and on a Desk session's page the open Desk sessions under it,
//! in the order they were started (frames 25, 36; D148).

use std::rc::Rc;

use crate::bridge::Ui;
use crate::chrome;
use crate::sidebar::nav;
use crate::theme::Look;
use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_sidebar::{Row, RowKey, RowKind, SidebarAction};

/// The Desk row, and, when the sidebar lists them, the open Desk sessions
/// under it.
pub(crate) fn rows(ui: &Rc<Ui>, state: &AppState, look: Look) -> Div {
    let mut list = div().flex().flex_col().child(nav(
        ui,
        "sidebar-desk",
        "Desk",
        IconName::SquareTerminal,
        Route::Desk,
        state.route == Route::Desk,
        None,
        look,
    ));
    if state.desk_in_sidebar() {
        list = list.children(
            state
                .sidebar
                .desk
                .iter()
                .filter_map(|entry| session(ui, state, entry, look)),
        );
    }
    list
}

/// One open Desk session, drawn as the frame draws it: a status dot and the
/// title, indented 18 under the Desk row.
fn session(ui: &Rc<Ui>, state: &AppState, entry: &Row, look: Look) -> Option<AnyElement> {
    let key = entry.key.clone()?;
    let RowKey::Session(id) = &key else {
        return None;
    };
    let RowKind::Session { title, status, unread, .. } = &entry.kind else {
        return None;
    };
    let on = state.route == Route::Session(id.clone());
    let ui = ui.clone();
    let unread = *unread > 0;
    Some(
        div()
            .id(SharedString::from(format!("sidebar-session-{}", id.0)))
            .role(Role::TreeItem)
            .aria_label(title.clone())
            .aria_selected(on)
            .flex()
            .items_center()
            .gap(px(7.))
            .h(px(26.))
            .mx(px(10.))
            .pl(px(26.))
            .pr(px(8.))
            .rounded(px(6.))
            .text_color(rgb(if on { 0xffffff } else { look.text }))
            .when(on, |x| x.bg(rgb(look.selection)))
            .child(chrome::status(*status))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .when(unread || on, |x| x.font_weight(FontWeight::SEMIBOLD))
                    .child(title.clone()),
            )
            .on_click(move |_, _, cx| {
                ui.send(AppAction::Sidebar(SidebarAction::Activate(key.clone())), cx)
            })
            .test_support()
            .into_any_element(),
    )
}
