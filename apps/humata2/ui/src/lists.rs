//! Home's grouped lists and the wide Inbox, over the existing feature state.
use crate::{
    bridge::Ui,
    chrome, request_card,
    theme::Look,
    widgets::{self, col, row},
};
use gpui_kit::base::TestTextExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_clients::model::{Session, Status};
use humata2_inbox::{History, InboxAction};
use humata2_session::{Panel, SessionAction};
use std::rc::Rc;

pub(crate) fn toolbar(
    ui: &Rc<Ui>,
    state: &AppState,
    title: String,
    hidden: bool,
    look: Look,
) -> Div {
    row()
        .h(px(52.))
        .px(px(20.))
        .gap(px(10.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(look.line))
        .when(hidden, |bar| bar.child(chrome::lights()))
        .when(!state.trail.is_empty(), |bar| {
            bar.child(widgets::ghost(ui, "back", "‹ Back", AppAction::Back))
        })
        .child(
            div()
                .id("screen-title")
                .role(Role::Heading)
                .aria_label(title.clone())
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title)
                .test_support(),
        )
        .when(state.route == Route::Inbox, |bar| {
            bar.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(look.secondary))
                    .child(format!("{} waiting", state.workspace.waiting().count())),
            )
        })
        .child(div().flex_1())
        .when(state.route == Route::Home, |bar| {
            bar.child(
                widgets::primary(
                    ui,
                    "home-new-session",
                    "New session",
                    AppAction::StartNewSession,
                )
                .small()
                .h(px(24.))
                .bg(rgb(look.accent))
                .text_color(rgb(0xffffff)),
            )
        })
        .when(state.route == Route::Inbox, |bar| {
            bar.child(
                row().p(px(2.)).rounded(px(7.)).bg(rgb(look.fill)).children(
                    [
                        ("Waiting", History::Waiting, !state.inbox.show_answered),
                        ("Answered", History::Answered, state.inbox.answered_only),
                        (
                            "All",
                            History::All,
                            state.inbox.show_answered && !state.inbox.answered_only,
                        ),
                    ]
                    .into_iter()
                    .map(|(label, history, on)| {
                        let ui = ui.clone();
                        div()
                            .id(SharedString::from(format!("inbox-history-{label}")))
                            .role(Role::Tab)
                            .aria_label(label)
                            .aria_selected(on)
                            .px(px(12.))
                            .py(px(4.))
                            .text_size(px(12.))
                            .rounded(px(6.))
                            .when(on, |x| x.bg(rgb(look.card)).shadow_sm())
                            .on_click(move |_, _, cx| {
                                ui.send(AppAction::Inbox(InboxAction::SetHistory(history)), cx)
                            })
                            .child(label)
                            .test_support()
                    }),
                ),
            )
        })
}

fn home_row(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    group: &'static str,
    index: usize,
    look: Look,
) -> impl IntoElement {
    let ui2 = ui.clone();
    let id = session.id.clone();
    let channel = state
        .workspace
        .channel(&session.channel)
        .map_or("", |c| c.name.as_str());
    let description = if let Some(parent) = session
        .parent
        .as_ref()
        .and_then(|id| state.workspace.session(id))
    {
        format!("fork of {}", parent.title)
    } else {
        format!("# {channel}")
    };
    let pull = state
        .workspace
        .pulls
        .iter()
        .find(|p| p.session.as_ref() == Some(&session.id));
    let open_url = if group == "home-recent" && !session.title.contains('#') {
        pull.map(|p| format!("https://github.com/{}/pull/{}", p.repo, p.number))
    } else {
        None
    };
    let need = matches!(session.status, Status::NeedsYou | Status::Exited { .. });
    let label = if matches!(session.status, Status::Exited { .. }) {
        Some("Resume")
    } else if session.status == Status::NeedsYou {
        Some("Review")
    } else if open_url.is_some() {
        Some("Open PR")
    } else if group == "home-recent" {
        Some("Review")
    } else {
        None
    };
    row()
        .id((group, index))
        .role(Role::Link)
        .aria_label(session.title.clone())
        .gap(px(10.))
        .px(px(12.))
        .py(px(9.))
        .border_b_1()
        .border_color(rgb(look.line))
        .cursor_pointer()
        .child(chrome::status(session.status))
        .child(
            col()
                .flex_1()
                .min_w_0()
                .child(
                    row()
                        .gap(px(8.))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(session.title.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(look.secondary))
                                .child(description),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(if session.status == Status::NeedsYou {
                            0xff9f0a
                        } else {
                            look.secondary
                        }))
                        .child(session.activity.clone())
                        .test_text(session.activity.clone()),
                ),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(session.active.clone()),
        )
        .children(label.map(|label| {
            let ui = ui.clone();
            let url = open_url.clone();
            let session = session.id.clone();
            let exited = matches!(session_status(state, &session), Some(Status::Exited { .. }));
            Button::new(SharedString::from(format!("{group}-action-{index}")))
                .label(label)
                .small()
                .h(px(24.))
                .px(px(10.))
                .bg(rgb(if need && !exited {
                    look.accent
                } else {
                    look.fill
                }))
                .text_color(rgb(if need && !exited { 0xffffff } else { look.text }))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    if let Some(url) = &url {
                        cx.open_url(url);
                        return;
                    }
                    ui.send(AppAction::Navigate(Route::Session(session.clone())), cx);
                    if exited {
                        ui.send(AppAction::Session(SessionAction::Resume), cx);
                    } else if need {
                        ui.send(
                            AppAction::Session(SessionAction::SetPanel(Some(Panel::Inbox))),
                            cx,
                        );
                    }
                })
        }))
        .on_click(move |_, _, cx| ui2.send(AppAction::Navigate(Route::Session(id.clone())), cx))
        .test_support()
}
fn session_status(state: &AppState, id: &humata2_clients::model::SessionId) -> Option<Status> {
    state.workspace.session(id).map(|s| s.status)
}

