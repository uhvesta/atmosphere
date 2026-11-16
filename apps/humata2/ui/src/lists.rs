//! Home's grouped lists and the wide Inbox, over the existing feature state.
use crate::{
    bridge::Ui,
    chrome, request_card,
    theme::Look,
    widgets::{self, col, row},
};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_clients::model::{Answer, MachineKind, Session, Status};
use humata2_inbox::{Bulk, History, InboxAction};
use humata2_session::{Panel, SessionAction};
use std::rc::Rc;

/// The frames' 24 pt button: filled with the accent, or grey with a hairline.
fn dress(button: Button, look: Look, primary: bool) -> Button {
    let button = button
        .small()
        .h(px(24.))
        .px(px(11.))
        .rounded(px(6.))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .flex_shrink_0();
    if primary {
        button.bg(rgb(look.accent)).text_color(rgb(0xffffff))
    } else {
        button
            .bg(rgb(look.fill))
            .text_color(rgb(look.text))
            .border_1()
            .border_color(rgb(look.line))
    }
}

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
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary))
                    .child(format!("{} waiting", state.workspace.waiting().count())),
            )
        })
        .child(div().flex_1())
        .when(
            state.route == Route::Home && !state.workspace.channels.is_empty(),
            |bar| {
                bar.child(dress(
                    widgets::primary(
                        ui,
                        "home-new-session",
                        "New session",
                        AppAction::StartNewSession,
                    ),
                    look,
                    true,
                ))
            },
        )
        .when(state.route == Route::Inbox, |bar| {
            bar.child(
                row().p(px(2.)).rounded(px(7.)).bg(rgb(look.fill)).flex_shrink_0().children(
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
                            .py(px(2.))
                            .text_size(px(11.5))
                            .rounded(px(5.))
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
        // HM-1: a row names its machine when that isn't This Mac.
        match state.workspace.machine(&session.machine) {
            Some(machine) if !matches!(machine.kind, MachineKind::ThisMac) => {
                format!("# {channel} · {}", machine.name)
            }
            _ => format!("# {channel}"),
        }
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
                    div()
                        .flex()
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
            dress(
                Button::new(SharedString::from(format!("{group}-action-{index}"))).label(label),
                look,
                need && !exited,
            )
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

fn card(look: Look) -> Div {
    col()
        .w_full()
        .rounded(px(9.))
        .border_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.card))
}

/// A titled group on Home: its small heading over `body`.
fn group(look: Look, title: String, body: Div) -> Div {
    col()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(look.secondary))
                .child(title),
        )
        .child(body)
}

/// A group of session rows, or `none` in the card when there is no session.
fn session_group(
    ui: &Rc<Ui>,
    state: &AppState,
    title: String,
    id: &'static str,
    sessions: &[&Session],
    none: &'static str,
    look: Look,
) -> Div {
    let body = if sessions.is_empty() {
        card(look)
            .px(px(14.))
            .py(px(12.))
            .text_color(rgb(look.tertiary))
            .child(none)
    } else {
        card(look).overflow_hidden().children(
            sessions
                .iter()
                .enumerate()
                .map(|(index, session)| home_row(ui, state, session, id, index, look)),
        )
    };
    group(look, title, body)
}

/// One row of Get set up: what to do, why, and the button that starts it.
fn step(
    look: Look,
    glyph: IconName,
    title: &'static str,
    note: &'static str,
    action: Button,
) -> Div {
    row()
        .gap(px(12.))
        .px(px(14.))
        .py(px(12.))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(chrome::icon(glyph, look.accent, 15.))
        .child(
            col()
                .flex_1()
                .min_w_0()
                .child(div().font_weight(FontWeight::MEDIUM).child(title))
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(look.secondary))
                        .child(note),
                ),
        )
        .child(action)
}

