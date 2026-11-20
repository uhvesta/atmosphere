//! The Desk in the sidebar (DK-1, DK-12, D793): a Desk row under To-do and,
//! while it is not folded, on every page, the Desk sessions under it: the
//! one showing and the ones used last, up to the cap Settings sets, in the
//! order they were started (frames 25, 36; D148), then "K more" for the rest.

use std::rc::Rc;

use crate::bridge::Ui;
use crate::chrome;
use crate::theme::Look;
use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_sidebar::{Row, RowKey, RowKind, SidebarAction};

/// The Desk row, the Desk sessions the sidebar lists under it and, when
/// some are left out, the "more" row.
pub fn rows(ui: &Rc<Ui>, state: &AppState, look: Look) -> Div {
    let mut list = div().flex().flex_col().child(desk_row(ui, state, look));
    list = list.children(
        state
            .sidebar
            .desk
            .iter()
            .filter_map(|entry| session(ui, state, entry, look)),
    );
    if state.sidebar.desk_more > 0 {
        list = list.child(more_row(ui, state, look));
    }
    list
}

/// The Desk row: a click opens the Desk page; while there are open Desk
/// sessions its chevron folds them. Folded, it is highlighted while a Desk
/// session shows, since that session has no row of its own.
fn desk_row(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    let folded = state.sidebar.folding.desk;
    let on = match &state.route {
        Route::Desk => true,
        Route::Session(id) => folded && state.is_desk_session(id),
        _ => false,
    };
    let at_cursor = state.sidebar.cursor.as_ref() == Some(&RowKey::Desk);
    let open = ui.clone();
    let mut row = div()
        .id("sidebar-desk")
        .role(Role::Link)
        .aria_label("Desk")
        .aria_selected(on)
        .flex()
        .items_center()
        .gap(px(7.))
        .h(px(26.))
        .mx(px(10.))
        .px(px(8.))
        .rounded(px(6.))
        .when(at_cursor && !on, |x| x.bg(rgb(look.fill)))
        .when(on, |x| x.bg(rgb(look.selection)).text_color(rgb(0xffffff)))
        .child(chrome::icon(
            IconName::SquareTerminal,
            if on { 0xffffff } else { look.accent },
            15.,
        ))
        .child(div().flex_1().child("Desk"));
    if state.sidebar.desk_open > 0 {
        let fold = ui.clone();
        row = row.aria_expanded(!folded).child(
            div()
                .id("sidebar-desk-fold")
                .size(px(12.))
                .flex_shrink_0()
                .role(Role::Button)
                .aria_label(if folded { "Unfold" } else { "Fold" })
                .child(chrome::icon(
                    if folded {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    },
                    if on { 0xffffff } else { look.tertiary },
                    12.,
                ))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    fold.send(AppAction::Sidebar(SidebarAction::Toggle(RowKey::Desk)), cx)
                })
                .test_support(),
        );
    }
    row.on_click(move |_, _, cx| open.send(AppAction::Switch(Route::Desk), cx))
        .test_support()
}

/// "K more": the Desk sessions the cap leaves out; it opens the Desk page.
fn more_row(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    let at_cursor = state.sidebar.cursor.as_ref() == Some(&RowKey::DeskMore);
    let label = format!("{} more", state.sidebar.desk_more);
    let ui = ui.clone();
    div()
        .id("sidebar-desk-more")
        .role(Role::Link)
        .aria_label(label.clone())
        .flex()
        .items_center()
        .h(px(26.))
        .mx(px(10.))
        .pl(px(26.))
        .pr(px(8.))
        .rounded(px(6.))
        .text_color(rgb(look.tertiary))
        .when(at_cursor, |x| x.bg(rgb(look.fill)))
        .child(label)
        .on_click(move |_, _, cx| {
            ui.send(AppAction::Sidebar(SidebarAction::Activate(RowKey::DeskMore)), cx)
        })
        .test_support()
}

/// The Desk row under an open channel while it has open Desk sessions
/// (CD-2, frame 95): a click, or Enter with the keyboard cursor on it, opens
/// the channel on its Desk tab.
pub fn channel_row(
    ui: &Rc<Ui>,
    channel: &humata2_clients::model::ChannelId,
    count: usize,
    at_cursor: bool,
    look: Look,
) -> AnyElement {
    let ui = ui.clone();
    let open = AppAction::Sidebar(SidebarAction::Activate(RowKey::ChannelDesk(channel.clone())));
    div()
        .id(SharedString::from(format!(
            "sidebar-channel-desk-{}",
            channel.0
        )))
        .role(Role::TreeItem)
        .aria_label("Desk")
        .flex()
        .items_center()
        .gap(px(7.))
        .h(px(crate::theme::ROW))
        .mx(px(10.))
        .pl(px(18.))
        .pr(px(8.))
        .rounded(px(6.))
        .when(at_cursor, |x| x.bg(rgb(look.fill)))
        .child(chrome::icon(IconName::SquareTerminal, look.tertiary, 12.))
        .child(div().flex_1().min_w_0().child("Desk"))
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(look.secondary))
                .child(count.to_string()),
        )
        .on_click(move |_, _, cx| ui.send(open.clone(), cx))
        .test_support()
        .into_any_element()
}

/// One open Desk session, drawn as the frame draws it: a status dot and the
/// title, indented 18 under the Desk row.
pub fn session(
    ui: &Rc<Ui>,
    state: &AppState,
    entry: &Row,
    look: Look,
) -> Option<AnyElement> {
    let key = entry.key.clone()?;
    let RowKey::Session(id) = &key else {
        return None;
    };
    let RowKind::Session { title, status, unread, .. } = &entry.kind else {
        return None;
    };
    let on = state.route == Route::Session(id.clone());
    let at_cursor = state.sidebar.cursor.as_ref() == Some(&key);
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
            .when(at_cursor && !on, |x| x.bg(rgb(look.fill)))
            .when(on, |x| x.bg(rgb(look.selection)))
            .child(chrome::status(*status))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .when(unread, |x| x.font_weight(FontWeight::SEMIBOLD))
                    .child(title.clone()),
            )
            .on_click(move |_, _, cx| {
                ui.send(AppAction::Sidebar(SidebarAction::Activate(key.clone())), cx)
            })
            .test_support()
            .into_any_element(),
    )
}
