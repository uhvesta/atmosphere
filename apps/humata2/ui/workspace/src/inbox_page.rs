//! The Inbox page view (shared by the global Inbox route and channel Inbox tab).

use std::rc::Rc;

use gpui_kit::base::Disableable as _;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::model::{Answer, Request, Session};
use humata2_inbox::{Bulk, Group, InboxAction, MigrateStatus};

use humata2_ui_core::bridge::{DETAIL_GAP, DETAIL_PAD, InboxRoom, Measured, Ui};
use crate::request_card;
use humata2_ui_core::theme::Look;
use humata2_ui_core::widgets::{self, col, row};

/// Allow all n and Deny all over the list (IN-6), or the question Allow all
/// asks first when a waiting request can't be undone.
fn bulk_strip(ui: &Rc<Ui>, confirm: Option<Bulk>, waiting: usize, look: Look) -> Div {
    let bar = row()
        .flex_wrap()
        .gap(px(8.))
        .px(px(14.))
        .py(px(10.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(look.line));
    let Some(bulk) = confirm else {
        return bar
            .child(widgets::push_quiet(
                ui,
                "inbox-allow-all",
                format!("Allow all {waiting}"),
                AppAction::Inbox(InboxAction::Bulk(Bulk::ApproveAll)),
                look,
            ))
            .child(widgets::push_quiet(
                ui,
                "inbox-deny-all",
                "Deny all",
                AppAction::Inbox(InboxAction::Bulk(Bulk::DenyAll)),
                look,
            ));
    };
    let (question, yes) = match bulk {
        Bulk::ApproveAll => (
            format!("Allow all {waiting}? Some of them can't be undone once allowed."),
            "Allow all",
        ),
        Bulk::DenyAll => (format!("Deny all {waiting}?"), "Deny all"),
    };
    bar.child(
        div()
            .w_full()
            .text_size(px(11.5))
            .text_color(rgb(look.secondary))
            .child(question),
    )
    .child(widgets::push_primary(
        ui,
        "inbox-bulk-confirm",
        yes,
        AppAction::Inbox(InboxAction::ConfirmBulk),
        look,
    ))
    .child(widgets::push_quiet(
        ui,
        "inbox-bulk-cancel",
        "Cancel",
        AppAction::Inbox(InboxAction::CancelBulk),
        look,
    ))
}

/// A list row's second line: a Migrate item is from Humata, and says where it
/// stands; any other request names who asks and how it was answered.
fn row_from(state: &AppState, request: &Request, session: Option<&Session>) -> String {
    let Some(card) = request.migrate() else {
        return asked_from(session, request.answer.as_ref(), request.on_screen.is_some());
    };
    let status = match state.inbox.migrate_status(&request.id) {
        Some(MigrateStatus::Migrating) => "migrating…".to_owned(),
        Some(MigrateStatus::Failed(_)) => "couldn't migrate".to_owned(),
        Some(MigrateStatus::Migrated { .. }) => "migrated".to_owned(),
        _ => state
            .workspace
            .machine(&card.machine)
            .map_or_else(|| card.machine.to_string(), |machine| machine.name.clone()),
    };
    format!("Humata · {status}")
}

/// A row's second line: who asks and from which session, then how it was
/// answered once it is (IN-7). `on_screen`: a question on its agent's
/// screen, whose Deny is Dismiss and leaves the question there (D781).
fn asked_from(session: Option<&Session>, answer: Option<&Answer>, on_screen: bool) -> String {
    let mut parts = Vec::new();
    if let Some(session) = session {
        parts.push(
            if session.agent == humata2_clients::model::Agent::Claude {
                "Claude Code"
            } else {
                session.agent.name()
            }
            .to_owned(),
        );
        parts.push(session.title.clone());
    }
    match answer {
        None => {}
        Some(Answer::Approved) => parts.push("approved".to_owned()),
        Some(Answer::Always { .. }) => parts.push("always allowed".to_owned()),
        Some(Answer::Denied { reply: None }) if on_screen => {
            parts.push("dismissed; the question is still on its screen".to_owned())
        }
        Some(Answer::Denied { reply: None }) => parts.push("denied".to_owned()),
        Some(Answer::Denied { reply: Some(note) }) => parts.push(format!("denied: {note}")),
        Some(Answer::Replied { text }) => parts.push(format!("replied: {text}")),
    }
    parts.join(" · ")
}

/// Under a question on its agent's screen: the live terminal at full width,
/// then the hint and Dismiss (D781). The terminal is the one the Inbox links
/// for the open item; until it is made, the hint shows alone. The hint sends
/// you to the terminal only while it shows the screen.
fn screen_question(ui: &Rc<Ui>, state: &AppState, request: &Request, look: Look) -> AnyElement {
    let pending = state.inbox.pending.contains(&request.id);
    let hint = if ui.inbox_room().screen() {
        "Answer in the terminal or pick a choice above. Dismiss closes this item and presses nothing."
    } else {
        "Pick a choice above. Dismiss closes this item and presses nothing."
    };
    col()
        .w_full()
        .gap(px(DETAIL_GAP))
        .children(ui.inbox_terminal())
        .child(
            row()
                .id("inbox-screen-hint")
                .role(Role::Group)
                .aria_label(hint)
                .test_support()
                .relative()
                .gap(px(12.))
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(11.5))
                        .text_color(rgb(look.secondary))
                        .child(hint),
                )
                .child(
                    request_card::button(
                        ui,
                        SharedString::from(format!("dismiss {}", request.id)),
                        if pending { "Dismissing…" } else { "Dismiss" },
                        vec![AppAction::Inbox(InboxAction::Dismiss(request.id.clone()))],
                        look,
                        false,
                    )
                    .disabled(pending),
                )
                .child(InboxRoom::measure(ui, Measured::Hint)),
        )
        .into_any_element()
}