/// What Home offers when there is no channel yet (DK-6, frame 34).
fn get_set_up(ui: &Rc<Ui>, look: Look) -> Div {
    group(
        look,
        "Get set up".to_owned(),
        card(look)
            .overflow_hidden()
            .child(step(
                look,
                IconName::Plus,
                "Make a channel",
                "A project: its repos, the machines that work on it, its board and its sessions.",
                dress(
                    widgets::primary(
                        ui,
                        "first-channel-create",
                        "New channel…",
                        AppAction::StartNewChannel,
                    ),
                    look,
                    true,
                ),
            ))
            .child(step(
                look,
                IconName::Server,
                "Add a machine",
                "Run sessions on another computer over SSH. This Mac works already.",
                dress(
                    widgets::send(
                        ui,
                        "first-machine-add",
                        "Add a machine…",
                        AppAction::Hosts(humata2_hosts::HostsAction::StartAdd),
                    ),
                    look,
                    false,
                ),
            )),
    )
}

fn counted(name: &str, count: usize) -> String {
    if count == 0 {
        name.to_owned()
    } else {
        format!("{name} · {count}")
    }
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
    let groups = col()
        .w_full()
        .max_w(px(720.))
        .gap(px(20.))
        // After a restart (frame 47): what couldn't be resumed and what came back idle.
        .children(crate::restart_report::home_groups(ui, state, look))
        .child(session_group(
            ui,
            state,
            counted("Needs you", needs.len()),
            "home-needs",
            &needs,
            "Nothing needs you.",
            look,
        ))
        .child(session_group(
            ui,
            state,
            counted("Working", working.len()),
            "home-working",
            &working,
            "No sessions are running.",
            look,
        ));
    let groups = if state.workspace.channels.is_empty() {
        groups.child(get_set_up(ui, look)).child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child("GitHub access comes from the gh CLI. There is no sign-in in Humata."),
        )
    } else {
        groups.child(session_group(
            ui,
            state,
            "Recent".to_owned(),
            "home-recent",
            &recent,
            "Nothing recent yet.",
            look,
        ))
    };
    col()
        .id("home")
        .size_full()
        .overflow_y_scroll()
        .p(px(24.))
        .items_center()
        .text_size(px(13.))
        .child(groups)
}

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
            .child(dress(
                widgets::send(
                    ui,
                    "inbox-allow-all",
                    format!("Allow all {waiting}"),
                    AppAction::Inbox(InboxAction::Bulk(Bulk::ApproveAll)),
                ),
                look,
                false,
            ))
            .child(dress(
                widgets::send(
                    ui,
                    "inbox-deny-all",
                    "Deny all",
                    AppAction::Inbox(InboxAction::Bulk(Bulk::DenyAll)),
                ),
                look,
                false,
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
    .child(dress(
        widgets::primary(
            ui,
            "inbox-bulk-confirm",
            yes,
            AppAction::Inbox(InboxAction::ConfirmBulk),
        ),
        look,
        true,
    ))
    .child(dress(
        widgets::send(
            ui,
            "inbox-bulk-cancel",
            "Cancel",
            AppAction::Inbox(InboxAction::CancelBulk),
        ),
        look,
        false,
    ))
}

/// A row's second line: who asks and from which session, then how it was
/// answered once it is (IN-7).
fn asked_from(session: Option<&Session>, answer: Option<&Answer>) -> String {
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
        Some(Answer::Denied { reply: None }) => parts.push("denied".to_owned()),
        Some(Answer::Denied { reply: Some(note) }) => parts.push(format!("denied: {note}")),
        Some(Answer::Replied { text }) => parts.push(format!("replied: {text}")),
    }
    parts.join(" · ")
}

pub(crate) fn inbox(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    let shown = state.inbox.shown();
    let selected = state
        .inbox
        .selection()
        .filter(|r| shown.iter().any(|v| v.id == r.id))
        .or_else(|| shown.first().copied());
    let waiting = shown.iter().filter(|r| r.answer.is_none()).count();
    let strip = (waiting > 1 || state.inbox.confirm.is_some())
        .then(|| bulk_strip(ui, state.inbox.confirm, waiting, look));
    let list = col()
        .id("global-inbox-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
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
                            .opacity(if request.answer.is_some() { 0.55 } else { 1. })
                            .child(
                                row()
                                    .gap(px(6.))
                                    .when(
                                        // The VIP mark asks for attention: answered rows don't carry it.
                                        request.answer.is_none() && session.is_some_and(|s| s.vip),
                                        |x| {
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
                                    .child(asked_from(session, request.answer.as_ref())),
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
