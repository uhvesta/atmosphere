//! A Desk session's Board panel (DK-4, BD-7, frame 37): every channel's
//! items in lanes by status, each with its id, title and channel, and
//! "added by this session" on the ones the session added. A Desk session
//! has no board of its own (DK-12).
use crate::{
    bridge::Ui,
    chrome,
    theme::Look,
    widgets::{col, row},
};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::{prelude::*, *};
use humata2_app::layout::FullScreen;
use humata2_app::{AppAction, AppState, Route};
use humata2_clients::model::{ItemStatus, Scope, Session};
use humata2_desk::LaneItem;
use humata2_session::Panel;
use std::rc::Rc;

/// Text a flow test and a snapshot can read.
fn text(value: impl Into<SharedString>) -> Div {
    let value = value.into();
    div().child(div().child(value.clone()).test_text(value))
}

/// The frames' quiet text.
fn dim(look: Look, value: impl Into<SharedString>) -> Div {
    text(value)
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
}

/// The scope the panel shows, drawn as the frame's pop-up. It is the only
/// scope a Desk session has, so it carries no role and opens nothing.
fn scope(look: Look) -> Div {
    row()
        .gap(px(6.))
        .h(px(24.))
        .pl(px(9.))
        .pr(px(5.))
        .rounded(px(6.))
        .bg(rgb(look.fill))
        .border_1()
        .border_color(rgb(look.line))
        .text_size(px(12.))
        .flex_shrink_0()
        .child(text("All channels"))
        .child(chrome::icon(
            IconName::ChevronsUpDown,
            look.secondary,
            12.,
        ))
}

/// A lane's heading: its status and how many items it holds.
fn lane(look: Look, title: String) -> Div {
    div().pt(px(8.)).child(
        text(title)
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(look.secondary)),
    )
}

/// Where an item lives: "# channel", or "Global" for a global item.
fn place(state: &AppState, scope: &Scope) -> String {
    match scope {
        Scope::Channel(id) | Scope::Session { channel: id, .. } => format!(
            "# {}",
            state
                .workspace
                .channel(id)
                .map_or(id.as_str(), |channel| channel.name.as_str())
        ),
        Scope::Global => "Global".to_owned(),
    }
}

/// An item's card: its id and title over its channel, outlined in the
/// accent when this session added it. Clicking opens the session working
/// on the item, as a channel session's Board panel does.
fn item(ui: &Rc<Ui>, state: &AppState, entry: LaneItem<'_>, look: Look) -> AnyElement {
    let item = entry.item;
    let session = item
        .session
        .as_ref()
        .and_then(|id| state.workspace.session(id));
    let mut card = col()
        .id(SharedString::from(format!("desk-board-item-{}", item.id)))
        .role(Role::ListItem)
        .aria_label(format!("{} {}", item.id, item.title))
        .test_support()
        .gap(px(3.))
        .p(px(9.))
        .rounded(px(7.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(if entry.added {
            look.accent
        } else {
            look.line
        }))
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(text(item.id.to_string()).text_color(rgb(look.secondary)))
                .child(
                    text(item.title.clone())
                        .flex_1()
                        .font_weight(FontWeight::MEDIUM),
                ),
        )
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(dim(look, place(state, &item.scope)))
                .when(entry.added, |line| {
                    line.child(
                        text("added by this session")
                            .text_size(px(11.))
                            .text_color(rgb(look.accent)),
                    )
                }),
        );
    // Done fades, as in the compact board.
    if item.status == ItemStatus::Done {
        card = card.opacity(0.6);
    }
    if let Some(session) = session {
        let ui = ui.clone();
        let id = session.id.clone();
        card = card
            .cursor_pointer()
            .on_click(move |_, _, cx| ui.send(AppAction::Navigate(Route::Session(id.clone())), cx));
    }
    card.into_any_element()
}

/// The Board panel of `session`, or `None` when it isn't a Desk session:
/// the header with the scope, then the lanes in the board's order, leaving
/// out the ones with nothing in them.
pub fn panel(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    look: Look,
) -> Option<AnyElement> {
    if !humata2_clients::desk::is_desk(session) {
        return None;
    }
    let full = state.full_screen() == Some(FullScreen::Panel);
    let mut rows = Vec::<AnyElement>::new();
    for found in state.desk.board(&session.id) {
        rows.push(
            lane(
                look,
                format!("{} · {}", found.status.name(), found.items.len()),
            )
            .into_any_element(),
        );
        for entry in found.items {
            rows.push(item(ui, state, entry, look));
        }
    }
    Some(
        col()
            .size_full()
            .child(humata2_ui_core::chrome::panel_header(
                ui,
                state,
                Panel::Board,
                full,
                Some(scope(look)),
                look,
            ))
            .child(
                col()
                    .id("desk-board")
                    .role(Role::List)
                    .aria_label("Board, all channels")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .gap(px(6.))
                    .p(px(12.))
                    .text_size(px(12.5))
                    .children(rows),
            )
            .into_any_element(),
    )
}