pub fn inbox(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    let shown = state.inbox.shown();
    let selected = state
        .inbox
        .selection()
        .filter(|r| shown.iter().any(|v| v.id == r.id))
        .or_else(|| shown.first().copied());
    // Allow all and Deny all never answer a Migrate item.
    let waiting = state.inbox.bulk_count();
    let strip = (waiting > 1 || state.inbox.confirm.is_some())
        .then(|| bulk_strip(ui, state.inbox.confirm, waiting, look));
    let list = col()
        .id("global-inbox-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .children(state.inbox.groups().into_iter().map(|(group, requests)| {
            let heading = match &group {
                Group::Humata => "Humata".to_owned(),
                Group::Channel(id) if *id == humata2_clients::desk::channel() => "Desk".to_owned(),
                Group::Channel(id) => state
                    .workspace
                    .channel(id)
                    .map_or_else(|| format!("# {id}"), |channel| format!("# {}", channel.name)),
            };
            {
                col()
                    .child(
                        div()
                            .px(px(14.))
                            .pt(px(10.))
                            .pb(px(4.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(11.))
                            .text_color(rgb(look.secondary))
                            .child(heading),
                    )
                    .children(requests.into_iter().map(|request| {
                        let session = state.workspace.session(&request.session);
                        let on = selected.is_some_and(|r| r.id == request.id);
                        let ui = ui.clone();
                        let id = request.id.clone();
                        col()
                            .id(SharedString::from(format!("global-request {}", id)))
                            .role(Role::ListBoxOption)
                            .aria_label(request.title.clone())
                            .aria_selected(on)
                            .px(px(14.))
                            .py(px(9.))
                            .gap(px(2.))
                            .border_b_1()
                            .border_color(rgb(look.line))
                            .when(on, |x| x.bg(rgb(look.selection)))
                            .text_color(rgb(if on { 0xffffff } else { look.text }))
                            .opacity(if request.answer.is_some() { 0.55 } else { 1. })
                            .child(
                                row()
                                    .gap(px(6.))
                                    // The VIP mark asks for attention: answered rows don't carry it.
                                    .when(state.inbox.vip(request), |x| {
                                        x.child(request_card::vip_badge())
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(request.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_size(px(11.))
                                            .text_color(rgb(if on {
                                                0xffffff
                                            } else {
                                                look.secondary
                                            }))
                                            .child(request.asked.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(if on { 0xffffff } else { look.secondary }))
                                    .child(row_from(state, request, session)),
                            )
                            .on_click(move |_, _, cx| {
                                ui.send(AppAction::Inbox(InboxAction::Select(id.clone())), cx)
                            })
                            .test_support()
                    }))
            }
        }));
    row()
        .size_full()
        .items_stretch()
        .text_size(px(12.5))
        .child(
            col()
                .w(px(330.))
                .max_w(relative(0.48))
                .flex_shrink_0()
                .h_full()
                .border_r_1()
                .border_color(rgb(look.line))
                .children(strip)
                .child(list),
        )
        .child(
            col()
                .id("global-inbox-detail")
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .p(px(DETAIL_PAD))
                .gap(px(DETAIL_GAP))
                .child(InboxRoom::measure(ui, Measured::Pane))
                .children(selected.map(|r| {
                    div()
                        .relative()
                        .w_full()
                        .max_w(px(560.))
                        .child(request_card::card(ui, state, r, true, look))
                        .child(InboxRoom::measure(ui, Measured::Card))
                }))
                .children(
                    selected
                        .filter(|r| r.on_screen.is_some() && r.answer.is_none())
                        .map(|r| screen_question(ui, state, r, look)),
                )
                .when(selected.is_none(), |x| {
                    x.child(widgets::empty_in(
                        look,
                        "Nothing waiting.",
                        "New requests appear here.",
                    ))
                }),
        )
}