pub(crate) fn home(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    let all = state.workspace.sessions.iter().filter(|s| !s.archived);
    let needs = all
        .clone()
        .filter(|s| matches!(s.status, Status::NeedsYou | Status::Exited { .. }))
        .collect::<Vec<_>>();
    let working = all
        .clone()
        .filter(|s| s.status == Status::Working)
        .collect::<Vec<_>>();
    let recent = all
        .filter(|s| {
            matches!(s.status, Status::Idle | Status::ProbablyIdle) && !s.activity.is_empty()
        })
        .collect::<Vec<_>>();
    col()
        .id("home")
        .size_full()
        .overflow_y_scroll()
        .p(px(24.))
        .items_center()
        .text_size(px(13.))
        .child(
            col().w_full().max_w(px(720.)).gap(px(20.)).children(
                [
                    (format!("Needs you · {}", needs.len()), "home-needs", needs),
                    (
                        format!("Working · {}", working.len()),
                        "home-working",
                        working,
                    ),
                    ("Recent".into(), "home-recent", recent),
                ]
                .into_iter()
                .map(|(title, group, sessions)| {
                    col()
                        .gap(px(6.))
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(look.secondary))
                                .child(title),
                        )
                        .child(
                            col()
                                .w_full()
                                .rounded(px(9.))
                                .border_1()
                                .border_color(rgb(look.line))
                                .bg(rgb(look.card))
                                .overflow_hidden()
                                .children(sessions.iter().enumerate().map(|(index, session)| {
                                    home_row(ui, state, session, group, index, look)
                                }))
                                .when(sessions.is_empty(), |x| {
                                    x.child(
                                        div()
                                            .p(px(12.))
                                            .text_color(rgb(look.secondary))
                                            .child("Nothing here right now."),
                                    )
                                }),
                        )
                }),
            ),
        )
}

pub(crate) fn inbox(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    let shown = state.inbox.shown();
    let selected = state
        .inbox
        .selection()
        .filter(|r| shown.iter().any(|v| v.id == r.id))
        .or_else(|| shown.first().copied());
    let list = col()
        .id("global-inbox-list")
        .w(px(330.))
        .max_w(relative(0.48))
        .flex_shrink_0()
        .h_full()
        .overflow_y_scroll()
        .border_r_1()
        .border_color(rgb(look.line))
        .children(state.workspace.channels.iter().filter_map(|channel| {
            let requests = shown
                .iter()
                .filter(|r| r.channel == channel.id)
                .copied()
                .collect::<Vec<_>>();
            if requests.is_empty() {
                return None;
            }
            Some(
                col()
                    .child(
                        div()
                            .px(px(14.))
                            .pt(px(10.))
                            .pb(px(4.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(11.))
                            .text_color(rgb(look.secondary))
                            .child(format!("# {}", channel.name)),
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
                            .opacity(if request.answer.is_some() { 0.6 } else { 1. })
                            .child(
                                row()
                                    .gap(px(6.))
                                    .when(session.is_some_and(|s| s.vip), |x| {
                                        x.child(
                                            div()
                                                .px(px(4.))
                                                .rounded(px(3.))
                                                .bg(rgb(0xbf5af2))
                                                .text_size(px(9.5))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(rgb(0xffffff))
                                                .child("VIP"),
                                        )
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(request.title.clone()),
                                    )
                                    .child(
                                        div()
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
                                    .child(session.map_or_else(String::new, |s| {
                                        format!(
                                            "{} · {}",
                                            if s.agent == humata2_clients::model::Agent::Claude {
                                                "Claude Code"
                                            } else {
                                                s.agent.name()
                                            },
                                            s.title
                                        )
                                    })),
                            )
                            .on_click(move |_, _, cx| {
                                ui.send(AppAction::Inbox(InboxAction::Select(id.clone())), cx)
                            })
                            .test_support()
                    })),
            )
        }));
    row()
        .size_full()
        .items_stretch()
        .text_size(px(12.5))
        .child(list)
        .child(
            col()
                .id("global-inbox-detail")
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .p(px(20.))
                .gap(px(12.))
                .children(selected.map(|r| {
                    div()
                        .w_full()
                        .max_w(px(560.))
                        .child(request_card::card(ui, state, r, true, look))
                }))
                .when(selected.is_none(), |x| {
                    x.child(widgets::empty(
                        "Nothing waiting.",
                        "New requests appear here.",
                    ))
                }),
        )
}
