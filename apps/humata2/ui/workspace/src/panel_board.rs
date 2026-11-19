//! The Board panel: the compact lanes and the full-screen kanban.
use crate::panel_prelude::*;

pub fn board(ui: &Rc<Ui>, state: &AppState, session: &Session, full: bool, look: Look) -> Div {
    // DK-4, DK-12: a Desk session has no board of its own; its panel shows
    // every channel's items (frame 37).
    if let Some(panel) = humata2_ui_desk::desk_board::panel(ui, state, session, look) {
        return col().size_full().child(panel);
    }
    let channel = state.workspace.channel(&session.channel);
    let items = humata2_board::session_panel(&state.workspace, session);
    let statuses = {
        let mut statuses = if full {
            vec![
                ItemStatus::Backlog,
                ItemStatus::Todo,
                ItemStatus::InProgress,
                ItemStatus::InReview,
                ItemStatus::Done,
            ]
        } else {
            // L14: beside the terminal, active lanes first and Done last,
            // leaving out lanes with nothing in them.
            ItemStatus::COMPACT[..5].to_vec()
        };
        let used = |status: &ItemStatus| items.iter().any(|item| item.status == *status);
        for status in ItemStatus::ALL {
            if !statuses.contains(&status) && used(&status) {
                statuses.push(status);
            }
        }
        if !full {
            statuses.retain(used);
        }
        statuses
    };
    col()
        .size_full()
        .child(panel_header(
            ui,
            state,
            Panel::Board,
            full,
            channel.map(|channel| secondary(look, channel.name.clone())),
            look,
        ))
        .child(
            div()
                .id("session-board")
                .flex_1()
                .min_h_0()
                .flex()
                .overflow_y_scroll()
                .when(full, |board| board.gap(px(10.)).p(px(14.)))
                .when(!full, |board| board.flex_col().gap(px(6.)).p(px(10.)))
                .children(statuses.into_iter().enumerate().map(|(index, status)| {
                    let lane: Vec<_> = items.iter().filter(|item| item.status == status).collect();
                    col()
                        .gap(px(if full { 8. } else { 6. }))
                        .text_size(px(12.5))
                        .when(full, |column| {
                            column
                                .flex_1()
                                .min_w_0()
                                .p(px(8.))
                                .rounded(px(9.))
                                .bg(rgb(look.panel))
                        })
                        .when(!full && index > 0, |column| column.pt(px(6.)))
                        .child(if full {
                            row()
                                .gap(px(6.))
                                .child(text(status.name()).font_weight(FontWeight::SEMIBOLD))
                                .child(secondary(look, lane.len().to_string()))
                                .child(div().flex_1())
                                .child(icon(IconName::Plus, look.tertiary, 12.))
                        } else {
                            row().child(
                                secondary(look, format!("{} · {}", status.name(), lane.len()))
                                    .font_weight(FontWeight::SEMIBOLD),
                            )
                        })
                        .children(
                            lane.into_iter()
                                .map(|item| board_item(ui, state, item, full, look)),
                        )
                })),
        )
}

pub fn board_item(
    ui: &Rc<Ui>,
    state: &AppState,
    item: &Item,
    full: bool,
    look: Look,
) -> impl IntoElement {
    let session = item
        .session
        .as_ref()
        .and_then(|id| state.workspace.session(id));
    let asking = state.board.request(item).map(|request| request.id.clone());
    let label = session.is_none().then(|| {
        state
            .workspace
            .item_details
            .get(&item.id)
            .and_then(|details| details.session_label.clone())
    });
    let label = label.flatten();
    let mut card = card(look)
        .id(SharedString::from(format!("board-item {}", item.id)))
        .role(Role::ListItem)
        .aria_label(format!("{} {}", item.id, item.title))
        .test_support()
        .px(px(if full { 9. } else { 10. }))
        .py(px(7.))
        .gap(px(6.))
        // Done fades only in the compact panel; the full board (frame 12)
        // keeps every column at full strength.
        .when(!full && item.status == ItemStatus::Done, |card| card.opacity(0.6))
        .when(asking.is_some(), |card| card.bg(rgb(look.attention_bg)));
    if full {
        card = card
            .p(px(9.))
            .child(
                row()
                    .gap(px(6.))
                    .child(secondary(look, item.id.to_string()))
                    .child(div().flex_1())
                    .children(match &item.scope {
                        Scope::Channel(id) | Scope::Session { channel: id, .. } => state
                            .workspace
                            .channel(id)
                            .map(|channel| secondary(look, channel.name.clone())),
                        Scope::Global => None,
                    }),
            )
            .child(text(item.title.clone()).font_weight(FontWeight::MEDIUM))
            .children(session.map(|session| {
                row()
                    .gap(px(5.))
                    .px(px(6.))
                    .py(px(1.))
                    .rounded(px(4.))
                    .bg(rgb(look.fill))
                    .when(item.status != ItemStatus::Done, |chip| {
                        chip.child(status(session.status))
                    })
                    .child(secondary(look, session.title.clone()))
            }))
            .children(label.map(|label| {
                row()
                    .px(px(6.))
                    .py(px(1.))
                    .rounded(px(4.))
                    .bg(rgb(look.fill))
                    .child(secondary(look, label))
            }))
            .children(asking.clone().map(|request| {
                // Review opens the request in the Inbox panel, where it's
                // answered.
                let ui = ui.clone();
                row()
                    .gap(px(6.))
                    .items_center()
                    .text_size(px(11.))
                    .text_color(rgb(ORANGE))
                    .child(icon(IconName::Inbox, ORANGE, 12.))
                    .child(
                        text("Asks to move to Done")
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap(),
                    )
                    .child(
                        widgets::push_quiet_shape(
                            SharedString::from(format!("board-review {}", item.id)),
                            "Review",
                            look,
                        )
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            ui.send(
                                AppAction::Session(SessionAction::SetPanel(Some(Panel::Inbox))),
                                cx,
                            );
                            ui.send(AppAction::Inbox(InboxAction::Select(request.clone())), cx);
                        }),
                    )
            }));
    } else {
        card = card.child(
            row()
                .gap(px(7.))
                .child(secondary(look, item.id.to_string()))
                .child(text(item.title.clone()).flex_1())
                .children(if asking.is_some() {
                    Some(status(Status::NeedsYou))
                } else {
                    session.map(|session| status(session.status))
                }),
        );
    }
    if let Some(session) = session {
        let ui = ui.clone();
        let id = session.id.clone();
        card = card
            .cursor_pointer()
            .on_click(move |_, _, cx| ui.send(AppAction::Navigate(Route::Session(id.clone())), cx));
    }
    card
}
