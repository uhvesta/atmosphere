//! The session's Inbox panel.
use crate::panel_prelude::*;

pub fn inbox(ui: &Rc<Ui>, state: &AppState, full: bool, look: Look) -> Div {
    let family = family(state);
    let requests: Vec<_> = state
        .workspace
        .requests
        .iter()
        .filter(|request| family.contains(&request.session) && request.answer.is_none())
        .collect();
    col()
        .size_full()
        .child(panel_header(
            ui,
            state,
            Panel::Inbox,
            full,
            Some(secondary(look, format!("this session · {} waiting on you", requests.len()))),
            look,
        ))
        .child(
            col()
                .id("session-inbox")
                .test_support()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .gap(px(8.))
                .p(px(10.))
                .when(requests.is_empty(), |body| {
                    body.child(secondary(look, "Nothing waiting for you"))
                })
                // The card every Inbox draws, at panel width.
                .children(requests.iter().map(|request| {
                    crate::request_card::card(ui, state, request, false, look)
                }))
                .child(secondary(
                    look,
                    "Only this session and its forks are listed here. Everything waiting on you is in the Agent inbox.",
                ))
                .children(state.inbox.error.as_ref().map(|error| {
                    text(error.clone())
                        .id("panel-error")
                        .role(Role::Alert)
                        .aria_label("Answer failed")
                        .text_color(rgb(RED))
                        .test_support()
                })),
        )
}

