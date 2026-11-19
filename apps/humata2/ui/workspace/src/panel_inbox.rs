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
    // One request is open at a time, the picked one or else the first; the
    // rest fold to a line that opens them.
    let selected = state
        .inbox
        .selected
        .as_ref()
        .filter(|id| requests.iter().any(|request| request.id == **id))
        .or_else(|| requests.first().map(|request| &request.id));
    col()
        .size_full()
        .child(panel_header(
            ui,
            state,
            Panel::Inbox,
            full,
            Some(secondary(look, "this session and its forks")),
            look,
        ))
        .child(
            col()
                .id("session-inbox")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .gap(px(8.))
                .p(px(10.))
                .when(requests.is_empty(), |body| {
                    body.child(secondary(look, "Nothing waiting for you"))
                })
                // The card every Inbox draws, narrow: it leaves out what the
                // agent was doing, since its terminal is beside it (IN-9).
                .children(requests.iter().map(|request| {
                    if selected == Some(&request.id) {
                        crate::request_card::card(ui, state, request, false, look)
                            .into_any_element()
                    } else {
                        folded(ui, state, request, look).into_any_element()
                    }
                }))
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

/// A waiting request that isn't open: its title and age; a click opens it.
fn folded(ui: &Rc<Ui>, state: &AppState, request: &Request, look: Look) -> impl IntoElement {
    let ui = ui.clone();
    let id = request.id.clone();
    let by = crate::request_card::requester(state, request, false);
    col()
        .id(SharedString::from(format!("request {}", request.id)))
        .role(Role::Button)
        .aria_label(request.title.clone())
        .test_support()
        .gap(px(4.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .px(px(12.))
        .py(px(8.))
        .text_size(px(12.5))
        .cursor_pointer()
        .on_click(move |_, _, cx| ui.send(AppAction::Inbox(InboxAction::Select(id.clone())), cx))
        .child(
            row()
                .gap(px(6.))
                .child(status(Status::NeedsYou))
                .child(text(request.title.clone()).flex_1())
                .child(secondary(look, request.asked.clone())),
        )
        .when(!by.is_empty(), |c| c.child(secondary(look, by)))
}
