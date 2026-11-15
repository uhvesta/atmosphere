//! The screens in the middle of the window, one per route.

use std::rc::Rc;

use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_clients::model::{Session, Speaker, Status};

use crate::bridge::Ui;
use crate::theme::{self, color};
use crate::widgets::{self, col, row};

/// The screen's title, for the top bar.
#[must_use]
pub fn title(state: &AppState) -> String {
    match &state.route {
        Route::Home => "Home".to_owned(),
        Route::Inbox => "Inbox".to_owned(),
        Route::Board(_) => "Board".to_owned(),
        Route::Channel(id) => state
            .workspace
            .channels
            .iter()
            .find(|channel| &channel.id == id)
            .map_or_else(|| "Channel".to_owned(), |channel| format!("#{}", channel.name)),
        Route::Session(id) => state
            .workspace
            .sessions
            .iter()
            .find(|session| &session.id == id)
            .map_or_else(|| "Session".to_owned(), |session| session.title.clone()),
        Route::Hosts => "Hosts".to_owned(),
        Route::Files => "Files".to_owned(),
        Route::Views => "Views".to_owned(),
        Route::Settings => "Settings".to_owned(),
    }
}

/// The screen for the current route.
pub fn page(ui: &Rc<Ui>, state: &AppState, _window: &mut Window, _cx: &mut App) -> AnyElement {
    match &state.route {
        Route::Home => home(ui, state).into_any_element(),
        Route::Session(id) => state
            .workspace
            .sessions
            .iter()
            .find(|session| &session.id == id)
            .map_or_else(
                || widgets::empty("This session is gone.", "Pick another in the sidebar.").into_any_element(),
                |session| terminal(session).into_any_element(),
            ),
        _ => widgets::empty("Nothing here yet.", "This screen is being built.").into_any_element(),
    }
}

fn session_row(ui: &Rc<Ui>, session: &Session, index: usize, group: &'static str) -> impl IntoElement {
    let ui = ui.clone();
    let id = session.id.clone();
    widgets::list_item((group, index), Role::Link, session.title.clone(), false)
        .h(px(34.))
        .child(widgets::dot(session.status).id((group, index + 10_000)).tooltip({
            let words = session.status.label();
            move |window, cx| gpui_kit::component::tooltip::Tooltip::new(words.clone()).build(window, cx)
        }))
        .child(div().flex_1().child(session.title.clone()))
        .child(widgets::faint(session.active.clone()))
        .on_click(move |_, _, cx| ui.send(AppAction::Navigate(Route::Session(id.clone())), cx))
        .test_support()
}

pub(crate) fn home(ui: &Rc<Ui>, state: &AppState) -> impl IntoElement {
    let needs: Vec<&Session> = state
        .workspace
        .sessions
        .iter()
        .filter(|session| matches!(session.status, Status::NeedsYou) && !session.archived)
        .collect();
    let working: Vec<&Session> = state
        .workspace
        .sessions
        .iter()
        .filter(|session| matches!(session.status, Status::Working) && !session.archived)
        .collect();
    let section = |heading: &'static str, group: &'static str, sessions: Vec<&Session>, none: &'static str| {
        col()
            .gap_1()
            .child(widgets::heading(heading))
            .children(sessions.iter().enumerate().map(|(index, session)| session_row(ui, session, index, group)))
            .when(sessions.is_empty(), |list| list.child(widgets::dim(none)))
    };
    col()
        .id("home")
        .size_full()
        .overflow_y_scroll()
        .p_6()
        .child(
            col()
                .w_full()
                .max_w(px(880.))
                .mx_auto()
                .gap_6()
                .child(section("Needs you", "home-needs", needs, "Nothing is waiting on you."))
                .child(section("Working", "home-working", working, "No agent is working right now.")),
        )
}

pub(crate) fn terminal(session: &Session) -> impl IntoElement {
    col()
        .id("terminal")
        .role(Role::Log)
        .aria_label(format!("{} terminal", session.title))
        .size_full()
        .overflow_y_scroll()
        .p_3()
        .gap_1()
        .bg(color(theme::TERM))
        .font_family("Menlo")
        .text_size(px(12.))
        .children(session.transcript.iter().map(|line| {
            row().gap_2().child(
                div()
                    .text_color(color(match line.from {
                        Speaker::You => theme::ACCENT,
                        _ => theme::TEXT,
                    }))
                    .child(line.text.clone()),
            )
        }))
        .test_support()
}
