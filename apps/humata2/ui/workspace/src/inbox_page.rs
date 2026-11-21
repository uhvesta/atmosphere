//! The Inbox page view (shared by the global Inbox route and channel Inbox tab).

use std::rc::Rc;

use gpui_kit::base::Disableable as _;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::model::{Request, Status};
use humata2_inbox::InboxAction;

use humata2_ui_core::bridge::{DETAIL_GAP, InboxRoom, Measured, Ui};
use crate::request_card;
use humata2_ui_core::theme::Look;
use humata2_ui_core::widgets::{col, row};

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

/// A channel chip over the cards: its name and how many of its requests wait.
fn chip(ui: &Rc<Ui>, id: String, name: String, count: usize, on: bool, filter: humata2_inbox::Filter, look: Look) -> impl IntoElement {
    let ui = ui.clone();
    let dark = look.text > look.content;
    let label = if count > 0 { format!("{name} {count}") } else { name.clone() };
    row()
        .id(SharedString::from(id))
        .role(Role::Button)
        .aria_label(label)
        .aria_selected(on)
        .test_support()
        .flex_shrink_0()
        .whitespace_nowrap()
        .px(px(9.))
        .py(px(3.))
        .rounded(px(6.))
        .text_size(px(12.5))
        .cursor_pointer()
        .when(!on, |chip| chip.bg(rgb(look.fill)).text_color(rgb(if dark { 0xc4c4ca } else { 0x55555b })))
        .when(on, |chip| chip.bg(rgb(look.text)).text_color(rgb(look.content)).font_weight(FontWeight::MEDIUM))
        .child(name)
        .when(count > 0, |chip| {
            chip.child(div().ml(px(4.)).opacity(0.65).font_weight(FontWeight::NORMAL).child(count.to_string()))
        })
        .on_click(move |_, _, cx| ui.send(AppAction::Inbox(InboxAction::SetFilter(filter.clone())), cx))
}

/// The empty page (frame 200).
fn empty(ui: &Rc<Ui>, state: &AppState, look: Look) -> Div {
    let working = state
        .workspace
        .sessions
        .iter()
        .filter(|session| session.status == Status::Working)
        .count();
    let answered = state.workspace.requests.iter().filter(|request| request.answer.is_some()).count();
    // To-dos an agent suggested or asked about: they wait in To-do for you.
    let attention = state
        .todo
        .items
        .iter()
        .filter(|entry| entry.todo.closed.is_none() && (entry.todo.suggested || entry.todo.ask.is_some()))
        .count();
    let history = state.inbox.show_answered;
    let teal = if look.text > look.content { 0x35c4b5 } else { 0x0f766e };
    let link = |ui: &Rc<Ui>, id: &'static str, label: &'static str, action: AppAction| {
        let ui = ui.clone();
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .cursor_pointer()
            .text_color(rgb(look.accent))
            .child(label)
            .on_click(move |_, _, cx| ui.send(action.clone(), cx))
            .test_support()
    };
    col()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .pb(px(80.))
        .child(
            row()
                .size(px(46.))
                .rounded_full()
                .bg(rgb(if look.text > look.content { 0x153a36 } else { 0xdff5f1 }))
                .text_color(rgb(teal))
                .text_size(px(22.))
                .justify_center()
                .child("✓"),
        )
        .child(
            div()
                .id("inbox-empty-title")
                .role(Role::Heading)
                .aria_label(if history { "Nothing answered yet" } else { "Nothing is waiting on you" })
                .mt(px(6.))
                .text_size(px(18.))
                .font_weight(FontWeight::BOLD)
                .child(if history { "Nothing answered yet" } else { "Nothing is waiting on you" })
                .test_support(),
        )
        .child(
            div()
                .max_w(px(520.))
                .text_center()
                .text_color(rgb(look.secondary))
                .line_height(relative(1.5))
                .child(format!(
                    "{working} {} working. An agent stops and asks here before it moves or copies files or sessions, needs new access, or has a question for you.",
                    if working == 1 { "agent is" } else { "agents are" }
                )),
        )
        .child(
            row()
                .mt(px(10.))
                .gap(px(4.))
                .text_color(rgb(look.secondary))
                .child(humata2_inbox::today_summary(
                    answered,
                    state.workspace.allowed_without_asking_today,
                ))
                .child(link(ui, "inbox-empty-history", if history { "Back to waiting" } else { "History" }, AppAction::Inbox(InboxAction::ToggleHistory))),
        )
        .child(
            row()
                .gap(px(4.))
                .text_color(rgb(look.secondary))
                .child(if attention == 0 {
                    "Changes to your to-dos wait in To-do ·".to_owned()
                } else {
                    format!("{attention} to-dos need attention when you get to them ·")
                })
                .child(link(ui, "inbox-empty-todo", "Open To-do", AppAction::Navigate(humata2_app::Route::Todo))),
        )
}

pub fn inbox(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    let shown = state.inbox.shown();
    let cli_logins = crate::cli_login::inbox_cards(ui, state, look);
    if shown.is_empty() && cli_logins.is_none() {
        return empty(ui, state, look).into_any_element();
    }
    let total = state.inbox.chips().iter().map(|(_, count)| count).sum::<usize>();
    let chips = state.inbox.chips();
    let all_on = state.inbox.filter == humata2_inbox::Filter::All;
    let bar = row()
        .id("inbox-chips")
        .gap(px(6.))
        .items_center()
        .pt(px(12.))
        .pb(px(4.))
        .flex_shrink_0()
        .child(chip(ui, "chip all".to_owned(), "All".to_owned(), total, all_on, humata2_inbox::Filter::All, look))
        .children(chips.into_iter().map(|(id, count)| {
            let name = if id == humata2_clients::desk::channel() {
                "Desk".to_owned()
            } else {
                state.workspace.channel(&id).map_or_else(|| id.to_string(), |channel| channel.name.clone())
            };
            let on = state.inbox.filter == humata2_inbox::Filter::Channel(id.clone());
            chip(ui, format!("chip {id}"), name, count, on, humata2_inbox::Filter::Channel(id), look).into_any_element()
        }))
        .child(div().flex_1().min_w_0())
        .child(
            div()
                .text_size(px(12.))
                .text_color(rgb(look.secondary))
                .child("Agents that are stopped until you answer. Changes to your to-dos wait in To-do instead."),
        );
    col()
        .id("global-inbox-list")
        .size_full()
        .px(px(28.))
        .text_size(px(12.5))
        .child(bar)
        .child(
            col()
                .id("global-inbox-detail")
                .test_support()
                .relative()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .py(px(12.))
                .gap(px(14.))
                .child(InboxRoom::measure(ui, Measured::Pane))
                .children(cli_logins)
                .children(shown.into_iter().enumerate().map(|(index, request): (usize, &Request)| {
                    let card = div()
                        .relative()
                        .w_full()
                        .child(request_card::card(ui, state, request, true, look))
                        .when(index == 0, |card| card.child(InboxRoom::measure(ui, Measured::Card)));
                    col()
                        .gap(px(DETAIL_GAP))
                        .child(card)
                        .children(
                            (request.on_screen.is_some() && request.answer.is_none() && index == 0)
                                .then(|| screen_question(ui, state, request, look)),
                        )
                        .into_any_element()
                })),
        )
        .into_any_element()
}
